/// Sweep unit — present on Pulse 1 and Pulse 2.
///
/// Clocked once per half-frame (120 Hz). Adjusts the pulse timer period to
/// produce pitch slides. Pulse 1 uses one's-complement negation; Pulse 2 uses
/// two's-complement negation.
pub struct SweepUnit {
    enabled: bool,
    /// Divider period P; the sweep fires every P+1 half-frames.
    period: u8,
    negate: bool,
    shift: u8,
    divider: u8,
    reload_flag: bool,
    /// true for Pulse 1 (one's complement subtract), false for Pulse 2.
    ones_complement: bool,
}

impl SweepUnit {
    pub fn new(ones_complement: bool) -> Self {
        Self {
            enabled: false,
            period: 0,
            negate: false,
            shift: 0,
            divider: 0,
            reload_flag: false,
            ones_complement,
        }
    }

    pub fn write(&mut self, data: u8) {
        self.enabled = data & 0x80 != 0;
        self.period = (data >> 4) & 0x07;
        self.negate = data & 0x08 != 0;
        self.shift = data & 0x07;
        self.reload_flag = true;
    }

    /// Returns true if the channel should be silenced regardless of the sweep
    /// divider. The channel mutes when the current period < 8 or when the
    /// target period would exceed $7FF.
    pub fn muting(&self, period: u16) -> bool {
        period < 8 || self.target(period) > 0x7FF
    }

    fn target(&self, current: u16) -> u16 {
        let delta = current >> self.shift;
        if self.negate {
            if self.ones_complement {
                current.wrapping_sub(delta).wrapping_sub(1)
            } else {
                current.wrapping_sub(delta)
            }
        } else {
            current.saturating_add(delta)
        }
    }

    /// Half-frame clock. Mutates `period` if the sweep fires.
    pub fn clock(&mut self, period: &mut u16) {
        if self.reload_flag {
            if self.divider == 0 && self.enabled && self.shift > 0 && !self.muting(*period) {
                *period = self.target(*period);
            }
            self.divider = self.period;
            self.reload_flag = false;
            return;
        }
        if self.divider > 0 {
            self.divider -= 1;
        } else {
            self.divider = self.period;
            if self.enabled && self.shift > 0 && !self.muting(*period) {
                *period = self.target(*period);
            }
        }
    }
}
