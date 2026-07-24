use serde::{Deserialize, Serialize};
/// Envelope generator — shared by Pulse 1, Pulse 2, and Noise.
///
/// Clocked once per quarter-frame (240 Hz). Produces a volume level that either
/// stays constant (constant_volume flag) or decays from 15 down to 0 and wraps
/// when the loop flag is set.
#[derive(Serialize, Deserialize)]
pub struct Envelope {
    start_flag: bool,
    pub loop_flag: bool,
    constant_volume: bool,
    /// Divider period / constant volume level (bits 3–0 of the control register).
    period: u8,
    divider: u8,
    decay_level: u8,
}

impl Envelope {
    pub fn new() -> Self {
        Self {
            start_flag: false,
            loop_flag: false,
            constant_volume: false,
            period: 0,
            divider: 0,
            decay_level: 0,
        }
    }

    /// Load from the channel's first register (bits 5–0; bits 7–6 are handled
    /// by the caller for duty / control flag).
    pub fn write(&mut self, data: u8) {
        self.loop_flag = data & 0x20 != 0;
        self.constant_volume = data & 0x10 != 0;
        self.period = data & 0x0F;
    }

    /// Writing the channel's fourth register ($4003/$4007/$400F) restarts the
    /// envelope on the next quarter-frame clock.
    pub fn restart(&mut self) {
        self.start_flag = true;
    }

    /// Quarter-frame clock.
    pub fn clock(&mut self) {
        if self.start_flag {
            self.start_flag = false;
            self.decay_level = 15;
            self.divider = self.period;
            return;
        }
        if self.divider > 0 {
            self.divider -= 1;
        } else {
            self.divider = self.period;
            if self.decay_level > 0 {
                self.decay_level -= 1;
            } else if self.loop_flag {
                self.decay_level = 15;
            }
        }
    }

    pub fn volume(&self) -> u8 {
        if self.constant_volume {
            self.period
        } else {
            self.decay_level
        }
    }
}
