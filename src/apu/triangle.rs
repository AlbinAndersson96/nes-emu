use super::length::LengthCounter;

/// 32-step staircase triangle waveform (15 → 0, then 0 → 15).
const TRIANGLE_TABLE: [u8; 32] = [
    15, 14, 13, 12, 11, 10, 9, 8, 7, 6, 5, 4, 3, 2, 1, 0,
     0,  1,  2,  3,  4,  5, 6, 7, 8, 9,10,11,12,13,14,15,
];

/// Triangle channel ($4008–$400B).
///
/// Unlike the pulse channels, the triangle timer counts at the full CPU clock
/// rate (not APU rate). Volume is fixed — it is silenced by the length counter
/// or the linear counter, never by an envelope.
pub struct TriangleChannel {
    enabled: bool,
    /// $4008 bit 7: halts both length counter and linear counter reload.
    control_flag: bool,
    linear_reload_value: u8,
    linear_counter: u8,
    linear_reload_flag: bool,
    /// 11-bit timer reload value.
    timer_period: u16,
    timer: u16,
    seq_pos: u8,
    pub length: LengthCounter,
}

impl TriangleChannel {
    pub fn new() -> Self {
        Self {
            enabled: false,
            control_flag: false,
            linear_reload_value: 0,
            linear_counter: 0,
            linear_reload_flag: false,
            timer_period: 0,
            timer: 0,
            seq_pos: 0,
            length: LengthCounter::new(),
        }
    }

    /// Register write. `reg` is 0, 2, or 3 ($4008, $400A, $400B); $4009 is unused.
    pub fn write_reg(&mut self, reg: u8, data: u8) {
        match reg {
            0 => {
                self.control_flag = data & 0x80 != 0;
                self.linear_reload_value = data & 0x7F;
                self.length.halt = self.control_flag;
            }
            2 => {
                self.timer_period = (self.timer_period & 0xFF00) | u16::from(data);
            }
            3 => {
                self.timer_period =
                    (self.timer_period & 0x00FF) | (u16::from(data & 0x07) << 8);
                if self.enabled {
                    self.length.load(data >> 3);
                }
                self.linear_reload_flag = true;
            }
            _ => {}
        }
    }

    pub fn set_enabled(&mut self, enabled: bool) {
        self.enabled = enabled;
        if !enabled {
            self.length.force_zero();
        }
    }

    /// CPU-rate clock (every CPU cycle, unlike pulse/noise which tick at APU rate).
    pub fn clock_timer(&mut self) {
        if self.timer == 0 {
            self.timer = self.timer_period;
            if self.length.active() && self.linear_counter > 0 {
                self.seq_pos = (self.seq_pos + 1) & 31;
            }
        } else {
            self.timer -= 1;
        }
    }

    /// Quarter-frame clock (240 Hz) — clocks the linear counter.
    pub fn clock_linear_counter(&mut self) {
        if self.linear_reload_flag {
            self.linear_counter = self.linear_reload_value;
        } else if self.linear_counter > 0 {
            self.linear_counter -= 1;
        }
        if !self.control_flag {
            self.linear_reload_flag = false;
        }
    }

    /// Half-frame clock (120 Hz) — clocks the length counter.
    pub fn clock_length(&mut self) {
        self.length.clock();
    }

    pub fn length_active(&self) -> bool {
        self.length.active()
    }

    /// 4-bit sample output (0 = silent).
    pub fn output(&self) -> u8 {
        if !self.enabled || !self.length.active() || self.linear_counter == 0 {
            return 0;
        }
        TRIANGLE_TABLE[self.seq_pos as usize]
    }
}
