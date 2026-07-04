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

    pub fn read(&self, addr: u16) -> u8 {
        match addr {
            0x6000..=0x7FFF => self.prg_ram[(addr - 0x6000) as usize],
            0x8000..=0xFFFF => {
                let offset = self.mapper.prg_offset(self.prg_rom.len(), addr);
                self.prg_rom.get(offset).copied().unwrap_or(0)
            }
            _ => 0,
        }
    }

    pub fn write(&mut self, addr: u16, data: u8) {
        match addr {
            0x6000..=0x7FFF => self.prg_ram[(addr - 0x6000) as usize] = data,
            0x8000..=0xFFFF => self.mapper.write_prg(addr, data),
            _ => {}
        }
    }

    /// Read from the PPU pattern table space ($0000–$1FFF).
    pub fn chr_read(&self, addr: u16) -> u8 {
        if self.chr_rom.is_empty() {
            self.chr_ram[(addr & 0x1FFF) as usize]
        } else {
            let offset = self.mapper.chr_offset(addr);
            self.chr_rom.get(offset).copied().unwrap_or(0)
        }
    }

    /// Write to the PPU pattern table space (CHR-RAM only; ignored for CHR-ROM).
    pub fn chr_write(&mut self, addr: u16, data: u8) {
        if self.chr_rom.is_empty() {
            self.chr_ram[(addr & 0x1FFF) as usize] = data;
        }
    }

    /// Current nametable mirroring mode. Mappers may override the header value.
    pub fn mirroring(&self) -> Mirroring {
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
