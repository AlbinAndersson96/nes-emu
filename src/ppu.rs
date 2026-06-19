/// Minimal PPU stub — tracks frame position to generate a VBlank flag on $2002.
///
/// NTSC timing (see docs/bus.md for the address map):
///   CPU clock:              1,789,773 Hz
///   PPU clock:              5,369,318 Hz (3× CPU)
///   PPU cycles per frame:  262 scanlines × 341 dots = 89,342
///   CPU cycles per frame:  89,342 / 3 ≈ 29,781
///
///   VBlank flag SET at scanline 241 dot 1  → PPU cycle 82,182 → CPU cycle 27,394
///   VBlank flag CLEAR at scanline 261 dot 1 → PPU cycle 89,002 → CPU cycle 29,667
const CPU_CYCLES_PER_FRAME: u64 = 29_781;
const VBLANK_START: u64 = 27_394;
const VBLANK_END: u64 = 29_667;

pub struct Ppu {
    cycle: u64,
    /// Set on the rising edge of VBlank; cleared by take_nmi().
    nmi_pending: bool,
}

impl Ppu {
    pub fn new() -> Self {
        Self { cycle: 0, nmi_pending: false }
    }

    /// Advance the PPU by the given number of CPU cycles.
    /// Detects the VBlank rising edge and latches nmi_pending.
    pub fn tick(&mut self, cpu_cycles: u64) {
        let was_in_vblank = self.in_vblank();
        self.cycle = self.cycle.wrapping_add(cpu_cycles);
        if !was_in_vblank && self.in_vblank() {
            self.nmi_pending = true;
        }
    }

    /// Returns true (and clears the latch) if VBlank just started since the last call.
    pub fn take_nmi(&mut self) -> bool {
        let v = self.nmi_pending;
        self.nmi_pending = false;
        v
    }

    fn in_vblank(&self) -> bool {
        let frame_cycle = self.cycle % CPU_CYCLES_PER_FRAME;
        (VBLANK_START..VBLANK_END).contains(&frame_cycle)
    }

    /// Returns the PPUSTATUS byte ($2002). Bit 7 is set during VBlank.
    pub fn read_status(&self) -> u8 {
        if self.in_vblank() { 0x80 } else { 0x00 }
    }
}
