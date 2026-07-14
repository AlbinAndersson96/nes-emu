mod accuracycoin;
mod bus;
mod cartridge;
mod cpu;
mod ppu;
mod ppu_roms;
mod roms;
mod sprite_hit_roms;
mod text_console_roms;

use crate::cpu::Bus as CpuBus;

/// Flat 64 KB address space used by CPU tests — no mirroring or side effects.
pub(crate) struct TestBus {
    pub mem: Box<[u8; 65536]>,
    /// Every read/write this session, in order: (address, is_write). Lets
    /// cycle-accuracy tests assert the exact bus-access sequence an
    /// instruction performs, not just its final register state.
    pub trace: Vec<(u16, bool)>,
}

impl TestBus {
    pub fn new() -> Self {
        Self {
            mem: Box::new([0u8; 65536]),
            trace: Vec::new(),
        }
    }
}

impl CpuBus for TestBus {
    fn read(&mut self, addr: u16) -> u8 {
        self.trace.push((addr, false));
        self.mem[addr as usize]
    }
    fn write(&mut self, addr: u16, data: u8) {
        self.trace.push((addr, true));
        self.mem[addr as usize] = data;
    }
}
