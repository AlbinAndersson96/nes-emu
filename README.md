# nes-emu

A NES emulator written in Rust.

## Status

| Component | Status |
|-----------|--------|
| CPU — 6502 official opcodes | Complete |
| CPU — unofficial/illegal opcodes | Complete |
| PPU — VBlank timing ($2002) | Stub |
| PPU — rendering | Not started |
| APU | Not started |
| Cartridge — NROM (mapper 0) | Complete |
| Cartridge — MMC1 (mapper 1) | Complete |
| Controllers | Partial (serial shift register wired, no input source) |

All 17 blargg `instr_test-v5` ROM tests pass.

## Building and running

```bash
cargo build
cargo run <rom.nes>
cargo test
cargo clippy
cargo fmt
```

## Project layout

```
src/
  main.rs            — entry point, emulation loop
  bus.rs             — system bus: RAM, PPU, APU, controllers, cartridge
  ppu.rs             — PPU stub (VBlank flag only, no rendering)
  cartridge.rs       — iNES parser; mapper 0 (NROM) and mapper 1 (MMC1)
  cpu/
    mod.rs           — Cpu struct, registers, addressing modes, Bus trait
    instructions.rs  — opcode dispatcher (all official + unofficial opcodes)
  tests/
    mod.rs           — TestBus used by unit tests
    bus.rs           — bus unit tests
    cpu.rs           — CPU unit tests
    roms.rs          — blargg ROM test harness
docs/
  bus.md             — address map and bus design notes
  cpu_instructions.md — 6502 instruction reference
tests/roms/          — blargg instr_test-v5 ROM files
```

## Docs

- [`docs/bus.md`](docs/bus.md) — NES address map and bus design
- [`docs/cpu_instructions.md`](docs/cpu_instructions.md) — 6502 instruction reference

## What's next

- PPU rendering (background tiles, sprites, palette, scrolling)
- APU audio synthesis
- NMI / IRQ interrupt delivery
- Additional mappers (UxROM, CNxROM, MMC3, …)
- Window / display output
