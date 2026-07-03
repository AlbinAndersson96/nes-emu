# CLAUDE.md

This file provides guidance to Claude Code (claude.ai/code) when working with code in this repository.

## Project

A NES emulator written in Rust. The CPU (full 6502 instruction set including unofficial opcodes), a full PPU (background rendering, sprites, palette, scrolling, OAM DMA), and the full APU (all five channels plus frame counter) are implemented. 154 of 159 blargg ROM tests pass: all 17 `instr_test-v5` tests, `instr_timing`, all 5 `instr_misc` tests, and `cpu_interrupts_v2/1-cli_latency`. The remaining 5 failures require sub-instruction cycle-accurate CPU emulation (see Known gaps).

## Commands

```bash
cargo build          # compile
cargo run <rom.nes>  # run a ROM
cargo test           # run all tests (includes blargg ROM tests in tests/roms/cpu/ and tests/roms/ppu/)
cargo test <name>    # run a single test by name
cargo clippy         # lint
cargo fmt            # format
```

## Module structure

- **`src/cpu/mod.rs`** — `Cpu` struct, register file, micro-op queue, `tick()` entry point. The `Bus` trait (`fn read(&mut self, u16) -> u8` / `fn write(&mut self, u16, u8)`) is defined here. `tick()` executes exactly one micro-op from the queue; when the queue empties a new instruction is decoded (via `RunInstruction`) or an interrupt is serviced. IRQ is level-triggered: `irq_pending` is re-asserted each call to `tick_apu` when the APU line is high, so masked IRQs cannot accumulate.
- **`src/cpu/instructions.rs`** — `execute()` dispatcher; one match arm per opcode including all unofficial opcodes (LAX, SAX, DCP, ISB, SLO, SRE, RLA, RRA, SHA, SHX, SHY, TAS, ANC, ALR, ARR, XAA, LAS).
- **`src/bus.rs`** — `Bus` struct implements `CpuBus`. Wires RAM, PPU, APU, controllers, and cartridge into the 16-bit address space. Exposes `bus.ppu` and `bus.apu` publicly. `tick_apu(cycles)` advances the APU and returns the current IRQ line level.
- **`src/apu/mod.rs`** — `Apu` struct: orchestrates all five channels, the NTSC frame counter (4-step / 5-step modes), and the audio mixer. `tick(cpu_cycles)` drives the frame counter and channel timers, then returns `take_irq()` which yields the live IRQ line level (`frame_irq_flag && !irq_inhibit || dmc.irq_flag`) without consuming anything. Reading `$4015` clears `frame_irq_flag`; writing `$4017` with bit 6 set (irq_inhibit) also clears it.
- **`src/apu/pulse.rs`** — Pulse channel with duty cycle sequencer, length counter, envelope, and sweep unit.
- **`src/apu/triangle.rs`** — Triangle channel with linear counter and length counter.
- **`src/apu/noise.rs`** — Noise channel with 15-bit LFSR (mode 0: bit 1 feedback, mode 1: bit 6 feedback), length counter, and envelope.
- **`src/apu/dmc.rs`** — Delta-modulation channel: streams 1-bit delta-PCM from CPU memory. Signals DMA need via `needs_dma()` / `dma_address()`; caller supplies the byte via `supply_dma_byte()`. The 4-cycle CPU stall is implemented in `Bus::tick_dma()`.
- **`src/apu/envelope.rs`** — Shared envelope generator (used by pulse and noise).
- **`src/apu/length.rs`** — Length counter lookup table and helpers.
- **`src/apu/sweep.rs`** — Sweep unit (used by pulse channels; pulse 1 uses ones-complement negation, pulse 2 uses twos-complement).
- **`src/ppu/mod.rs`** — Full `Ppu` implementation. Dot/scanline-accurate NTSC timing (341 dots × 262 scanlines). Implements all CPU-visible registers ($2000–$2007, $4014), the Loopy v/t/x/w scroll registers, background tile pipeline (nametable + attribute + CHR fetches into 16-bit shift registers), sprite evaluation and pattern fetch for up to 8 sprites per scanline, sprite-0 hit, priority multiplexer, palette RAM (32 bytes), NMI edge detection, and OAM DMA write helper. Outputs a 256×240 frame buffer of NES palette indices.
- **`src/renderer.rs`** — `Renderer` wraps a `winit` window and a `pixels` framebuffer. `present(&frame)` maps the PPU's palette-index frame through the NES master palette to RGBA and blits it to the window. `maybe_configure_wsl2_gpu()` in `main.rs` sets `WGPU_BACKEND=vulkan` and points `VK_ICD_FILENAMES` at the Mesa LLVMpipe ICD when running under WSL2.
- **`src/cartridge.rs`** — iNES parser; supports NROM (mapper 0) and MMC1 (mapper 1). MMC1 implements the full 5-bit serial shift register protocol and all four PRG bank modes (32 KB switch, fix-first, fix-last). 8 KB PRG-RAM at $6000–$7FFF is always present regardless of the iNES header flag, because blargg test ROMs write their results there unconditionally.
- **`src/tests/mod.rs`** — `TestBus`: flat 64 KB address space used by unit tests (no mirroring, no side effects).
- **`src/tests/bus.rs`** — bus unit tests.
- **`src/tests/cpu.rs`** — CPU unit tests.
- **`src/tests/roms.rs`** — Blargg CPU ROM test harness. Polls $6000/$6001–$6003 for test completion. Per-cycle run loop: when DMA is active calls `bus.tick_dma()`; otherwise calls `cpu.tick(bus)`. After each cycle calls `bus.tick_ppu(delta)` (returns true → `cpu.nmi()`) and `bus.tick_apu(delta)` (returns true → `cpu.irq()`). Runs the `instr_test-v5` suite (17 tests, all passing), `instr_timing` (passing), all 5 `instr_misc` tests (passing), plus `cpu_interrupts_v2` (1 passing, 4 failing — see Known gaps).
- **`src/tests/ppu_roms.rs`** — Blargg PPU ROM test harness. Runs each ROM for 300 frames (~5 s NES time), then reads the result code from the nametable (the ROMs render `$XX` in ASCII tiles at nametable-0 row 5, col 2–4) and looks up its meaning from the per-ROM table in the README. On failure the panic message includes the result code and its description. Also saves a PNG screenshot to `tests/screenshots/ppu/output/` and pixel-compares against a golden in `tests/screenshots/ppu/golden/` if one exists. To bless a new golden: `cp tests/screenshots/ppu/output/<name>.png tests/screenshots/ppu/golden/<name>.png`.
- **`docs/bus.md`** — NES address map and bus design notes.
- **`docs/cpu_instructions.md`** — 6502 instruction reference (official opcodes, addressing modes, cycle counts).
- **`docs/cpu_interrupts.md`** — NMI/IRQ/BRK dispatch, the micro-op interrupt-service sequence, NMI hijacking BRK or an in-progress IRQ, and the interrupt-polling-granularity (deferred-edge) fix. Start here before touching interrupt timing; links to the full investigation log for anything not yet resolved.
- **`docs/apu.md`** — APU channel register reference, frame counter sequences, mixer formula, and implementation notes.
- **`docs/ppu.md`** — Full PPU implementation reference: memory map, registers, OAM, Loopy registers, rendering pipeline, scrolling, pixel priority, hardware quirks, and implementation checklist.
- **`docs/investigations/cpu_interrupt_debug_log.md`** — chronological investigation log for the `cpu_interrupts_v2` ROM tests: hypotheses tried, what was ruled out and why, and open questions. Reference material, not a maintained doc — read `docs/cpu_interrupts.md` first for the current understanding.

