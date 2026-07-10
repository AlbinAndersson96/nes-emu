mod instructions;

#[derive(Debug, Clone, Copy)]
pub(in crate::cpu) enum MicroOp {
    // Placeholder: executes entire remaining instruction at once (transitional).
    RunInstruction(u8), // carries the opcode
    /// T4 of a page-crossing branch. Applies the correct high byte to PC,
    /// unless an IRQ/NMI is pending — in which case the page fix is aborted
    /// and the interrupt is serviced with the page-wrong PC on the stack.
    BranchPageFix,
    /// T3/T4/T5 of an interrupt service sequence: push PCH/PCL/P to stack.
    PushPcHi,
    PushPcLo,
    PushP(u8), // carries the P value to push (FLAG_B already set or clear)
    /// T6 of interrupt service: read vector low byte. Checks pending_nmi for
    /// NMI hijack and redirects the address to 0xFFFA if so.
    VectorFetch(u16), // address of the vector low byte
    /// T7 of interrupt service: read vector high byte, set PC.
    VectorFetchHi,
}

#[derive(Debug, Clone)]
pub struct Cpu {
    pub a: u8,
    pub x: u8,
    pub y: u8,
    pub sp: u8,
    pub pc: u16,
    pub p: u8,
    pub cycles: u64,
    nmi_pending: bool,
    irq_pending: bool,
    /// Set by CLI/PLP when FLAG_I transitions 1→0. Suppresses the IRQ check
    /// at T1 of the next instruction (one-instruction latency). The APU is
    /// level-triggered so irq_pending is re-asserted by the next tick_apu call,
    /// and the IRQ fires naturally at T1 of the instruction after that.
    pub(in crate::cpu) irq_inhibit_next: bool,
    /// Set when an IRQ arrives during CLI/PLP latency (irq_inhibit_next was true).
    /// Fires at the end of the latency instruction via the RunInstruction arm.
    /// irq_deferred_blocked (set by RTI restoring FLAG_I=1) suppresses the fire.
    pub(in crate::cpu) pending_deferred_irq: bool,
    pub(in crate::cpu) irq_deferred_blocked: bool,
    pub(in crate::cpu) queue: [MicroOp; 8],
    pub(in crate::cpu) queue_len: u8,
    pub(in crate::cpu) queue_head: u8,
    /// Low byte of the interrupt vector read by VectorFetch; consumed by VectorFetchHi.
    pub(in crate::cpu) vector_lo: u8,
    pub(in crate::cpu) pending_nmi: bool,
    /// Correct high byte of the branch target when a page crossing occurs.
    /// Set by the branch() helper; consumed by BranchPageFix.
    pub(in crate::cpu) branch_target_hi: u8,
    /// Address of the vector low byte chosen by VectorFetch (possibly redirected
    /// from IRQ to NMI vector by NMI hijack). VectorFetchHi reads addr+1 from here.
    pub(in crate::cpu) vector_base: u16,
    /// Set when a taken non-page-crossing branch (3-cycle) just completed.
    /// A taken branch ignores IRQ on its last clock (T3): an IRQ that FIRST
    /// asserts on that clock (irq_asserted_on_last_cycle) is deferred until
    /// after the NEXT instruction executes. IRQs already asserted earlier in
    /// the branch service normally at the next dispatch.
    pub(in crate::cpu) branch_delay_irq: bool,
    /// Set by irq_on_last_cycle(): the IRQ line first asserted on the final
    /// cycle of the just-completed instruction. Consumed (cleared) at the next
    /// dispatch; only consulted when branch_delay_irq is also set.
    pub(in crate::cpu) irq_asserted_on_last_cycle: bool,
    /// Set by VectorFetchHi when an interrupt service sequence completes.
    /// Interrupt sequences do not poll the interrupt lines, so the handler's
    /// first instruction always executes before another interrupt can be
    /// serviced (nesdev CPU_interrupts: "The interrupt sequences themselves
    /// do not perform interrupt polling, meaning at least one instruction
    /// from the interrupt handler will execute before another interrupt is
    /// serviced"). Suppresses the pending_nmi/IRQ dispatch checks for exactly
    /// one queue-empty dispatch; the pending flags themselves survive.
    pub(in crate::cpu) interrupt_poll_suppressed: bool,
}

