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
/// CPU cycles the APU is pre-advanced when $4015 is read (see Bus::read).
const APU_READ_PREADVANCE: u32 = 4;
/// Which `apu.cycle_parity()` value corresponds to a 3-cycle (vs. 4-cycle)
/// DMC DMA stall — the DMC-DMA analog of the OAM-DMA 513/514 decision.
/// Best-effort initial value; sweep against AccuracyCoin's page 13 DMA tests
/// if they don't pass (see docs/superpowers/plans/2026-07-13-dmc-dma-cycle-accurate.md).
const DMC_DMA_ALIGNED_PARITY: bool = true;

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
    /// Extra CPU cycles consumed by an in-line DMC DMA stall since the last
    /// `take_dma_stall_cycles()` call.
    dma_stall_extra_cycles: u32,
    /// Guards against the DMC-DMA halt/fetch sequence's own nested `read`
    /// calls re-triggering DMA arming.
    dmc_dma_in_progress: bool,
    /// CPU cycles by which the PPU was pre-advanced during a $2002 read (to
    /// simulate T4-read timing). Consumed by the run loop to avoid double-advancing.
    ppu_preadvance_cycles: u32,
    /// NMI that fired during a $2002 pre-advance; the run loop must deliver it.
    ppu_preadvance_nmi: bool,
    /// CPU cycles by which the APU was pre-advanced during a $4015 read (the
    /// read cycle is the 4th/last cycle of the reading instruction, but the
    /// bus applies reads while the APU has only been ticked through the
    /// instruction's first cycle). Repaid internally by tick_apu, so run
    /// loops need no changes and total APU time is conserved.
    apu_preadvance_cycles: u32,
    /// CPU open-bus latch: the 6502 data bus retains the last byte
    /// transferred (every read result and every written value updates it —
    /// including opcode/operand fetches, which is why `LDA $4016` sees $40
    /// in the top bits: the operand high byte was the last bus transfer).
    /// Reads of undriven addresses return it: $4000-$4014 and $4018-$401F
    /// entirely, $4015 bit 5, $4016/$4017 bits 5-7. Unlike the PPU's decay
    /// register there is no decay — the value persists until overwritten.
    cpu_open_bus: u8,
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
            dma_stall_extra_cycles: 0,
            dmc_dma_in_progress: false,
            ppu_preadvance_cycles: 0,
            ppu_preadvance_nmi: false,
            apu_preadvance_cycles: 0,
            cpu_open_bus: 0,
        }
    }

    /// Side-effect-free memory inspection for test harnesses and debuggers:
    /// reads RAM and cartridge space WITHOUT touching the CPU open-bus latch
    /// or any register side effects. Harness polling (e.g. the $6000 result
    /// protocol) must use this instead of `read` — emulator-external reads
    /// through `read` corrupt the open-bus latch between the emulated
    /// program's own bus transfers (caught by cpu_exec_space_apu, which
    /// executes from open bus and needs the latch to hold the last byte the
    /// PROGRAM transferred). I/O regions return 0 rather than open bus so
    /// the result is stable and side-effect-free.
    pub fn peek(&self, addr: u16) -> u8 {
        match addr {
            0x0000..=0x1FFF => self.ram[(addr & 0x07FF) as usize],
            0x4020..=0xFFFF => self
                .cartridge
                .as_ref()
                .and_then(|c| c.read(addr))
                .unwrap_or(0),
            _ => 0,
        }
    }

    /// Test-only accessor: exposes a controller port's raw shift-register
    /// state for tests that need to distinguish "exactly one $4016 read
    /// happened" from "extra reads of the same address happened," since each
    /// read shifts the register by exactly one bit (see the `0x4016` arm of
    /// `read_decoded`).
    #[cfg(test)]
    pub(crate) fn peek_controller_shift(&self, port: usize) -> u8 {
        self.controller_shift[port]
    }

    /// Returns true when OAM DMA is in progress (CPU must be stalled).
    /// DMC DMA no longer needs a separate stepping loop here — it happens
    /// synchronously inside `CpuBus::read`, and its cycle count is reported
    /// back via `take_dma_stall_cycles`.
    pub fn dma_active(&self) -> bool {
        self.oam_dma_active
    }

    /// Advance one DMA cycle. OAM DMA copies one byte every two cycles.
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

    /// Advance the APU by `cpu_cycles`. Returns true if an IRQ should be
    /// raised — the OR of the APU's IRQ line and the cartridge mapper's
    /// (e.g. the MMC3 scanline counter, clocked by the PPU ticking that runs
    /// before this within the same cycle). Both lines are level-triggered.
    /// DMC DMA is no longer armed here: it's detected and serviced
    /// synchronously, mid-instruction, by `CpuBus::read`/`write` (see
    /// `maybe_dmc_dma`), which reports its stall cycles via
    /// `take_dma_stall_cycles` instead.
    pub fn tick_apu(&mut self, cpu_cycles: u64) -> bool {
        // Repay any $4015-read pre-advance first so total APU time is conserved.
        let repay = (self.apu_preadvance_cycles as u64).min(cpu_cycles);
        self.apu_preadvance_cycles -= repay as u32;
        let irq = self.apu.tick((cpu_cycles - repay) as u32);
        irq || self.cartridge.as_ref().is_some_and(|c| c.irq_pending())
    }

    /// Strobe the controller shift registers. Writing 1 to bit 0 of $4016
    /// continuously reloads the latch; writing 0 freezes it and starts serial read.
    ///
    /// `buttons` bit order (LSB first, matches real NES serial order):
    /// bit0=A, bit1=B, bit2=Select, bit3=Start, bit4=Up, bit5=Down, bit6=Left, bit7=Right.
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
        self.oam_dma_len = if self.apu.cycle_parity() { 513 } else { 514 };
        self.oam_dma_cycles_left = self.oam_dma_len;
        self.oam_dma_page = page;
        self.oam_dma_byte_idx = 0;
    }
}