## Key design notes

**PPU ticking**: The run loop (and test harness) must call `bus.tick_ppu(delta)` after every `cpu.tick()`. It returns `true` when an NMI edge is detected; the caller should then call `cpu.nmi()`. Without this, `$2002` always returns 0 and the blargg test framework loops forever waiting for VBlank.

**APU ticking**: `bus.tick_apu(delta)` must also be called after every `cpu.tick()`. It returns `true` when the APU's IRQ line is currently asserted; the caller should then call `cpu.irq()`. The APU IRQ line is level-triggered: it stays asserted until `frame_irq_flag` is cleared (by reading `$4015` or writing `$4017` with bit 6 set). Because `tick_apu` is called every cycle, a masked IRQ cannot accumulate and fire unexpectedly when FLAG_I is later cleared.

**SHY / SHX page-cross behavior**: On a page-crossing access, these opcodes write to the *pre-carry* address `(addr_hi << 8) | ((lo + index) & 0xFF)` rather than the effective address. `addr_hi` is the high byte of the *operand*, not the effective address. This is required for the `07-abs_xy` ROM test.

**Unofficial opcode reference**: When implementing or fixing an unofficial opcode, cross-check against the expected hash in the relevant blargg ROM test rather than trusting any single written source — documented behavior varies between references.

## Test ROM layout

```
tests/roms/
  cpu/                      # blargg CPU test ROMs (instr_test-v5, instr_misc, instr_timing, cpu_interrupts_v2)
  ppu/                      # blargg PPU test ROMs (palette_ram, sprite_ram, vbl_clear_time, vram_access, power_up_palette)
tests/screenshots/
  ppu/
    output/                 # generated each test run (gitignored)
    golden/                 # committed reference screenshots; test fails if output differs
```

## Known gaps

These are confirmed missing features tied to failing blargg ROM tests. The project currently passes **154 of 159** blargg CPU tests and **3 of 5** blargg PPU tests.

### Fixed (previously listed here)

1. **CLI/SEI/PLP interrupt-disable latency** — Fixed via `irq_inhibit_next` latch and deferred IRQ service. `cpu_interrupts_v2/1-cli_latency` now passes.
2. **RMW and addressing-mode dummy/spurious reads** — Fixed by adding observable reads to `addr_absolute_x`, `addr_absolute_y`, `addr_indirect_x`, `addr_indirect_y` and their store/RMW variants. `instr_misc/03-dummy_reads` and `04-dummy_reads_apu` now pass.
3. **`$2002` read side effects** — Already implemented correctly in the PPU.
4. **DMC DMA stall** — 4-cycle CPU stall implemented in `Bus::tick_dma()`; byte is fetched and supplied to the DMC on the final stall cycle. `instr_misc/04-dummy_reads_apu` passes.