// P register flag masks
pub const FLAG_C: u8 = 0b0000_0001; // Carry
pub const FLAG_Z: u8 = 0b0000_0010; // Zero
pub const FLAG_I: u8 = 0b0000_0100; // Interrupt disable
pub const FLAG_D: u8 = 0b0000_1000; // Decimal (no effect on 2A03)
pub const FLAG_B: u8 = 0b0001_0000; // Break (stack only)
pub const FLAG_U: u8 = 0b0010_0000; // Unused (always 1)
pub const FLAG_V: u8 = 0b0100_0000; // Overflow
pub const FLAG_N: u8 = 0b1000_0000; // Negative

impl Cpu {
    pub fn new() -> Self {
        Self {
            a: 0,
            x: 0,
            y: 0,
            sp: 0xFD,
            pc: 0,
            p: FLAG_U | FLAG_I,
            cycles: 0,
            nmi_pending: false,
            irq_pending: false,
            irq_inhibit_next: false,
            pending_deferred_irq: false,
            irq_deferred_blocked: false,
            queue: [MicroOp::RunInstruction(0); 8],
            queue_len: 0,
            queue_head: 0,
            vector_lo: 0,
            pending_nmi: false,
            branch_target_hi: 0,
            vector_base: 0,
            branch_delay_irq: false,
            irq_asserted_on_last_cycle: false,
            interrupt_poll_suppressed: false,
        }
    }

    /// Signal a non-maskable interrupt. Serviced at the top of the next step().
    pub fn nmi(&mut self) {
        self.nmi_pending = true;
    }

    /// Signal a maskable interrupt. Serviced at the top of the next step() if FLAG_I is clear.
    pub fn irq(&mut self) {
        self.irq_pending = true;
    }

    /// Signal a maskable interrupt whose line FIRST asserted on the final cycle
    /// of the instruction that just completed. Run loops that tick the APU one
    /// cycle at a time use this instead of irq() for that specific case: a
    /// taken branch ignores IRQ on its last clock, so such an assertion is
    /// deferred past the next instruction when branch_delay_irq is set.
    pub fn irq_on_last_cycle(&mut self) {
        self.irq_pending = true;
        self.irq_asserted_on_last_cycle = true;
    }

    /// True when the IRQ line has been signalled and not yet consumed by a dispatch.
    /// Used by DMA-stalling run loops to distinguish an IRQ that was already
    /// pending when the DMA began from one that first asserted during the stall.
    pub fn irq_line_pending(&self) -> bool {
        self.irq_pending
    }

    /// True when the micro-op queue is empty, i.e. the last tick completed an
    /// instruction (or interrupt sequence) and the next tick is a dispatch.
    /// Run loops use this to tell whether a tick's final cycle was also an
    /// instruction's final cycle (see Cpu::irq_on_last_cycle).
    pub fn instruction_boundary(&self) -> bool {
        self.queue_len == 0
    }

    /// Suppress interrupt servicing for the next queue-empty dispatch (the pending
    /// flags survive; the interrupt fires one instruction later). Run loops call
    /// this when an interrupt first asserts during a DMA stall: the interrupt
    /// missed the stalled instruction's poll point (the poll happens on an
    /// instruction's penultimate cycle, before the DMA halts the CPU), so the
    /// first post-DMA instruction executes before the interrupt can be serviced.
    pub fn suppress_next_interrupt_poll(&mut self) {
        self.interrupt_poll_suppressed = true;
    }

    /// Diagnostic accessor for interrupt-timing tracers: (pending_nmi, nmi_pending, queue_len).
    #[cfg(test)]
    pub(crate) fn debug_nmi_state(&self) -> (bool, bool, u8) {
        (self.pending_nmi, self.nmi_pending, self.queue_len)
    }

    /// Diagnostic accessor for the NMI-vs-IRQ arbitration tracer:
    /// (irq_pending, irq_inhibit_next, flag(FLAG_I)).
    #[cfg(test)]
    pub(crate) fn debug_irq_state(&self) -> (bool, bool, bool) {
        (self.irq_pending, self.irq_inhibit_next, self.flag(FLAG_I))
    }

