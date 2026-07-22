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

// ---------------------------------------------------------------------------
// Helpers for the mappers whose banking granularity is finer than 16 KB PRG /
// 8 KB CHR. `build_banked` fills each 8 KB PRG bank with its bank index and
// each 1 KB CHR page with its page index, so a read at any bus address reveals
// exactly which physical 8 KB PRG bank / 1 KB CHR page it maps to, at any
// mapper's banking granularity. `flags6` supplies the low nibble of iNES byte
// 6 (bit 0 = vertical mirroring).
// ---------------------------------------------------------------------------

fn build_banked(mapper: u8, prg_8k: u16, chr_1k: u16, flags6: u8) -> Vec<u8> {
    assert!(
        prg_8k % 2 == 0,
        "PRG size must be a whole number of 16 KB banks"
    );
    assert!(
        chr_1k % 8 == 0,
        "CHR size must be a whole number of 8 KB banks"
    );
    let mut data = vec![0u8; 16];
    data[0..4].copy_from_slice(b"NES\x1A");
    data[4] = (prg_8k / 2) as u8;
    data[5] = (chr_1k / 8) as u8;
    data[6] = ((mapper & 0x0F) << 4) | (flags6 & 0x0F);
    data[7] = mapper & 0xF0;
    for bank in 0..prg_8k {
        data.extend(std::iter::repeat_n(bank as u8, 8192));
    }
    for page in 0..chr_1k {
        data.extend(std::iter::repeat_n(page as u8, 1024));
    }
    data
}

fn load_banked(mapper: u8, prg_8k: u16, chr_1k: u16, flags6: u8) -> Cartridge {
    Cartridge::from_ines(&build_banked(mapper, prg_8k, chr_1k, flags6))
        .unwrap_or_else(|e| panic!("failed to load synthetic mapper-{mapper} image: {e}"))
}

// --- Mapper 11 (Color Dreams) ---

#[test]
fn color_dreams_switches_prg_32k_and_chr_8k() {
    // 16 × 8 KB PRG = 4 × 32 KB banks; 32 × 1 KB CHR = 4 × 8 KB banks.
    let mut cart = load_banked(11, 16, 32, 0);
    for pb in 0..4u8 {
        for cb in 0..4u8 {
            cart.write(0x8000, pb | (cb << 4));
            // A 32 KB PRG bank pb covers 8 KB banks 4pb..4pb+3.
            assert_eq!(cart.read(0x8000), Some(pb * 4), "prg {pb} slot0");
            assert_eq!(cart.read(0xE000), Some(pb * 4 + 3), "prg {pb} slot3");
            // An 8 KB CHR bank cb covers 1 KB pages 8cb..8cb+7.
            assert_eq!(cart.chr_read(0x0000), cb * 8, "chr {cb} page0");
            assert_eq!(cart.chr_read(0x1FFF), cb * 8 + 7, "chr {cb} page7");
        }
    }
}

// --- Mapper 66 (GxROM) ---

#[test]
fn gxrom_switches_prg_32k_high_nibble_and_chr_8k_low_nibble() {
    let mut cart = load_banked(66, 16, 32, 0);
    for pb in 0..4u8 {
        for cb in 0..4u8 {
            cart.write(0x8000, (pb << 4) | cb);
            assert_eq!(cart.read(0x8000), Some(pb * 4));
            assert_eq!(cart.read(0xFFFF), Some(pb * 4 + 3));
            assert_eq!(cart.chr_read(0x0000), cb * 8);
        }
    }
}

// --- Mapper 71 (Camerica / Codemasters) ---

#[test]
fn camerica_switches_16k_low_via_c000_and_fixes_last() {
    // 16 × 8 KB = 8 × 16 KB PRG banks; CHR-RAM (no CHR-ROM).
    let mut cart = load_banked(71, 16, 0, 0);
    assert_eq!(cart.read(0x8000), Some(0));
    assert_eq!(cart.read(0xC000), Some(14)); // last 16 KB bank = 8 KB banks 14,15
    for b in [1u8, 3, 7, 0] {
        cart.write(0xC000, b);
        assert_eq!(cart.read(0x8000), Some(b * 2), "select 16K bank {b}");
        assert_eq!(cart.read(0xC000), Some(14), "last stays fixed");
    }
}

