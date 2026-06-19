mod bus;
mod cpu;
mod roms;

use crate::cpu::Bus as CpuBus;

/// Flat 64 KB address space used by CPU tests — no mirroring or side effects.
pub(crate) struct TestBus {
    pub mem: Box<[u8; 65536]>,
}

impl TestBus {
    pub fn new() -> Self {
        Self {
            mem: Box::new([0u8; 65536]),
        }
    }
}

impl CpuBus for TestBus {
    fn read(&mut self, addr: u16) -> u8 {
        self.mem[addr as usize]
    }
    fn write(&mut self, addr: u16, data: u8) {
        self.mem[addr as usize] = data;
    }
}
