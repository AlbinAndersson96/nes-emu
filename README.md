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

All 17 blargg `instr_test-v5` ROM tests pass. Additional blargg suites
(`cpu_interrupts_v2`, `instr_misc`, `instr_timing`) are wired as tests and
reveal the next set of unimplemented features (IRQ/NMI delivery, APU frame
counter, RMW dummy reads — see CLAUDE.md "Known gaps").

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
tests/roms/          — blargg ROM files (instr_test-v5, cpu_interrupts_v2, instr_misc, instr_timing)
```

## Docs

- [`docs/bus.md`](docs/bus.md) — NES address map and bus design
- [`docs/cpu_instructions.md`](docs/cpu_instructions.md) — 6502 instruction reference

## Credits

The ROM test files in `tests/roms/` are from two suites, both written by
**Shay Green** (gblargg@gmail.com):

- [`instr_test-v5`](https://github.com/christopherpow/nes-test-roms/tree/master/instr_test-v5) — instruction correctness (official + unofficial opcodes)
- [`cpu_interrupts_v2`](https://github.com/christopherpow/nes-test-roms/tree/master/cpu_interrupts_v2) — IRQ/NMI interrupt timing
- [`instr_misc`](https://github.com/christopherpow/nes-test-roms/tree/master/instr_misc) — instruction edge cases (address wrap, dummy reads)
- [`instr_timing`](https://github.com/christopherpow/nes-test-roms/tree/master/instr_timing) — cycle-accurate instruction timing

## What's next

- PPU rendering (background tiles, sprites, palette, scrolling)
- APU audio synthesis
- NMI / IRQ interrupt delivery
- Additional mappers (UxROM, CNxROM, MMC3, …)
- Window / display output