    pub fn reset(&mut self, bus: &mut dyn Bus) {
        self.a = 0;
        self.x = 0;
        self.y = 0;
        self.sp = 0xFD;
        self.p = FLAG_U | FLAG_I;
        self.pc = self.read_u16(bus, 0xFFFC);
        self.cycles = 8;
    }

    /// Simulates a real NES warm reset (the reset button) — unlike `reset`
    /// (cold power-on), A/X/Y are left untouched: only the I flag is forced
    /// set, SP is decremented by 3 (nothing is written to the stack), and PC
    /// is refetched from the reset vector. `self.cycles` is left untouched —
    /// the 7-cycle reset sequence itself must be accounted for by the caller
    /// (advance PPU/APU by 7 cycles and add 7 to `cpu.cycles` / any cycle
    /// budget), the same way callers already do for the initial `reset()` at
    /// boot.
    ///
    /// Currently only exercised by the `$81`-status handling in the ROM test
    /// harness (`src/tests/roms.rs`) and its own unit test; not yet wired
    /// into the real run loop (no in-app Reset-button UI exists yet), hence
    /// the explicit `allow` below.
    #[allow(dead_code)]
    pub fn warm_reset(&mut self, bus: &mut dyn Bus) {
        self.p |= FLAG_I;
        self.sp = self.sp.wrapping_sub(3);
        self.pc = self.read_u16(bus, 0xFFFC);
    }

    pub(in crate::cpu) fn enqueue(&mut self, op: MicroOp) {
        debug_assert!(self.queue_len < 8, "micro-op queue overflow");
        self.queue[(self.queue_head as usize + self.queue_len as usize) % 8] = op;
        self.queue_len += 1;
    }

    /// Queue the five micro-ops that form T3–T7 of an interrupt service sequence
    /// (push PCH, PCL, P; then fetch vector lo and hi). The caller handles T1–T2.
    pub(in crate::cpu) fn queue_interrupt_sequence(&mut self, vector: u16, push_p: u8) {
        self.enqueue(MicroOp::PushPcHi);
        self.enqueue(MicroOp::PushPcLo);
        self.enqueue(MicroOp::PushP(push_p));
        self.enqueue(MicroOp::VectorFetch(vector));
        self.enqueue(MicroOp::VectorFetchHi);
    }

