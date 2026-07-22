/// Nametable mirroring mode (determines how the PPU maps the two 1 KB VRAM banks
/// to the four logical nametable addresses $2000/$2400/$2800/$2C00).
#[derive(Clone, Copy, PartialEq, Debug)]
pub enum Mirroring {
    Horizontal, // NT0=A NT1=A NT2=B NT3=B
    Vertical,   // NT0=A NT1=B NT2=A NT3=B
    SingleLow,  // all → bank A
    SingleHigh, // all → bank B
    FourScreen, // cartridge-supplied four-screen VRAM (not yet supported)
}

/// Error returned when a ROM cannot be loaded.
#[derive(Debug)]
pub enum CartridgeError {
    InvalidHeader,
    Truncated,
    UnsupportedMapper(u8),
}

impl std::fmt::Display for CartridgeError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::InvalidHeader => write!(f, "not a valid iNES file"),
            Self::Truncated => write!(f, "iNES file truncated"),
            Self::UnsupportedMapper(id) => write!(f, "unsupported mapper {id}"),
        }
    }
}

pub struct Cartridge {
    prg_rom: Vec<u8>,
    /// WRAM at $6000–$7FFF — always present regardless of the iNES header flag,
    /// because blargg-style test ROMs write results there unconditionally.
    prg_ram: [u8; 8192],
    /// CHR-ROM from the iNES image. Empty when the game uses CHR-RAM.
    chr_rom: Vec<u8>,
    /// 8 KB CHR-RAM used by games with no CHR-ROM (chr_rom.is_empty()).
    chr_ram: Box<[u8; 8192]>,
    mapper: Box<dyn Mapper>,
    /// Mirroring mode from the iNES header (mappers may override dynamically).
    header_mirroring: Mirroring,
}

impl Cartridge {
    pub fn from_ines(data: &[u8]) -> Result<Self, CartridgeError> {
        if data.len() < 16 || &data[0..4] != b"NES\x1A" {
            return Err(CartridgeError::InvalidHeader);
        }
        let prg_banks = data[4] as usize;
        let chr_banks = data[5] as usize;
        let mapper_id = (data[6] >> 4) | (data[7] & 0xF0);
        let has_trainer = data[6] & 0x04 != 0;
        let prg_start = 16 + if has_trainer { 512 } else { 0 };
        let prg_size = prg_banks * 16384;
        let chr_size = chr_banks * 8192;
        if data.len() < prg_start + prg_size + chr_size {
            return Err(CartridgeError::Truncated);
        }
        let prg_rom = data[prg_start..prg_start + prg_size].to_vec();
        let chr_rom = data[prg_start + prg_size..prg_start + prg_size + chr_size].to_vec();

        let header_mirroring = if data[6] & 0x08 != 0 {
            Mirroring::FourScreen
        } else if data[6] & 0x01 != 0 {
            Mirroring::Vertical
        } else {
            Mirroring::Horizontal
        };

        let mapper: Box<dyn Mapper> = match mapper_id {
            0 => Box::new(Nrom),
            1 => Box::new(Mmc1::new(prg_banks)),
            2 => Box::new(Uxrom::new()),
            3 => Box::new(Cnrom::new()),
            4 => Box::new(Mmc3::new(prg_banks)),
            7 => Box::new(Axrom::new()),
            9 => Box::new(Mmc2::new()),
            10 => Box::new(Mmc4::new()),
            11 => Box::new(ColorDreams::new()),
            // Mapper 34 covers two very different boards: NINA-001 (has
            // CHR-ROM, registers at $7FFD–$7FFF) and BNROM (CHR-RAM, 32 KB PRG
            // switch at $8000). Disambiguate by CHR-ROM presence, as most
            // emulators do.
            34 => Box::new(Mapper34::new(chr_banks > 0)),
            66 => Box::new(Gxrom::new()),
            69 => Box::new(Fme7::new()),
            71 => Box::new(Camerica::new()),
            87 => Box::new(Mapper87::new()),
            206 => Box::new(Namco118::new()),
            _ => return Err(CartridgeError::UnsupportedMapper(mapper_id)),
        };

        Ok(Self {
            prg_rom,
            prg_ram: [0u8; 8192],
            chr_rom,
            chr_ram: Box::new([0u8; 8192]),
            mapper,
            header_mirroring,
        })
    }

    /// Read from CPU-visible cartridge space. Returns `None` where the
    /// cartridge drives nothing (the $4020-$5FFF expansion area on the
    /// mappers implemented here) — the bus then supplies its open-bus value.
    pub fn read(&self, addr: u16) -> Option<u8> {
        match addr {
            0x6000..=0x7FFF => Some(self.prg_ram[(addr - 0x6000) as usize]),
            0x8000..=0xFFFF => {
                let offset = self.mapper.prg_offset(self.prg_rom.len(), addr);
                Some(self.prg_rom.get(offset).copied().unwrap_or(0))
            }
            _ => None,
        }
    }

    pub fn write(&mut self, addr: u16, data: u8) {
        match addr {
            0x6000..=0x7FFF => {
                self.prg_ram[(addr - 0x6000) as usize] = data;
                // Some mappers latch bank registers from writes into this
                // window (NINA-001, mapper 87); the RAM write above still
                // happens, as on hardware where both the RAM and the register
                // see the write.
                self.mapper.write_prg_ram(addr, data);
            }
            0x8000..=0xFFFF => self.mapper.write_prg(addr, data),
            _ => {}
        }
    }

    /// Advance any CPU-cycle-clocked mapper timer (the FME-7 IRQ counter) by
    /// `cycles` CPU cycles. Called once per CPU cycle from `Bus::tick_apu`.
    pub fn tick_cpu(&mut self, cycles: u64) {
        self.mapper.tick_cpu(cycles);
    }

