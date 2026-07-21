//! Whole-ROM integration harnesses. Each boots a `.nes` image through the real
//! `Bus`/`Cpu`/`SystemClock` and asserts the ROM's own verdict — via the
//! blargg `$6000` result protocol (`roms`), on-screen result codes
//! (`ppu_roms`, `apu_2005_roms`, `text_console_roms`, `sprite_hit_roms`), or
//! the AccuracyCoin RAM dump (`accuracycoin`, `#[ignore]`d).

mod accuracycoin;
mod apu_2005_roms;
mod ppu_roms;
mod roms;
mod sprite_hit_roms;
mod text_console_roms;
