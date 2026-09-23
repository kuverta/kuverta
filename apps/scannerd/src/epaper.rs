//! The Waveshare 2.13″ e-paper HAT (V3/V4): an SSD1680 controller behind SPI
//! and four GPIO lines.
//!
//! Identified on the rig by its answers rather than its label: after a
//! software reset it holds BUSY high for a few milliseconds, which the SSD16xx
//! family does and the older UC81xx panels do not; its status register reads
//! 0x01; and its temperature sensor reads the room. The panel's size is not
//! something the controller knows — 250×122 is what fits on a board the size
//! of a Pi Zero.
//!
//! The sequence is Waveshare's own for this panel, in `epd2in13_V4.py`: a full
//! refresh with the waveform in the controller's OTP, and a partial one that
//! writes the new picture and has the controller drive only what changed. After
//! a partial refresh the picture is written again as the "previous" one, so the
//! next partial refresh changes what is on the glass and not what was there two
//! pictures ago.
//!
//! Wiring is the HAT's: `/dev/spidev0.0` (so `dtparam=spi=on`), and BCM pins
//! 17 reset, 25 data/command, 24 busy, 18 power. Pin 17 is also where the
//! readme used to put the letter button; with the HAT on, the button moves.

use std::thread::sleep;
use std::time::{Duration, Instant};

use anyhow::{bail, Context, Result};
use rppal::gpio::{Gpio, InputPin, OutputPin};
use rppal::spi::{Bus, Mode, SlaveSelect, Spi};

use crate::display::{Frame, Panel, HEIGHT, WIDTH};

const RESET: u8 = 17;
const DATA_COMMAND: u8 = 25;
const BUSY: u8 = 24;
/// Newer HATs (rev 2.2 on) switch the panel's supply from this pin.
const POWER: u8 = 18;

/// The controller takes up to 20 MHz for writes; there is no reason to push it.
const CLOCK: u32 = 4_000_000;
/// A full refresh takes about two seconds. Longer than this is a panel that
/// is not there.
const BUSY_LIMIT: Duration = Duration::from_secs(10);

pub struct Ssd1680 {
    spi: Spi,
    reset: OutputPin,
    data_command: OutputPin,
    busy: InputPin,
    _power: OutputPin,
}

impl Ssd1680 {
    pub fn open() -> Result<Self> {
        let gpio = Gpio::new()
            .context("could not reach the GPIO pins (is scannerd in the `gpio` group?)")?;
        let pin = |number: u8| {
            gpio.get(number)
                .with_context(|| format!("GPIO {number}, which the display uses, is taken"))
        };
        let spi = Spi::new(Bus::Spi0, SlaveSelect::Ss0, CLOCK, Mode::Mode0).context(
            "could not open /dev/spidev0.0 for the display \
             (dtparam=spi=on in /boot/firmware/config.txt, and scannerd in the `spi` group)",
        )?;
        let mut panel = Self {
            spi,
            reset: pin(RESET)?.into_output_high(),
            data_command: pin(DATA_COMMAND)?.into_output_low(),
            busy: pin(BUSY)?.into_input(),
            _power: pin(POWER)?.into_output_high(),
        };
        sleep(Duration::from_millis(20));
        panel.init()?;
        Ok(panel)
    }

    /// Hardware reset and the registers for a whole-panel update. Also wakes
    /// the controller from deep sleep, which keeps what is in its memory.
    fn init(&mut self) -> Result<()> {
        self.hardware_reset();
        self.wait()?;
        self.command(0x12, &[])?; // software reset
        self.wait()?;
        self.registers(0x03)?;
        self.command(0x21, &[0x00, 0x80])?; // display update control
        self.command(0x18, &[0x80])?; // the internal temperature sensor
        self.wait()
    }

    fn hardware_reset(&mut self) {
        self.reset.set_high();
        sleep(Duration::from_millis(20));
        self.reset.set_low();
        sleep(Duration::from_millis(2));
        self.reset.set_high();
        sleep(Duration::from_millis(20));
    }

    /// Gate count, data entry, the window covering the whole panel and the
    /// cursor at its start; and the border waveform.
    fn registers(&mut self, border: u8) -> Result<()> {
        let last_row = (WIDTH - 1) as u16;
        let last_column_byte = ((HEIGHT - 1) / 8) as u8;
        self.command(0x01, &[last_row as u8, (last_row >> 8) as u8, 0x00])?;
        self.command(0x11, &[0x03])?; // x then y, both increasing
        self.command(0x44, &[0x00, last_column_byte])?;
        self.command(0x45, &[0x00, 0x00, last_row as u8, (last_row >> 8) as u8])?;
        self.command(0x4E, &[0x00])?;
        self.command(0x4F, &[0x00, 0x00])?;
        self.command(0x3C, &[border])
    }

    fn command(&mut self, command: u8, data: &[u8]) -> Result<()> {
        self.data_command.set_low();
        self.spi.write(&[command])?;
        if !data.is_empty() {
            self.data_command.set_high();
            // spidev takes 4096 bytes a transfer; a frame is 4000.
            for chunk in data.chunks(4096) {
                self.spi.write(chunk)?;
            }
        }
        Ok(())
    }

    /// Until the controller lets go of BUSY. Polled, not spun on: the Zero has
    /// one core and the capture loop wants it.
    fn wait(&mut self) -> Result<()> {
        let started = Instant::now();
        while self.busy.is_high() {
            if started.elapsed() > BUSY_LIMIT {
                bail!("the display stayed busy for {BUSY_LIMIT:?}; is it connected?");
            }
            sleep(Duration::from_millis(10));
        }
        Ok(())
    }

    fn refresh(&mut self, mode: u8) -> Result<()> {
        self.command(0x22, &[mode])?;
        self.command(0x20, &[])?;
        self.wait()
    }

    /// Deep sleep that keeps the picture memory: nothing is driven between
    /// pictures, which Waveshare asks for, and a reset wakes it.
    fn rest(&mut self) -> Result<()> {
        self.command(0x10, &[0x01])
    }
}

impl Panel for Ssd1680 {
    fn show(&mut self, frame: &Frame, full: bool) -> Result<()> {
        if full {
            self.init()?;
            // Both memories, so the first partial refresh after this one has
            // the right picture to change from.
            self.command(0x24, &frame.bytes)?;
            self.command(0x26, &frame.bytes)?;
            self.refresh(0xF7)?;
        } else {
            // A reset puts the registers back, the temperature sensor to an
            // external one there is none of among them; the memory it keeps.
            self.hardware_reset();
            self.registers(0x80)?;
            self.command(0x18, &[0x80])?;
            self.command(0x24, &frame.bytes)?;
            self.refresh(0xFF)?;
            self.command(0x4E, &[0x00])?;
            self.command(0x4F, &[0x00, 0x00])?;
            self.command(0x26, &frame.bytes)?;
        }
        self.rest()
    }
}