#[test]
fn camerica_bf9097_mirroring_override_via_8000() {
    let mut cart = load_banked(71, 16, 0, 0x01); // header = vertical
    // No $8000-$9FFF write yet: header mirroring is used.
    assert_eq!(cart.mirroring(), Mirroring::Vertical);
    cart.write(0x8000, 0x10);
    assert_eq!(cart.mirroring(), Mirroring::SingleHigh);
    cart.write(0x9000, 0x00);
    assert_eq!(cart.mirroring(), Mirroring::SingleLow);
}

// --- Mapper 87 (Jaleco/Konami CHR select) ---

#[test]
fn mapper87_chr_bank_via_6000_is_bit_swapped() {
    let mut cart = load_banked(87, 4, 32, 0);
    // Low two data bits are swapped into the bank number: D1 → bit0, D0 → bit1.
    for (data, bank) in [(0u8, 0u8), (1, 2), (2, 1), (3, 3)] {
        cart.write(0x6000, data);
        assert_eq!(
            cart.chr_read(0x0000),
            bank * 8,
            "data {data} → chr bank {bank}"
        );
    }
    // PRG stays fixed NROM-style regardless of register writes.
    assert_eq!(cart.read(0x8000), Some(0));
    assert_eq!(cart.read(0xE000), Some(3));
}

// --- Mapper 34 (BNROM / NINA-001) ---

#[test]
fn bnrom_switches_prg_32k_with_chr_ram() {
    // No CHR-ROM ⇒ BNROM board (CHR-RAM).
    let mut cart = load_banked(34, 16, 0, 0);
    assert_eq!(cart.read(0x8000), Some(0));
    assert_eq!(cart.read(0xFFFF), Some(3));
    for b in [1u8, 3, 0] {
        cart.write(0x8000, b);
        assert_eq!(cart.read(0x8000), Some(b * 4));
        assert_eq!(cart.read(0xFFFF), Some(b * 4 + 3));
    }
    cart.chr_write(0x0000, 0xAB);
    assert_eq!(cart.chr_read(0x0000), 0xAB, "CHR-RAM is writable/unbanked");
}

#[test]
fn nina001_registers_live_in_prg_ram_space() {
    // Has CHR-ROM ⇒ NINA-001 board; 16 × 1 KB CHR = 4 × 4 KB banks.
    let mut cart = load_banked(34, 8, 16, 0);
    // $7FFD selects a 32 KB PRG bank (1 bit): 8 × 8 KB = 2 × 32 KB banks.
    cart.write(0x7FFD, 1);
    assert_eq!(cart.read(0x8000), Some(4));
    cart.write(0x7FFD, 0);
    assert_eq!(cart.read(0x8000), Some(0));
    // $7FFE / $7FFF select the two independent 4 KB CHR banks.
    cart.write(0x7FFE, 3); // $0000 half → 4 KB bank 3 → 1 KB pages 12..15
    cart.write(0x7FFF, 1); // $1000 half → 4 KB bank 1 → 1 KB pages 4..7
    assert_eq!(cart.chr_read(0x0000), 12);
    assert_eq!(cart.chr_read(0x1000), 4);
}

// --- Mapper 206 (Namco 118 / DxROM) ---

