use crate::cartridge::Cartridge;
use crate::cpu::Bus as CpuBus;
use crate::ppu::Ppu;

/// The NES system bus — implements the CPU's full 16-bit address space.
///
/// Address map (see docs/bus.md for full details):
///   $0000–$07FF  2 KB internal RAM
///   $0800–$1FFF  Mirrors of RAM (addr & 0x07FF)
///   $2000–$2007  PPU registers
///   $2008–$3FFF  PPU register mirrors (addr & 0x2007)
///   $4000–$4015  APU registers
///   $4016        Controller 1
///   $4017        Controller 2 / APU frame counter
///   $4018–$401F  Disabled
///   $4020–$FFFF  Cartridge (mapper, PRG-ROM, WRAM)
pub struct Bus {
    ram: [u8; 2048],
    cartridge: Option<Cartridge>,
    pub ppu: Ppu,
    ppu_registers: [u8; 8], // write-side mirrors; reads route through Ppu where applicable
    apu_io: [u8; 24],       // $4000–$4017
    controller_latch: [u8; 2],
    controller_shift: [u8; 2],
}

impl Bus {
    pub fn new() -> Self {
        Self {
            ram: [0u8; 2048],
            cartridge: None,
            ppu: Ppu::new(),
            ppu_registers: [0u8; 8],
            apu_io: [0u8; 24],
            controller_latch: [0u8; 2],
            controller_shift: [0u8; 2],
        }
    }

    pub fn insert_cartridge(&mut self, cartridge: Cartridge) {
        self.cartridge = Some(cartridge);
    }

    /// Strobe the controller shift registers. Writing 1 to bit 0 of $4016
    /// continuously reloads the latch; writing 0 freezes it and starts serial read.
    pub fn set_controller_state(&mut self, port: usize, buttons: u8) {
        if port < 2 {
            self.controller_latch[port] = buttons;
        }
    }
}

impl CpuBus for Bus {
    fn read(&mut self, addr: u16) -> u8 {
        match addr {
            // Internal RAM + mirrors
            0x0000..=0x1FFF => self.ram[(addr & 0x07FF) as usize],

            // PPU registers + mirrors (every 8 bytes).
            // $2002 (PPUSTATUS) routes through the PPU stub for the VBlank flag;
            // all other registers return the last value written to them.
            0x2000..=0x3FFF => {
                let reg = (addr & 0x0007) as usize;
                if reg == 2 {
                    self.ppu.read_status()
                } else {
                    self.ppu_registers[reg]
                }
            }

            // APU / I/O
            0x4000..=0x4015 => self.apu_io[(addr - 0x4000) as usize],
            0x4016 => self.controller_shift[0] & 0x01,
            0x4017 => self.controller_shift[1] & 0x01,

            // Disabled region
            0x4018..=0x401F => 0,

            // Cartridge
            0x4020..=0xFFFF => self
                .cartridge
                .as_ref()
                .map_or(0, |c| c.read(addr)),
        }
    }

    fn write(&mut self, addr: u16, data: u8) {
        match addr {
            // Internal RAM + mirrors
            0x0000..=0x1FFF => self.ram[(addr & 0x07FF) as usize] = data,

            // PPU registers + mirrors
            0x2000..=0x3FFF => self.ppu_registers[(addr & 0x0007) as usize] = data,

            // APU / I/O
            0x4000..=0x4015 => self.apu_io[(addr - 0x4000) as usize] = data,
            0x4016 => {
                // Controller strobe: reload shift registers while bit 0 is set
                if data & 0x01 != 0 {
                    self.controller_shift = self.controller_latch;
                }
            }
            0x4017 => self.apu_io[0x17] = data, // APU frame counter

            // Disabled region
            0x4018..=0x401F => {}

            // Cartridge
            0x4020..=0xFFFF => {
                if let Some(ref mut c) = self.cartridge {
                    c.write(addr, data);
                }
            }
        }
    }
}
