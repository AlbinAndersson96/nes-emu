# CLAUDE.md

This file provides guidance to Claude Code (claude.ai/code) when working with code in this repository.

## Project

A NES emulator written in Rust. The CPU (full 6502 instruction set including unofficial opcodes), a minimal PPU stub, and the full APU (all five channels plus frame counter) are implemented. 20 blargg ROM tests pass: all 17 `instr_test-v5` tests and `instr_timing`.

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

- **`src/cpu/mod.rs`** — `Cpu` struct, register file, addressing modes, stack helpers. The `Bus` trait (`fn read(&self, u16) -> u8` / `fn write(&mut self, u16, u8)`) is defined here. Holds `irq_pending` and `nmi_pending` flags; both are consumed at the top of `step()` — IRQ is level-triggered (consumed every step regardless of FLAG_I so masked IRQs don't linger).
- **`src/cpu/instructions.rs`** — `execute()` dispatcher; one match arm per opcode including all unofficial opcodes (LAX, SAX, DCP, ISB, SLO, SRE, RLA, RRA, SHA, SHX, SHY, TAS, ANC, ALR, ARR, XAA, LAS).
- **`src/bus.rs`** — `Bus` struct implements `CpuBus`. Wires RAM, PPU, APU, controllers, and cartridge into the 16-bit address space. Exposes `bus.ppu` and `bus.apu` publicly. `tick_apu(cycles)` advances the APU and returns the current IRQ line level.
- **`src/apu/mod.rs`** — `Apu` struct: orchestrates all five channels, the NTSC frame counter (4-step / 5-step modes), and the audio mixer. `tick(cpu_cycles)` drives the frame counter and channel timers, then returns `take_irq()` which yields the live IRQ line level (`frame_irq_flag && !irq_inhibit || dmc.irq_flag`) without consuming anything. Reading `$4015` clears `frame_irq_flag`; writing `$4017` with bit 6 set (irq_inhibit) also clears it.
- **`src/apu/pulse.rs`** — Pulse channel with duty cycle sequencer, length counter, envelope, and sweep unit.
- **`src/apu/triangle.rs`** — Triangle channel with linear counter and length counter.
- **`src/apu/noise.rs`** — Noise channel with 15-bit LFSR (mode 0: bit 1 feedback, mode 1: bit 6 feedback), length counter, and envelope.
- **`src/apu/dmc.rs`** — Delta-modulation channel: streams 1-bit delta-PCM from CPU memory. Signals DMA need via `needs_dma()` / `dma_address()`; caller supplies the byte via `supply_dma_byte()`. DMC DMA stall (4-cycle CPU halt) is not yet implemented.
- **`src/apu/envelope.rs`** — Shared envelope generator (used by pulse and noise).
- **`src/apu/length.rs`** — Length counter lookup table and helpers.
- **`src/apu/sweep.rs`** — Sweep unit (used by pulse channels; pulse 1 uses ones-complement negation, pulse 2 uses twos-complement).
- **`src/ppu.rs`** — Minimal `Ppu` stub. Tracks CPU-cycle count and derives the NTSC VBlank flag from frame timing (cycle % 29,781 ∈ [27,394, 29,667) → bit 7 of $2002 set). No rendering.
- **`src/cartridge.rs`** — iNES parser; supports NROM (mapper 0) and MMC1 (mapper 1). MMC1 implements the full 5-bit serial shift register protocol and all four PRG bank modes (32 KB switch, fix-first, fix-last). 8 KB PRG-RAM at $6000–$7FFF is always present regardless of the iNES header flag, because blargg test ROMs write their results there unconditionally.
- **`src/tests/mod.rs`** — `TestBus`: flat 64 KB address space used by unit tests (no mirroring, no side effects).
- **`src/tests/bus.rs`** — bus unit tests.
- **`src/tests/cpu.rs`** — CPU unit tests.
- **`src/tests/roms.rs`** — Blargg ROM test harness. Polls $6000/$6001–$6003 for test completion; calls `bus.ppu.tick(cycles)` and `bus.tick_apu(cycles)` after every CPU step; wires APU IRQ and PPU NMI to the CPU. Runs the `instr_test-v5` suite (17 tests, all passing), `instr_timing` (passing), plus `cpu_interrupts_v2` and `instr_misc` suites (11 tests; currently failing — see Known gaps).
- **`docs/bus.md`** — NES address map and bus design notes.
- **`docs/cpu_instructions.md`** — 6502 instruction reference (official opcodes, addressing modes, cycle counts).

## Key design notes

**PPU ticking**: The run loop (and test harness) must call `bus.ppu.tick(cycles)` after every `cpu.step()`. Without this, `$2002` always returns 0 and the blargg test framework loops forever waiting for VBlank.

**APU ticking**: `bus.tick_apu(cycles)` must also be called after every `cpu.step()`. It returns `true` when the APU's IRQ line is currently asserted; the caller should then call `cpu.irq()`. The APU IRQ line is level-triggered: it stays asserted until `frame_irq_flag` is cleared (by reading `$4015` or writing `$4017` with bit 6 set). The CPU in turn consumes `irq_pending` on every `step()` regardless of `FLAG_I`, so a masked IRQ cannot accumulate and fire unexpectedly when the flag is later cleared.

**SHY / SHX page-cross behavior**: On a page-crossing access, these opcodes write to the *pre-carry* address `(addr_hi << 8) | ((lo + index) & 0xFF)` rather than the effective address. `addr_hi` is the high byte of the *operand*, not the effective address. This is required for the `07-abs_xy` ROM test.

**Unofficial opcode reference**: When implementing or fixing an unofficial opcode, cross-check against the expected hash in the relevant blargg ROM test rather than trusting any single written source — documented behavior varies between references.

## Known gaps

These are confirmed missing features, each tied to at least one failing blargg ROM test. Tests that were previously passing are not affected.

### 1. CLI / SEI interrupt-disable latency (`cpu_interrupts_v2/1-cli_latency`)

On the real 6502, `CLI` and `SEI` modify `FLAG_I` at the *end* of the instruction, but the new value doesn't take effect for interrupt polling until *after* the following instruction completes. The current implementation sets `FLAG_I` immediately in `instructions.rs`. Test `1-cli_latency` fails with error code 3, which corresponds to wrong IRQ timing relative to `CLI`.

### 2. Read-modify-write dummy reads (`instr_misc/03-dummy_reads`, `instr_misc/04-dummy_reads_apu`)

RMW instructions (`ASL`, `LSR`, `ROL`, `ROR`, `INC`, `DEC`) on memory addressing modes perform two bus cycles at the effective address: one read, then one write of the modified value. The intermediate read is architecturally observable (it triggers side effects on registers like `$2002`). The current `execute()` in `instructions.rs` calls `bus.read()` once to fetch the value and `bus.write()` once to store it, which is correct in count but the read must also happen at the *effective* address (not just via the operand fetch) to trigger side effects.

Specifically: `03-dummy_reads` iterates over RMW instructions aimed at PPU/APU addresses and checks that the dummy read side effect fires. `04-dummy_reads_apu` does the same for APU registers.

### 3. `$2002` read side effects (`instr_misc/03-dummy_reads`)

Reading `$2002` on real hardware clears bit 7 (VBlank flag) and resets the PPUADDR/PPUSCROLL write-toggle. `Ppu::read_status()` currently takes `&self` and returns a computed value without mutation (`ppu.rs:30`). The `Bus` trait already uses `&mut self` for reads, so the plumbing is ready — `read_status` just needs to become `&mut self` and clear the flag after returning it.

### 4. DMC DMA stall (`instr_misc/04-dummy_reads_apu`, `cpu_interrupts_v2/4-irq_and_dma`)

When the DMC reader needs a byte (`apu.dmc_needs_dma()` returns true), the CPU must be stalled for 4 cycles while the bus performs a read from `apu.dmc_dma_address()` and supplies it via `apu.dmc_supply_byte(data)`. The stall and DMA read are currently stubbed out with a TODO in `bus.rs::tick_apu`.

### 5. NMI / IRQ timing accuracy (`cpu_interrupts_v2/*`)

The `cpu_interrupts_v2` suite tests cycle-accurate interrupt sequencing: NMI-during-BRK, NMI-during-IRQ, branch-delay IRQ, and `$4017` write-jitter (the frame counter reset actually takes effect 2–3 CPU cycles after the write). These require sub-instruction interrupt polling precision that the current one-instruction-granularity model cannot provide.
