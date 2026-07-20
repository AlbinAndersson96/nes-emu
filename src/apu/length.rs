/// Length counter lookup table — indexed by the upper 5 bits of the channel's
/// fourth register ($4003/$4007/$400B/$400F). Shared by all four channels.
pub const LENGTH_TABLE: [u8; 32] = [
    0x0A, 0xFE, 0x14, 0x02, 0x28, 0x04, 0x50, 0x06, 0xA0, 0x08, 0x3C, 0x0A, 0x0E, 0x0C, 0x1A, 0x0E,
    0x0C, 0x10, 0x18, 0x12, 0x30, 0x14, 0x60, 0x16, 0xC0, 0x18, 0x48, 0x1A, 0x10, 0x1C, 0x20, 0x1E,
];

/// CPU cycles between `Apu::write` queueing a halt/length-reload write and
/// its effect landing. `Apu::write` runs while the APU still sits at the
/// START of the writing instruction (an APU register store is in practice an
/// absolute store whose bus write happens on its 4th/last cycle — the same
/// "+4" the $4017 frame-reset delay accounts for), and blargg's 2005
/// frame-counter doc adds one more: "write to halt flag is delayed by one
/// clock", i.e. the effect lands one cycle after the write cycle — 4 + 1.
/// Calibrated by sweeping 3-6 against blargg_apu_2005.07.30's
/// 10.len_halt_timing + 11.len_reload_timing (each brackets the effect cycle
/// from both sides): 5 is the unique passing value.
const WRITE_EFFECT_DELAY: u8 = 5;

pub struct LengthCounter {
    halt: bool,
    count: u8,
    /// In-flight halt-bit write: (ticks until the real write cycle, value).
    pending_halt: Option<(u8, bool)>,
    /// In-flight length reload: (ticks until the real write cycle, table index).
    pending_load: Option<(u8, u8)>,
    /// Set by `clock()` to whether the counter was nonzero just before that
    /// clock; consumed and cleared by `end_cycle` the same CPU cycle. Drives
    /// the hardware rule that a reload landing on a length-clock cycle is
    /// completely ignored when the counter was nonzero before clocking.
    clocked_pre_nonzero: Option<bool>,
}

impl LengthCounter {
    pub fn new() -> Self {
        Self {
            halt: false,
            count: 0,
            pending_halt: None,
            pending_load: None,
            clocked_pre_nonzero: None,
        }
    }

    /// Queue a halt-bit write; takes effect on the real write cycle, AFTER
    /// any length clock landing on that same cycle.
    pub fn schedule_halt(&mut self, halt: bool) {
        self.pending_halt = Some((WRITE_EFFECT_DELAY, halt));
    }

    /// Queue a length reload; takes effect on the real write cycle unless a
    /// length clock on that cycle found the counter nonzero (then ignored).
    pub fn schedule_load(&mut self, index: u8) {
        self.pending_load = Some((WRITE_EFFECT_DELAY, index));
    }

    /// Clocked once per half-frame (120 Hz). Decrements if not halted.
    pub fn clock(&mut self) {
        self.clocked_pre_nonzero = Some(self.count > 0);
        if !self.halt && self.count > 0 {
            self.count -= 1;
        }
    }

    /// Per-CPU-cycle tick, called by `Apu::tick_one` after the frame-counter
    /// events so pending writes apply after (never before) a length clock on
    /// the same cycle. `enabled` is the channel's $4015 enable bit, sampled
    /// at the write's effective cycle — a reload on a disabled channel is
    /// dropped.
    pub fn end_cycle(&mut self, enabled: bool) {
        if let Some((ticks, halt)) = self.pending_halt {
            if ticks <= 1 {
                self.halt = halt;
                self.pending_halt = None;
            } else {
                self.pending_halt = Some((ticks - 1, halt));
            }
        }
        if let Some((ticks, index)) = self.pending_load {
            if ticks <= 1 {
                if enabled && self.clocked_pre_nonzero != Some(true) {
                    self.count = LENGTH_TABLE[(index & 0x1F) as usize];
                }
                self.pending_load = None;
            } else {
                self.pending_load = Some((ticks - 1, index));
            }
        }
        self.clocked_pre_nonzero = None;
    }

    pub fn active(&self) -> bool {
        self.count > 0
    }

    /// Disabling a channel via $4015 immediately zeros the counter (and drops
    /// any in-flight reload — the channel is disabled at its write cycle).
    pub fn force_zero(&mut self) {
        self.count = 0;
        self.pending_load = None;
    }
}
