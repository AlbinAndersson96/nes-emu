mod instructions;

#[derive(Debug, Clone, Copy)]
pub(in crate::cpu) enum MicroOp {
    // Placeholder: executes entire remaining instruction at once (transitional).
    RunInstruction(u8), // carries the opcode
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
    /// Set by CLI/PLP when FLAG_I transitions 1→0. Suppresses the IRQ at the
    /// START of the next instruction so that instruction executes first. After
    /// that instruction, the IRQ fires with the current (possibly modified) P.
    pub(in crate::cpu) irq_inhibit_next: bool,
    /// Set by RTI when it restores FLAG_I=1. Prevents the deferred IRQ from
    /// firing after the latency instruction, because RTI's P restore has
    /// immediate effect on interrupt polling (unlike CLI/SEI/PLP which delay).
    pub(in crate::cpu) irq_deferred_blocked: bool,
    pub(in crate::cpu) queue: [MicroOp; 8],
    pub(in crate::cpu) queue_len: u8,
    pub(in crate::cpu) queue_head: u8,
    pub(in crate::cpu) scratch: [u8; 4],
    pub(in crate::cpu) scratch_len: u8,
    pub(in crate::cpu) pending_nmi: bool,
    pub(in crate::cpu) pending_irq: bool,
    /// Mirrors the deferred-IRQ path in the old step(): set when irq && inhibit,
    /// so the IRQ fires after the latency instruction rather than before it.
    pub(in crate::cpu) pending_deferred_irq: bool,
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
            irq_deferred_blocked: false,
            queue: [MicroOp::RunInstruction(0); 8],
            queue_len: 0,
            queue_head: 0,
            scratch: [0u8; 4],
            scratch_len: 0,
            pending_nmi: false,
            pending_irq: false,
            pending_deferred_irq: false,
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

    /// Advance exactly one bus cycle.
    pub fn tick(&mut self, bus: &mut dyn Bus) {
        if self.queue_len == 0 {
            // Check for NMI/IRQ that preempt the next instruction fetch,
            // mirroring the early-return paths in the old step().
            if self.nmi_pending {
                self.nmi_pending = false;
                let c = self.service_interrupt(bus, 0xFFFA);
                self.cycles += c as u64;
                return;
            }
            // IRQ is level-triggered: consume the pending flag every step
            // regardless of FLAG_I so a masked IRQ doesn't linger.
            let irq = self.irq_pending;
            self.irq_pending = false;
            // CLI/PLP: irq_inhibit_next suppresses immediate IRQ for one step.
            let inhibit = self.irq_inhibit_next;
            self.irq_inhibit_next = false;
            if irq && !inhibit && !self.flag(FLAG_I) {
                let c = self.service_interrupt(bus, 0xFFFE);
                self.cycles += c as u64;
                return;
            }
            // Deferred IRQ: mirrors the old step() path where inhibit && irq
            // causes the IRQ to fire AFTER the next instruction (latency).
            if irq && inhibit {
                self.pending_deferred_irq = true;
            }

            // Fetch opcode and queue a single RunInstruction op.
            let opcode = self.fetch(bus);
            self.queue_head = 0;
            self.queue_len = 0;
            self.enqueue(MicroOp::RunInstruction(opcode));
            // Do NOT add +1 here: execute() already returns the total cycle count
            // including the opcode-fetch cycle.
            return;
        }

        // Pop the front micro-op.
        let op = self.queue[self.queue_head as usize];
        self.queue_head = (self.queue_head + 1) % 8;
        self.queue_len -= 1;

        match op {
            MicroOp::RunInstruction(opcode) => {
                // Execute remaining cycles of this instruction all at once
                // (transitional: will be replaced per-opcode in later tasks).
                let cycles = instructions::execute(self, bus, opcode);
                self.cycles += cycles as u64;

                // Deferred IRQ service: fire the pending IRQ after the latency
                // instruction (the one that followed CLI/PLP). Mirrors the
                // `inhibit && irq && !blocked` path in the old step().
                let blocked = self.irq_deferred_blocked;
                self.irq_deferred_blocked = false;
                if self.pending_deferred_irq {
                    self.pending_deferred_irq = false;
                    if !blocked {
                        let c = self.service_interrupt(bus, 0xFFFE);
                        self.cycles += c as u64;
                    }
                }
            }
        }
    }

    pub fn step(&mut self, bus: &mut dyn Bus) -> u8 {
        let cycles_before = self.cycles;
        // Advance one full instruction (or one interrupt service sequence).
        self.tick(bus); // either: services NMI/IRQ, OR fetches opcode + queues RunInstruction
        // If queue is non-empty, the first tick fetched an opcode; now execute it.
        while self.queue_len > 0 {
            self.tick(bus);
        }
        (self.cycles - cycles_before) as u8
    }

    fn service_interrupt(&mut self, bus: &mut dyn Bus, vector: u16) -> u8 {
        self.push_u16(bus, self.pc);
        let p = (self.p & !FLAG_B) | FLAG_U;
        self.push(bus, p);
        self.set_flag(FLAG_I, true);
        self.pc = self.read_u16(bus, vector);
        7
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
