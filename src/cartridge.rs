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
            0x6000..=0x7FFF => self.prg_ram[(addr - 0x6000) as usize] = data,
            0x8000..=0xFFFF => self.mapper.write_prg(addr, data),
            _ => {}
        }
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
    /// MMC3 uses it to clock its A12-driven IRQ counter.
    fn ppu_bus_addr(&mut self, _addr: u16, _dots: u64) {}
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
