pub struct Cartridge {
    prg_rom: Vec<u8>,
    /// WRAM at $6000–$7FFF — always present regardless of the iNES header flag,
    /// because blargg-style test ROMs write results there unconditionally.
    prg_ram: [u8; 8192],
    mapper: Box<dyn Mapper>,
}

impl Cartridge {
    pub fn from_ines(data: &[u8]) -> Result<Self, &'static str> {
        if data.len() < 16 || &data[0..4] != b"NES\x1A" {
            return Err("not a valid iNES file");
        }
        let prg_banks = data[4] as usize;
        let mapper_id = (data[6] >> 4) | (data[7] & 0xF0);
        let has_trainer = data[6] & 0x04 != 0;
        let prg_start = 16 + if has_trainer { 512 } else { 0 };
        let prg_size = prg_banks * 16384;
        if data.len() < prg_start + prg_size {
            return Err("iNES file truncated");
        }
        let prg_rom = data[prg_start..prg_start + prg_size].to_vec();

        let mapper: Box<dyn Mapper> = match mapper_id {
            0 => Box::new(Nrom),
            1 => Box::new(Mmc1::new(prg_banks)),
            _ => return Err("unsupported mapper"),
        };

        Ok(Self { prg_rom, prg_ram: [0u8; 8192], mapper })
    }

    pub fn read(&self, addr: u16) -> u8 {
        match addr {
            0x6000..=0x7FFF => self.prg_ram[(addr - 0x6000) as usize],
            0x8000..=0xFFFF => self.mapper.read_prg(addr, &self.prg_rom),
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
}

// ---------------------------------------------------------------------------
// Mapper trait
// ---------------------------------------------------------------------------

trait Mapper {
    fn read_prg(&self, addr: u16, prg_rom: &[u8]) -> u8;
    fn write_prg(&mut self, addr: u16, data: u8);
}

// ---------------------------------------------------------------------------
// Mapper 0 — NROM
// ---------------------------------------------------------------------------

struct Nrom;

impl Mapper for Nrom {
    fn read_prg(&self, addr: u16, prg_rom: &[u8]) -> u8 {
        prg_rom[(addr - 0x8000) as usize % prg_rom.len()]
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

    fn prg_offset(&self, addr: u16) -> usize {
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
    fn read_prg(&self, addr: u16, prg_rom: &[u8]) -> u8 {
        prg_rom.get(self.prg_offset(addr)).copied().unwrap_or(0)
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
                _ =>               self.prg_bank = value,
            }
        }
    }
}
