use super::envelope::Envelope;
use super::length::LengthCounter;
use serde::{Deserialize, Serialize};

/// Timer reload periods for the noise channel (NTSC), indexed by $400E bits 3–0.
const NTSC_PERIOD: [u16; 16] = [
    4, 8, 16, 32, 64, 96, 128, 160, 202, 254, 380, 508, 762, 1016, 2034, 4068,
];

/// Noise channel ($400C–$400F).
///
/// Uses a 15-bit LFSR clocked by the APU timer. Mode flag selects a 32,767-step
/// (mode=0) or 93-step (mode=1, short) pseudo-random sequence.
#[derive(Serialize, Deserialize)]
pub struct NoiseChannel {
    enabled: bool,
    /// Short-sequence mode: XOR bit 6 instead of bit 1 into the LFSR.
    mode: bool,
    timer_period: u16,
    timer: u16,
    /// 15-bit LFSR; initialized to 1 on power-up.
    lfsr: u16,
    pub envelope: Envelope,
    pub length: LengthCounter,
}

impl NoiseChannel {
    pub fn new() -> Self {
        Self {
            enabled: false,
            mode: false,
            timer_period: NTSC_PERIOD[0],
            timer: 0,
            lfsr: 1,
            envelope: Envelope::new(),
            length: LengthCounter::new(),
        }
    }

    /// Register write. `reg` is 0, 2, or 3 ($400C, $400E, $400F); $400D is unused.
    pub fn write_reg(&mut self, reg: u8, data: u8) {
        match reg {
            0 => {
                self.length.schedule_halt(data & 0x20 != 0);
                self.envelope.write(data);
            }
            2 => {
                self.mode = data & 0x80 != 0;
                self.timer_period = NTSC_PERIOD[(data & 0x0F) as usize];
            }
            3 => {
                self.length.schedule_load(data >> 3);
                self.envelope.restart();
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
            let other_bit = if self.mode {
                (self.lfsr >> 6) & 1
            } else {
                (self.lfsr >> 1) & 1
            };
            let feedback = (self.lfsr & 1) ^ other_bit;
            self.lfsr = (self.lfsr >> 1) | (feedback << 14);
        } else {
            self.timer -= 1;
        }
    }

    /// Quarter-frame clock (240 Hz).
    pub fn clock_envelope(&mut self) {
        self.envelope.clock();
    }

    /// Half-frame clock (120 Hz).
    pub fn clock_length(&mut self) {
        self.length.clock();
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
        // LFSR bit 0 set means silence
        if !self.enabled || self.lfsr & 1 == 1 || !self.length.active() {
            return 0;
        }
        self.envelope.volume()
    }
}
