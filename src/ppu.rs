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
}

impl Ppu {
    pub fn new() -> Self {
        Self { cycle: 0 }
    }

    /// Advance the PPU by the given number of CPU cycles.
    pub fn tick(&mut self, cpu_cycles: u64) {
        self.cycle = self.cycle.wrapping_add(cpu_cycles);
    }

    /// Returns the PPUSTATUS byte ($2002). Bit 7 is set during VBlank.
    pub fn read_status(&self) -> u8 {
        let frame_cycle = self.cycle % CPU_CYCLES_PER_FRAME;
        if (VBLANK_START..VBLANK_END).contains(&frame_cycle) {
            0x80
        } else {
            0x00
        }
    }
}
