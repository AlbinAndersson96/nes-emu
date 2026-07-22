use crate::bus::Bus;
use crate::cpu::Cpu;
use serde::{Deserialize, Serialize};

/// Drives the CPU, PPU, and APU in lockstep, one CPU tick (or one DMA stall
/// cycle) per `step` call, with the interrupt-delivery rules that were
/// verified against the blargg `cpu_interrupts_v2` suite (see
/// `docs/cpu_interrupts.md`):
///
/// - **Deferred NMI edges**: an NMI edge landing on a tick's own last cycle
///   isn't visible to the 6502's interrupt poll (sampled going into a cycle,
///   using state from before it begins) — it is delivered one tick later than
///   an edge on an earlier cycle would be.
/// - **Per-cycle APU ticking**: an IRQ line that FIRST asserts on an
///   instruction's final cycle is flagged via `Cpu::irq_on_last_cycle`, so a
///   taken branch can ignore an IRQ on its last clock while servicing one
///   asserted earlier normally.
/// - **DMA interrupt deferral**: interrupts that first assert during a DMA
///   stall missed the stalled instruction's poll point. They are delivered
///   when the DMA ends with the CPU's next dispatch poll suppressed, so the
///   first post-DMA instruction executes before the interrupt is serviced. An
///   IRQ already pending when the DMA began services immediately after it.
///
/// Holds the loop-persistent state those rules need; create one per powered-on
/// machine and feed every tick through it.
#[derive(Serialize, Deserialize)]
pub struct SystemClock {
    deferred_nmi: bool,
    dma_nmi_deferred: bool,
    dma_irq_deferred: bool,
    in_dma: bool,
}

/// What a single `SystemClock::step` did — cycle count for pacing.
pub struct StepResult {
    pub cycles: u64,
}

impl SystemClock {
    pub fn new() -> Self {
        Self {
            deferred_nmi: false,
            dma_nmi_deferred: false,
            dma_irq_deferred: false,
            in_dma: false,
        }
    }

    /// Advance the machine by one CPU tick, or by one cycle of a DMA stall.
    pub fn step(&mut self, cpu: &mut Cpu, bus: &mut Bus) -> StepResult {
        let cycles_before = cpu.cycles;

        if bus.dma_active() {
            self.in_dma = true;
            bus.tick_dma();
            if bus.tick_ppu(1) {
                self.dma_nmi_deferred = true;
            }
            if bus.tick_apu(1) && !cpu.irq_line_pending() {
                self.dma_irq_deferred = true;
            }
            return StepResult { cycles: 1 };
        }

        if self.in_dma {
            self.in_dma = false;
            if self.dma_nmi_deferred || self.dma_irq_deferred {
                if self.dma_nmi_deferred {
                    cpu.nmi();
                }
                if self.dma_irq_deferred {
                    cpu.irq();
                }
                cpu.suppress_next_interrupt_poll();
            }
            self.dma_nmi_deferred = false;
            self.dma_irq_deferred = false;
        }

        cpu.tick(bus);
        let delta = cpu.cycles - cycles_before;

        // Consume any PPU cycles pre-advanced during a $2002 read, then tick
        // the remaining cycles one-at-a-time for accurate NMI delivery.
        let (extra, extra_nmi) = bus.take_ppu_preadvance();
        let mut got_nmi = extra_nmi;
        let remaining = delta.saturating_sub(extra as u64);
        let mut new_deferred = false;
        for i in 0..remaining {
            if bus.tick_ppu(1) {
                if i + 1 == remaining {
                    new_deferred = true;
                } else {
                    got_nmi = true;
                }
            }
        }
        if self.deferred_nmi {
            got_nmi = true;
        }
        self.deferred_nmi = new_deferred;
        if got_nmi {
            cpu.nmi();
        }

        // Tick the APU one cycle at a time so an IRQ line that FIRST asserts
        // on the instruction's final cycle can be flagged: a taken branch
        // ignores IRQ on its last clock (cpu.irq_on_last_cycle +
        // branch_delay_irq), while assertions on earlier cycles service
        // normally at the next dispatch.
        let instr_done = cpu.instruction_boundary();
        for i in 0..delta {
            if bus.tick_apu(1) {
                if !cpu.irq_line_pending() && instr_done && i + 1 == delta {
                    cpu.irq_on_last_cycle();
                } else {
                    cpu.irq();
                }
            } else {
                // Level-sensed IRQ: the line is low this cycle, so withdraw any
                // latched-but-unserviced IRQ. Matches the 6502 sampling the IRQ
                // level at each poll — a source cleared mid-instruction (e.g. an
                // implied op's $4015 dummy read clearing the frame-IRQ flag) is
                // not pending at the next dispatch.
                cpu.irq_deassert();
            }
        }

        StepResult { cycles: delta }
    }
}

impl Default for SystemClock {
    fn default() -> Self {
        Self::new()
    }
}