#[test]
fn namco118_mmc3_style_banking_without_irq() {
    let mut cart = load_banked(206, 32, 64, 0);
    // PRG: R6 → $8000, R7 → $A000; $C000/$E000 fixed to the last two 8 KB banks.
    cart.write(0x8000, 6);
    cart.write(0x8001, 3);
    cart.write(0x8000, 7);
    cart.write(0x8001, 5);
    assert_eq!(cart.read(0x8000), Some(3));
    assert_eq!(cart.read(0xA000), Some(5));
    assert_eq!(cart.read(0xC000), Some(30));
    assert_eq!(cart.read(0xE000), Some(31));
    // CHR: R0/R1 are 2 KB banks (low bit ignored), R2-R5 are 1 KB banks.
    for (reg, val) in [(0u8, 4u8), (1, 8), (2, 20), (3, 21), (4, 22), (5, 23)] {
        cart.write(0x8000, reg);
        cart.write(0x8001, val);
    }
    assert_eq!(cart.chr_read(0x0000), 4); // R0 → pages 4,5
    assert_eq!(cart.chr_read(0x0400), 5);
    assert_eq!(cart.chr_read(0x0800), 8); // R1 → pages 8,9
    assert_eq!(cart.chr_read(0x1000), 20); // R2
    assert_eq!(cart.chr_read(0x1C00), 23); // R5
    // The Namco 118 has no IRQ counter — MMC3 IRQ-register writes do nothing.
    cart.write(0xC000, 1);
    cart.write(0xC001, 0);
    cart.write(0xE001, 0);
    assert!(!cart.irq_pending());
}

// --- Mapper 9 (MMC2 / PxROM) ---

#[test]
fn mmc2_prg_layout_and_automatic_chr_latch() {
    // 16 × 8 KB PRG; 32 × 1 KB CHR = 8 × 4 KB banks.
    let mut cart = load_banked(9, 16, 32, 0);

    // PRG: 8 KB switchable at $8000, last three 8 KB banks fixed above it.
    cart.write(0xA000, 3);
    assert_eq!(cart.read(0x8000), Some(3));
    assert_eq!(cart.read(0xA000), Some(13));
    assert_eq!(cart.read(0xC000), Some(14));
    assert_eq!(cart.read(0xE000), Some(15));

    // CHR: one 4 KB bank per half, each selected by its FD/FE latch.
    cart.write(0xB000, 1); // $0000 when latch = FD → 4 KB bank 1 → pages 4..7
    cart.write(0xC000, 2); // $0000 when latch = FE → bank 2 → pages 8..11
    cart.write(0xD000, 3); // $1000 when latch = FD → bank 3 → pages 12..15
    cart.write(0xE000, 5); // $1000 when latch = FE → bank 5 → pages 20..23

    // Latches power up reading FD.
    assert_eq!(cart.chr_read(0x0000), 4);
    assert_eq!(cart.chr_read(0x1000), 12);

    // Fetching tile $FE from the low pattern table flips the low latch to FE;
    // the high latch is independent.
    cart.ppu_bus_addr(0x0FE0, 0);
    assert_eq!(cart.chr_read(0x0000), 8);
    assert_eq!(cart.chr_read(0x1000), 12);
    // Fetching tile $FD flips it back.
    cart.ppu_bus_addr(0x0FD0, 0);
    assert_eq!(cart.chr_read(0x0000), 4);
    // The high latch responds only to high-table fetches.
    cart.ppu_bus_addr(0x1FE0, 0);
    assert_eq!(cart.chr_read(0x1000), 20);
    // Non-pattern (nametable) addresses never touch the latch.
    cart.ppu_bus_addr(0x2FE0, 0);
    assert_eq!(cart.chr_read(0x1000), 20);

    // $F000 controls mirroring.
    cart.write(0xF000, 1);
    assert_eq!(cart.mirroring(), Mirroring::Horizontal);
    cart.write(0xF000, 0);
    assert_eq!(cart.mirroring(), Mirroring::Vertical);
}

// --- Mapper 10 (MMC4 / FxROM) ---

