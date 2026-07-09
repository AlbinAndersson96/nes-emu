# nes-emu

A NES emulator written in Rust.

## Status

| Component | Status |
|-----------|--------|
| CPU — 6502 official opcodes | Complete |
| CPU — unofficial/illegal opcodes | Complete |
| PPU — registers, scrolling, NMI | Complete |
| PPU — background + sprite rendering | Complete |
| APU — all 5 channels + frame counter | Complete |
| Cartridge — NROM (mapper 0) | Complete |
| Cartridge — MMC1 (mapper 1) | Complete |
| Controllers | Partial (serial shift register wired, no input source) |
| Display output | Complete (winit + egui/egui-wgpu, WSL2-compatible) |

All 159 blargg CPU ROM tests pass: all 17 `instr_test-v5` tests, `instr_timing`,
all 5 `instr_misc` tests, and all of `cpu_interrupts_v2` (tests 1-5 plus the
combined suite). 4 of 5 blargg PPU tests pass; see CLAUDE.md "Known gaps" for
the remaining PPU test failures.

## Building and running

### System dependencies (Ubuntu/Debian)

```bash
sudo apt install pkg-config libgtk-3-dev libxkbcommon-x11-0
```

- `pkg-config` — required by the build system to locate native libraries
- `libgtk-3-dev` — required by the native file dialog (`rfd` crate with `gtk3` feature)
- `libxkbcommon-x11-0` — required at runtime by `winit` for keyboard handling

### Commands

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
  main.rs            — entry point, event loop, WSL2 GPU setup
  app.rs             — App state machine (ROM loading, per-frame stepping)
  system.rs          — SystemClock: shared per-cycle stepping + interrupt delivery rules
  bus.rs             — system bus: RAM, PPU, APU, controllers, cartridge; OAM/DMC DMA
  renderer.rs        — winit window + egui/egui-wgpu renderer; NES palette → RGBA
  cartridge.rs       — iNES parser; mapper 0 (NROM) and mapper 1 (MMC1)
  cpu/
    mod.rs           — Cpu struct, micro-op queue, tick(), Bus trait
    instructions.rs  — opcode dispatcher (all official + unofficial opcodes)
  ppu/
    mod.rs           — full PPU: scanline timing, bg/sprite pipeline, OAM, NMI
  apu/
    mod.rs           — APU orchestration, frame counter, mixer
    pulse.rs         — pulse channels (duty, envelope, sweep)
    triangle.rs      — triangle channel
    noise.rs         — noise channel (15-bit LFSR)
    dmc.rs           — delta-modulation channel
    envelope.rs      — shared envelope generator
    length.rs        — length counter table
    sweep.rs         — sweep unit
  tests/
    mod.rs           — TestBus used by unit tests
    bus.rs           — bus unit tests
    cpu.rs           — CPU unit tests
    ppu.rs           — PPU unit tests
    roms.rs          — blargg CPU ROM test harness
    ppu_roms.rs      — blargg PPU ROM test harness (screenshot comparison)
    sprite_hit_roms.rs — blargg sprite-0-hit ROM test harness
docs/
  bus.md             — address map and bus design notes
  cpu_instructions.md — 6502 instruction reference
  cpu_interrupts.md  — NMI/IRQ/BRK dispatch, hijacking, and polling rules
  apu.md             — APU register reference and implementation notes
  ppu.md             — PPU implementation reference and checklist
  investigations/    — chronological debugging logs (reference, not maintained docs)
tests/roms/          — blargg ROM files (cpu/ and ppu/ suites)
```

## Docs

- [`docs/bus.md`](docs/bus.md) — NES address map and bus design
- [`docs/cpu_instructions.md`](docs/cpu_instructions.md) — 6502 instruction reference
- [`docs/cpu_interrupts.md`](docs/cpu_interrupts.md) — interrupt dispatch, hijacking, and polling rules
- [`docs/apu.md`](docs/apu.md) — APU channel registers and frame counter
- [`docs/ppu.md`](docs/ppu.md) — PPU implementation reference

## Credits

The ROM test files in `tests/roms/` are from two suites, both written by
**Shay Green** (gblargg@gmail.com):

- [`instr_test-v5`](https://github.com/christopherpow/nes-test-roms/tree/master/instr_test-v5) — instruction correctness (official + unofficial opcodes)
- [`cpu_interrupts_v2`](https://github.com/christopherpow/nes-test-roms/tree/master/cpu_interrupts_v2) — IRQ/NMI interrupt timing
- [`instr_misc`](https://github.com/christopherpow/nes-test-roms/tree/master/instr_misc) — instruction edge cases (address wrap, dummy reads)
- [`instr_timing`](https://github.com/christopherpow/nes-test-roms/tree/master/instr_timing) — cycle-accurate instruction timing

## What's next

- Remaining PPU test failures (sprite-0-hit timing; see CLAUDE.md "Known gaps")
- APU audio output (sample generation is implemented; audio device / SDL output not yet wired)
- Additional mappers (UxROM, CNxROM, MMC3, …)
- Controller input (keyboard / gamepad mapping)