    /// Read from the PPU pattern table space ($0000–$1FFF). Mapper CHR
    /// banking applies to CHR-RAM too (folded mod 8 KB, its physical size).
    pub fn chr_read(&self, addr: u16) -> u8 {
        let offset = self.mapper.chr_offset(addr);
        if self.chr_rom.is_empty() {
            self.chr_ram[offset % self.chr_ram.len()]
        } else {
            self.chr_rom[offset % self.chr_rom.len()]
        }
    }

    /// Write to the PPU pattern table space (CHR-RAM only; ignored for CHR-ROM).
    pub fn chr_write(&mut self, addr: u16, data: u8) {
        if self.chr_rom.is_empty() {
            let offset = self.mapper.chr_offset(addr) % self.chr_ram.len();
            self.chr_ram[offset] = data;
        }
    }

    /// Notify the mapper of a PPU address-bus change (rendering fetches,
    /// $2006 writes, $2007 accesses and their post-increment). `dots` is the
    /// PPU's free-running dot counter, used by the MMC3's A12 low-time
    /// filter.
    pub fn ppu_bus_addr(&mut self, addr: u16, dots: u64) {
        self.mapper.ppu_bus_addr(addr & 0x3FFF, dots);
    }

    /// Level of the mapper's IRQ output line (level-triggered; stays asserted
    /// until acknowledged through the mapper's own registers).
    pub fn irq_pending(&self) -> bool {
        self.mapper.irq_pending()
    }

    /// Current nametable mirroring mode. Mappers may override the header
    /// value — except four-screen carts, whose extra VRAM bypasses the
    /// mapper's mirroring control entirely.
    pub fn mirroring(&self) -> Mirroring {
        if self.header_mirroring == Mirroring::FourScreen {
            return Mirroring::FourScreen;
        }
        self.mapper.mirroring().unwrap_or(self.header_mirroring)
    }
}

// ---------------------------------------------------------------------------
// Mapper trait
// ---------------------------------------------------------------------------

/// Address translation for PRG-ROM. `prg_offset` returns a byte index into the
/// PRG-ROM slice; the cartridge does the actual array access.
trait Mapper {
    fn prg_offset(&self, rom_len: usize, addr: u16) -> usize;
    fn write_prg(&mut self, addr: u16, data: u8);
    /// CHR address → byte offset into CHR-ROM. Default: identity (no banking).
    fn chr_offset(&self, addr: u16) -> usize {
        (addr & 0x1FFF) as usize
    }
    /// Dynamic mirroring override. Returns None to use the header's mirroring value.
    fn mirroring(&self) -> Option<Mirroring> {
        None
    }
    /// PPU address-bus change notification (every address the PPU puts on its
    /// bus: rendering fetches, $2006/$2007 accesses). Default: ignored; the
    /// MMC3 uses it to clock its A12-driven IRQ counter and MMC2/MMC4 use it to
    /// flip their CHR bank latches.
    fn ppu_bus_addr(&mut self, _addr: u16, _dots: u64) {}
    /// Observe a CPU write into the $6000–$7FFF PRG-RAM window. Some mappers
    /// place bank registers there (NINA-001 at $7FFD–$7FFF, mapper 87 across
    /// the whole window). The cartridge still writes PRG-RAM as usual — this is
    /// an additional notification, not a replacement. Default: ignored.
    fn write_prg_ram(&mut self, _addr: u16, _data: u8) {}
    /// Advance any CPU-cycle-clocked mapper timer by `cycles` CPU cycles
    /// (the Sunsoft FME-7 IRQ counter). Called once per CPU cycle by the bus.
    /// Default: no timer.
    fn tick_cpu(&mut self, _cycles: u64) {}
    /// Level of the mapper's IRQ output. Default: never asserted.
    fn irq_pending(&self) -> bool {
        false
    }
}

// ---------------------------------------------------------------------------
// Mapper 0 — NROM
// ---------------------------------------------------------------------------

struct Nrom;

impl Mapper for Nrom {
    fn prg_offset(&self, rom_len: usize, addr: u16) -> usize {
        (addr - 0x8000) as usize % rom_len
    }

    fn write_prg(&mut self, _addr: u16, _data: u8) {}
}

// ---------------------------------------------------------------------------
// Mapper 1 — MMC1 (SxROM)
//
// Four internal 5-bit registers written via a serial shift register:
//   Control   ($8000–$9FFF): mirroring, PRG/CHR bank modes
//   CHR bank 0 ($A000–$BFFF)
//   CHR bank 1 ($C000–$DFFF)
//   PRG bank  ($E000–$FFFF): bits 0–3 = bank number, bit 4 = WRAM disable
//
// PRG bank modes (control bits 2–3):
//   0,1 — switch 32 KB at $8000 (low bit of bank ignored)
//     2 — fix first 16 KB at $8000, switch 16 KB at $C000
//     3 — switch 16 KB at $8000, fix last 16 KB at $C000  ← reset default
// ---------------------------------------------------------------------------

struct Mmc1 {
    shift: u8,       // bits accumulated LSB-first
    shift_count: u8, // bits written so far (0–4); write completes at 5

    control: u8,
    chr_bank_0: u8,
    chr_bank_1: u8,
    prg_bank: u8,

    prg_bank_count: usize,
}

impl Mmc1 {
    fn new(prg_banks: usize) -> Self {
        Self {
            shift: 0,
            shift_count: 0,
            control: 0x0C, // PRG mode 3 on reset
            chr_bank_0: 0,
            chr_bank_1: 0,
            prg_bank: 0,
            prg_bank_count: prg_banks,
        }
    }