### Remaining failures (all require sub-instruction cycle-accurate emulation)

The 5 remaining failures (`cpu_interrupts_v2` tests 2–5 plus the combined suite) exercise interrupt-sequencing behaviour that depends on *which cycle within a multi-cycle instruction* a signal arrives.

- **`cpu_interrupts_v2/2-nmi_and_brk`** — see `docs/cpu_interrupts.md` for the confirmed
  NMI-hijacks-BRK mechanics (B=1 is preserved, not cleared — a previous version of this note
  had that backwards) and the interrupt-polling-granularity fix (deferred NMI-edge delivery,
  `src/tests/roms.rs`) that took this test from ~2/10 to 8-9/10 correct rows (verified against
  the readme's expected table), zero regressions elsewhere. One remaining known defect: a
  single stray flag bit on the last two rows, tied to `pending_nmi` surviving past
  `VectorFetchHi` into the next instruction — see the investigation log for detail.

- **`cpu_interrupts_v2/3-nmi_and_irq`** — NMI hijacking an in-progress *IRQ* (not BRK) service
  sequence; see `docs/cpu_interrupts.md` for the confirmed dispatch/hijack mechanics. This
  session traced the exact mechanism (IRQ preempts the pending instruction, pushes PC/P, NMI
  hijacks the vector fetch within the T1-T6 window) and confirmed it's mechanically identical
  across every row of this test — which is itself the problem: the mechanism as understood
  predicts the same captured byte for every row where the hijack applies, but the ROM's own
  `readme.txt` documents different bytes for different rows. Three independent subsystems
  (the delay routines, `sync_vbl`'s contract, and the APU frame-IRQ's absolute timing) were
  each rigorously proven correct in isolation this session, so this isn't an under-investigated
  gap — the confirmed facts mechanically contradict the expected output, which needs an
  external hardware reference to resolve rather than more guessing. Also confirmed: applying
  the same interrupt-polling-granularity fix used for NMI to the IRQ line is wrong and
  regresses an otherwise-correct row (IRQ is level-triggered and already correctly
  re-sampled every tick; it doesn't have the edge-triggered NMI's "last cycle invisible"
  problem). See the investigation log for the full trace evidence.

- **`cpu_interrupts_v2/4-irq_and_dma`** — During OAM-DMA, the run loop calls `bus.tick_dma()` and then `cpu.irq()` on every DMA cycle, but `cpu.irq()` only sets `irq_pending = true`. That flag is not consumed until the next `cpu.tick()` call, which does not happen while DMA is active. The result is that every IRQ that fires during DMA is indistinguishable to the CPU: they all appear to have arrived at the moment DMA ended. On real hardware the CPU samples the IRQ line at a specific DMA cycle, so the number of cycles between the IRQ signal and the start of the interrupt service depends on *when during the DMA* the signal was asserted. ROM output shows a `53 +N` table of per-offset IRQ latency measurements; values go wrong at `+4` onwards (first offset where the IRQ reaches the CPU one DMA cycle later than expected).

- **`cpu_interrupts_v2/5-branch_delays_irq`** — A page-crossing branch should abort its T4 page-fix cycle when IRQ is pending at T3, taking only 3 cycles total and pushing the page-wrong PC. `BranchPageFix` already handles the case where `pending_irq` was set at opcode-fetch time. The remaining bug: `pending_irq` is only set in the opcode-fetch path (queue empty, IRQ visible before the branch opcode is read). If the APU fires during the branch's *RunInstruction* tick (which consumes T1+T2+T3 all at once), `cpu.irq()` is called after that tick completes with `delta=3`, but at that point the queue is non-empty so the irq-to-`pending_irq` promotion never happens. `BranchPageFix` then sees `pending_irq=false` and applies the page fix instead of aborting it. ROM output (`T+ CK PC` table): T+0 and T+1 are correct (IRQ was pending before opcode fetch → `pending_irq` set → abort works, PC=`$0E`); T+2 onwards are wrong (IRQ arrives during RunInstruction, page fix completes, PC=`$03`).

- **`cpu_interrupts_v2` combined suite** — fails because the individual tests above fail.

### Failing PPU tests

- **`ppu/sprite_ram`** — result code 7: *$4014 DMA copy should start at value in $2003 and wrap*. OAM DMA ignores the starting offset in $2003; it always copies from OAM byte 0 instead of wrapping around from the value in $2003.
- **`ppu/power_up_palette`** — result code 2: *Palette differs from table*. Power-up palette contents don't match the specific values on the test author's NES (this test is hardware-specific and may not be fixable in a general emulator).

`ppu/vbl_clear_time` now passes (fixed as a side effect of the deferred NMI-edge-delivery fix —
see `docs/cpu_interrupts.md`).
