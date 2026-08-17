use anyhow::Result;
use esp_idf_svc::hal::gpio::{Gpio0, Gpio7, Gpio17, Gpio18, Input, PinDriver, Pull};

#[derive(Clone, Copy, Debug, Default)]
pub struct Buttons {
    pub up: bool,
    pub right: bool,
    pub down: bool,
    pub left: bool,
}

impl Buttons {
    pub fn any(self) -> bool {
        self.up || self.right || self.down || self.left
    }
}

pub struct BadgeInput {
    up: PinDriver<'static, Input>,
    right: PinDriver<'static, Input>,
    down: PinDriver<'static, Input>,
    left: PinDriver<'static, Input>,
}

impl BadgeInput {
    pub fn new(
        up: Gpio7<'static>,
        right: Gpio18<'static>,
        down: Gpio17<'static>,
        left: Gpio0<'static>,
    ) -> Result<Self> {
        Ok(Self {
            up: PinDriver::input(up, Pull::Up)?,
            right: PinDriver::input(right, Pull::Up)?,
            down: PinDriver::input(down, Pull::Up)?,
            left: PinDriver::input(left, Pull::Up)?,
        })
    }

    pub fn sample(&self) -> Buttons {
        Buttons {
            up: self.up.is_low(),
            right: self.right.is_low(),
            down: self.down.is_low(),
            left: self.left.is_low(),
        }
    }
}