    fn bank_offset(&self, addr: u16) -> usize {
        let mode = (self.control >> 2) & 0x03;
        let bank = (self.prg_bank & 0x0F) as usize;
        let last = self.prg_bank_count - 1;

        match mode {
            0 | 1 => {
                // 32 KB switch: pair banks by clearing the low bit
                let base = bank & !1;
                if addr < 0xC000 {
                    base * 0x4000 + (addr - 0x8000) as usize
                } else {
                    (base + 1) * 0x4000 + (addr - 0xC000) as usize
                }
            }
            2 => {
                // First bank fixed at $8000, switchable at $C000
                if addr < 0xC000 {
                    (addr - 0x8000) as usize
                } else {
                    bank * 0x4000 + (addr - 0xC000) as usize
                }
            }
            _ => {
                // Switchable at $8000, last bank fixed at $C000
                if addr < 0xC000 {
                    bank * 0x4000 + (addr - 0x8000) as usize
                } else {
                    last * 0x4000 + (addr - 0xC000) as usize
                }
            }
        }
    }
}

impl Mapper for Mmc1 {
    fn prg_offset(&self, _rom_len: usize, addr: u16) -> usize {
        self.bank_offset(addr)
    }

    fn mirroring(&self) -> Option<Mirroring> {
        Some(match self.control & 0x03 {
            0 => Mirroring::SingleLow,
            1 => Mirroring::SingleHigh,
            2 => Mirroring::Vertical,
            _ => Mirroring::Horizontal,
        })
    }

    fn write_prg(&mut self, addr: u16, data: u8) {
        if data & 0x80 != 0 {
            // Reset: clear shift register and force PRG mode 3
            self.shift = 0;
            self.shift_count = 0;
            self.control |= 0x0C;
            return;
        }

        self.shift |= (data & 1) << self.shift_count;
        self.shift_count += 1;

        if self.shift_count == 5 {
            let value = self.shift & 0x1F;
            self.shift = 0;
            self.shift_count = 0;

            match addr {
                0x8000..=0x9FFF => self.control = value,
                0xA000..=0xBFFF => self.chr_bank_0 = value,
                0xC000..=0xDFFF => self.chr_bank_1 = value,
                _ => self.prg_bank = value,
            }
        }
    }
}

// ---------------------------------------------------------------------------
// Mapper 2 — UxROM
//
// Any $8000-$FFFF write selects the 16 KB PRG-ROM bank at $8000-$BFFF; the
// last bank is fixed at $C000-$FFFF. CHR is unbanked (these boards carry
// 8 KB CHR-RAM). Bus conflicts are not modeled, as for CNROM below.
// ---------------------------------------------------------------------------

struct Uxrom {
    prg_bank: usize,
}

impl Uxrom {
    fn new() -> Self {
        Self { prg_bank: 0 }
    }
}

impl Mapper for Uxrom {
    fn prg_offset(&self, rom_len: usize, addr: u16) -> usize {
        let bank_count = rom_len / 0x4000;
        if addr < 0xC000 {
            (self.prg_bank % bank_count) * 0x4000 + (addr - 0x8000) as usize
        } else {
            (bank_count - 1) * 0x4000 + (addr - 0xC000) as usize
        }
    }

    fn write_prg(&mut self, _addr: u16, data: u8) {
        // 4 register bits covers UNROM (8 banks) and UOROM (16 banks).
        self.prg_bank = (data & 0x0F) as usize;
    }
}

// ---------------------------------------------------------------------------
// Mapper 3 — CNROM
//
// Fixed NROM-style PRG; any $8000-$FFFF write selects an 8 KB CHR-ROM bank
// (2 bits on real boards, up to 32 KB). Bus conflicts (the written value is
// ANDed with the ROM byte at that address on real hardware) are not modeled —
// well-behaved ROMs (including blargg's) write to locations whose ROM byte
// equals the value, making the AND a no-op.
// ---------------------------------------------------------------------------

struct Cnrom {
    chr_bank: usize,
}

impl Cnrom {
    fn new() -> Self {
        Self { chr_bank: 0 }
    }
}

impl Mapper for Cnrom {
    fn prg_offset(&self, rom_len: usize, addr: u16) -> usize {
        (addr - 0x8000) as usize % rom_len
    }

    fn write_prg(&mut self, _addr: u16, data: u8) {
        self.chr_bank = (data & 0x03) as usize;
    }

    fn chr_offset(&self, addr: u16) -> usize {
        self.chr_bank * 0x2000 + (addr & 0x1FFF) as usize
    }
}

