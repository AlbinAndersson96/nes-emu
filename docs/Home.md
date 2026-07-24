# nes-emu Documentation

A cycle-accurate NES emulator written in Rust. This is the documentation index —
each page pairs the NES hardware reference with how *this* emulator implements it.
It is structured to double as a GitHub wiki (this file is the wiki **Home** page).

## Start here

- **[User Guide](usage.md)** — install, build, run a ROM, controls, save states,
  input replay, and troubleshooting. Read this first if you just want to play.

## Emulator status

| Component | Status |
|-----------|--------|
| CPU — full 6502, official + unofficial opcodes | Complete |
| PPU — background, sprites, scrolling, NMI, timing | Complete |
| APU — all 5 channels + frame counter | Complete (sample generation; no audio device wired yet) |
| Mappers | 15 implemented (0–4, 7, 9–11, 34, 66, 69, 71, 87, 206) |
| Controllers | Configurable keyboard input, 2 ports |
| Display | winit + egui/egui-wgpu (WSL2-compatible) |
| Save states | 10 slots + file dialogs |

**Test conformance:** `cargo test` is fully green — all 159 blargg CPU ROM tests,
all 26 wired blargg APU ROM tests, the PPU / sprite / mapper suites, plus unit
tests. The AccuracyCoin all-in-one accuracy ROM scores 125/141. See the
[README](../README.md) and [`accuracycoin_outcome.md`](accuracycoin_outcome.md)
for the breakdown.

## Hardware & implementation reference

| Page | Covers |
|------|--------|
| [System bus](bus.md) | 16-bit address map, mirroring, register decoding, DMA, open bus |
| [CPU instructions](cpu_instructions.md) | 6502 registers, flags, addressing modes, every official and unofficial opcode |
| [CPU interrupts](cpu_interrupts.md) | NMI/IRQ/BRK dispatch, NMI hijacking, interrupt-polling granularity |
| [PPU](ppu.md) | Memory map, registers, OAM, scrolling (t/v/x/w), rendering pipeline, quirks |
| [APU](apu.md) | Channel registers, frame counter, mixer, DMC DMA, reset behavior |
| [Controller input](input.md) | Keybinding config format, defaults, fallback behavior |
| [Save states](savestate.md) | Snapshot design, hotkeys, what is/isn't saved, serialization |

## Accuracy & investigation notes

- [`accuracycoin_outcome.md`](accuracycoin_outcome.md) — per-test AccuracyCoin
  results, fixes applied, and remaining-failure analysis (session-by-session).
- [`investigations/`](investigations/) — chronological debugging logs (CPU
  interrupts, sprite-hit timing, the tick-based CPU refactor). These are
  reference material, **not** maintained docs — read the topic page above first.

## For contributors

The project's engineering guide lives in [`CLAUDE.md`](../CLAUDE.md) at the repo
root: module-by-module structure, the shared `SystemClock` stepping rules, every
"key design note" behind a subtle timing fix, and the known-gaps list. Keep the
docs in this folder in sync with the code as you change it (see the note at the
top of `CLAUDE.md`).
