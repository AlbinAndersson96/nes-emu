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
    /// OAM DMA state: set when a write to $4014 triggers a 513/514-cycle DMA.
    oam_dma_active: bool,
    oam_dma_cycles_left: u16,
    /// Total stall length of the current OAM DMA (513, or 514 for odd-cycle starts).
    oam_dma_len: u16,
    oam_dma_page: u8,
    oam_dma_byte_idx: u16,
    /// DMC DMA: counts down 4 cycles while the CPU is stalled for a sample fetch.
    dmc_dma_cycles_left: u8,
    /// CPU cycles by which the PPU was pre-advanced during a $2002 read (to
    /// simulate T4-read timing). Consumed by the run loop to avoid double-advancing.
    ppu_preadvance_cycles: u32,
    /// NMI that fired during a $2002 pre-advance; the run loop must deliver it.
    ppu_preadvance_nmi: bool,
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
            oam_dma_active: false,
            oam_dma_cycles_left: 0,
            oam_dma_len: 0,
            oam_dma_page: 0,
            oam_dma_byte_idx: 0,
            dmc_dma_cycles_left: 0,
            ppu_preadvance_cycles: 0,
            ppu_preadvance_nmi: false,
        }
    }

    /// Returns true when OAM or DMC DMA is in progress (CPU must be stalled).
    pub fn dma_active(&self) -> bool {
        self.oam_dma_active || self.dmc_dma_cycles_left > 0
    }

    /// Advance one DMA cycle. OAM DMA copies one byte every two cycles;
    /// DMC DMA simply counts down and fetches the byte on the last cycle.
    pub fn tick_dma(&mut self) {
        if self.oam_dma_active {
            // Wait cycles first (1, or 2 for an odd-cycle start), then one byte
            // copied every second cycle, aligned to the same get/put phase.
            let cycle_num = self.oam_dma_len - self.oam_dma_cycles_left;
            let waits = self.oam_dma_len - 512;
            if cycle_num >= waits && (cycle_num - waits) % 2 == 1 && self.oam_dma_byte_idx < 256 {
                let addr = ((self.oam_dma_page as u16) << 8) | self.oam_dma_byte_idx;
                let data = self.read(addr);
                self.ppu.oam_dma_write(self.oam_dma_byte_idx as u8, data);
                self.oam_dma_byte_idx += 1;
            }
            self.oam_dma_cycles_left -= 1;
            if self.oam_dma_cycles_left == 0 {
                self.oam_dma_active = false;
            }
        } else if self.dmc_dma_cycles_left > 0 {
            self.dmc_dma_cycles_left -= 1;
            if self.dmc_dma_cycles_left == 0 && self.apu.dmc_needs_dma() {
                let addr = self.apu.dmc_dma_address();
                let data = self.read(addr);
                self.apu.dmc_supply_byte(data);
            }
        }
    }

    pub fn insert_cartridge(&mut self, cartridge: Cartridge) {
        self.ppu.set_mirroring(cartridge.mirroring());
        self.cartridge = Some(cartridge);
    }

    /// Consume pre-advanced PPU cycles and any NMI that fired during a $2002 read.
    /// Returns `(cycles_already_advanced, nmi_pending)`. The run loop must subtract
    /// `cycles_already_advanced` from its post-tick `tick_ppu` call and deliver the
    /// NMI if `nmi_pending` is true.
    pub fn take_ppu_preadvance(&mut self) -> (u32, bool) {
        let c = self.ppu_preadvance_cycles;
        let n = self.ppu_preadvance_nmi;
        self.ppu_preadvance_cycles = 0;
        self.ppu_preadvance_nmi = false;
        (c, n)
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
    /// If the DMC reader needs a byte, arms a 4-cycle DMA stall; tick_dma() will
    /// fetch and supply the byte on the final cycle of the stall.
    pub fn tick_apu(&mut self, cpu_cycles: u64) -> bool {
        let irq = self.apu.tick(cpu_cycles as u32);
        if self.apu.dmc_needs_dma() && self.dmc_dma_cycles_left == 0 {
            self.dmc_dma_cycles_left = 4;
        }
        irq
    }

    /// Strobe the controller shift registers. Writing 1 to bit 0 of $4016
    /// continuously reloads the latch; writing 0 freezes it and starts serial read.
    pub fn set_controller_state(&mut self, port: usize, buttons: u8) {
        if port < 2 {
            self.controller_latch[port] = buttons;
        }
    }

    /// Initiate an OAM DMA transfer triggered by a write to $4014.
    /// The stall is 513 cycles (1 wait + 256 read/write pairs), +1 when the
    /// write lands on an odd CPU cycle relative to the APU's divide-by-2
    /// get/put clock. The APU's free-running counter supplies that parity;
    /// it currently sits at the START of the writing instruction (tick_apu
    /// runs after the whole instruction), and the $4014 write cycle is the
    /// 4th/last cycle of an absolute store — an even offset, so the counter's
    /// parity now equals the write cycle's parity.
    fn oam_dma(&mut self, page: u8) {
        self.oam_dma_active = true;
        self.oam_dma_len = if self.apu.cycle_parity() { 514 } else { 513 };
        self.oam_dma_cycles_left = self.oam_dma_len;
        self.oam_dma_page = page;
        self.oam_dma_byte_idx = 0;
    }
}