impl Bus {
    /// The address-decoded read; `CpuBus::read` wraps it to keep the CPU
    /// open-bus latch updated with every transferred byte.
    fn read_decoded(&mut self, addr: u16) -> u8 {
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
                    let value = self.ppu.read_register(reg, self.cartridge.as_mut());
                    self.ppu.tick_dots(1, self.cartridge.as_mut());
                    if self.ppu.take_nmi() {
                        self.ppu_preadvance_nmi = true;
                    }
                    return value;
                }
                self.ppu.read_register(reg, self.cartridge.as_mut())
            }

            // APU / I/O
            0x4015 => {
                // The $4015 read cycle is the reading instruction's 4th/last
                // cycle, but this read is applied while the APU has only been
                // ticked through the instruction's first (dispatch) cycle.
                // Pre-advance so the frame-IRQ flag value (and the read-clear
                // side effect) are sampled at the read cycle rather than 3-4
                // cycles before it; tick_apu repays the debt so total APU time
                // is conserved. Calibrated against the CK column of
                // cpu_interrupts_v2/5-branch_delays_irq (a 1-cycle-per-iteration
                // sampling-window walk over the flag's set moment).
                self.apu.tick(APU_READ_PREADVANCE);
                self.apu_preadvance_cycles += APU_READ_PREADVANCE;
                // The APU drives every $4015 bit except bit 5, which stays
                // at the CPU open-bus value.
                (self.apu.read(addr) & 0xDF) | (self.cpu_open_bus & 0x20)
            }
            // Write-only APU/OAM-DMA registers: nothing drives the bus, the
            // read returns the CPU open-bus latch.
            0x4000..=0x4014 => self.cpu_open_bus,
            // Controllers drive the low bits; bits 5-7 stay at open bus
            // (classically $40 — the operand high byte of the LDA $4016).
            0x4016 => {
                let bit = self.controller_shift[0] & 0x01;
                self.controller_shift[0] = (self.controller_shift[0] >> 1) | 0x80;
                (self.cpu_open_bus & 0xE0) | bit
            }
            0x4017 => {
                let bit = self.controller_shift[1] & 0x01;
                self.controller_shift[1] = (self.controller_shift[1] >> 1) | 0x80;
                (self.cpu_open_bus & 0xE0) | bit
            }

            // Disabled region: open bus
            0x4018..=0x401F => self.cpu_open_bus,

            // Cartridge; regions the cartridge doesn't drive (its $4020-$5FFF
            // expansion area on the supported mappers) read as open bus.
            0x4020..=0xFFFF => self
                .cartridge
                .as_ref()
                .and_then(|c| c.read(addr))
                .unwrap_or(self.cpu_open_bus),
        }
    }

    /// Like `read_decoded`, but for `$2002`/`$4015` skips the one-time
    /// instruction-start-relative pre-advance dance (the `tick_dots(8)`/
    /// `apu.tick(APU_READ_PREADVANCE)` jumps). Used for the nested dummy
    /// reads a DMC-DMA halt/alignment cycle performs at the CPU's own
    /// in-flight address: those reads are already at their correct real-time
    /// position (this whole call sequence IS the real-time stepping), so
    /// applying the lump pre-advance again would double-count PPU dots/APU
    /// cycles. Side effects (VBlank clear, $2007 latch reset, frame-IRQ-flag
    /// clear) still apply normally.
    fn read_decoded_live(&mut self, addr: u16) -> u8 {
        match addr {
            0x2000..=0x3FFF => {
                let reg = (addr & 0x0007) as u8;
                self.ppu.read_register(reg, self.cartridge.as_mut())
            }
            0x4015 => (self.apu.read(addr) & 0xDF) | (self.cpu_open_bus & 0x20),
            _ => self.read_decoded(addr),
        }
    }

    /// Advance the DMC channel's real-time clock for one CPU cycle
    /// (see `Apu::dmc_tick_realtime`).
    fn dmc_realtime_advance(&mut self) {
        self.apu.dmc_tick_realtime();
    }

    /// A read that also advances the DMC real-time clock — used for the
    /// halt/alignment/fetch cycles the DMA sequence itself performs, and for
    /// the DMC's own sample fetch.
    fn read_live(&mut self, addr: u16) -> u8 {
        self.dmc_realtime_advance();
        let value = self.read_decoded_live(addr);
        self.cpu_open_bus = value;
        value
    }

    /// Called at the top of every CPU read. Advances the DMC real-time clock
    /// for this cycle; if a fetch is now due (and neither an OAM DMA nor a
    /// nested DMC-DMA sequence is already in progress), halts the CPU here —
    /// real hardware can only sample RDY on a read cycle, never mid-write —
    /// performing 2 or 3 dummy re-reads of `addr` (the CPU's own in-flight
    /// target address, matching real hardware repeating the same read while
    /// halted) followed by the real sample fetch at the DMC's own address.
    /// Each of those cycles is a real, side-effecting bus access: a dummy
    /// re-read of $2002/$4015/$4016/$2007 during the halt has the same read
    /// side effects an ordinary read there would.
    fn maybe_dmc_dma(&mut self, addr: u16) {
        if self.dmc_dma_in_progress || self.oam_dma_active {
            self.dmc_realtime_advance();
            return;
        }
        self.dmc_realtime_advance();
        if !self.apu.dmc_needs_dma() {
            return;
        }
        self.dmc_dma_in_progress = true;
        let aligned = self.apu.cycle_parity() == DMC_DMA_ALIGNED_PARITY;
        let halt_cycles = if aligned { 2 } else { 3 };
        for _ in 0..halt_cycles {
            let _ = self.read_live(addr);
            self.dma_stall_extra_cycles += 1;
        }
        let dmc_addr = self.apu.dmc_dma_address();
        let data = self.read_live(dmc_addr);
        self.apu.dmc_supply_byte(data);
        self.dma_stall_extra_cycles += 1;
        self.dmc_dma_in_progress = false;
    }
}

