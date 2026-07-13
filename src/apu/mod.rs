mod dmc;
mod envelope;
mod length;
mod noise;
mod pulse;
mod sweep;
mod triangle;

use dmc::DmcChannel;
use noise::NoiseChannel;
use pulse::PulseChannel;
use triangle::TriangleChannel;

/// NTSC frame-counter step sequences (CPU cycles from the last $4017 write).
///
/// Each tuple is (cpu_cycle, quarter_frame, half_frame, set_irq, reset).
/// `reset` marks the end of the sequence — frame_cycles wraps to 0.
/// The 4-step mode (MODE0) fires the IRQ flag on cycles 29828, 29829, AND 29830,
/// which matches hardware behaviour (three consecutive IRQ assertions per frame).
const MODE0: [(u32, bool, bool, bool, bool); 6] = [
    (7_457, true, false, false, false),
    (14_913, true, true, false, false),
    (22_371, true, false, false, false),
    (29_828, false, false, true, false), // IRQ (cycle before step 4)
    (29_829, true, true, true, false),   // Q+H+IRQ (step 4)
    (29_830, false, false, true, true),  // IRQ + sequence restart
];

const MODE1: [(u32, bool, bool, bool, bool); 6] = [
    (7_457, true, false, false, false),
    (14_913, true, true, false, false),
    (22_371, true, false, false, false),
    (29_829, false, false, false, false), // silent step (no IRQ, no reset)
    (37_281, true, true, false, false),
    (37_282, false, false, false, true), // sequence restart
];

pub struct Apu {
    pub pulse1: PulseChannel,
    pub pulse2: PulseChannel,
    pub triangle: TriangleChannel,
    pub noise: NoiseChannel,
    pub dmc: DmcChannel,

    // Frame counter
    frame_mode: bool, // false = 4-step (mode 0), true = 5-step (mode 1)
    irq_inhibit: bool,
    frame_irq_flag: bool,
    /// CPU cycles elapsed since the last frame counter reset.
    frame_cycles: u32,
    /// Cycles remaining until a $4017-triggered frame counter reset takes effect.
    /// Writing $4017 sets this to 7; each tick decrements it; at 0 frame_cycles
    /// resets. See the $4017 write handler for the derivation of 7.
    frame_reset_delay: u8,
    /// Free-running CPU-cycle counter — never reset by register writes (unlike
    /// frame_cycles). Its parity models the APU's divide-by-2 get/put clock,
    /// which decides whether an OAM DMA takes 513 or 514 cycles.
    cycle_count: u64,

    /// APU-rate clocks already applied to the DMC channel by
    /// `dmc_tick_realtime` (called mid-instruction from `Bus::read`/`write`),
    /// which `tick_one`'s own DMC clocking must skip so the channel's timer
    /// is never decremented twice for the same CPU cycle.
    dmc_debt: u32,
    /// True when the next `dmc_tick_realtime` call must reseed its phase
    /// from `frame_cycles` (i.e. no debt is outstanding, so `frame_cycles`
    /// is currently authoritative). Set whenever `tick_one` drains `dmc_debt`
    /// back to 0.
    dmc_realtime_needs_reseed: bool,
    /// This-CPU-cycle's DMC APU-rate phase once seeded (true = the DMC
    /// channel's timer clocks on this cycle). Only meaningful between a
    /// reseed and the next debt drain.
    dmc_realtime_parity: bool,
}

impl Apu {
    pub fn new() -> Self {
        Self {
            pulse1: PulseChannel::new(true),  // ones-complement negation
            pulse2: PulseChannel::new(false), // twos-complement negation
            triangle: TriangleChannel::new(),
            noise: NoiseChannel::new(),
            dmc: DmcChannel::new(),
            frame_mode: false,
            irq_inhibit: false,
            frame_irq_flag: false,
            frame_cycles: 0,
            frame_reset_delay: 0,
            cycle_count: 0,
            dmc_debt: 0,
            dmc_realtime_needs_reseed: true,
            dmc_realtime_parity: false,
        }
    }

    /// Advance the APU by `cpu_cycles`. Returns true if an IRQ line should be
    /// asserted this tick (frame counter IRQ or DMC IRQ).
    pub fn tick(&mut self, cpu_cycles: u32) -> bool {
        for _ in 0..cpu_cycles {
            self.tick_one();
        }
        self.take_irq()
    }