// ---------------------------------------------------------------------------
// Mapper 4 — MMC3 (TxROM)
//
// Registers come in even/odd pairs on $8000-$FFFF:
//   $8000/$8001 — bank select / bank data (R0-R7)
//   $A000/$A001 — mirroring / PRG-RAM protect (protect not modeled: PRG-RAM
//                 stays enabled so the blargg $6000 result protocol always
//                 works; only the MMC6-specific tests would notice)
//   $C000/$C001 — IRQ latch (reload value) / IRQ counter clear
//   $E000/$E001 — IRQ disable+acknowledge / IRQ enable
//
// CHR: R0/R1 select 2 KB banks, R2-R5 1 KB banks; bank-select bit 7 swaps
// the $0000/$1000 halves. PRG: R6 + R7 select 8 KB banks; bit 6 chooses
// which of $8000/$C000 is fixed to the second-last bank; $E000 is always
// the last bank.
//
// IRQ counter (the "new"/sharp MMC3 revision, which blargg's mmc3_test
// 5-MMC3 and mmc3_irq_tests 6.MMC3_rev_B verify): on each A12 clock,
// counter==0 reloads from the latch, otherwise the counter decrements;
// after that, IRQ asserts if the counter is 0 and IRQs are enabled. The
// rev-A behavior (IRQ only on forced reloads; tested by 5.MMC3_rev_A,
// 6-MMC6, 6-MMC3_alt) is mutually exclusive and not implemented.
//
// A12 clock source: the counter clocks on a PPU address-bus A12 rising edge,
// but only after A12 has stayed low long enough (hardware: ~3 CPU M2 falls).
// The PPU reports every bus address via ppu_bus_addr; the filter must
//   - reject the 7-dot lows between successive pattern-fetch events inside
//     a scanline (one pattern event per 8-dot tile group / sprite slot,
//     with nametable/attribute lows in between), and
//   - reject the 12-dot low across the scanline boundary (last prefetch
//     pattern event at dot 336 → dummy-NT low at 337 → next line's first
//     pattern event at dot 8): blargg's 4-scanline_timing subtests 12/13
//     show hardware does NOT clock at the start of every line when the BG
//     uses $1000 — only the post-sprite-region prefetch fetch (dot 328
//     here) clocks once per line, plus one extra clock on the first
//     rendered line after rendering was enabled during VBlank, and
//   - accept every manual $2006/$2007-driven low (back-to-back register
//     writes leave A12 low for ≥24 dots).
// 16 sits between the 12- and 24-dot cases with margin on both sides.
// ---------------------------------------------------------------------------

const A12_FILTER_DOTS: u64 = 16;

struct Mmc3 {
    bank_select: u8,
    bank_regs: [u8; 8],
    mirroring: u8, // $A000 bit 0: 0=vertical, 1=horizontal

    irq_latch: u8,
    irq_counter: u8,
    irq_enabled: bool,
    irq_flag: bool,

    a12_level: bool,
    a12_low_since: u64,

    prg_bank_count_8k: usize,
}

impl Mmc3 {
    fn new(prg_banks_16k: usize) -> Self {
        Self {
            bank_select: 0,
            bank_regs: [0; 8],
            mirroring: 0,
            irq_latch: 0,
            irq_counter: 0,
            irq_enabled: false,
            irq_flag: false,
            a12_level: false,
            a12_low_since: 0,
            prg_bank_count_8k: prg_banks_16k * 2,
        }
    }

    fn clock_irq_counter(&mut self) {
        if self.irq_counter == 0 {
            self.irq_counter = self.irq_latch;
        } else {
            self.irq_counter -= 1;
        }
        if self.irq_counter == 0 && self.irq_enabled {
            self.irq_flag = true;
        }
    }
}

impl Mapper for Mmc3 {
    fn prg_offset(&self, _rom_len: usize, addr: u16) -> usize {
        let last = self.prg_bank_count_8k - 1;
        let swap = self.bank_select & 0x40 != 0;
        let bank = match (addr >> 13) & 0x03 {
            0 => {
                if swap {
                    last - 1
                } else {
                    self.bank_regs[6] as usize
                }
            }
            1 => self.bank_regs[7] as usize,
            2 => {
                if swap {
                    self.bank_regs[6] as usize
                } else {
                    last - 1
                }
            }
            _ => last,
        };
        (bank % self.prg_bank_count_8k) * 0x2000 + (addr & 0x1FFF) as usize
    }

    fn chr_offset(&self, addr: u16) -> usize {
        // Bank-select bit 7 exchanges the $0000-$0FFF and $1000-$1FFF halves
        // (banking only — the IRQ counter always sees the raw bus A12).
        let addr = if self.bank_select & 0x80 != 0 {
            addr ^ 0x1000
        } else {
            addr
        } & 0x1FFF;
        let bank = match addr >> 10 {
            0 | 1 => (self.bank_regs[0] & !1) as usize + (addr >> 10) as usize,
            2 | 3 => (self.bank_regs[1] & !1) as usize + ((addr >> 10) as usize - 2),
            r => self.bank_regs[r as usize - 2] as usize,
        };
        bank * 0x400 + (addr & 0x3FF) as usize
    }

    fn mirroring(&self) -> Option<Mirroring> {
        Some(if self.mirroring & 1 == 0 {
            Mirroring::Vertical
        } else {
            Mirroring::Horizontal
        })
    }

    fn write_prg(&mut self, addr: u16, data: u8) {
        match addr & 0xE001 {
            0x8000 => self.bank_select = data,
            0x8001 => self.bank_regs[(self.bank_select & 0x07) as usize] = data,
            0xA000 => self.mirroring = data & 1,
            0xA001 => {} // PRG-RAM protect — not modeled (see header comment)
            0xC000 => self.irq_latch = data,
            // Clearing the counter does NOT set the IRQ flag and does not
            // reload immediately — the next A12 clock reloads from the latch
            // (because the counter is 0 then), per blargg 1-clocking #5/#6
            // and 2-details #6.
            0xC001 => self.irq_counter = 0,
            0xE000 => {
                self.irq_enabled = false;
                self.irq_flag = false; // acknowledge
            }
            _ => self.irq_enabled = true,
        }
    }

    fn ppu_bus_addr(&mut self, addr: u16, dots: u64) {
        let a12 = addr & 0x1000 != 0;
        if a12 {
            if !self.a12_level && dots.wrapping_sub(self.a12_low_since) >= A12_FILTER_DOTS {
                self.clock_irq_counter();
            }
        } else if self.a12_level {
            self.a12_low_since = dots;
        }
        self.a12_level = a12;
    }

    fn irq_pending(&self) -> bool {
        self.irq_flag
    }
}

