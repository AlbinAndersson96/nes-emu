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
    pub(in crate::cpu) scratch: [u8; 4],
    pub(in crate::cpu) scratch_len: u8,
    pub(in crate::cpu) pending_nmi: bool,
    pub(in crate::cpu) pending_irq: bool,
    /// Correct high byte of the branch target when a page crossing occurs.
    /// Set by the branch() helper; consumed by BranchPageFix.
    pub(in crate::cpu) branch_target_hi: u8,
    /// Set by VectorFetch when NMI hijacks an in-progress IRQ/BRK service.
    /// Cleared by VectorFetchHi.
    pub(in crate::cpu) nmi_redirect: bool,
    /// Holds the address of the vector low byte chosen by VectorFetch (possibly
    /// redirected from IRQ to NMI vector). VectorFetchHi reads addr+1 from here.
    pub(in crate::cpu) vector_base: u16,
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
            scratch: [0u8; 4],
            scratch_len: 0,
            pending_nmi: false,
            pending_irq: false,
            branch_target_hi: 0,
            nmi_redirect: false,
            vector_base: 0,
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

    pub fn reset(&mut self, bus: &mut dyn Bus) {
        self.a = 0;
        self.x = 0;
        self.y = 0;
        self.sp = 0xFD;
        self.p = FLAG_U | FLAG_I;
        self.pc = self.read_u16(bus, 0xFFFC);
        self.cycles = 8;
    }

    pub(in crate::cpu) fn enqueue(&mut self, op: MicroOp) {
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
            // NMI has highest priority. Service it directly (no instruction executes).
            if self.pending_nmi {
                self.pending_nmi = false;
                let _ = bus.read(self.pc); self.cycles += 1; // T1 dummy
                let _ = bus.read(self.pc); self.cycles += 1; // T2 dummy
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

            // Fetch opcode. For branch instructions we never fire the IRQ
            // immediately (real hardware polls at T3, the penultimate cycle).
            // Instead save the flag in pending_irq and let RunInstruction /
            // BranchPageFix fire it at the correct point.
            let opcode = self.fetch(bus);

            if !inhibit && irq && !self.flag(FLAG_I) {
                if matches!(opcode, 0x10 | 0x30 | 0x50 | 0x70 | 0x90 | 0xB0 | 0xD0 | 0xF0) {
                    // Branch: defer IRQ to BranchPageFix (page-cross case) or
                    // end of RunInstruction (non-page-cross / not-taken case).
                    self.pending_irq = true;
                } else {
                    // Non-branch: IRQ fires before this instruction.
                    // T1: the opcode fetch already happened (phantom read); count it.
                    self.cycles += 1;
                    // Undo PC so the pushed return address is the interrupted instruction.
                    self.pc = self.pc.wrapping_sub(1);
                    // T2: dummy read at interrupted PC.
                    let _ = bus.read(self.pc); self.cycles += 1;
                    let p = (self.p & !FLAG_B) | FLAG_U;
                    self.queue_interrupt_sequence(0xFFFE, p);
                    return;
                }
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
                self.cycles += cycles as u64;

                // Only fire pending IRQs when no follow-up micro-op was queued.
                // If BranchPageFix was queued it will handle the abort decision.
                if self.queue_len == 0 {
                    let blocked = self.irq_deferred_blocked;
                    self.irq_deferred_blocked = false;
                    if self.pending_deferred_irq || self.pending_irq {
                        self.pending_deferred_irq = false;
                        self.pending_irq = false;
                        if !blocked {
                            self.cycles += 1; // T1 phantom
                            let _ = bus.read(self.pc); self.cycles += 1; // T2 dummy
                            let p = (self.p & !FLAG_B) | FLAG_U;
                            self.queue_interrupt_sequence(0xFFFE, p);
                        }
                    }
                }
            }
            MicroOp::BranchPageFix => {
                if self.pending_irq {
                    // IRQ aborts T4: page-fix cycle skipped, no +1 cycle.
                    // cpu.pc is already the page-wrong address — that is what
                    // gets pushed on the stack.
                    self.pending_irq = false;
                    self.cycles += 1; // T1 phantom at page_wrong_pc
                    let _ = bus.read(self.pc); self.cycles += 1; // T2 dummy
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
                // NMI can hijack any in-progress service sequence at T6.
                let real_addr = if self.pending_nmi {
                    self.pending_nmi = false;
                    self.nmi_redirect = true;
                    0xFFFA_u16
                } else {
                    addr
                };
                self.vector_base = real_addr;
                self.scratch[0] = bus.read(real_addr);
            }
            MicroOp::VectorFetchHi => {
                self.cycles += 1;
                let hi = bus.read(self.vector_base.wrapping_add(1)) as u16;
                let lo = self.scratch[0] as u16;
                self.pc = (hi << 8) | lo;
                self.nmi_redirect = false;
                self.pending_irq = false;
            }
        }
    }

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