    fn tick_one(&mut self) {
        self.frame_cycles += 1;
        self.cycle_count += 1;

        // $4017 write-to-reset delay; see the $4017 write handler for derivation.
        if self.frame_reset_delay > 0 {
            self.frame_reset_delay -= 1;
            if self.frame_reset_delay == 0 {
                self.frame_cycles = 0;
            }
        }

        let c = self.frame_cycles;

        // Frame counter — fire events and reset at the end of each sequence.
        let steps: &[(u32, bool, bool, bool, bool)] = if self.frame_mode { &MODE1 } else { &MODE0 };

        for &(at, quarter, half, irq, reset) in steps {
            if c == at {
                if quarter {
                    self.clock_quarter_frame();
                }
                if half {
                    self.clock_half_frame();
                }
                if irq && !self.irq_inhibit {
                    self.frame_irq_flag = true;
                }
                if reset {
                    self.frame_cycles = 0;
                }
                break;
            }
        }

        // Channel timers.
        // Pulse and noise timers count at APU rate (every 2 CPU cycles).
        if c & 1 == 0 {
            self.pulse1.clock_timer();
            self.pulse2.clock_timer();
            self.noise.clock_timer();
            // DMC timer also clocks at APU rate, but may already have been
            // advanced in real time by dmc_tick_realtime (called mid-
            // instruction from Bus::read/write) — skip re-clocking it here
            // for cycles already paid for, so it's never decremented twice.
            if self.dmc_debt > 0 {
                self.dmc_debt -= 1;
                if self.dmc_debt == 0 {
                    self.dmc_realtime_needs_reseed = true;
                }
            } else {
                self.dmc.clock_timer();
            }
        }
        // Triangle timer counts at full CPU rate.
        self.triangle.clock_timer();
    }

    fn clock_quarter_frame(&mut self) {
        self.pulse1.clock_envelope();
        self.pulse2.clock_envelope();
        self.triangle.clock_linear_counter();
        self.noise.clock_envelope();
    }

    fn clock_half_frame(&mut self) {
        self.pulse1.clock_length_and_sweep();
        self.pulse2.clock_length_and_sweep();
        self.triangle.clock_length();
        self.noise.clock_length();
    }

    /// Parity of the free-running APU cycle counter (the divide-by-2 get/put
    /// clock). Sampled by the bus when $4014 is written to pick the 513- or
    /// 514-cycle OAM DMA stall.
    pub fn cycle_parity(&self) -> bool {
        self.cycle_count & 1 == 1
    }

    /// Real-time, DMC-only clock: advances just the DMC channel's internal
    /// APU-rate timer by one CPU cycle, called directly from `Bus::read`/
    /// `write` so a fetch can be detected and serviced mid-instruction
    /// (unlike every other channel and the frame counter, which only ever
    /// advance via the post-hoc `tick`/`tick_one` called after a whole
    /// instruction completes). Does not touch `cycle_count` or
    /// `frame_cycles`; `tick_one`'s own DMC clocking consults `dmc_debt` to
    /// avoid double-clocking the cycles already applied here.
    ///
    /// The phase seed (which CPU cycle within an APU-rate pair this is) is a
    /// best-effort initial calibration — sweep it against AccuracyCoin's
    /// page 13/14 DMA tests if they don't pass (see docs/superpowers/plans/
    /// 2026-07-13-dmc-dma-cycle-accurate.md).
    pub fn dmc_tick_realtime(&mut self) -> bool {
        if self.dmc_realtime_needs_reseed {
            self.dmc_realtime_parity = self.frame_cycles & 1 == 1;
            self.dmc_realtime_needs_reseed = false;
        } else {
            self.dmc_realtime_parity = !self.dmc_realtime_parity;
        }
        if self.dmc_realtime_parity {
            self.dmc_debt += 1;
            self.dmc.clock_timer()
        } else {
            self.dmc.needs_dma()
        }
    }

    /// The APU-rate phase the NEXT `dmc_tick_realtime` call will compute for
    /// the current CPU cycle, without advancing anything. Used by the DMC-DMA
    /// stall-length decision, which needs the live get/put phase of the halt
    /// cycle itself — `cycle_parity()` is post-hoc and stale mid-instruction.
    pub fn dmc_realtime_current_parity(&self) -> bool {
        if self.dmc_realtime_needs_reseed {
            self.frame_cycles & 1 == 1
        } else {
            !self.dmc_realtime_parity
        }
    }

    fn take_irq(&mut self) -> bool {
        // Level-triggered: the IRQ line is asserted as long as frame_irq_flag is
        // set (and not inhibited), or the DMC IRQ flag is set.  Nothing is
        // cleared here; flags are cleared by reading $4015 or writing $4017.
        (self.frame_irq_flag && !self.irq_inhibit) || self.dmc.irq_flag
    }

    // -----------------------------------------------------------------------
    // Bus interface
    // -----------------------------------------------------------------------

    pub fn read(&mut self, addr: u16) -> u8 {
        match addr {
            0x4015 => {
                let s = self.status_byte();
                self.frame_irq_flag = false; // reading $4015 clears the frame IRQ flag
                s
            }
            _ => 0,
        }
    }

    fn status_byte(&self) -> u8 {
        let mut s = 0u8;
        if self.pulse1.length_active() {
            s |= 0x01;
        }
        if self.pulse2.length_active() {
            s |= 0x02;
        }
        if self.triangle.length_active() {
            s |= 0x04;
        }
        if self.noise.length_active() {
            s |= 0x08;
        }
        if self.dmc.active() {
            s |= 0x10;
        }
        if self.frame_irq_flag {
            s |= 0x40;
        }
        if self.dmc.irq_flag {
            s |= 0x80;
        }
        s
    }