// ---------------------------------------------------------------------------
// Mapper 7 — AxROM
//
// Any $8000-$FFFF write selects a 32 KB PRG-ROM bank (bits 0-2) and the
// single-screen nametable page (bit 4). CHR is unbanked 8 KB CHR-RAM.
// Bus conflicts are not modeled (and most AxROM boards have none anyway).
// ---------------------------------------------------------------------------

struct Axrom {
    prg_bank: usize,
    /// Bit 4 of the last write: which 1 KB VRAM page all four nametables map to.
    vram_page_high: bool,
}

impl Axrom {
    fn new() -> Self {
        Self {
            prg_bank: 0,
            vram_page_high: false,
        }
    }
}

impl Mapper for Axrom {
    fn prg_offset(&self, rom_len: usize, addr: u16) -> usize {
        let bank_count = (rom_len / 0x8000).max(1);
        (self.prg_bank % bank_count) * 0x8000 + (addr - 0x8000) as usize
    }

    fn write_prg(&mut self, _addr: u16, data: u8) {
        self.prg_bank = (data & 0x07) as usize;
        self.vram_page_high = data & 0x10 != 0;
    }

    fn mirroring(&self) -> Option<Mirroring> {
        Some(if self.vram_page_high {
            Mirroring::SingleHigh
        } else {
            Mirroring::SingleLow
        })
    }
}

// ---------------------------------------------------------------------------
// Mapper 11 — Color Dreams
//
// One register at $8000-$FFFF: bits 0-1 select a 32 KB PRG bank, bits 4-7 an
// 8 KB CHR-ROM bank. Mirroring is hardwired (header). Bus conflicts exist on
// real boards but, as with CNROM/UxROM, are not modeled.
// ---------------------------------------------------------------------------

struct ColorDreams {
    prg_bank: usize,
    chr_bank: usize,
}

impl ColorDreams {
    fn new() -> Self {
        Self {
            prg_bank: 0,
            chr_bank: 0,
        }
    }
}

impl Mapper for ColorDreams {
    fn prg_offset(&self, rom_len: usize, addr: u16) -> usize {
        let banks = (rom_len / 0x8000).max(1);
        (self.prg_bank % banks) * 0x8000 + (addr - 0x8000) as usize
    }

    fn write_prg(&mut self, _addr: u16, data: u8) {
        self.prg_bank = (data & 0x03) as usize;
        self.chr_bank = ((data >> 4) & 0x0F) as usize;
    }

    fn chr_offset(&self, addr: u16) -> usize {
        self.chr_bank * 0x2000 + (addr & 0x1FFF) as usize
    }
}

// ---------------------------------------------------------------------------
// Mapper 66 — GxROM (GNROM / MHROM)
//
// One register at $8000-$FFFF: bits 4-5 select a 32 KB PRG bank, bits 0-1 an
// 8 KB CHR-ROM bank. Mirroring is hardwired (header).
// ---------------------------------------------------------------------------

struct Gxrom {
    prg_bank: usize,
    chr_bank: usize,
}

impl Gxrom {
    fn new() -> Self {
        Self {
            prg_bank: 0,
            chr_bank: 0,
        }
    }
}

impl Mapper for Gxrom {
    fn prg_offset(&self, rom_len: usize, addr: u16) -> usize {
        let banks = (rom_len / 0x8000).max(1);
        (self.prg_bank % banks) * 0x8000 + (addr - 0x8000) as usize
    }

    fn write_prg(&mut self, _addr: u16, data: u8) {
        self.prg_bank = ((data >> 4) & 0x03) as usize;
        self.chr_bank = (data & 0x03) as usize;
    }

    fn chr_offset(&self, addr: u16) -> usize {
        self.chr_bank * 0x2000 + (addr & 0x1FFF) as usize
    }
}

// ---------------------------------------------------------------------------
// Mapper 71 — Camerica / Codemasters (BF9093 / BF9097)
//
// UxROM-like: a 16 KB switchable bank at $8000-$BFFF and the fixed last 16 KB
// at $C000-$FFFF. The PRG bank register responds to $C000-$FFFF. The BF9097
// board (Fire Hawk) additionally uses $8000-$9FFF bit 4 for single-screen
// mirroring; boards that never write there keep the header mirroring. CHR is
// unbanked 8 KB CHR-RAM. Bus conflicts are not modeled.
// ---------------------------------------------------------------------------

struct Camerica {
    prg_bank: usize,
    mirror_override: Option<Mirroring>,
}

impl Camerica {
    fn new() -> Self {
        Self {
            prg_bank: 0,
            mirror_override: None,
        }
    }
}

impl Mapper for Camerica {
    fn prg_offset(&self, rom_len: usize, addr: u16) -> usize {
        let banks = (rom_len / 0x4000).max(1);
        if addr < 0xC000 {
            (self.prg_bank % banks) * 0x4000 + (addr - 0x8000) as usize
        } else {
            (banks - 1) * 0x4000 + (addr - 0xC000) as usize
        }
    }

    fn write_prg(&mut self, addr: u16, data: u8) {
        match addr {
            0x8000..=0x9FFF => {
                // BF9097 single-screen mirroring select (bit 4).
                self.mirror_override = Some(if data & 0x10 != 0 {
                    Mirroring::SingleHigh
                } else {
                    Mirroring::SingleLow
                });
            }
            0xC000..=0xFFFF => self.prg_bank = (data & 0x0F) as usize,
            _ => {} // $A000-$BFFF: unused
        }
    }

    fn mirroring(&self) -> Option<Mirroring> {
        self.mirror_override
    }
}

