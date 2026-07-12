# CLAUDE.md

This file provides guidance to Claude Code (claude.ai/code) when working with code in this repository.

## Project

A NES emulator written in Rust. The CPU (full 6502 instruction set including unofficial opcodes), a full PPU (background rendering, sprites, palette, scrolling, OAM DMA), the full APU (all five channels plus frame counter), and mappers 0 (NROM), 1 (MMC1), 3 (CNROM), and 4 (MMC3, including the A12-clocked scanline IRQ counter) are implemented. All 159 blargg CPU ROM tests pass: all 17 `instr_test-v5` tests, `instr_timing`, all 5 `instr_misc` tests, and all of `cpu_interrupts_v2` (tests 1-5 plus the combined suite). See Known gaps for the remaining PPU test failures.

## Commands

```bash
cargo build          # compile
cargo run <rom.nes>  # run a ROM
cargo test           # run all tests (includes blargg ROM tests in tests/roms/)
cargo test <name>    # run a single test by name
cargo clippy         # lint
cargo fmt            # format
```

- **`build.rs`** — seeds `keybindings.toml` (copied from `assets/keybindings.toml`) next to the compiled binary on first build; never overwrites an existing copy, so user edits survive rebuilds.

## Module structure

- **`src/cpu/mod.rs`** — `Cpu` struct, register file, micro-op queue, `tick()` entry point. The `Bus` trait (`fn read(&mut self, u16) -> u8` / `fn write(&mut self, u16, u8)`) is defined here. `tick()` executes exactly one micro-op from the queue; when the queue empties a new instruction is decoded (via `RunInstruction`) or an interrupt is serviced. IRQ is level-triggered: `irq_pending` is re-asserted each call to `tick_apu` when the APU line is high, so masked IRQs cannot accumulate.
- **`src/cpu/instructions.rs`** — `execute()` dispatcher; one match arm per opcode including all unofficial opcodes (LAX, SAX, DCP, ISB, SLO, SRE, RLA, RRA, SHA, SHX, SHY, TAS, ANC, ALR, ARR, XAA, LAS, KIL/JAM).
- **`src/system.rs`** — `SystemClock`: the shared per-tick stepping loop (CPU tick or DMA stall cycle, then per-cycle PPU/APU ticking) carrying the blargg-verified interrupt-delivery rules: deferred NMI edges (an edge on a tick's last cycle is delivered one tick later), last-cycle IRQ flagging (`Cpu::irq_on_last_cycle`, needed for the taken-branch last-clock ignore), and DMA interrupt deferral (interrupts first asserting during a DMA stall are delivered at DMA end with one dispatch poll suppressed). Used by BOTH the real run loop (`App::step_frame`) and the ROM test harness — change it in one place only.
- **`src/bus.rs`** — `Bus` struct implements `CpuBus`. Wires RAM, PPU, APU, controllers, and cartridge into the 16-bit address space. Exposes `bus.ppu` and `bus.apu` publicly. `tick_apu(cycles)` advances the APU and returns the current IRQ line level — the OR of the APU line and the cartridge mapper's IRQ line (`Cartridge::irq_pending`, e.g. the MMC3 scanline counter), both level-triggered.
- **`src/apu/mod.rs`** — `Apu` struct: orchestrates all five channels, the NTSC frame counter (4-step / 5-step modes), and the audio mixer. `tick(cpu_cycles)` drives the frame counter and channel timers, then returns `take_irq()` which yields the live IRQ line level (`frame_irq_flag && !irq_inhibit || dmc.irq_flag`) without consuming anything. Reading `$4015` clears `frame_irq_flag`; writing `$4017` with bit 6 set (irq_inhibit) also clears it.
- **`src/apu/pulse.rs`** — Pulse channel with duty cycle sequencer, length counter, envelope, and sweep unit.
- **`src/apu/triangle.rs`** — Triangle channel with linear counter and length counter.
- **`src/apu/noise.rs`** — Noise channel with 15-bit LFSR (mode 0: bit 1 feedback, mode 1: bit 6 feedback), length counter, and envelope.
- **`src/apu/dmc.rs`** — Delta-modulation channel: streams 1-bit delta-PCM from CPU memory. Signals DMA need via `needs_dma()` / `dma_address()`; caller supplies the byte via `supply_dma_byte()`. The 4-cycle CPU stall is implemented in `Bus::tick_dma()`.
- **`src/apu/envelope.rs`** — Shared envelope generator (used by pulse and noise).
- **`src/apu/length.rs`** — Length counter lookup table and helpers.
- **`src/apu/sweep.rs`** — Sweep unit (used by pulse channels; pulse 1 uses ones-complement negation, pulse 2 uses twos-complement).
- **`src/ppu/mod.rs`** — Full `Ppu` implementation. Dot/scanline-accurate NTSC timing (341 dots × 262 scanlines). Implements all CPU-visible registers ($2000–$2007, $4014), the Loopy v/t/x/w scroll registers, background tile pipeline (nametable + attribute + CHR fetches into 16-bit shift registers), sprite evaluation and pattern fetch for up to 8 sprites per scanline, sprite-0 hit, priority multiplexer, palette RAM (32 bytes), NMI edge detection, and OAM DMA write helper. Outputs a 256×240 frame buffer of NES palette indices. Also reports PPU address-bus values to the mapper via `notify_ppu_bus` → `Cartridge::ppu_bus_addr` (rendering fetch addresses at calibrated dots, $2006/$2007-driven changes) — this drives the MMC3 A12 IRQ counter; see the MMC3 note under Key design notes before touching those dots.
- **`src/renderer.rs`** — `Renderer` wraps a `winit` window and renders via `egui`/`egui-wgpu` (`egui_wgpu::winit::Painter`). The NES framebuffer is uploaded each frame as an `egui::TextureHandle` and drawn via a `CentralPanel`; the `present()`/`present_placeholder()` methods just update that texture and call `window.request_redraw()` — actual GPU submission happens in `main.rs`'s `WindowEvent::RedrawRequested` handling (see `Renderer::redraw`). `maybe_configure_wsl2_gpu()` in `main.rs` sets `WGPU_BACKEND=vulkan` and points `VK_ICD_FILENAMES` at the Mesa LLVMpipe ICD when running under WSL2.
- **`src/cartridge.rs`** — iNES parser; supports NROM (mapper 0), MMC1 (mapper 1), CNROM (mapper 3), and MMC3 (mapper 4). MMC1 implements the full 5-bit serial shift register protocol and all four PRG bank modes (32 KB switch, fix-first, fix-last). CNROM is fixed PRG + 8 KB CHR bank select (bus conflicts not modeled). MMC3 implements the $8000-$E001 even/odd register pairs (PRG/CHR banking, mirroring control, IRQ latch/clear/disable/enable) and the A12-clocked IRQ counter with the "new"/sharp revision semantics (counter==0 → reload from latch, else decrement; IRQ when the result is 0 and IRQs enabled); $A001 PRG-RAM protection is not modeled — PRG-RAM stays enabled so the blargg $6000 result protocol always works. 8 KB PRG-RAM at $6000–$7FFF is always present regardless of the iNES header flag, because blargg test ROMs write their results there unconditionally. Mapper CHR banking also applies to CHR-RAM (folded mod 8 KB).
- **`src/input.rs`** — `KeyMap`: maps keyboard `KeyCode`s to (controller port, button bit) pairs, loaded from a TOML config file at startup (`KeyMap::load`) or a hardcoded fallback (`KeyMap::default`). Button bit order matches the NES's real serial order (A, B, Select, Start, Up, Down, Left, Right).
- **`src/tests/mod.rs`** — `TestBus`: flat 64 KB address space used by unit tests (no mirroring, no side effects).
- **`src/tests/bus.rs`** — bus unit tests.
- **`src/tests/cpu.rs`** — CPU unit tests.
- **`src/tests/roms.rs`** — harness for every blargg suite using the $6000 result protocol. Polls $6000/$6001–$6003 for test completion; steps the machine via the shared `SystemClock` (`src/system.rs`), which carries all the per-cycle interrupt-delivery rules; handles status $81 (ROM requests a delayed warm reset, used by `cpu_reset`); asserts status 0 on completion — a nonzero status fails the test with the ROM's code and text. Covers `instr_test-v5`, `instr_timing`, `instr_misc`, `cpu_interrupts_v2`, `instr_test-v3`, `nes_instr_test`, `cpu_dummy_writes`, `cpu_exec_space`, `cpu_reset`, `oam_read`, `oam_stress`, `ppu_open_bus`, `ppu_vbl_nmi`, `ppu_read_buffer`, `mmc3_test`, and `mmc3_test_2` (the last two minus their rev-A/MMC6 ROMs — see Known gaps).
- **`src/tests/ppu_roms.rs`** — Blargg PPU ROM test harness. Runs each ROM for 300 frames (~5 s NES time), then reads the result code from the nametable (the ROMs render `$XX` in ASCII tiles at nametable-0 row 5, col 2–4) and looks up its meaning from the per-ROM table in the README. On failure the panic message includes the result code and its description. Also saves a PNG screenshot to `tests/screenshots/ppu/output/` and pixel-compares against a golden in `tests/screenshots/ppu/golden/` if one exists. To bless a new golden: `cp tests/screenshots/ppu/output/<name>.png tests/screenshots/ppu/golden/<name>.png`.
- **`src/tests/text_console_roms.rs`** — harness for blargg ROM suites that print `PASSED`/`Passed`/`FAILED #<n>`/`Error <n>` text directly into PPU nametable 0 instead of using the `$6000` protocol (`vbl_nmi_timing`, `sprite_overflow_tests`, `branch_timing_tests`, `cpu_timing_test6`, `blargg_nes_cpu_test5`, `cpu_dummy_reads`, `mmc3_irq_tests` minus its rev-A ROM). Asserts the ROM's on-screen verdict — any failure marker fails the test, so the ROMs failing because of known emulator gaps show up as real `cargo test` failures.
- **`docs/bus.md`** — NES address map and bus design notes.
- **`docs/cpu_instructions.md`** — 6502 instruction reference (official opcodes, addressing modes, cycle counts).
- **`docs/cpu_interrupts.md`** — NMI/IRQ/BRK dispatch, the micro-op interrupt-service sequence, NMI hijacking BRK or an in-progress IRQ, and the interrupt-polling-granularity (deferred-edge) fix. Start here before touching interrupt timing; links to the full investigation log for anything not yet resolved.
- **`docs/apu.md`** — APU channel register reference, frame counter sequences, mixer formula, and implementation notes.
- **`docs/ppu.md`** — Full PPU implementation reference: memory map, registers, OAM, Loopy registers, rendering pipeline, scrolling, pixel priority, hardware quirks, and implementation checklist.
- **`docs/input.md`** — Controller keybinding config file format, defaults table, and fallback behavior.
- **`docs/investigations/cpu_interrupt_debug_log.md`** — chronological investigation log for the `cpu_interrupts_v2` ROM tests: hypotheses tried, what was ruled out and why, and open questions. Reference material, not a maintained doc — read `docs/cpu_interrupts.md` first for the current understanding.

## Key design notes

**PPU ticking**: The run loop (and test harness) must call `bus.tick_ppu(delta)` after every `cpu.tick()`. It returns `true` when an NMI edge is detected; the caller should then call `cpu.nmi()`. Without this, `$2002` always returns 0 and the blargg test framework loops forever waiting for VBlank. Use `SystemClock::step` (`src/system.rs`) instead of hand-rolling this — it also implements the deferred-edge and DMA rules.

**APU ticking**: `bus.tick_apu(...)` must also be called after every `cpu.tick()` (the test harness ticks it one cycle at a time). It returns `true` when the APU's IRQ line is currently asserted; the caller should then call `cpu.irq()` — or `cpu.irq_on_last_cycle()` when the line first asserts on an instruction's final cycle (needed for the taken-branch last-clock IRQ ignore). The APU IRQ line is level-triggered: it stays asserted until `frame_irq_flag` is cleared (by reading `$4015` or writing `$4017` with bit 6 set). Because `tick_apu` is called every cycle, a masked IRQ cannot accumulate and fire unexpectedly when FLAG_I is later cleared. Note `bus.tick_apu` internally repays the 4-cycle pre-advance done by `$4015` reads.

**MMC3 A12 IRQ clocking**: The MMC3 scanline counter clocks on PPU address-bus A12 rising edges, filtered by a minimum A12-low time (`A12_FILTER_DOTS`, `src/cartridge.rs`). The PPU reports bus addresses to the mapper at specific dots: background pattern events on each tile group's last dot (dots 8, 16, …, 256, 328, 336), sprite pattern events on each slot's last dot (264+8k, emitted for all 8 slots — empty slots report the hardware's dummy tile-$FF address), nametable/attribute lows at their fetch dots plus dummy-NT events at dots 337/339, and $2006/$2007-driven changes (including the post-increment address) as they happen. These dots and the filter threshold were calibrated as a set against blargg's `mmc3_test/4-scanline_timing` (which brackets the IRQ moment to 1 PPU dot in both `$2000=$08` and `$2000=$10` modes) — don't move any of them independently. Notification is deliberately decoupled from data fetching (`Ppu::notify_ppu_bus` vs `ppu_read`); the mapper IRQ line reaches the CPU through `Bus::tick_apu`'s return value, OR'd with the APU line. The counter semantics are the "new"/sharp revision — the rev-A ROMs (`mmc3_test/6-MMC6`, `mmc3_test_2/rom_singles/6-MMC3_alt`, `mmc3_irq_tests/5.MMC3_rev_A`) are deliberately not wired into the harnesses because they're mutually exclusive with the rev-B ROMs that are.

**SHY / SHX page-cross behavior**: On a page-crossing access, these opcodes write to the *pre-carry* address `(addr_hi << 8) | ((lo + index) & 0xFF)` rather than the effective address. `addr_hi` is the high byte of the *operand*, not the effective address. This is required for the `07-abs_xy` ROM test.

**Unofficial opcode reference**: When implementing or fixing an unofficial opcode, cross-check against the expected hash in the relevant blargg ROM test rather than trusting any single written source — documented behavior varies between references.

## Test ROM layout

```
tests/roms/
  <suite-name>/              # one directory per blargg test suite, e.g.:
    instr_test-v5/, instr_test-v3/, nes_instr_test/, instr_misc/, instr_timing/,
    cpu_interrupts_v2/, cpu_dummy_writes/, cpu_exec_space/, cpu_reset/,
    oam_read/, oam_stress/, ppu_open_bus/, ppu_vbl_nmi/,
    blargg_ppu_tests_2005.09.15b/, sprite_hit_tests_2005.10.05/,
    vbl_nmi_timing/, sprite_overflow_tests/, branch_timing_tests/,
    cpu_timing_test6/, blargg_nes_cpu_test5/,
    cpu_dummy_reads/, ppu_read_buffer/, mmc3_test/, mmc3_test_2/, mmc3_irq_tests/
      # (mapper 3/4 suites — wired into roms.rs / text_console_roms.rs,
      #  except the mutually-exclusive rev-A/MMC6 ROMs; see Known gaps)
    apu_mixer/, apu_mixer_recordings/, apu_reset/, apu_test/, blargg_apu_2005.07.30/
      # APU suites — not wired into any test harness yet
tests/screenshots/
  ppu/
    output/                 # generated each test run (gitignored)
    golden/                 # committed reference screenshots; test fails if output differs
```

## Known gaps

**`cargo test` is fully green**: every wired-up blargg suite passes and asserts its ROM's own verdict (303 tests; the single `#[ignore]`d one is the JAM-opcode ROM that can never finish). This section records what was fixed and where the remaining unwired/unsupported territory is (the APU suites, mappers beyond 0/1/3/4).

### Mapper 3/4 suites: wired and passing (minus the mutually-exclusive revision ROMs)

`cpu_dummy_reads` and `ppu_read_buffer` (mapper 3 / CNROM) and `mmc3_test`, `mmc3_test_2`, `mmc3_irq_tests` (mapper 4 / MMC3) all pass. Three ROMs are deliberately NOT wired into any harness: `mmc3_test/6-MMC6.nes`, `mmc3_test_2/rom_singles/6-MMC3_alt.nes`, and `mmc3_irq_tests/5.MMC3_rev_A.nes` test the MMC6/rev-A IRQ-counter behavior (IRQ on forced reloads only), which is mutually exclusive with the "new"/sharp rev-B behavior the implementation models and the wired revision ROMs (`5-MMC3`, `6.MMC3_rev_B`) verify — at most one of the two revision families can ever pass (each suite's readme says so). The A12 rise-dot calibration story lives under "MMC3 A12 IRQ clocking" in Key design notes.

### Fixed (previously listed here)

1. **CLI/SEI/PLP interrupt-disable latency** — Fixed via `irq_inhibit_next` latch and deferred IRQ service. `cpu_interrupts_v2/1-cli_latency` now passes.
2. **RMW and addressing-mode dummy/spurious reads** — Fixed by adding observable reads to `addr_absolute_x`, `addr_absolute_y`, `addr_indirect_x`, `addr_indirect_y` and their store/RMW variants. `instr_misc/03-dummy_reads` and `04-dummy_reads_apu` now pass.
3. **`$2002` read side effects** — Already implemented correctly in the PPU.
4. **DMC DMA stall** — 4-cycle CPU stall implemented in `Bus::tick_dma()`; byte is fetched and supplied to the DMC on the final stall cycle. `instr_misc/04-dummy_reads_apu` passes.
5. **OAM DMA start offset/wrap** — `Ppu::oam_dma_write` now indexes OAM at `oam_addr.wrapping_add(offset)` instead of raw `offset`, so the DMA copy starts at the value in `$2003` and wraps mod 256 (and leaves `$2003` itself intact). `ppu/sprite_ram` now passes.
6. **`cpu_interrupts_v2/2-nmi_and_brk`** — **now passes.** Two fixes: the
   interrupt-polling-granularity fix (deferred NMI-edge delivery, `src/tests/roms.rs`) took it
   from ~2/10 to 8/10 correct rows; the final 2 rows were fixed by modeling nesdev's
   "interrupt sequences do not perform interrupt polling" rule (`interrupt_poll_suppressed`,
   `src/cpu/mod.rs`): the interrupt handler's first instruction always executes before a
   pending interrupt that missed the T6 hijack window can be serviced. See
   `docs/cpu_interrupts.md`.

7. **`cpu_interrupts_v2/4-irq_and_dma`** — **now passes.** Three fixes: (a) `frame_reset_delay = 7` (the $4017 write is applied while the APU still sits at the start of the writing instruction, 4 cycles before the hardware write cycle, plus the hardware's 3-cycle post-write delay); (b) interrupts that first assert during a DMA stall are delivered at DMA end with one dispatch poll suppressed, so the first post-DMA instruction executes before service (`suppress_next_interrupt_poll`, mirroring the interrupt-sequences-don't-poll rule); (c) OAM DMA takes 514 cycles instead of 513 when the $4014 write lands on an odd APU cycle (`Apu::cycle_parity`).

8. **`cpu_interrupts_v2/3-nmi_and_irq`** — **now passes.** Two fixes beyond the ones above: `frame_reset_delay = 7` (see item 7), and the `$2002` read pre-advance now samples PPU state on the read cycle's final dot instead of one dot past it (`Bus::read` + `Ppu::tick_dots`). `sync_vbl` dot-locks blargg test code to VBlank onset through that read, and since a frame is a non-integer 29780⅔ CPU cycles, the one-dot sampling error only crossed a cycle boundary on this test's 2-frame delay chain — making its whole table one row early while 1-frame tests (like test 2) were unaffected. Earlier sessions' "confirmed contradiction" for this test dissolved once the source was read correctly: the NMI is a real VBL edge (not the $2000 instant-fire quirk), the NMI handler's `bit SNDCHN` acks the APU IRQ (hence `IRQ=00` rows), and rows 10-11 additionally depend on the interrupt-sequences-don't-poll rule.

9. **`cpu_interrupts_v2/5-branch_delays_irq` and the combined suite** — **now pass** (with them, all 159 blargg CPU ROM tests pass). Four fixes: (a) `$4015` reads pre-advance the APU 4 cycles so the frame-IRQ flag is sampled at the read cycle instead of the instruction's start (`APU_READ_PREADVANCE`, `src/bus.rs`; `tick_apu` repays the debt internally so total APU time is conserved); (b) an IRQ already visible at a branch's dispatch preempts the branch like any other instruction (the old dispatch-path special case is removed); (c) an IRQ-aborted branch page-fix cycle still elapses before the 7-cycle interrupt sequence (handler entry at branch T1+11); (d) the taken-branch last-clock IRQ ignore applies only to an IRQ that *first* asserts on that clock — the test harness ticks the APU per cycle and flags such assertions via `Cpu::irq_on_last_cycle`, and dispatch defers only `branch_delay_irq && irq_asserted_on_last_cycle`. Each fix was verified against a specific sub-table of the ROM's readme.

### Remaining failures

None on the CPU side. The blargg-verified per-cycle interrupt-delivery behavior lives in `SystemClock` (`src/system.rs`), shared by the real run loop (`App::step_frame`) and the ROM test harness — the previously-noted harness/main.rs divergence is resolved.

- **`blargg_nes_cpu_test5/cpu.nes` (06-abs_xy)** — reports "Error 1" on unofficial opcodes `9C`/`9E` (SHY/SHX). Newly discovered and unconfirmed; SHY/SHX already pass `instr_test-v5/07-abs_xy`, so the discrepancy is in some untested case. The test is `#[ignore]`d for a different reason (the ROM's opcode sweep hits a JAM/KIL opcode and hangs, on real hardware too); tracked here so it isn't lost.

### PPU tests: all passing

`ppu/power_up_palette` now passes: `Ppu::new` initializes palette RAM to the nesdev-documented 2C02 power-up palette — the same table the ROM was recorded from (its source says "these values are probably unique to my NES", but nesdev adopted them as the canonical power-up state and other accuracy-focused emulators ship them too). The golden screenshot was re-blessed for the passing "$01" screen.

`ppu/vbl_clear_time` passes (fixed as a side effect of the deferred NMI-edge-delivery fix —
see `docs/cpu_interrupts.md`).

### $6000-protocol tests: all passing

The `roms.rs` harness (previously report-only for many suites) asserts every ROM's $6000 status, and all of those suites now pass. The last stragglers were fixed by open-bus modeling:

- **PPU open-bus decay register** (`Ppu::io_bus` + `open_bus()`/`refresh_open_bus()`, `src/ppu/mod.rs`): writes to any PPU register refresh all 8 bits, reads return decay-register bits for everything the PPU doesn't drive ($2002 drives bits 7-5, $2004 all, $2007 all / bits 5-0 for palette reads, write-only registers nothing), and each bit decays to 0 after ~600 ms (`OPEN_BUS_DECAY_DOTS`) without a 1-refresh. Fixed `ppu_open_bus`, `cpu_dummy_writes_ppumem`, `oam_stress`, and (with the one-byte-opcode dummy fetch below) `cpu_exec_space_ppuio`. Two adjacent fixes rode along: $2004 reads mask attribute bytes' nonexistent bits 2-4 to 0, and every one-byte opcode performs its hardware T2 dummy fetch of the following byte (`src/cpu/instructions.rs::execute`) — observable when executing out of PPU I/O space, where an RTS at $2001 must dummy-read $2002 and clear the PPU address latch.
- **CPU open-bus latch** (`Bus::cpu_open_bus`, `src/bus.rs`): every bus transfer (reads, writes, opcode/operand fetches) updates the latch; reads of undriven addresses return it — $4000-$4014 and $4018-$401F entirely, $4015 bit 5, $4016/$4017 bits 5-7 (so `LDA $4016` naturally sees $40 in the top bits), and the cartridge's unmapped $4020-$5FFF expansion area (`Cartridge::read` returns `Option<u8>`; `None` = nothing drives the bus). No decay, unlike the PPU latch. Fixed `cpu_exec_space_apu`.

**Harness rule**: emulator-external inspection (the $6000 result-protocol polling, debuggers) must use the side-effect-free `Bus::peek`, never `Bus::read` — harness reads through `read` corrupt the CPU open-bus latch between the emulated program's own transfers, which `cpu_exec_space_apu` (executing from open bus) catches immediately.

`ppu_vbl_nmi/rom_singles/10-even_odd_timing` (and with it the combined `ppu_vbl_nmi.nes`) now passes. Two fixes: $2001 writes get the same 8-dots/apply/1-dot pre-advance as $2000 (previously they applied ~12 dots early, at the start of the writing instruction), and the odd-frame skipped-dot decision samples `rendering_enabled` at the start of pre-render dot 338 — one dot earlier than the wrap check (`Ppu::render_prev_dot`); all four of the ROM's sub-tests pin that single sample point.

### Text-console tests: all passing

All 5 `sprite_overflow_tests` now pass. Sprite evaluation was rewritten as a per-dot state machine (`Ppu::evaluate_sprites`, driven on odd dots 65-255 of visible scanlines): an out-of-range sprite costs 2 dots, an in-range sprite 8 dots (4-byte copy), so the overflow flag sets at the hardware-exact dot (3.Timing). Once 8 sprites are found, the scan continues with the hardware bug — both `n` and `m` increment on out-of-range checks, misinterpreting successive bytes of successive sprites as Y coordinates (4.Obscure's diagonal scan) — and stops without wrapping when `n` walks past sprite 63. The flag only sets when a 9th in-range (possibly misread) Y is found, never merely because 8 exist (2.Details #9). The pre-render scanline no longer evaluates sprites (hardware doesn't): it just clears the evaluation state, so scanline 0 always renders with an empty sprite set, as on real hardware.

All 7 `vbl_nmi_timing` tests now pass. Three fixes: (a) the harness was switched to the shared `SystemClock` (its old hand-rolled loop never consumed the $2002-read PPU pre-advance, drifting the PPU 3 CPU cycles per read — this alone fixed `4.vbl_clear_timing` and `7.nmi_timing`); (b) the PPU's NMI is now modeled as hardware does it — a level (`vblank && nmi-enable`) edge-sampled once per CPU cycle — which yields the 2-dot NMI-suppression windows for $2002 reads and $2000 NMI-disables around VBL onset, plus the flag-never-sets race for a $2002 read 1 dot before VBL (see `src/ppu/mod.rs`, `Bus::read`/`Bus::write`); (c) the $4017 frame-counter reset delay is parity-dependent (see below).

The rest of the text-console suite passes and asserts: all 7 `vbl_nmi_timing`, `sprite_overflow_tests/1.Basics` and `5.Emulator`, all 3 `branch_timing_tests`, `cpu_timing_test6`, and `blargg_nes_cpu_test5/official.nes`.

### Sprite-0 hit latch delay is ~1 dot (the old 21 was harness drift)

`SPRITE0_HIT_LATCH_DOTS` (`src/ppu/mod.rs`) is 1: the flag is visible via $2002 almost immediately after the colliding pixel (valid calibration window 0-2 dots against the `sprite_hit_tests_2005.10.05` timing ROMs). The previous value of 21 was calibrated while `sprite_hit_roms.rs` used a stale run loop that never consumed the $2002-read PPU pre-advance, double-advancing the PPU 9 dots per read — the "internal pipeline delay" was compensating for that drift. `sprite_hit_roms.rs` and `ppu_roms.rs` now step via the shared `SystemClock` like every other harness; if a sprite-hit timing test ever fails after a timing change, recalibrate the constant by sweeping it rather than trusting the old 18-23 window.

### $4017 write-to-reset delay is parity-dependent

The APU frame-counter reset takes effect 3 CPU cycles after the $4017 write cycle when that cycle lands on an even APU get/put cycle, 4 when odd (`Apu::write`, `frame_reset_delay = 7 or 8`). A constant delay silently defeats blargg's `sync_apu` parity equalizer (`bit SNDCHN` / `bne`), leaving the CPU-APU parity dependent on everything executed before the sync — it only ever worked because the pre-sync frame count happened to be constant. This surfaced as `cpu_interrupts_v2/4-irq_and_dma`'s +526 row flipping (513- vs 514-cycle OAM DMA) when the VBL-read suppression race changed how many frames the shell's $2002 poll loops consumed.
