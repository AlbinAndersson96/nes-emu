use std::path::PathBuf;

use crate::cartridge::{Cartridge, CartridgeError};

fn load_rom(name: &str) -> Vec<u8> {
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests/roms")
        .join(name);
    std::fs::read(&path).unwrap_or_else(|_| panic!("ROM not found: {}", path.display()))
}

/// These suites need a mapper `src/cartridge.rs` doesn't implement yet (only
/// mapper 0/NROM and mapper 1/MMC1 exist). Until mapper 3 (CNROM) and mapper 4
/// (MMC3) support is added, all we can verify is that loading fails cleanly
/// instead of panicking.
fn assert_unsupported_mapper(filename: &str, expected_mapper: u8) {
    let data = load_rom(filename);
    match Cartridge::from_ines(&data) {
        Err(CartridgeError::UnsupportedMapper(id)) => {
            assert_eq!(
                id, expected_mapper,
                "{filename}: expected mapper {expected_mapper}, header says {id}"
            );
        }
        Err(e) => panic!("{filename}: expected UnsupportedMapper({expected_mapper}), got {e}"),
        Ok(_) => panic!(
            "{filename}: loaded successfully — mapper {expected_mapper} support may have been \
             added; this suite should be moved to a real test harness"
        ),
    }
}

#[test]
fn mmc3_test_unsupported() {
    assert_unsupported_mapper("mmc3_test/1-clocking.nes", 4);
}

#[test]
fn mmc3_test_2_unsupported() {
    assert_unsupported_mapper("mmc3_test_2/rom_singles/1-clocking.nes", 4);
}

#[test]
fn mmc3_irq_tests_unsupported() {
    assert_unsupported_mapper("mmc3_irq_tests/1.Clocking.nes", 4);
}

#[test]
fn cpu_dummy_reads_unsupported() {
    assert_unsupported_mapper("cpu_dummy_reads/cpu_dummy_reads.nes", 3);
}

#[test]
fn ppu_read_buffer_unsupported() {
    assert_unsupported_mapper("ppu_read_buffer/test_ppu_read_buffer.nes", 3);
}
