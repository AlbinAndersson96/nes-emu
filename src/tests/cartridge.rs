//! Cartridge/mapper unit tests for the discrete-logic mappers that have no
//! blargg ROM suite in tests/roms/ (UxROM and AxROM). Each test builds a
//! minimal iNES image in memory with every 16 KB PRG bank filled with its
//! own index, so a single read reveals which bank a bus address hits.

use crate::cartridge::{Cartridge, Mirroring};

/// Builds an iNES image: `prg_banks` 16 KB PRG banks (bank n filled with
/// byte n) and `chr_banks` 8 KB CHR banks (bank n filled with 0x40 + n).
fn build_ines(mapper: u8, prg_banks: u8, chr_banks: u8) -> Vec<u8> {
    let mut data = vec![0u8; 16];
    data[0..4].copy_from_slice(b"NES\x1A");
    data[4] = prg_banks;
    data[5] = chr_banks;
    data[6] = (mapper & 0x0F) << 4; // low mapper nibble, horizontal mirroring
    data[7] = mapper & 0xF0;
    for bank in 0..prg_banks {
        data.extend(std::iter::repeat_n(bank, 16384));
    }
    for bank in 0..chr_banks {
        data.extend(std::iter::repeat_n(0x40 + bank, 8192));
    }
    data
}

fn load(mapper: u8, prg_banks: u8, chr_banks: u8) -> Cartridge {
    Cartridge::from_ines(&build_ines(mapper, prg_banks, chr_banks))
        .unwrap_or_else(|e| panic!("failed to load synthetic mapper-{mapper} image: {e}"))
}

// --- Mapper 2 (UxROM) ---

#[test]
fn uxrom_switches_low_bank_and_fixes_last() {
    let mut cart = load(2, 4, 0);

    // Power-on: bank 0 at $8000, last bank fixed at $C000.
    assert_eq!(cart.read(0x8000), Some(0));
    assert_eq!(cart.read(0xBFFF), Some(0));
    assert_eq!(cart.read(0xC000), Some(3));
    assert_eq!(cart.read(0xFFFF), Some(3));

    // Any $8000-$FFFF write selects the $8000 bank; $C000 stays fixed.
    for bank in [1u8, 2, 3, 0] {
        cart.write(0xFFFF, bank);
        assert_eq!(cart.read(0x8000), Some(bank), "after selecting bank {bank}");
        assert_eq!(cart.read(0xC000), Some(3), "after selecting bank {bank}");
    }
}

#[test]
fn uxrom_bank_select_wraps_at_bank_count() {
    let mut cart = load(2, 4, 0);
    // Register bits beyond the ROM's bank count wrap (open address lines).
    cart.write(0x8000, 6);
    assert_eq!(cart.read(0x8000), Some(6 % 4));
}

#[test]
fn uxrom_chr_ram_is_writable_and_unbanked() {
    let mut cart = load(2, 4, 0);
    cart.chr_write(0x0000, 0xAA);
    cart.chr_write(0x1FFF, 0x55);
    assert_eq!(cart.chr_read(0x0000), 0xAA);
    assert_eq!(cart.chr_read(0x1FFF), 0x55);
    // PRG bank writes must not disturb CHR-RAM contents.
    cart.write(0x8000, 1);
    assert_eq!(cart.chr_read(0x0000), 0xAA);
}

// --- Mapper 7 (AxROM) ---

#[test]
fn axrom_switches_32k_banks() {
    // 8 × 16 KB = 4 × 32 KB banks. 32 KB bank n covers 16 KB banks 2n, 2n+1.
    let mut cart = load(7, 8, 0);

    assert_eq!(cart.read(0x8000), Some(0));
    assert_eq!(cart.read(0xFFFF), Some(1));

    for bank in 0..4u8 {
        cart.write(0x8000, bank);
        assert_eq!(
            cart.read(0x8000),
            Some(bank * 2),
            "32K bank {bank}, low half"
        );
        assert_eq!(
            cart.read(0xFFFF),
            Some(bank * 2 + 1),
            "32K bank {bank}, high half"
        );
    }

    // Bank bits wrap at the ROM's bank count.
    cart.write(0x8000, 5);
    assert_eq!(cart.read(0x8000), Some((5 % 4) * 2));
}

#[test]
fn axrom_selects_single_screen_page_via_bit_4() {
    let mut cart = load(7, 8, 0);

    // AxROM ignores the header's mirroring bit: always single-screen,
    // page selected by bit 4 of the last write.
    assert_eq!(cart.mirroring(), Mirroring::SingleLow);
    cart.write(0x8000, 0x10);
    assert_eq!(cart.mirroring(), Mirroring::SingleHigh);
    cart.write(0x8000, 0x07);
    assert_eq!(cart.mirroring(), Mirroring::SingleLow);
    // Bank bits and the page bit are independent.
    cart.write(0x8000, 0x13);
    assert_eq!(cart.mirroring(), Mirroring::SingleHigh);
    assert_eq!(cart.read(0x8000), Some(3 * 2));
}