    /// Advance exactly one bus cycle.
    pub fn tick(&mut self, bus: &mut dyn Bus) {
        // Poll NMI at the start of every tick so that if NMI arrives while
        // a BRK/IRQ service sequence is in the queue, VectorFetch can hijack it.
        if self.nmi_pending {
            self.nmi_pending = false;
            self.pending_nmi = true;
        }

        if self.queue_len == 0 {
            // If an interrupt sequence just completed (VectorFetchHi last tick),
            // skip interrupt servicing for this one dispatch: the handler's first
            // instruction always executes. Pending flags are left intact so the
            // interrupt fires at the NEXT dispatch instead.
            let poll_suppressed = self.interrupt_poll_suppressed;
            self.interrupt_poll_suppressed = false;

            // NMI has highest priority. Service it directly (no instruction executes).
            if self.pending_nmi && !poll_suppressed {
                self.pending_nmi = false;
                let _ = bus.read(self.pc);
                self.cycles += 1; // T1 dummy
                let _ = bus.read(self.pc);
                self.cycles += 1; // T2 dummy
                let p = (self.p & !FLAG_B) | FLAG_U;
                self.queue_interrupt_sequence(0xFFFA, p);
                return;
            }

            // IRQ is level-triggered: consume the pending flag every fetch.
            let irq = self.irq_pending;
            self.irq_pending = false;

            // CLI/PLP: irq_inhibit_next suppresses the IRQ check for one instruction.
            let inhibit = self.irq_inhibit_next;
            self.irq_inhibit_next = false;

            // If the IRQ arrived during that latency window, defer it: fire at the
            // end of the latency instruction (RunInstruction arm) rather than before.
            if irq && inhibit {
                self.pending_deferred_irq = true;
            }

            // Branch last-cycle IRQ delay: a taken non-page-crossing branch
            // ignores IRQ at its last clock (T3). Only an IRQ that FIRST
            // asserted on that clock is deferred to after the NEXT instruction;
            // one already asserted during T1/T2 services normally here
            // (verified against test_branch_taken rows 2-3 vs 4-8 of
            // cpu_interrupts_v2/5-branch_delays_irq).
            let branch_delay = self.branch_delay_irq;
            self.branch_delay_irq = false;
            let irq_last = self.irq_asserted_on_last_cycle;
            self.irq_asserted_on_last_cycle = false;
            let irq = if branch_delay && irq_last && irq && !inhibit && !self.flag(FLAG_I) {
                self.pending_deferred_irq = true;
                false
            } else {
                irq
            };

            // Fetch opcode. An IRQ already visible at dispatch preempts ANY
            // opcode, branches included (verified against all four sub-test
            // tables of cpu_interrupts_v2/5-branch_delays_irq: their earliest
            // rows push the branch's own address). The branch-specific timing
            // rules apply only to IRQs arriving DURING the branch: the
            // level-refresh (irq_pending) covers T2/T3 arrivals, BranchPageFix
            // aborts T4 on a page cross, and branch_delay_irq models the
            // taken-branch last-clock ignore.
            let opcode = self.fetch(bus);
            self.cycles += 1; // T1: opcode fetch cycle

            if !inhibit && irq && !poll_suppressed && !self.flag(FLAG_I) {
                // IRQ fires before this instruction.
                // T1 already counted above. Undo PC so the pushed return address
                // is the interrupted instruction's address.
                self.pc = self.pc.wrapping_sub(1);
                // T2: dummy read at interrupted PC.
                let _ = bus.read(self.pc);
                self.cycles += 1;
                let p = (self.p & !FLAG_B) | FLAG_U;
                self.queue_interrupt_sequence(0xFFFE, p);
                return;
            }

            self.queue_head = 0;
            self.queue_len = 0;
            self.enqueue(MicroOp::RunInstruction(opcode));
            return;
        }

        // Pop the front micro-op.
        let op = self.queue[self.queue_head as usize];
        self.queue_head = (self.queue_head + 1) % 8;
        self.queue_len -= 1;

        match op {
            MicroOp::RunInstruction(opcode) => {
                let cycles = instructions::execute(self, bus, opcode);
                // T1 (opcode fetch) was already counted in the queue-empty path.
                self.cycles += (cycles - 1) as u64;

                // A taken non-page-crossing branch (3 cycles, no BranchPageFix queued)
                // ignores IRQ at its last clock (T3). If an IRQ arrives during this
                // RunInstruction, mark that the IRQ must be deferred by one instruction.
                let is_branch = matches!(
                    opcode,
                    0x10 | 0x30 | 0x50 | 0x70 | 0x90 | 0xB0 | 0xD0 | 0xF0
                );
                if is_branch && cycles == 3 && self.queue_len == 0 {
                    self.branch_delay_irq = true;
                }

                // Only fire pending IRQs when no follow-up micro-op was queued.
                // If BranchPageFix was queued it will handle the abort decision.
                if self.queue_len == 0 {
                    let blocked = self.irq_deferred_blocked;
                    self.irq_deferred_blocked = false;
                    if self.pending_deferred_irq {
                        self.pending_deferred_irq = false;
                        if !blocked {
                            self.cycles += 1; // T1 phantom
                            let _ = bus.read(self.pc);
                            self.cycles += 1; // T2 dummy
                            let p = (self.p & !FLAG_B) | FLAG_U;
                            self.queue_interrupt_sequence(0xFFFE, p);
                        }
                    }
                }
            }
            MicroOp::BranchPageFix => {
                // irq_pending: the IRQ line asserted during the branch's T1-T3
                // (an IRQ already visible at dispatch preempts the branch like
                // any other instruction and never reaches this micro-op).
                // Aborts T4 if the IRQ line is not masked.
                let irq_abort = !self.flag(FLAG_I) && self.irq_pending;
                if irq_abort {
                    // IRQ aborts T4: the fixup cycle still elapses, but the page
                    // fix is not applied — cpu.pc stays the page-wrong address,
                    // and that is what gets pushed on the stack. The full 7-cycle
                    // interrupt sequence follows (handler entry at branch T1
                    // + 11), calibrated against the CK column of
                    // cpu_interrupts_v2/5-branch_delays_irq's
                    // test_branch_taken_pagecross rows 2-5, whose readme values
                    // are consistent only with the aborted cycle elapsing.
                    self.irq_pending = false;
                    self.cycles += 1; // aborted fixup cycle (no page fix applied)
                    self.cycles += 1; // T1 phantom at page_wrong_pc
                    let _ = bus.read(self.pc);
                    self.cycles += 1; // T2 dummy
                    let p = (self.p & !FLAG_B) | FLAG_U;
                    self.queue_interrupt_sequence(0xFFFE, p);
                } else {
                    // Normal page fix: T4 cycle + correct high byte.
                    self.cycles += 1;
                    self.pc = (self.pc & 0x00FF) | ((self.branch_target_hi as u16) << 8);
                }
            }
            MicroOp::PushPcHi => {
                self.cycles += 1;
                let hi = (self.pc >> 8) as u8;
                self.push(bus, hi);
            }
            MicroOp::PushPcLo => {
                self.cycles += 1;
                let lo = self.pc as u8;
                self.push(bus, lo);
            }
            MicroOp::PushP(p) => {
                self.cycles += 1;
                self.push(bus, p);
                self.set_flag(FLAG_I, true);
            }
            MicroOp::VectorFetch(addr) => {
                self.cycles += 1;
                // NMI can hijack any in-progress service sequence at T6. (nesdev's
                // CPU_interrupts page documents real hardware's checkpoint as the
                // T4/T5 boundary — "first four ticks" — but our harness's NMI-edge
                // delivery already has its own one-tick defer baked in (to model
                // edges landing on an instruction's own last cycle correctly), and
                // empirically checking here at T6 is what reproduces the readme's
                // expected table; checking one tick earlier at T5/PushP instead
                // requires an edge to have arrived a full tick earlier than
                // hardware's rule to hijack — tested and reverted, see
                // docs/investigations/cpu_interrupt_debug_log.md.)
                let real_addr = if self.pending_nmi {
                    self.pending_nmi = false;
                    0xFFFA_u16
                } else {
                    addr
                };
                self.vector_base = real_addr;
                self.vector_lo = bus.read(real_addr);
            }
            MicroOp::VectorFetchHi => {
                self.cycles += 1;
                let hi = bus.read(self.vector_base.wrapping_add(1)) as u16;
                let lo = self.vector_lo as u16;
                self.pc = (hi << 8) | lo;
                self.interrupt_poll_suppressed = true;
            }
        }
    }

