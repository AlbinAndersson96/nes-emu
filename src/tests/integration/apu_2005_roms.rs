//! Harness for blargg's 2005-era APU frame-counter/length-counter suite
//! (`tests/roms/blargg_apu_2005.07.30/`). These ROMs predate the $6000
//! result protocol: like the 2005 PPU suite (`ppu_roms.rs`), each ROM
//! displays a result code on screen ("$XX" in ASCII tiles at nametable-0
//! row 5, col 2-4) and beeps it out. Code $01 means pass; other codes are
//! looked up in the per-ROM tables from `tests.txt`.

use std::path::PathBuf;

use crate::bus::Bus;
use crate::cartridge::Cartridge;
use crate::cpu::Cpu;
use crate::system::SystemClock;

// Run each ROM for 5 seconds of NES time (~300 frames at 60 fps) — the
// same budget as the 2005 PPU suite; every ROM settles on its verdict
// well within it.
const FRAMES: u32 = 300;

const ROM_DIR: &str = "tests/roms/nes-test-roms/blargg_apu_2005.07.30";

fn run_apu_rom(filename: &str) -> u8 {
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join(ROM_DIR)
        .join(filename);
    let data = std::fs::read(&path).unwrap_or_else(|_| panic!("ROM not found: {}", path.display()));
    let cartridge =
        Cartridge::from_ines(&data).unwrap_or_else(|e| panic!("failed to parse {filename}: {e}"));
    let mut bus = Bus::new();
    bus.insert_cartridge(cartridge);
    let mut cpu = Cpu::new();
    cpu.reset(&mut bus);
    // Match hardware's reset alignment (see roms.rs::run_rom).
    let _ = bus.tick_ppu(7);
    let _ = bus.tick_apu(8);

    let mut clock = SystemClock::new();
    let mut frames_done = 0u32;
    loop {
        clock.step(&mut cpu, &mut bus);

        if bus.ppu.frame_ready {
            bus.ppu.frame_ready = false;
            frames_done += 1;
            if frames_done >= FRAMES {
                // "$XX" at nametable-0 tile (row=5, col=2..4); ASCII-mapped
                // tiles, so subtracting 0x30 yields the digit.
                return bus.ppu.nt0_tile(5, 4).wrapping_sub(0x30);
            }
        }
    }
}

/// Run `filename` and assert its on-screen result code is 1 (pass).
///
/// `meanings` maps failure codes 2.. to their descriptions (index 0 =
/// code 2, etc.) — taken directly from the suite's `tests.txt`.
fn verify(filename: &str, meanings: &[&str]) {
    let code = run_apu_rom(filename);
    if code != 1 {
        let meaning = meanings
            .get(code.saturating_sub(2) as usize)
            .copied()
            .unwrap_or("unknown result code");
        panic!("{filename}: ROM reports result code {code} — {meaning}");
    }
}

#[test]
fn apu_2005_len_ctr() {
    verify(
        "01.len_ctr.nes",
        &[
            "Problem with length counter load or $4015",
            "Problem with length table, timing, or $4015",
            "Writing $80 to $4017 should clock length immediately",
            "Writing $00 to $4017 shouldn't clock length immediately",
            "Clearing enable bit in $4015 should clear length counter",
            "When disabled via $4015, length shouldn't allow reloading",
            "Halt bit should suspend length clocking",
        ],
    );
}

#[test]
fn apu_2005_len_table() {
    verify(
        "02.len_table.nes",
        &[
            "Bad length table entry (screen shows $ll $ee $cc: load value, emulator value, correct value)",
        ],
    );
}

#[test]
fn apu_2005_irq_flag() {
    verify(
        "03.irq_flag.nes",
        &[
            "Flag shouldn't be set in $4017 mode $40",
            "Flag shouldn't be set in $4017 mode $80",
            "Flag should be set in $4017 mode $00",
            "Reading flag should clear it",
            "Writing $00 or $80 to $4017 shouldn't affect flag",
            "Writing $40 or $c0 to $4017 should clear flag",
        ],
    );
}

#[test]
fn apu_2005_clock_jitter() {
    verify(
        "04.clock_jitter.nes",
        &[
            "Frame irq is set too soon",
            "Frame irq is set too late",
            "Even jitter not handled properly",
            "Odd jitter not handled properly",
        ],
    );
}

#[test]
fn apu_2005_len_timing_mode0() {
    verify(
        "05.len_timing_mode0.nes",
        &[
            "First length is clocked too soon",
            "First length is clocked too late",
            "Second length is clocked too soon",
            "Second length is clocked too late",
            "Third length is clocked too soon",
            "Third length is clocked too late",
        ],
    );
}

#[test]
fn apu_2005_len_timing_mode1() {
    verify(
        "06.len_timing_mode1.nes",
        &[
            "First length is clocked too soon",
            "First length is clocked too late",
            "Second length is clocked too soon",
            "Second length is clocked too late",
            "Third length is clocked too soon",
            "Third length is clocked too late",
        ],
    );
}

#[test]
fn apu_2005_irq_flag_timing() {
    verify(
        "07.irq_flag_timing.nes",
        &[
            "Flag first set too soon",
            "Flag first set too late",
            "Flag last set too soon",
            "Flag last set too late",
        ],
    );
}

#[test]
fn apu_2005_irq_timing() {
    verify(
        "08.irq_timing.nes",
        &[
            "IRQ occurred too soon",
            "IRQ occurred too late",
            "IRQ never occurred",
        ],
    );
}

#[test]
fn apu_2005_reset_timing() {
    verify(
        "09.reset_timing.nes",
        &[
            "$4015 didn't read back as $00 at power-up",
            "Fourth step occurs too soon",
            "Fourth step occurs too late",
        ],
    );
}

#[test]
fn apu_2005_len_halt_timing() {
    verify(
        "10.len_halt_timing.nes",
        &[
            "Length shouldn't be clocked when halted at 14914",
            "Length should be clocked when halted at 14915",
            "Length should be clocked when unhalted at 14914",
            "Length shouldn't be clocked when unhalted at 14915",
        ],
    );
}

#[test]
fn apu_2005_len_reload_timing() {
    verify(
        "11.len_reload_timing.nes",
        &[
            "Reload just before length clock should work normally",
            "Reload just after length clock should work normally",
            "Reload during length clock when ctr = 0 should work normally",
            "Reload during length clock when ctr > 0 should be ignored",
        ],
    );
}