// ---------------------------------------------------------------------------
// Mapper 87 — Jaleco/Konami CHR-select (JF-05/06/07/…)
//
// NROM-style fixed PRG; a write into $6000-$7FFF selects the 8 KB CHR-ROM
// bank, with the two low bits swapped on their way into the bank number
// (D0 → CHR A14, D1 → CHR A13). Mirroring is hardwired (header).
// ---------------------------------------------------------------------------

struct Mapper87 {
    chr_bank: usize,
}

impl Mapper87 {
    fn new() -> Self {
        Self { chr_bank: 0 }
    }
}

impl Mapper for Mapper87 {
    fn prg_offset(&self, rom_len: usize, addr: u16) -> usize {
        (addr - 0x8000) as usize % rom_len
    }

    fn write_prg(&mut self, _addr: u16, _data: u8) {}

    fn write_prg_ram(&mut self, _addr: u16, data: u8) {
        // Bit-swapped: D1 is the low bank bit, D0 the high one.
        self.chr_bank = (((data & 0x02) >> 1) | ((data & 0x01) << 1)) as usize;
    }

    fn chr_offset(&self, addr: u16) -> usize {
        self.chr_bank * 0x2000 + (addr & 0x1FFF) as usize
    }
}

// ---------------------------------------------------------------------------
// Mapper 34 — BNROM / NINA-001
//
// BNROM: a $8000-$FFFF write selects a 32 KB PRG bank; CHR is unbanked 8 KB
// CHR-RAM. NINA-001: three registers in PRG-RAM space — $7FFD selects a 32 KB
// PRG bank (1 bit), $7FFE/$7FFF select the two 4 KB CHR-ROM banks at
// $0000/$1000. The board is chosen by CHR-ROM presence (BNROM has none).
// Mirroring is hardwired (header) on both.
// ---------------------------------------------------------------------------

struct Mapper34 {
    is_nina: bool,
    prg_bank: usize,
    chr_bank_lo: usize,
    chr_bank_hi: usize,
}

impl Mapper34 {
    fn new(is_nina: bool) -> Self {
        Self {
            is_nina,
            prg_bank: 0,
            chr_bank_lo: 0,
            chr_bank_hi: 0,
        }
    }
}

impl Mapper for Mapper34 {
    fn prg_offset(&self, rom_len: usize, addr: u16) -> usize {
        let banks = (rom_len / 0x8000).max(1);
        (self.prg_bank % banks) * 0x8000 + (addr - 0x8000) as usize
    }

    fn write_prg(&mut self, _addr: u16, data: u8) {
        if !self.is_nina {
            // BNROM: full-byte 32 KB bank select (wraps at the ROM's bank count).
            self.prg_bank = data as usize;
        }
    }

    fn write_prg_ram(&mut self, addr: u16, data: u8) {
        if self.is_nina {
            match addr {
                0x7FFD => self.prg_bank = (data & 0x01) as usize,
                0x7FFE => self.chr_bank_lo = (data & 0x0F) as usize,
                0x7FFF => self.chr_bank_hi = (data & 0x0F) as usize,
                _ => {}
            }
        }
    }

    fn chr_offset(&self, addr: u16) -> usize {
        if self.is_nina {
            if addr & 0x1000 == 0 {
                self.chr_bank_lo * 0x1000 + (addr & 0x0FFF) as usize
            } else {
                self.chr_bank_hi * 0x1000 + (addr & 0x0FFF) as usize
            }
        } else {
            (addr & 0x1FFF) as usize
        }
    }
}

// ---------------------------------------------------------------------------
// Mapper 206 — Namco 118 / DxROM (the non-IRQ MMC3 ancestor)
//
// The MMC3 register interface ($8000/$8001 even/odd pair, R0-R7) minus the
// features MMC3 added later: no IRQ, no A12 clocking, no PRG-mode/CHR-invert
// bits (bank-select bits 6-7 ignored), and mirroring is hardwired (header).
// R0/R1 select 2 KB CHR banks, R2-R5 1 KB banks; R6/R7 select the two
// switchable 8 KB PRG banks at $8000/$A000, with $C000/$E000 fixed to the last
// two 8 KB banks.
// ---------------------------------------------------------------------------

struct Namco118 {
    bank_select: u8,
    bank_regs: [u8; 8],
}

impl Namco118 {
    fn new() -> Self {
        Self {
            bank_select: 0,
            bank_regs: [0; 8],
        }
    }
}

impl Mapper for Namco118 {
    fn prg_offset(&self, rom_len: usize, addr: u16) -> usize {
        let banks = (rom_len / 0x2000).max(1);
        let last = banks - 1;
        let bank = match (addr >> 13) & 0x03 {
            0 => self.bank_regs[6] as usize,
            1 => self.bank_regs[7] as usize,
            2 => last - 1,
            _ => last,
        };
        (bank % banks) * 0x2000 + (addr & 0x1FFF) as usize
    }

    fn chr_offset(&self, addr: u16) -> usize {
        let addr = addr & 0x1FFF;
        let bank = match addr >> 10 {
            0 | 1 => (self.bank_regs[0] & !1) as usize + (addr >> 10) as usize,
            2 | 3 => (self.bank_regs[1] & !1) as usize + ((addr >> 10) as usize - 2),
            r => self.bank_regs[r as usize - 2] as usize,
        };
        bank * 0x400 + (addr & 0x3FF) as usize
    }

    fn write_prg(&mut self, addr: u16, data: u8) {
        match addr & 0xE001 {
            0x8000 => self.bank_select = data & 0x07,
            0x8001 => self.bank_regs[(self.bank_select & 0x07) as usize] = data,
            _ => {} // no mirroring/IRQ registers on the Namco 118
        }
    }
}

