/// Length counter lookup table — indexed by the upper 5 bits of the channel's
/// fourth register ($4003/$4007/$400B/$400F). Shared by all four channels.
pub const LENGTH_TABLE: [u8; 32] = [
    0x0A, 0xFE, 0x14, 0x02, 0x28, 0x04, 0x50, 0x06,
    0xA0, 0x08, 0x3C, 0x0A, 0x0E, 0x0C, 0x1A, 0x0E,
    0x0C, 0x10, 0x18, 0x12, 0x30, 0x14, 0x60, 0x16,
    0xC0, 0x18, 0x48, 0x1A, 0x10, 0x1C, 0x20, 0x1E,
];

pub struct LengthCounter {
    pub halt: bool,
    count: u8,
}

impl LengthCounter {
    pub fn new() -> Self {
        Self { halt: false, count: 0 }
    }

    pub fn load(&mut self, index: u8) {
        self.count = LENGTH_TABLE[(index & 0x1F) as usize];
    }

    /// Clocked once per half-frame (120 Hz). Decrements if not halted.
    pub fn clock(&mut self) {
        if !self.halt && self.count > 0 {
            self.count -= 1;
        }
    }

    pub fn active(&self) -> bool {
        self.count > 0
    }

    /// Disabling a channel via $4015 immediately zeros the counter.
    pub fn force_zero(&mut self) {
        self.count = 0;
    }
}