    #[cfg(test)]
    pub(crate) fn step(&mut self, bus: &mut dyn Bus) -> u8 {
        let cycles_before = self.cycles;
        // Advance one full instruction (or one interrupt service sequence).
        self.tick(bus); // either: services NMI/IRQ, OR fetches opcode + queues RunInstruction
        // If queue is non-empty, the first tick fetched an opcode; now execute it.
        while self.queue_len > 0 {
            self.tick(bus);
        }
        (self.cycles - cycles_before) as u8
    }

    // --- flag helpers ---

    pub fn flag(&self, mask: u8) -> bool {
        self.p & mask != 0
    }

    pub fn set_flag(&mut self, mask: u8, value: bool) {
        if value {
            self.p |= mask;
        } else {
            self.p &= !mask;
        }
    }

    pub fn set_nz(&mut self, value: u8) {
        self.set_flag(FLAG_Z, value == 0);
        self.set_flag(FLAG_N, value & 0x80 != 0);
    }

    // --- memory ---

    pub fn read(&self, bus: &mut dyn Bus, addr: u16) -> u8 {
        bus.read(addr)
    }

    pub fn write(&mut self, bus: &mut dyn Bus, addr: u16, data: u8) {
        bus.write(addr, data);
    }

    pub fn read_u16(&self, bus: &mut dyn Bus, addr: u16) -> u16 {
        let lo = bus.read(addr) as u16;
        let hi = bus.read(addr.wrapping_add(1)) as u16;
        (hi << 8) | lo
    }

