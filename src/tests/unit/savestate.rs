//! Save-state round-trip tests.
//!
//! These exercise the whole-machine snapshot/restore path (`crate::savestate`)
//! against a real `Cpu`/`Bus`/`SystemClock` driving a synthetic NROM/UxROM
//! image, plus the `Cartridge` state helpers in isolation.

use crate::bus::Bus;
use crate::cartridge::Cartridge;
use crate::cpu::Cpu;
use crate::savestate;
use crate::system::SystemClock;

/// A tiny program that keeps the machine busy touching CPU registers, zero-page
/// RAM, and a PPU register (so many subsystems' state diverges over time):
///
/// ```text
///         LDX #$00
/// loop:   INX
///         TXA
///         STA $00,X       ; walk incrementing values through zero page
///         STA $2005       ; poke PPUSCROLL (toggles the w latch, moves t/x)
///         JMP loop
/// ```
const PROG: [u8; 12] = [
    0xA2, 0x00, // LDX #$00
    0xE8, // INX                (loop target = $8002)
    0x8A, // TXA
    0x95, 0x00, // STA $00,X
    0x8D, 0x05, 0x20, // STA $2005
    0x4C, 0x02, 0x80, // JMP $8002
];

/// Build a 16 KB single-bank NROM iNES image running `program` from $8000,
/// with the reset vector pointing at $8000 and CHR-RAM (no CHR-ROM).
fn build_nrom(program: &[u8]) -> Vec<u8> {
    let mut prg = vec![0xEAu8; 16384]; // NOP fill
    prg[..program.len()].copy_from_slice(program);
    prg[0x3FFC] = 0x00; // reset vector low  -> $8000
    prg[0x3FFD] = 0x80; // reset vector high
    let mut data = vec![0u8; 16];
    data[0..4].copy_from_slice(b"NES\x1A");
    data[4] = 1; // 1 × 16 KB PRG bank
    data[5] = 0; // 0 CHR banks -> CHR-RAM
    data.extend_from_slice(&prg);
    data
}

/// Build a UxROM (mapper 2) image with `prg_banks` 16 KB banks, bank n filled
/// with byte n (so a read reveals which physical bank is mapped).
fn build_uxrom(prg_banks: u8) -> Vec<u8> {
    let mut data = vec![0u8; 16];
    data[0..4].copy_from_slice(b"NES\x1A");
    data[4] = prg_banks;
    data[5] = 0;
    data[6] = 2 << 4; // mapper 2 low nibble
    for bank in 0..prg_banks {
        data.extend(std::iter::repeat_n(bank, 16384));
    }
    data
}

/// Boot a fresh machine from `rom`, mirroring `App::load_rom`'s power-on.
fn boot(rom: &[u8]) -> (Cpu, Bus, SystemClock) {
    let cart = Cartridge::from_ines(rom).expect("valid iNES image");
    let mut bus = Bus::new();
    bus.insert_cartridge(cart);
    let mut cpu = Cpu::new();
    cpu.reset(&mut bus);
    let _ = bus.tick_ppu(7);
    let _ = bus.tick_apu(8);
    (cpu, bus, SystemClock::new())
}

fn run(cpu: &mut Cpu, bus: &mut Bus, clock: &mut SystemClock, steps: usize) {
    for _ in 0..steps {
        clock.step(cpu, bus);
    }
}

#[test]
fn restore_then_save_reproduces_exact_bytes() {
    let rom = build_nrom(&PROG);
    let (mut cpu, mut bus, mut clock) = boot(&rom);
    run(&mut cpu, &mut bus, &mut clock, 20_000);

    let snap = savestate::save(&cpu, &bus, &clock).expect("save");

    // Restore into a fresh machine with the same ROM; re-serializing the
    // restored machine must reproduce the exact same bytes (nothing that the
    // snapshot round-trips through is lost or reordered).
    let (mut cpu2, mut bus2, mut clock2) = boot(&rom);
    savestate::load(&snap, &mut cpu2, &mut bus2, &mut clock2).expect("load");
    let snap2 = savestate::save(&cpu2, &bus2, &clock2).expect("re-save");
    assert_eq!(snap, snap2, "restore→save is not idempotent");
}

#[test]
fn restored_machine_evolves_identically() {
    let rom = build_nrom(&PROG);
    let (mut cpu, mut bus, mut clock) = boot(&rom);
    run(&mut cpu, &mut bus, &mut clock, 20_000);
    let snap0 = savestate::save(&cpu, &bus, &clock).expect("save");

    // Reference trajectory: run further from the original machine.
    run(&mut cpu, &mut bus, &mut clock, 7_000);
    let reference = savestate::save(&cpu, &bus, &clock).expect("save ref");

    // Restore the snapshot into a fresh machine and run the identical steps.
    // If any state affecting future evolution were omitted from the snapshot,
    // the two trajectories would diverge.
    let (mut cpu2, mut bus2, mut clock2) = boot(&rom);
    savestate::load(&snap0, &mut cpu2, &mut bus2, &mut clock2).expect("load");
    run(&mut cpu2, &mut bus2, &mut clock2, 7_000);
    let restored = savestate::save(&cpu2, &bus2, &clock2).expect("save restored");

    assert_eq!(
        reference, restored,
        "a machine restored from a save state must evolve bit-for-bit like the original"
    );
    // Sanity: the CPU actually advanced (the state isn't trivially frozen).
    assert!(cpu2.cycles > 0);
}

#[test]
fn load_rejects_garbage() {
    let rom = build_nrom(&PROG);
    let (mut cpu, mut bus, mut clock) = boot(&rom);
    let err = savestate::load(b"not a save state at all", &mut cpu, &mut bus, &mut clock)
        .expect_err("garbage must be rejected");
    assert!(!err.is_empty());
}

#[test]
fn cartridge_state_restores_prg_ram_and_mapper_banking() {
    let mut cart = Cartridge::from_ines(&build_uxrom(4)).expect("valid UxROM image");

    // Establish distinctive mutable state: select PRG bank 2 and stash a byte
    // in PRG-RAM.
    cart.write(0x8000, 2);
    cart.write(0x6000, 0xAB);
    assert_eq!(cart.read(0x8000), Some(2), "bank 2 mapped");
    assert_eq!(cart.read(0x6000), Some(0xAB), "PRG-RAM written");

    let saved = cart.capture_state();

    // Mutate away from the saved state.
    cart.write(0x8000, 0);
    cart.write(0x6000, 0x00);
    assert_eq!(cart.read(0x8000), Some(0));
    assert_eq!(cart.read(0x6000), Some(0x00));

    // Restore must bring back both the mapper register and PRG-RAM.
    cart.restore_state(&saved);
    assert_eq!(cart.read(0x8000), Some(2), "mapper bank restored");
    assert_eq!(cart.read(0x6000), Some(0xAB), "PRG-RAM restored");
}
