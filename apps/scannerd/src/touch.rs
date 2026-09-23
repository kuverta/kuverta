//! The LCD's touchscreen, read by asking it.
//!
//! The 3.5″ panel on the rig carries an XPT2046 resistive touch controller on
//! SPI chip select 1, and the `piscreen` overlay binds the kernel's `ads7846`
//! driver to it. That driver waits for the controller's pen interrupt before it
//! reads anything — and on this board the interrupt never arrives: pressed
//! for forty seconds, GPIO 17 did not move, nor did any other pin. So scannerd
//! asks instead, thirty times a second, over `/dev/spidev0.1` (the kernel
//! driver kept off it; see the readme).
//!
//! It asks for two things: how hard the film is pressed, and how far down.
//! Only how far down, because on this panel that is all there is: the
//! controller's X channel runs from about 560 at the top of the screen to 3644
//! at the bottom, and its Y channel reads 4095 wherever it is pressed — one
//! half of the film is not connected. The screen's buttons are bands its whole
//! width ([`crate::lcd::rows`]), so a tap's row is enough to know which.

use anyhow::{bail, Context, Result};

/// A press this firm or firmer is a finger. The rig's panel read 0 untouched
/// and 220–1200 pressed.
pub const PRESSED: u16 = 150;
/// Readings a press needs before it counts: a brush of the film is not a tap.
const SAMPLES: usize = 3;
/// Readings in a row without pressure before a press is over. One light
/// reading mid-press is the film, not a finger lifting.
const LIFTED: u32 = 2;

/// Which controller channel follows the screen's rows, and what it reads at
/// the top and at the bottom of the screen: `x:204:4000`, the rig's panel.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Rows {
    /// The controller's command byte for that channel.
    pub command: u8,
    pub at_top: f32,
    pub at_bottom: f32,
}

impl Rows {
    /// Measured on the rig: 560 at row 30 and 3644 at row 290 of 320, so 204
    /// at the top edge and 4000 at the bottom.
    pub const RIG: Rows = Rows {
        command: X,
        at_top: 204.0,
        at_bottom: 4000.0,
    };

    /// `channel:at_top:at_bottom`, channel `x` or `y` — as the env file
    /// gives it, for a panel mounted or wired the other way.
    pub fn parse(text: &str) -> Result<Self> {
        let parts: Vec<&str> = text.split(':').map(str::trim).collect();
        let [channel, top, bottom] = parts[..] else {
            bail!("touch rows are channel:at_top:at_bottom, such as x:204:4000");
        };
        let command = match channel {
            "x" | "X" => X,
            "y" | "Y" => Y,
            other => bail!("{other:?} is not a touch channel: x or y"),
        };
        let number = |value: &str| {
            value
                .parse::<f32>()
                .with_context(|| format!("{value:?} is not a reading"))
        };
        let (at_top, at_bottom) = (number(top)?, number(bottom)?);
        if (at_top - at_bottom).abs() < 100.0 {
            bail!("the readings at the top and the bottom must differ");
        }
        Ok(Self {
            command,
            at_top,
            at_bottom,
        })
    }

    /// The screen row a reading is at, on a screen `height` rows tall.
    pub fn row(&self, reading: u16, height: u32) -> i32 {
        let share = (reading as f32 - self.at_top) / (self.at_bottom - self.at_top);
        (share * height as f32)
            .round()
            .clamp(0.0, height as f32 - 1.0) as i32
    }
}

/// The controller's commands: channel, 12 bits, differential, powered down
/// between readings so its pen interrupt stays armed for whoever wants it.
pub const X: u8 = 0xD0;
pub const Y: u8 = 0x90;
pub const PRESSURE: u8 = 0xB0;

/// Turns readings into taps: a press of at least [`SAMPLES`] readings, ended
/// by [`LIFTED`] without pressure, is a tap at the middle of its positions.
#[derive(Debug, Default)]
pub struct Taps {
    positions: Vec<u16>,
    light: u32,
}

impl Taps {
    /// One reading of pressure and position. Returns a tap's position when a
    /// press has just ended.
    pub fn reading(&mut self, pressure: u16, position: u16) -> Option<u16> {
        if pressure >= PRESSED {
            self.light = 0;
            self.positions.push(position);
            return None;
        }
        if self.positions.is_empty() {
            return None;
        }
        self.light += 1;
        if self.light < LIFTED {
            return None;
        }
        let mut positions = std::mem::take(&mut self.positions);
        self.light = 0;
        if positions.len() < SAMPLES {
            return None;
        }
        // The first and last readings of a press are the film settling; the
        // middle of them all is where the finger was.
        positions.sort_unstable();
        Some(positions[positions.len() / 2])
    }
}

/// The action a tap at `row` falls on, with a little room either side of
/// each band for a finger that is not quite on it.
pub fn hit<A: Copy>(bands: &[(i32, i32, A)], row: i32) -> Option<A> {
    const SLACK: i32 = 4;
    bands
        .iter()
        .find(|(top, bottom, _)| row >= top - SLACK && row <= bottom + SLACK)
        .map(|(_, _, action)| *action)
}

/// Asks the controller on `device` (`/dev/spidevB.C`) about the film thirty
/// times a second, for as long as the process runs, and calls `tap` with the
/// row of each tap on a screen `height` rows tall.
#[cfg(target_os = "linux")]
pub fn watch(
    device: &str,
    rows: Rows,
    height: u32,
    mut tap: impl FnMut(i32) + Send + 'static,
) -> Result<()> {
    use rppal::spi::{Bus, Mode, SlaveSelect, Spi};

    let (bus, select) = device
        .strip_prefix("/dev/spidev")
        .and_then(|rest| rest.split_once('.'))
        .with_context(|| format!("{device} is not an SPI device such as /dev/spidev0.1"))?;
    let bus = match bus {
        "0" => Bus::Spi0,
        "1" => Bus::Spi1,
        other => bail!("SPI bus {other} is not one scannerd knows"),
    };
    let select = match select {
        "0" => SlaveSelect::Ss0,
        "1" => SlaveSelect::Ss1,
        "2" => SlaveSelect::Ss2,
        other => bail!("SPI chip select {other} is not one scannerd knows"),
    };
    let spi = Spi::new(bus, select, 500_000, Mode::Mode0).with_context(|| {
        format!(
            "could not open {device} for the touchscreen (is the kernel's ads7846 \
             driver kept off it, and scannerd in the `spi` group?)"
        )
    })?;
    let read = move |command: u8| -> Option<u16> {
        let mut answer = [0u8; 3];
        spi.transfer(&mut answer, &[command, 0, 0]).ok()?;
        Some(((answer[1] as u16) << 8 | answer[2] as u16) >> 3)
    };
    std::thread::spawn(move || {
        let mut taps = Taps::default();
        loop {
            if let (Some(pressure), Some(position)) = (read(PRESSURE), read(rows.command)) {
                if let Some(position) = taps.reading(pressure, position) {
                    tap(rows.row(position, height));
                }
            }
            std::thread::sleep(std::time::Duration::from_millis(33));
        }
    });
    Ok(())
}
