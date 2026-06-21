mod instructions;

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

    pub fn step(&mut self, bus: &mut dyn Bus) -> u8 {
        if self.nmi_pending {
            self.nmi_pending = false;
            return self.service_interrupt(bus, 0xFFFA);
        }
        // IRQ is level-triggered on real hardware: consume the pending flag every
        // step regardless of FLAG_I so a masked IRQ doesn't linger indefinitely.
        let irq = self.irq_pending;
        self.irq_pending = false;
        // CLI/PLP clear FLAG_I with a one-instruction delay: the change is written
        // to the register immediately, but interrupt polling still sees the old value
        // for one more instruction (irq_inhibit_next acts as the one-step latch).
        let inhibit = self.irq_inhibit_next;
        self.irq_inhibit_next = false;
        if irq && !inhibit && !self.flag(FLAG_I) {
            return self.service_interrupt(bus, 0xFFFE);
        }
        let opcode = self.fetch(bus);
        let cycles = instructions::execute(self, bus, opcode);
        // Deferred IRQ service: fire the pending IRQ after the latency instruction
        // (the one that followed CLI/PLP). At this point P reflects the current
        // state (e.g. FLAG_I=1 if SEI ran), which is what gets pushed to the stack.
        let blocked = self.irq_deferred_blocked;
        self.irq_deferred_blocked = false;
        if inhibit && irq && !blocked {
            return cycles + self.service_interrupt(bus, 0xFFFE);
        }
        cycles
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
