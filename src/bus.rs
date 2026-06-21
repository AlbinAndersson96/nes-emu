use crate::apu::Apu;
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
///   $4000–$4013  APU registers
///   $4014        OAM DMA
///   $4015        APU status
///   $4016        Controller 1
///   $4017        Controller 2 / APU frame counter
///   $4018–$401F  Disabled
///   $4020–$FFFF  Cartridge (mapper, PRG-ROM, WRAM)
pub struct Bus {
    ram: [u8; 2048],
    cartridge: Option<Cartridge>,
    pub ppu: Ppu,
    pub apu: Apu,
    controller_latch: [u8; 2],
    controller_shift: [u8; 2],
    /// CPU cycles to stall after an OAM DMA write to $4014.
    /// 513 cycles normally; 514 on an odd CPU cycle. The run loop consumes this.
    oam_dma_stall: u16,
    /// CPU cycles to stall when the DMC DMA fetches a sample byte (4 cycles).
    dmc_dma_stall: u8,
}

impl Bus {
    pub fn new() -> Self {
        Self {
            ram: [0u8; 2048],
            cartridge: None,
            ppu: Ppu::new(),
            apu: Apu::new(),
            controller_latch: [0u8; 2],
            controller_shift: [0u8; 2],
            oam_dma_stall: 0,
            dmc_dma_stall: 0,
        }
    }

    /// Returns the number of CPU cycles to stall after an OAM DMA, then clears it.
    /// The caller is responsible for advancing PPU/APU by the stall amount.
    pub fn take_oam_dma_stall(&mut self) -> u16 {
        let s = self.oam_dma_stall;
        self.oam_dma_stall = 0;
        s
    }

    pub fn insert_cartridge(&mut self, cartridge: Cartridge) {
        self.ppu.set_mirroring(cartridge.mirroring());
        self.cartridge = Some(cartridge);
    }

    /// Tick the PPU by `cycles` CPU cycles. Returns true if an NMI should fire.
    /// Also refreshes the PPU's mirroring mode so mapper-driven changes take effect.
    pub fn tick_ppu(&mut self, cycles: u64) -> bool {
        if let Some(ref cart) = self.cartridge {
            self.ppu.set_mirroring(cart.mirroring());
        }
        self.ppu.tick(cycles, self.cartridge.as_mut());
        self.ppu.take_nmi()
    }

    /// Advance the APU by `cpu_cycles`. Returns true if an IRQ should be raised.
    /// If the DMC reader needs a byte, fetches it from CPU memory and supplies it;
    /// sets `dmc_dma_stall` to 4 so the caller can stall the CPU accordingly.
    pub fn tick_apu(&mut self, cpu_cycles: u64) -> bool {
        let irq = self.apu.tick(cpu_cycles as u32);
        if self.apu.dmc_needs_dma() {
            let addr = self.apu.dmc_dma_address();
            let data = self.read(addr);
            self.apu.dmc_supply_byte(data);
            self.dmc_dma_stall = 4;
        }
        irq
    }

    /// Returns DMC DMA stall cycles accumulated since the last call, then resets.
    pub fn take_dmc_dma_stall(&mut self) -> u8 {
        let s = self.dmc_dma_stall;
        self.dmc_dma_stall = 0;
        s
    }

    /// Strobe the controller shift registers. Writing 1 to bit 0 of $4016
    /// continuously reloads the latch; writing 0 freezes it and starts serial read.
    pub fn set_controller_state(&mut self, port: usize, buttons: u8) {
        if port < 2 {
            self.controller_latch[port] = buttons;
        }
    }

    /// Execute an OAM DMA transfer: copy 256 bytes from `page` of CPU RAM into OAM.
    /// Sets oam_dma_stall so the run loop stalls the CPU for 513 cycles (the
    /// extra +1 for odd CPU cycles is applied by the run loop when it consumes it).
    fn oam_dma(&mut self, page: u8) {
        let base = (page as u16) << 8;
        for offset in 0u16..256 {
            let data = self.read(base | offset);
            self.ppu.oam_dma_write(offset as u8, data);
        }
        self.oam_dma_stall = 513;
    }
}

impl CpuBus for Bus {
    fn read(&mut self, addr: u16) -> u8 {
        match addr {
            // Internal RAM + mirrors
            0x0000..=0x1FFF => self.ram[(addr & 0x07FF) as usize],

            // PPU registers + mirrors (every 8 bytes)
            0x2000..=0x3FFF => self.ppu.read_register((addr & 0x0007) as u8, self.cartridge.as_ref()),

            // APU / I/O
            0x4000..=0x4015 => self.apu.read(addr),
            0x4016 => self.controller_shift[0] & 0x01,
            0x4017 => self.controller_shift[1] & 0x01,

            // Disabled region
            0x4018..=0x401F => 0,

            // Cartridge
            0x4020..=0xFFFF => self.cartridge.as_ref().map_or(0, |c| c.read(addr)),
        }
    }

    fn write(&mut self, addr: u16, data: u8) {
        match addr {
            // Internal RAM + mirrors
            0x0000..=0x1FFF => self.ram[(addr & 0x07FF) as usize] = data,

            // PPU registers + mirrors
            0x2000..=0x3FFF => self.ppu.write_register((addr & 0x0007) as u8, data, self.cartridge.as_mut()),

            // APU registers
            0x4000..=0x4013 => self.apu.write(addr, data),

            // OAM DMA
            0x4014 => self.oam_dma(data),

            // APU status
            0x4015 => self.apu.write(addr, data),

            0x4016 => {
                // Controller strobe: reload shift registers while bit 0 is set
                if data & 0x01 != 0 {
                    self.controller_shift = self.controller_latch;
                }
            }
            0x4017 => self.apu.write(0x4017, data),

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