impl CpuBus for Bus {
    fn read(&mut self, addr: u16) -> u8 {
        self.maybe_dmc_dma(addr);
        let value = self.read_decoded(addr);
        self.cpu_open_bus = value;
        value
    }

    fn write(&mut self, addr: u16, data: u8) {
        // Real hardware can only halt the CPU (sample RDY) on a read cycle,
        // never mid-write — so a write only advances the DMC's real-time
        // clock; if it makes needs_dma() true, the halt/fetch happens on the
        // CPU's next read instead.
        self.dmc_realtime_advance();
        self.cpu_open_bus = data;
        match addr {
            // Internal RAM + mirrors
            0x0000..=0x1FFF => self.ram[(addr & 0x07FF) as usize] = data,

            // PPU registers + mirrors
            0x2000..=0x3FFF => {
                let reg = (addr & 0x0007) as u8;
                if reg == 0 || reg == 1 {
                    // $2000/$2001 are written on the store's 4th/last cycle on
                    // real hardware, but this write is applied while the PPU
                    // still sits at the start of the instruction. Pre-advance
                    // the PPU with the same 8-dots/apply/1-dot structure as
                    // the $2002 read, so the write lands at the same sub-cycle
                    // position: one dot before a CPU-cycle NMI-line edge
                    // sample.
                    //
                    // For $2000, that placement is what gives NMI-disable its
                    // 2-dot suppression window (a disable 0-1 dots after VBL
                    // onset drops the line before the sample sees it), and
                    // what makes the mid-VBlank enable "instant NMI" edge
                    // latch on the very next sample — inside the trailing
                    // dot, whose edge is deliberately NOT consumed here: it
                    // flows into the run loop's per-cycle ticks, where the
                    // deferred-edge rule keeps it from affecting the very
                    // next dispatch (blargg-verified).
                    //
                    // For $2001, the placement puts rendering enable/disable
                    // at the write cycle instead of ~12 dots early, which the
                    // odd-frame skipped-dot decision depends on
                    // (ppu_vbl_nmi/10-even_odd_timing).
                    if let Some(ref cart) = self.cartridge {
                        self.ppu.set_mirroring(cart.mirroring());
                    }
                    self.ppu.tick_dots(8, self.cartridge.as_mut());
                    if self.ppu.take_nmi() {
                        self.ppu_preadvance_nmi = true;
                    }
                    self.ppu_preadvance_cycles = self.ppu_preadvance_cycles.saturating_add(3);
                    self.ppu.write_register(reg, data, self.cartridge.as_mut());
                    self.ppu.tick_dots(1, self.cartridge.as_mut());
                    return;
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

    fn take_dma_stall_cycles(&mut self) -> u32 {
        let n = self.dma_stall_extra_cycles;
        self.dma_stall_extra_cycles = 0;
        n
    }
}
