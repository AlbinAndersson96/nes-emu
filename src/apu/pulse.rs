use super::envelope::Envelope;
use super::length::LengthCounter;
use super::sweep::SweepUnit;

/// 8-step duty-cycle waveforms (indexed by duty[0..3], position[0..7]).
const DUTY_TABLE: [[u8; 8]; 4] = [
    [0, 1, 0, 0, 0, 0, 0, 0], // 12.5 %
    [0, 1, 1, 0, 0, 0, 0, 0], // 25 %
    [0, 1, 1, 1, 1, 0, 0, 0], // 50 %
    [1, 0, 0, 1, 1, 1, 1, 1], // 75 % (negated 25 %)
];

/// One of the two pulse channels ($4000–$4003 or $4004–$4007).
pub struct PulseChannel {
    enabled: bool,
    duty: u8,
    duty_pos: u8,
    /// 11-bit timer reload value.
    timer_period: u16,
    timer: u16,
    pub envelope: Envelope,
    pub sweep: SweepUnit,
    pub length: LengthCounter,
}

impl PulseChannel {
    /// `ones_complement` is true for Pulse 1, false for Pulse 2.
    pub fn new(ones_complement: bool) -> Self {
        Self {
            enabled: false,
            duty: 0,
            duty_pos: 0,
            timer_period: 0,
            timer: 0,
            envelope: Envelope::new(),
            sweep: SweepUnit::new(ones_complement),
            length: LengthCounter::new(),
        }
    }

    /// Register write. `reg` is 0–3 (offset from the channel's base address).
    pub fn write_reg(&mut self, reg: u8, data: u8) {
        match reg {
            0 => {
                self.duty = (data >> 6) & 0x03;
                self.length.schedule_halt(data & 0x20 != 0);
                self.envelope.write(data);
            }
            1 => self.sweep.write(data),
            2 => {
                self.timer_period = (self.timer_period & 0xFF00) | u16::from(data);
            }
            3 => {
                self.timer_period = (self.timer_period & 0x00FF) | (u16::from(data & 0x07) << 8);
                self.length.schedule_load(data >> 3);
                self.envelope.restart();
                self.duty_pos = 0;
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

    /// APU-rate clock (every 2 CPU cycles).
    pub fn clock_timer(&mut self) {
        if self.timer == 0 {
            self.timer = self.timer_period;
            self.duty_pos = (self.duty_pos + 1) & 7;
        } else {
            self.timer -= 1;
        }
    }

    /// Quarter-frame clock (240 Hz).
    pub fn clock_envelope(&mut self) {
        self.envelope.clock();
    }

    /// Half-frame clock (120 Hz).
    pub fn clock_length_and_sweep(&mut self) {
        self.length.clock();
        self.sweep.clock(&mut self.timer_period);
    }

    /// Per-CPU-cycle tick (after frame-counter events): applies pending
    /// halt/reload writes on their real write cycle.
    pub fn end_cycle(&mut self) {
        self.length.end_cycle(self.enabled);
    }

    pub fn length_active(&self) -> bool {
        self.length.active()
    }

    /// 4-bit sample output (0 = silent).
    pub fn output(&self) -> u8 {
        if !self.enabled
            || !self.length.active()
            || self.sweep.muting(self.timer_period)
            || DUTY_TABLE[self.duty as usize][self.duty_pos as usize] == 0
        {
            return 0;
        }
        self.envelope.volume()
    }
}