// ---------------------------------------------------------------------------
// Mapper 9 / 10 — MMC2 (PxROM) / MMC4 (FxROM)
//
// Both carry the same automatic CHR bank latch: the PPU fetching tile $FD or
// $FE from a pattern table flips that table's latch, so the CHR bank used for
// the LEFT ($0000) and RIGHT ($1000) halves each switch between an "FD" and an
// "FE" bank as rendering scans past the trigger tiles (Punch-Out!!'s big
// opponents; Fire Emblem's portraits). The latch is fed from `ppu_bus_addr`,
// which the PPU calls with every pattern-fetch address; we key on the tile
// number, which is robust to whether the notified address is the low- or
// high-plane fetch (MMC2 triggers on the exact $xFD8/$xFE8 high-plane fetch,
// MMC4 on the $xFD8-$xFDF/$xFE8-$xFEF range — identical at tile granularity).
//
// They differ only in PRG layout: MMC2 switches an 8 KB bank at $8000 with the
// last three 8 KB banks fixed; MMC4 switches a 16 KB bank at $8000 with the
// last 16 KB fixed at $C000. The register interface is shared:
//   $A000 — PRG bank
//   $B000/$C000 — CHR bank for $0000 when its latch reads FD / FE
//   $D000/$E000 — CHR bank for $1000 when its latch reads FD / FE
//   $F000 — mirroring (bit 0: 0=vertical, 1=horizontal)
// ---------------------------------------------------------------------------

/// The shared CHR bank latch + register state of MMC2/MMC4.
struct PxLatch {
    prg_bank: usize,
    chr_lo_fd: usize,
    chr_lo_fe: usize,
    chr_hi_fd: usize,
    chr_hi_fe: usize,
    /// Which bank each half currently selects: false = FD, true = FE.
    latch_lo_fe: bool,
    latch_hi_fe: bool,
    mirroring: u8,
}

impl PxLatch {
    fn new() -> Self {
        Self {
            prg_bank: 0,
            chr_lo_fd: 0,
            chr_lo_fe: 0,
            chr_hi_fd: 0,
            chr_hi_fe: 0,
            latch_lo_fe: false,
            latch_hi_fe: false,
            mirroring: 0,
        }
    }

    /// Apply a shared ($A000-$F000) register write. Returns false if `addr`
    /// isn't one of the shared registers, so the PRG-bank register (which
    /// differs in width between MMC2 and MMC4) can be handled by the caller.
    fn write_common(&mut self, addr: u16, data: u8) {
        match addr & 0xF000 {
            0xB000 => self.chr_lo_fd = (data & 0x1F) as usize,
            0xC000 => self.chr_lo_fe = (data & 0x1F) as usize,
            0xD000 => self.chr_hi_fd = (data & 0x1F) as usize,
            0xE000 => self.chr_hi_fe = (data & 0x1F) as usize,
            0xF000 => self.mirroring = data & 1,
            _ => {}
        }
    }

    fn chr_offset(&self, addr: u16) -> usize {
        if addr & 0x1000 == 0 {
            let bank = if self.latch_lo_fe {
                self.chr_lo_fe
            } else {
                self.chr_lo_fd
            };
            bank * 0x1000 + (addr & 0x0FFF) as usize
        } else {
            let bank = if self.latch_hi_fe {
                self.chr_hi_fe
            } else {
                self.chr_hi_fd
            };
            bank * 0x1000 + (addr & 0x0FFF) as usize
        }
    }

    fn mirroring(&self) -> Mirroring {
        if self.mirroring & 1 == 0 {
            Mirroring::Vertical
        } else {
            Mirroring::Horizontal
        }
    }

    /// Flip the latches from a pattern-fetch address (ignored for non-pattern
    /// addresses). Tile $FD selects the FD bank, tile $FE the FE bank.
    fn clock_latch(&mut self, addr: u16) {
        if addr >= 0x2000 {
            return;
        }
        let tile = (addr >> 4) & 0xFF;
        let high_table = addr & 0x1000 != 0;
        match tile {
            0xFD => {
                if high_table {
                    self.latch_hi_fe = false;
                } else {
                    self.latch_lo_fe = false;
                }
            }
            0xFE => {
                if high_table {
                    self.latch_hi_fe = true;
                } else {
                    self.latch_lo_fe = true;
                }
            }
            _ => {}
        }
    }
}

struct Mmc2 {
    latch: PxLatch,
}

impl Mmc2 {
    fn new() -> Self {
        Self {
            latch: PxLatch::new(),
        }
    }
}

impl Mapper for Mmc2 {
    fn prg_offset(&self, rom_len: usize, addr: u16) -> usize {
        let banks = (rom_len / 0x2000).max(1);
        let last = banks - 1;
        // 8 KB switchable at $8000; the last three 8 KB banks fixed above it.
        let bank = match (addr >> 13) & 0x03 {
            0 => self.latch.prg_bank,
            1 => last - 2,
            2 => last - 1,
            _ => last,
        };
        (bank % banks) * 0x2000 + (addr & 0x1FFF) as usize
    }

    fn write_prg(&mut self, addr: u16, data: u8) {
        if addr & 0xF000 == 0xA000 {
            self.latch.prg_bank = (data & 0x0F) as usize;
        } else {
            self.latch.write_common(addr, data);
        }
    }

    fn chr_offset(&self, addr: u16) -> usize {
        self.latch.chr_offset(addr)
    }

    fn mirroring(&self) -> Option<Mirroring> {
        Some(self.latch.mirroring())
    }

    fn ppu_bus_addr(&mut self, addr: u16, _dots: u64) {
        self.latch.clock_latch(addr);
    }
}

struct Mmc4 {
    latch: PxLatch,
}