impl CpuBus for Bus {
    fn read(&mut self, addr: u16) -> u8 {
        match addr {
            // Internal RAM + mirrors
            0x0000..=0x1FFF => self.ram[(addr & 0x07FF) as usize],

            // PPU registers + mirrors (every 8 bytes)
            0x2000..=0x3FFF => {
                let reg = (addr & 0x0007) as u8;
                if reg == 2 {
                    // $2002 is read at T4 on real hardware (3 CPU cycles after T1).
                    // Pre-advance the PPU so the read sees the T4 state, not T1:
                    // 8 dots before sampling and 1 after, so the value is captured
                    // ON the read cycle's final dot rather than one dot past it,
                    // while the 9-dot total keeps PPU-CPU alignment from drifting.
                    // The run loop subtracts these 3 cycles to avoid
                    // double-advancing. The sampling dot is a sub-cycle alignment
                    // constant: sync_vbl dot-locks blargg test code to VBlank
                    // onset through this read, and because a frame is a
                    // non-integer 29780⅔ CPU cycles, a one-dot sampling error
                    // shows up only after multi-frame delays (test 3 of
                    // cpu_interrupts_v2 spans 2 frames; its whole table was one
                    // row early at the old one-dot-later sampling point, while
                    // 1-frame tests were unaffected). See
                    // docs/investigations/cpu_interrupt_debug_log.md (2026-07-04).
                    if let Some(ref cart) = self.cartridge {
                        self.ppu.set_mirroring(cart.mirroring());
                    }
                    self.ppu.tick_dots(8, self.cartridge.as_mut());
                    if self.ppu.take_nmi() {
                        self.ppu_preadvance_nmi = true;
                    }
                    self.ppu_preadvance_cycles = self.ppu_preadvance_cycles.saturating_add(3);
                    let value = self.ppu.read_register(reg, self.cartridge.as_ref());
                    self.ppu.tick_dots(1, self.cartridge.as_mut());
                    if self.ppu.take_nmi() {
                        self.ppu_preadvance_nmi = true;
                    }
                    return value;
                }
                self.ppu.read_register(reg, self.cartridge.as_ref())
            }

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
            0x2000..=0x3FFF => {
                let reg = (addr & 0x0007) as u8;
                if reg == 0 {
                    // $2000 is written on the store's 4th/last cycle on real
                    // hardware, but this write is applied while the PPU still
                    // sits at the start of the instruction. Pre-advance the PPU
                    // 3 cycles (same mechanism as the $2002 read) so the write
                    // — in particular the NMI-enable mid-VBlank instant-fire
                    // quirk — takes effect at the correct dot. The instant-fire
                    // NMI edge then lands on the instruction's last cycle, where
                    // the run loop's deferred-edge rule correctly keeps it from
                    // affecting the very next dispatch.
                    if let Some(ref cart) = self.cartridge {
                        self.ppu.set_mirroring(cart.mirroring());
                    }
                    self.ppu.tick(3, self.cartridge.as_mut());
                    if self.ppu.take_nmi() {
                        self.ppu_preadvance_nmi = true;
                    }
                    self.ppu_preadvance_cycles = self.ppu_preadvance_cycles.saturating_add(3);
                }
                self.ppu.write_register(reg, data, self.cartridge.as_mut())
            }

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
