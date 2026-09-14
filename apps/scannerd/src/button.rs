//! The button that says a letter is finished.
//!
//! On the Pi, a push button between a GPIO pin and ground, made into a key by
//! the kernel. One line in `/boot/firmware/config.txt`,
//!
//! ```text
//! dtoverlay=gpio-key,gpio=17,active_low=1,gpio_pull=up,keycode=28,label=scannerd
//! ```
//!
//! turns a press into Enter on an input device, debounced by the `gpio-keys`
//! driver. So there is no GPIO code here, no library, and no version of
//! libgpiod to match: reading the button is reading fixed-size records from
//! `/dev/input/…`, and a USB keypad or any keyboard works exactly the same
//! way. On a laptop, standard input stands in — press Enter.

use std::fs::File;
use std::io::{BufRead, Read};
use std::path::Path;
use std::sync::mpsc::{self, Receiver, Sender};

use anyhow::{Context, Result};

/// `struct input_event` on 64-bit Linux: a 16-byte `timeval`, then type
/// (`u16`), code (`u16`) and value (`i32`), little-endian on a Pi.
pub const EVENT_SIZE: usize = 24;
const EV_KEY: u16 = 1;
/// `KEY_ENTER`, which the overlay line above sends.
pub const KEY_ENTER: u16 = 28;

/// How many presses of `key` a run of input events holds.
///
/// Releases (value 0) and auto-repeat (value 2) are not presses: a button
/// held down is one press, not a stream of letters closing.
pub fn presses(events: &[u8], key: u16) -> usize {
    events
        .chunks_exact(EVENT_SIZE)
        .filter(|event| {
            let kind = u16::from_le_bytes([event[16], event[17]]);
            let code = u16::from_le_bytes([event[18], event[19]]);
            let value = i32::from_le_bytes([event[20], event[21], event[22], event[23]]);
            kind == EV_KEY && code == key && value == 1
        })
        .count()
}

pub struct Button {
    presses: Receiver<()>,
}

impl Button {
    /// A button pressed whenever something is sent — for tests, and for
    /// anything that is not a device.
    pub fn channel() -> (Sender<()>, Self) {
        let (sender, presses) = mpsc::channel();
        (sender, Self { presses })
    }

    /// A key on an input device, such as the one the `gpio-key` overlay makes
    /// (`ls /dev/input/by-path/` to find it).
    pub fn input_device(path: &Path, key: u16) -> Result<Self> {
        let mut device = File::open(path).with_context(|| {
            format!(
                "could not open the button at {} (is scannerd in the `input` group?)",
                path.display()
            )
        })?;
        let (sender, button) = Self::channel();
        std::thread::spawn(move || {
            let mut event = [0u8; EVENT_SIZE];
            // `read_exact`: the kernel hands out whole events, but a short read
            // must not misalign every event after it.
            while device.read_exact(&mut event).is_ok() {
                for _ in 0..presses(&event, key) {
                    if sender.send(()).is_err() {
                        return;
                    }
                }
            }
            tracing::error!("the button stopped reporting; letters will close when left alone");
        });
        Ok(button)
    }

    /// Enter on standard input, for trying it without a Pi.
    pub fn stdin() -> Self {
        let (sender, button) = Self::channel();
        std::thread::spawn(move || {
            for line in std::io::stdin().lock().lines() {
                if line.is_err() || sender.send(()).is_err() {
                    return;
                }
            }
        });
        button
    }

    /// Whether it has been pressed since the last time anyone asked.
    ///
    /// Several presses in between count as one. A double tap closes the
    /// letter; it must not also close the next one, which has no pages yet,
    /// the moment its first page arrives.
    pub fn pressed(&self) -> bool {
        let mut any = false;
        while self.presses.try_recv().is_ok() {
            any = true;
        }
        any
    }
}