impl Mmc4 {
    fn new() -> Self {
        Self {
            latch: PxLatch::new(),
        }
    }
}

impl Mapper for Mmc4 {
    fn prg_offset(&self, rom_len: usize, addr: u16) -> usize {
        let banks = (rom_len / 0x4000).max(1);
        // 16 KB switchable at $8000; the last 16 KB fixed at $C000.
        if addr < 0xC000 {
            (self.latch.prg_bank % banks) * 0x4000 + (addr - 0x8000) as usize
        } else {
            (banks - 1) * 0x4000 + (addr - 0xC000) as usize
        }
    }

    fn write_prg(&mut self, addr: u16, data: u8) {
        if addr & 0xF000 == 0xA000 {
            self.latch.prg_bank = (data & 0x0F) as usize;
        } else {
            self.latch.write_common(addr, data);
        }
    }

    fn chr_offset(&self, addr: u16) -> usize {
        self.latch.chr_offset(addr)
    }

    fn mirroring(&self) -> Option<Mirroring> {
        Some(self.latch.mirroring())
    }

    fn ppu_bus_addr(&mut self, addr: u16, _dots: u64) {
        self.latch.clock_latch(addr);
    }
}

// ---------------------------------------------------------------------------
// Mapper 69 — Sunsoft FME-7 (5B)
//
// A command/parameter register pair: writing $8000 selects one of 16 internal
// registers, writing $A000 supplies its value.
//   0-7 — 1 KB CHR bank for the eight $0000-$1FFF slots
//   8   — PRG bank at $6000 (RAM/ROM select; we always keep the 8 KB PRG-RAM,
//         so this is ignored — no FME-7 game in scope maps ROM there)
//   9/A/B — 8 KB PRG banks at $8000/$A000/$C000 ($E000 fixed to the last bank)
//   C   — mirroring (0=V, 1=H, 2=single-A, 3=single-B)
//   D   — IRQ control: bit 0 = IRQ enable, bit 7 = counter enable; writing it
//         also acknowledges a pending IRQ
//   E/F — IRQ counter low/high byte
// The IRQ counter is a 16-bit down-counter clocked every CPU cycle while the
// counter is enabled; the borrow out of bit 15 ($0000 → $FFFF) asserts IRQ if
// IRQ-enable is set. The board's optional 5B expansion audio ($C000/$E000) is
// not modeled — the emulator has no audio-output path to mix it into yet.
// ---------------------------------------------------------------------------

struct Fme7 {
    command: u8,
    chr_regs: [u8; 8],
    prg_regs: [u8; 3], // $8000, $A000, $C000
    mirroring: u8,
    irq_counter: u16,
    irq_enable: bool,
    irq_counter_enable: bool,
    irq_flag: bool,
}

impl Fme7 {
    fn new() -> Self {
        Self {
            command: 0,
            chr_regs: [0; 8],
            prg_regs: [0; 3],
            mirroring: 0,
            irq_counter: 0,
            irq_enable: false,
            irq_counter_enable: false,
            irq_flag: false,
        }
    }
}

impl Mapper for Fme7 {
    fn prg_offset(&self, rom_len: usize, addr: u16) -> usize {
        let banks = (rom_len / 0x2000).max(1);
        let last = banks - 1;
        let bank = match (addr >> 13) & 0x03 {
            0 => self.prg_regs[0] as usize, // $8000-$9FFF
            1 => self.prg_regs[1] as usize, // $A000-$BFFF
            2 => self.prg_regs[2] as usize, // $C000-$DFFF
            _ => last,                      // $E000-$FFFF fixed
        };
        (bank % banks) * 0x2000 + (addr & 0x1FFF) as usize
    }

    fn chr_offset(&self, addr: u16) -> usize {
        let slot = ((addr >> 10) & 0x07) as usize;
        self.chr_regs[slot] as usize * 0x400 + (addr & 0x3FF) as usize
    }

    fn mirroring(&self) -> Option<Mirroring> {
        Some(match self.mirroring & 0x03 {
            0 => Mirroring::Vertical,
            1 => Mirroring::Horizontal,
            2 => Mirroring::SingleLow,
            _ => Mirroring::SingleHigh,
        })
    }

    fn write_prg(&mut self, addr: u16, data: u8) {
        match addr & 0xE000 {
            0x8000 => self.command = data & 0x0F,
            0xA000 => match self.command {
                0..=7 => self.chr_regs[self.command as usize] = data,
                8 => {} // PRG bank at $6000 — RAM kept, ROM mapping not modeled
                9 => self.prg_regs[0] = data & 0x3F,
                0xA => self.prg_regs[1] = data & 0x3F,
                0xB => self.prg_regs[2] = data & 0x3F,
                0xC => self.mirroring = data & 0x03,
                0xD => {
                    self.irq_enable = data & 0x01 != 0;
                    self.irq_counter_enable = data & 0x80 != 0;
                    self.irq_flag = false; // acknowledge
                }
                0xE => self.irq_counter = (self.irq_counter & 0xFF00) | data as u16,
                0xF => self.irq_counter = (self.irq_counter & 0x00FF) | ((data as u16) << 8),
                _ => {}
            },
            // $C000/$E000: 5B expansion audio — not modeled (no audio path).
            _ => {}
        }
    }

    fn tick_cpu(&mut self, cycles: u64) {
        if !self.irq_counter_enable {
            return;
        }
        for _ in 0..cycles {
            if self.irq_counter == 0 {
                self.irq_counter = 0xFFFF;
                if self.irq_enable {
                    self.irq_flag = true;
                }
            } else {
                self.irq_counter -= 1;
            }
        }
    }

    fn irq_pending(&self) -> bool {
        self.irq_flag
    }
}