#[test]
fn mmc4_prg_layout_and_shared_chr_latch() {
    // 16 × 8 KB = 8 × 16 KB PRG; CHR banking identical to MMC2.
    let mut cart = load_banked(10, 16, 32, 0);

    // PRG: 16 KB switchable at $8000, last 16 KB fixed at $C000.
    cart.write(0xA000, 2);
    assert_eq!(cart.read(0x8000), Some(4)); // 16 KB bank 2 = 8 KB banks 4,5
    assert_eq!(cart.read(0xBFFF), Some(5));
    assert_eq!(cart.read(0xC000), Some(14)); // last 16 KB bank = 8 KB banks 14,15
    assert_eq!(cart.read(0xE000), Some(15));

    // Same FD/FE latch behavior as MMC2.
    cart.write(0xB000, 1);
    cart.write(0xC000, 2);
    assert_eq!(cart.chr_read(0x0000), 4);
    cart.ppu_bus_addr(0x0FE8, 0); // MMC4 triggers on the $xFE8-$xFEF range
    assert_eq!(cart.chr_read(0x0000), 8);
}

// --- Mapper 69 (Sunsoft FME-7) ---

#[test]
fn fme7_prg_chr_banking_and_mirroring() {
    let mut cart = load_banked(69, 16, 32, 0);
    // PRG banks: command 9/A/B set the $8000/$A000/$C000 slots; $E000 is fixed.
    for (cmd, val) in [(9u8, 3u8), (0xA, 5), (0xB, 7)] {
        cart.write(0x8000, cmd);
        cart.write(0xA000, val);
    }
    assert_eq!(cart.read(0x8000), Some(3));
    assert_eq!(cart.read(0xA000), Some(5));
    assert_eq!(cart.read(0xC000), Some(7));
    assert_eq!(cart.read(0xE000), Some(15)); // fixed to the last 8 KB bank

    // CHR: commands 0-7 select the eight 1 KB slots.
    cart.write(0x8000, 0);
    cart.write(0xA000, 10);
    cart.write(0x8000, 7);
    cart.write(0xA000, 20);
    assert_eq!(cart.chr_read(0x0000), 10);
    assert_eq!(cart.chr_read(0x1C00), 20);

    // Command C sets mirroring (0=V, 1=H, 2=single-A, 3=single-B).
    cart.write(0x8000, 0xC);
    cart.write(0xA000, 1);
    assert_eq!(cart.mirroring(), Mirroring::Horizontal);
    cart.write(0xA000, 2); // command C persists until changed
    assert_eq!(cart.mirroring(), Mirroring::SingleLow);
}

#[test]
fn fme7_cpu_cycle_irq_counter_asserts_on_underflow() {
    let mut cart = load_banked(69, 16, 32, 0);
    // Counter = 10, then enable both the counter and IRQ output (command D).
    cart.write(0x8000, 0xE);
    cart.write(0xA000, 10); // counter low
    cart.write(0x8000, 0xF);
    cart.write(0xA000, 0); // counter high
    cart.write(0x8000, 0xD);
    cart.write(0xA000, 0x81); // counter-enable | irq-enable
    assert!(!cart.irq_pending());

    // Ten cycles bring the counter to 0 with no assert yet.
    cart.tick_cpu(10);
    assert!(!cart.irq_pending());
    // The next cycle underflows 0 → $FFFF and asserts IRQ.
    cart.tick_cpu(1);
    assert!(cart.irq_pending());

    // Rewriting command D acknowledges the pending IRQ.
    cart.write(0x8000, 0xD);
    cart.write(0xA000, 0x00);
    assert!(!cart.irq_pending());
}

#[test]
fn fme7_irq_counter_frozen_while_counter_disabled() {
    let mut cart = load_banked(69, 16, 32, 0);
    cart.write(0x8000, 0xE);
    cart.write(0xA000, 2);
    cart.write(0x8000, 0xF);
    cart.write(0xA000, 0);
    // IRQ enabled but counter disabled (bit 7 clear): no ticking, no IRQ.
    cart.write(0x8000, 0xD);
    cart.write(0xA000, 0x01);
    cart.tick_cpu(100);
    assert!(!cart.irq_pending());
}