    // Replicates the JMP ($xxFF) page-wrap hardware bug.
    pub fn read_u16_bugged(&self, bus: &mut dyn Bus, addr: u16) -> u16 {
        let lo = bus.read(addr) as u16;
        let hi_addr = (addr & 0xFF00) | ((addr.wrapping_add(1)) & 0x00FF);
        let hi = bus.read(hi_addr) as u16;
        (hi << 8) | lo
    }

    pub fn fetch(&mut self, bus: &mut dyn Bus) -> u8 {
        let byte = bus.read(self.pc);
        self.pc = self.pc.wrapping_add(1);
        byte
    }

    pub fn fetch_u16(&mut self, bus: &mut dyn Bus) -> u16 {
        let lo = self.fetch(bus) as u16;
        let hi = self.fetch(bus) as u16;
        (hi << 8) | lo
    }

    // --- stack ---

    pub fn push(&mut self, bus: &mut dyn Bus, data: u8) {
        bus.write(0x0100 | self.sp as u16, data);
        self.sp = self.sp.wrapping_sub(1);
    }

    pub fn pop(&mut self, bus: &mut dyn Bus) -> u8 {
        self.sp = self.sp.wrapping_add(1);
        bus.read(0x0100 | self.sp as u16)
    }

    pub fn push_u16(&mut self, bus: &mut dyn Bus, data: u16) {
        self.push(bus, (data >> 8) as u8);
        self.push(bus, (data & 0xFF) as u8);
    }

    pub fn pop_u16(&mut self, bus: &mut dyn Bus) -> u16 {
        let lo = self.pop(bus) as u16;
        let hi = self.pop(bus) as u16;
        (hi << 8) | lo
    }

    // --- addressing modes ---
    // Each returns the effective address and whether a page was crossed.
    // `pub(in crate::cpu)` — used only by instructions.rs; not part of the public API.

    pub(in crate::cpu) fn addr_zero_page(&mut self, bus: &mut dyn Bus) -> (u16, bool) {
        (self.fetch(bus) as u16, false)
    }

    pub(in crate::cpu) fn addr_zero_page_x(&mut self, bus: &mut dyn Bus) -> (u16, bool) {
        let base = self.fetch(bus);
        (base.wrapping_add(self.x) as u16, false)
    }

    pub(in crate::cpu) fn addr_zero_page_y(&mut self, bus: &mut dyn Bus) -> (u16, bool) {
        let base = self.fetch(bus);
        (base.wrapping_add(self.y) as u16, false)
    }

    pub(in crate::cpu) fn addr_absolute(&mut self, bus: &mut dyn Bus) -> (u16, bool) {
        (self.fetch_u16(bus), false)
    }

    pub(in crate::cpu) fn addr_absolute_x(&mut self, bus: &mut dyn Bus) -> (u16, bool) {
        let base = self.fetch_u16(bus);
        let addr = base.wrapping_add(self.x as u16);
        let crossed = page_crossed(base, addr);
        if crossed {
            let precarry = (base & 0xFF00) | (addr & 0x00FF);
            let _ = bus.read(precarry);
        }
        (addr, crossed)
    }

    pub(in crate::cpu) fn addr_absolute_y(&mut self, bus: &mut dyn Bus) -> (u16, bool) {
        let base = self.fetch_u16(bus);
        let addr = base.wrapping_add(self.y as u16);
        let crossed = page_crossed(base, addr);
        if crossed {
            let precarry = (base & 0xFF00) | (addr & 0x00FF);
            let _ = bus.read(precarry);
        }
        (addr, crossed)
    }

    /// Absolute,X addressing for store instructions.
    /// Always performs a dummy read at the pre-carry address (stores are always 5 cycles
    /// regardless of page crossing; the spurious read happens on cycle 4 unconditionally).
    pub(in crate::cpu) fn addr_absolute_x_store(&mut self, bus: &mut dyn Bus) -> u16 {
        let base = self.fetch_u16(bus);
        let addr = base.wrapping_add(self.x as u16);
        let precarry = (base & 0xFF00) | (addr & 0x00FF);
        let _ = bus.read(precarry);
        addr
    }

