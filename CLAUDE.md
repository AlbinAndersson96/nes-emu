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
- **`src/cartridge.rs`** — iNES parser; supports NROM (mapper 0) and MMC1 (mapper 1). MMC1 implements the full 5-bit serial shift register protocol and all four PRG bank modes (32 KB switch, fix-first, fix-last). 8 KB PRG-RAM at $6000–$7FFF is always present regardless of the iNES header flag, because blargg test ROMs write their results there unconditionally.
- **`src/tests/mod.rs`** — `TestBus`: flat 64 KB address space used by unit tests (no mirroring, no side effects).
- **`src/tests/bus.rs`** — bus unit tests.
- **`src/tests/cpu.rs`** — CPU unit tests.
- **`src/tests/roms.rs`** — Blargg ROM test harness. Polls $6000/$6001–$6003 for test completion; calls `bus.ppu.tick(cycles)` after every CPU step. Runs the `instr_test-v5` suite (17 tests, all passing) plus `cpu_interrupts_v2`, `instr_misc`, and `instr_timing` suites (12 additional tests; most currently fail — see Known gaps).
- **`docs/bus.md`** — NES address map and bus design notes.
- **`docs/cpu_instructions.md`** — 6502 instruction reference (official opcodes, addressing modes, cycle counts).

## Key design notes

**PPU ticking**: The run loop (and test harness) must call `bus.ppu.tick(cycles)` after every `cpu.step()`. Without this, `$2002` always returns 0 and the blargg test framework loops forever waiting for VBlank.

**SHY / SHX page-cross behavior**: On a page-crossing access, these opcodes write to the *pre-carry* address `(addr_hi << 8) | ((lo + index) & 0xFF)` rather than the effective address. `addr_hi` is the high byte of the *operand*, not the effective address. This is required for the `07-abs_xy` ROM test.

**Unofficial opcode reference**: When implementing or fixing an unofficial opcode, cross-check against the expected hash in the relevant blargg ROM test rather than trusting any single written source — documented behavior varies between references.

## Known gaps

These are confirmed missing features, each tied to at least one failing blargg ROM test. Tests that were previously passing are not affected.

### 1. IRQ / NMI delivery (`cpu_interrupts_v2/*`, `instr_timing`)

`Cpu` has no `irq()` or `nmi()` method. The CPU never receives an interrupt signal; `FLAG_I` is never checked between instructions. Implementing this requires:

- Adding `irq_pending` and `nmi_pending` flags to `Cpu`.
- At the top of `step()` (after the current instruction but before the next), checking those flags and pushing PC + P to the stack then jumping through `$FFFE`/`$FFFA`.
- Wiring the PPU VBlank → NMI line: when `PPUCTRL` bit 7 is set and VBlank begins, set `cpu.nmi_pending`.
- Wiring the APU frame counter → IRQ line (see gap 2).

### 2. APU frame counter IRQ (`cpu_interrupts_v2/1-cli_latency`, `instr_timing`)

`$4017` writes are stored in `apu_io[0x17]` but otherwise ignored (`bus.rs:103`). The frame counter never ticks, so no IRQ is ever raised. The `instr_timing` ROM uses frame-counter IRQ as its timing source and hangs forever waiting for it.

Implementing requires tracking elapsed CPU cycles in `Bus`, firing an IRQ at the correct frame-counter intervals, and respecting the mode/inhibit bits in `$4017`.

### 3. CLI / SEI interrupt-disable latency (`cpu_interrupts_v2/1-cli_latency`)

On the real 6502, `CLI` and `SEI` modify `FLAG_I` at the *end* of the instruction, but the new value doesn't take effect for interrupt polling until *after* the following instruction completes. The current implementation sets `FLAG_I` immediately in `instructions.rs`. Test `1-cli_latency` fails with error code 3, which corresponds to wrong IRQ timing relative to `CLI`.

### 4. Read-modify-write dummy reads (`instr_misc/03-dummy_reads`, `instr_misc/04-dummy_reads_apu`)

RMW instructions (`ASL`, `LSR`, `ROL`, `ROR`, `INC`, `DEC`) on memory addressing modes perform two bus cycles at the effective address: one read, then one write of the modified value. The intermediate read is architecturally observable (it triggers side effects on registers like `$2002`). The current `execute()` in `instructions.rs` calls `bus.read()` once to fetch the value and `bus.write()` once to store it, which is correct in count but the read must also happen at the *effective* address (not just via the operand fetch) to trigger side effects.

Specifically: `03-dummy_reads` iterates over RMW instructions aimed at PPU/APU addresses and checks that the dummy read side effect fires. `04-dummy_reads_apu` does the same for APU registers.

### 5. `$2002` read side effects (`instr_misc/03-dummy_reads`)

Reading `$2002` on real hardware clears bit 7 (VBlank flag) and resets the PPUADDR/PPUSCROLL write-toggle. `Ppu::read_status()` currently takes `&self` and returns a computed value without mutation (`ppu.rs:30`). The `Bus` trait already uses `&mut self` for reads, so the plumbing is ready — `read_status` just needs to become `&mut self` and clear the flag after returning it.
