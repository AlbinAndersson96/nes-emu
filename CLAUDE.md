# CLAUDE.md

This file provides guidance to Claude Code (claude.ai/code) when working with code in this repository.

## Project

A NES emulator written in Rust. The CPU (full 6502 instruction set including unofficial opcodes) and a minimal PPU stub are implemented. All 17 blargg `instr_test-v5` ROM tests pass.

## Commands

```bash
cargo build          # compile
cargo run <rom.nes>  # run a ROM
cargo test           # run all tests (includes blargg ROM tests in tests/roms/)
cargo test <name>    # run a single test by name
cargo clippy         # lint
cargo fmt            # format
```

## Module structure

- **`src/cpu/mod.rs`** — `Cpu` struct, register file, addressing modes, stack helpers. The `Bus` trait (`fn read(&self, u16) -> u8` / `fn write(&mut self, u16, u8)`) is defined here.
- **`src/cpu/instructions.rs`** — `execute()` dispatcher; one match arm per opcode including all unofficial opcodes (LAX, SAX, DCP, ISB, SLO, SRE, RLA, RRA, SHA, SHX, SHY, TAS, ANC, ALR, ARR, XAA, LAS).
- **`src/bus.rs`** — `Bus` struct implements `CpuBus`. Wires RAM, PPU, APU stubs, controllers, and cartridge into the 16-bit address space. Exposes `bus.ppu` publicly so the run loop can tick it.
- **`src/ppu.rs`** — Minimal `Ppu` stub. Tracks CPU-cycle count and derives the NTSC VBlank flag from frame timing (cycle % 29,781 ∈ [27,394, 29,667) → bit 7 of $2002 set). No rendering.
- **`src/cartridge.rs`** — iNES parser; supports NROM (mapper 0) and MMC1 (mapper 1).
- **`src/tests/roms.rs`** — Blargg ROM test harness. Polls $6000/$6001–$6003 for test completion; calls `bus.ppu.tick(cycles)` after every CPU step.

## Key design notes

**PPU ticking**: The run loop (and test harness) must call `bus.ppu.tick(cycles)` after every `cpu.step()`. Without this, `$2002` always returns 0 and the blargg test framework loops forever waiting for VBlank.

**SHY / SHX page-cross behavior**: On a page-crossing access, these opcodes write to the *pre-carry* address `(addr_hi << 8) | ((lo + index) & 0xFF)` rather than the effective address. `addr_hi` is the high byte of the *operand*, not the effective address. This is required for the `07-abs_xy` ROM test.

**Unofficial opcode reference**: When implementing or fixing an unofficial opcode, cross-check against the expected hash in the relevant blargg ROM test rather than trusting any single written source — documented behavior varies between references.