    /// Absolute,Y addressing for store instructions.
    /// Same unconditional pre-carry dummy read as addr_absolute_x_store.
    pub(in crate::cpu) fn addr_absolute_y_store(&mut self, bus: &mut dyn Bus) -> u16 {
        let base = self.fetch_u16(bus);
        let addr = base.wrapping_add(self.y as u16);
        let precarry = (base & 0xFF00) | (addr & 0x00FF);
        let _ = bus.read(precarry);
        addr
    }

    /// Absolute,X addressing for read-modify-write instructions.
    /// Always performs a dummy read at the pre-carry address (observable side
    /// effect — this read happens in cycle 4 even when no page is crossed).
    pub(in crate::cpu) fn addr_absolute_x_rmw(&mut self, bus: &mut dyn Bus) -> u16 {
        let base = self.fetch_u16(bus);
        let addr = base.wrapping_add(self.x as u16);
        let precarry = (base & 0xFF00) | (addr & 0x00FF);
        let _ = bus.read(precarry);
        addr
    }

    /// Absolute,Y addressing for read-modify-write instructions.
    /// Same pre-carry dummy read as addr_absolute_x_rmw.
    pub(in crate::cpu) fn addr_absolute_y_rmw(&mut self, bus: &mut dyn Bus) -> u16 {
        let base = self.fetch_u16(bus);
        let addr = base.wrapping_add(self.y as u16);
        let precarry = (base & 0xFF00) | (addr & 0x00FF);
        let _ = bus.read(precarry);
        addr
    }

    pub(in crate::cpu) fn addr_indirect_x(&mut self, bus: &mut dyn Bus) -> (u16, bool) {
        let base = self.fetch(bus) as u16;
        // Spurious read at the non-indexed ZP address (cycle 3 of the sequence).
        let _ = bus.read(base);
        let ptr = (base + self.x as u16) & 0x00FF;
        let lo = bus.read(ptr) as u16;
        let hi = bus.read((ptr + 1) & 0x00FF) as u16;
        ((hi << 8) | lo, false)
    }

    pub(in crate::cpu) fn addr_indirect_y(&mut self, bus: &mut dyn Bus) -> (u16, bool) {
        let ptr = self.fetch(bus) as u16;
        let lo = bus.read(ptr) as u16;
        let hi = bus.read(ptr.wrapping_add(1) & 0x00FF) as u16;
        let base = (hi << 8) | lo;
        let addr = base.wrapping_add(self.y as u16);
        let crossed = page_crossed(base, addr);
        if crossed {
            // On a page crossing the 6502 reads the pre-carry address before
            // fetching from the true effective address. The read is observable
            // (e.g. it clears $2002's VBlank flag on a PPU address).
            let precarry = (base & 0xFF00) | (addr & 0x00FF);
            let _ = bus.read(precarry);
        }
        (addr, crossed)
    }

    /// (Indirect),Y addressing for store instructions.
    /// Stores ALWAYS perform a dummy read at the pre-carry address (cycle 5),
    /// regardless of whether a page is crossed.
    pub(in crate::cpu) fn addr_indirect_y_store(&mut self, bus: &mut dyn Bus) -> u16 {
        let ptr = self.fetch(bus) as u16;
        let lo = bus.read(ptr) as u16;
        let hi = bus.read(ptr.wrapping_add(1) & 0x00FF) as u16;
        let base = (hi << 8) | lo;
        let addr = base.wrapping_add(self.y as u16);
        let precarry = (base & 0xFF00) | (addr & 0x00FF);
        let _ = bus.read(precarry);
        addr
    }
}

pub trait Bus {
    /// Reads a byte. Takes `&mut self` because some registers have read side effects
    /// (e.g. reading $2002 clears the VBlank flag on real hardware).
    fn read(&mut self, addr: u16) -> u8;
    fn write(&mut self, addr: u16, data: u8);
}

fn page_crossed(a: u16, b: u16) -> bool {
    (a & 0xFF00) != (b & 0xFF00)
}