    pub fn write(&mut self, addr: u16, data: u8) {
        match addr {
            0x4000 => self.pulse1.write_reg(0, data),
            0x4001 => self.pulse1.write_reg(1, data),
            0x4002 => self.pulse1.write_reg(2, data),
            0x4003 => self.pulse1.write_reg(3, data),
            0x4004 => self.pulse2.write_reg(0, data),
            0x4005 => self.pulse2.write_reg(1, data),
            0x4006 => self.pulse2.write_reg(2, data),
            0x4007 => self.pulse2.write_reg(3, data),
            0x4008 => self.triangle.write_reg(0, data),
            0x4009 => {} // unused
            0x400A => self.triangle.write_reg(2, data),
            0x400B => self.triangle.write_reg(3, data),
            0x400C => self.noise.write_reg(0, data),
            0x400D => {} // unused
            0x400E => self.noise.write_reg(2, data),
            0x400F => self.noise.write_reg(3, data),
            0x4010 => self.dmc.write_reg(0, data),
            0x4011 => self.dmc.write_reg(1, data),
            0x4012 => self.dmc.write_reg(2, data),
            0x4013 => self.dmc.write_reg(3, data),
            0x4015 => {
                self.pulse1.set_enabled(data & 0x01 != 0);
                self.pulse2.set_enabled(data & 0x02 != 0);
                self.triangle.set_enabled(data & 0x04 != 0);
                self.noise.set_enabled(data & 0x08 != 0);
                self.dmc.set_enabled(data & 0x10 != 0);
                self.dmc.irq_flag = false; // writing $4015 clears the DMC IRQ flag
            }
            0x4017 => {
                self.frame_mode = data & 0x80 != 0;
                self.irq_inhibit = data & 0x40 != 0;
                if self.irq_inhibit {
                    self.frame_irq_flag = false;
                }
                // On real hardware the frame counter reset takes effect 3 OR 4
                // CPU cycles after the WRITE CYCLE, depending on whether that
                // cycle lands on an even or odd APU get/put cycle (nesdev "APU
                // Frame Counter" write jitter). blargg's sync_apu depends on
                // this parity dependence: its `bit SNDCHN` / `bne` equalizer
                // only locks the CPU to the APU divider if exactly one of the
                // two parities sees the frame-IRQ flag at the probe read — a
                // constant delay here silently defeats the lock and lets
                // pre-sync parity leak into everything downstream (caught by
                // cpu_interrupts_v2/4's +526 row flipping when an unrelated
                // PPU change shifted frame counts).
                //
                // This write is applied while the APU still sits at the START
                // of the writing instruction: the caller only ticks the APU
                // (tick_apu(delta)) after the whole instruction, and a $4017
                // write is in practice always an absolute store whose bus
                // write happens on its 4th/last cycle. The APU is therefore 4
                // cycles behind the real write cycle here, so the delay is
                // 4 + (3 or 4). The parity mapping (even cycle_count → 3) was
                // calibrated against cpu_interrupts_v2/4-irq_and_dma with the
                // $2002-read VBL-suppression race implemented (a flat 7 was
                // verified earlier for the even-entry case only; a flat 4
                // shifts the whole table by exactly 3 rows) — see
                // docs/investigations/cpu_interrupt_debug_log.md (2026-07-04).
                self.frame_reset_delay = if self.cycle_count & 1 == 0 { 7 } else { 8 };
                // 5-step mode: immediate quarter/half-frame fires at write time.
                if self.frame_mode {
                    self.clock_quarter_frame();
                    self.clock_half_frame();
                }
            }
            _ => {}
        }
    }

    // -----------------------------------------------------------------------
    // DMC DMA helpers (called by the bus after it performs the CPU stall)
    // -----------------------------------------------------------------------

    /// True when the DMC reader needs a byte from CPU memory.
    pub fn dmc_needs_dma(&self) -> bool {
        self.dmc.needs_dma()
    }

    /// The CPU address the DMC reader wants next.
    pub fn dmc_dma_address(&self) -> u16 {
        self.dmc.dma_address()
    }

    /// Supply a DMA-fetched byte to the DMC reader.
    pub fn dmc_supply_byte(&mut self, data: u8) {
        self.dmc.supply_dma_byte(data);
    }

    // -----------------------------------------------------------------------
    // Audio mixer
    // -----------------------------------------------------------------------

    /// Mix all five channel outputs into a single sample in [0.0, 1.0] using
    /// the non-linear lookup formula from the NESdev wiki.
    ///
    /// Returns 0.0 until the channels are fully implemented.
    pub fn output(&self) -> f32 {
        let p1 = f32::from(self.pulse1.output());
        let p2 = f32::from(self.pulse2.output());
        let tri = f32::from(self.triangle.output());
        let noise = f32::from(self.noise.output());
        let dmc = f32::from(self.dmc.output());

        let pulse_out = if p1 + p2 > 0.0 {
            95.88 / (8128.0 / (p1 + p2) + 100.0)
        } else {
            0.0
        };
        let tnd_sum = tri / 8227.0 + noise / 12241.0 + dmc / 22638.0;
        let tnd_out = if tnd_sum > 0.0 {
            159.79 / (1.0 / tnd_sum + 100.0)
        } else {
            0.0
        };
        pulse_out + tnd_out
    }
}
