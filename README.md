# nes-emu

A cycle-accurate NES emulator written in Rust.

## Status

| Component | Status |
|-----------|--------|
| CPU — 6502 official opcodes | Complete |
| CPU — unofficial/illegal opcodes | Complete |
| PPU — registers, scrolling, NMI | Complete |
| PPU — background + sprite rendering | Complete |
| APU — all 5 channels + frame counter | Complete (sample generation; no audio device wired yet) |
| Cartridge — NROM (0), MMC1 (1), UxROM (2), CNROM (3), MMC3 (4), AxROM (7), MMC2 (9), MMC4 (10), Color Dreams (11), BNROM/NINA-001 (34), GxROM (66), FME-7 (69), Camerica (71), Jaleco CHR (87), Namco 118 (206) | Complete |
| Controllers — configurable keyboard input | Complete |
| Display output | Complete (winit + egui/egui-wgpu, WSL2-compatible) |

### Test conformance

`cargo test` is fully green. Highlights:

- **All 159 blargg CPU ROM tests** pass: all 17 `instr_test-v5` tests, `instr_timing`,
  all 5 `instr_misc` tests, all of `cpu_interrupts_v2` (tests 1-5 plus the combined
  suite), plus `instr_test-v3`, `nes_instr_test`, `cpu_dummy_writes`, `cpu_dummy_reads`,
  `cpu_exec_space`, `cpu_reset`, `cpu_timing_test6`, `branch_timing_tests`, and both
  `blargg_nes_cpu_test5` ROMs.
- **All 26 wired blargg APU ROM tests** pass: `apu_test` (9), `apu_reset` (6), and
  `blargg_apu_2005.07.30` (11).
- **PPU ROM tests** pass: `ppu_vbl_nmi`, `oam_read`, `oam_stress`, `ppu_open_bus`,
  `ppu_read_buffer`, `sprite_hit_tests`, `sprite_overflow_tests`, `vbl_nmi_timing`,
  and the 2005 PPU suite (with golden-screenshot comparison).
- **Mapper 3/4 ROM tests** pass: `mmc3_test`, `mmc3_test_2`, `mmc3_irq_tests`
  (minus the mutually-exclusive rev-A/MMC6 ROMs).
- **AccuracyCoin** (the 141-test all-in-one accuracy ROM): 125/141, run via an
  `#[ignore]`d harness. See [`docs/accuracycoin_outcome.md`](docs/accuracycoin_outcome.md)
  for the per-test breakdown and remaining-failure analysis.

See [CLAUDE.md](CLAUDE.md) "Known gaps" for what remains (the listen-only `apu_mixer`
suite, the subsystem-scale/expansion-audio mappers deferred until an audio-output path
exists — MMC5, and the Konami VRC / Namco 163 audio families — and the deepest
AccuracyCoin PPU/DMA bus-timing quirks).

## Building and running

### Test ROMs are git submodules

The ROM test suites live in submodules under `tests/roms/`. Fetch them before running
the tests (not needed just to build or run the emulator):

```bash
git submodule update --init --recursive
```

### System dependencies (Ubuntu/Debian)

```bash
sudo apt install pkg-config libgtk-3-dev libxkbcommon-x11-0
```

- `pkg-config` — required by the build system to locate native libraries
- `libgtk-3-dev` — required by the native file dialog (`rfd` crate with `gtk3` feature)
- `libxkbcommon-x11-0` — required at runtime by `winit` for keyboard handling

### Commands

```bash
cargo build            # compile
cargo run <rom.nes>    # run a ROM (or run with no argument and use the File menu)
                       #   the dev profile is optimized (opt-level 1) so this holds 60fps;
                       #   use `cargo run --release <rom.nes>` for maximum performance
cargo test             # run all tests (includes the blargg ROM suites)
cargo clippy           # lint
cargo fmt              # format
```

### Controls

Key bindings are loaded from a `keybindings.toml` file placed next to the compiled
binary — `build.rs` seeds it from [`assets/keybindings.toml`](assets/keybindings.toml)
on first build and never overwrites an existing copy, so your edits survive rebuilds.
If the file is missing, a hardcoded default mapping is used. See
[`docs/input.md`](docs/input.md) for the format and default bindings.

Press **P** to pause/resume. Emulation speed is adjustable while running: `-`
slows down and `=` speeds up (stepping through `1/32×`…`1×`…`8×`), and Backspace
resets to `1×`. See [Emulation speed](docs/usage.md#emulation-speed).

## Project layout

```
build.rs             — seeds keybindings.toml next to the binary on first build
assets/
  keybindings.toml   — default controller key bindings (copied by build.rs)
  fonts/             — UI fonts
src/
  main.rs            — entry point, event loop, keyboard→controller input, WSL2 GPU setup
  app.rs             — App state machine (ROM loading, per-frame stepping, replay)
  menu.rs            — egui File menu interaction
  input.rs           — KeyMap: keyboard KeyCode → (controller port, button) from TOML
  replay.rs          — input recording / playback
  system.rs          — SystemClock: shared per-cycle stepping + interrupt-delivery rules
  bus.rs             — system bus: RAM, PPU, APU, controllers, cartridge; OAM/DMC DMA
  renderer.rs        — winit window + egui/egui-wgpu renderer; NES palette → RGBA; info bar (FPS/speed/slot)
  cartridge.rs       — iNES parser; mappers 0-4, 7, 9-11, 34, 66, 69, 71, 87, 206
  cpu/
    mod.rs           — Cpu struct, micro-op queue, tick(), Bus trait
    instructions.rs  — opcode dispatcher (all official + unofficial opcodes)
  ppu/
    mod.rs           — full PPU: dot/scanline timing, bg/sprite pipeline, OAM, NMI
  apu/
    mod.rs           — APU orchestration, frame counter, mixer
    pulse.rs         — pulse channels (duty, envelope, sweep)
    triangle.rs      — triangle channel
    noise.rs         — noise channel (15-bit LFSR)
    dmc.rs           — delta-modulation channel + DMA
    envelope.rs      — shared envelope generator
    length.rs        — length counter (write-cycle-exact halt/reload timing)
    sweep.rs         — sweep unit
  tests/
    mod.rs           — test tree root (splits unit vs integration)
    unit/            — white-box tests that reach into crate internals
      mod.rs         — module list + TestBus (flat 64 KB space, access trace)
      bus.rs         — bus unit tests
      cpu.rs         — CPU unit tests (incl. cycle-by-cycle bus-access sequences)
      ppu.rs         — PPU unit tests
      cartridge.rs   — mapper unit tests (synthetic iNES images)
      savestate.rs   — save-state round-trip tests
    integration/     — whole-ROM harnesses (boot a .nes through the real machine)
      mod.rs         — module list
      roms.rs        — $6000-protocol blargg ROM harness (CPU, APU, MMC3)
      text_console_roms.rs — on-screen-text blargg ROM harness
      ppu_roms.rs    — blargg PPU ROM harness (golden-screenshot comparison)
      apu_2005_roms.rs — blargg 2005 APU frame-counter ROM harness
      sprite_hit_roms.rs — blargg sprite-0-hit ROM harness
      accuracycoin.rs  — headless AccuracyCoin all-in-one runner (#[ignore]d)
docs/
  Home.md            — wiki landing page / documentation index
  usage.md           — user guide: install, run, controls, save states, troubleshooting
  bus.md             — address map and bus design notes
  cpu_instructions.md — 6502 instruction reference (official + unofficial opcodes)
  cpu_interrupts.md  — NMI/IRQ/BRK dispatch, hijacking, and polling rules
  apu.md             — APU register reference and implementation notes
  ppu.md             — PPU implementation reference and checklist
  input.md           — controller keybinding config format and defaults
  savestate.md       — save-state design, hotkeys, and what is/isn't saved
  accuracycoin_outcome.md — AccuracyCoin per-test results and analysis
  investigations/    — chronological debugging logs (reference, not maintained docs)
tests/roms/          — git submodules (see "Test ROMs are git submodules" above)
  nes-test-roms/     — christopherpow/nes-test-roms: all blargg suites
  AccuracyCoin/      — 100thCoin/AccuracyCoin: the all-in-one accuracy ROM
tests/screenshots/   — golden PPU screenshots for the 2005 PPU suite
```

## Docs

The `docs/` folder doubles as the project wiki — [`docs/Home.md`](docs/Home.md) is
the index page. Each page mixes the NES hardware reference with how this emulator
implements it.

- [`docs/Home.md`](docs/Home.md) — documentation index / wiki landing page
- [`docs/usage.md`](docs/usage.md) — **user guide**: install, run a ROM, controls, save states, troubleshooting
- [`docs/bus.md`](docs/bus.md) — NES address map and bus design
- [`docs/cpu_instructions.md`](docs/cpu_instructions.md) — 6502 instruction reference (official + unofficial)
- [`docs/cpu_interrupts.md`](docs/cpu_interrupts.md) — interrupt dispatch, hijacking, and polling rules
- [`docs/apu.md`](docs/apu.md) — APU channel registers and frame counter
- [`docs/ppu.md`](docs/ppu.md) — PPU implementation reference
- [`docs/input.md`](docs/input.md) — controller keybinding config format and defaults
- [`docs/savestate.md`](docs/savestate.md) — save-state design and hotkeys
- [`docs/accuracycoin_outcome.md`](docs/accuracycoin_outcome.md) — AccuracyCoin per-test results and analysis

## Credits

The ROM test files under `tests/roms/` are consumed as git submodules of two
upstream repositories:

- [christopherpow/nes-test-roms](https://github.com/christopherpow/nes-test-roms) —
  the blargg CPU/PPU/APU/mapper test suites, written by **Shay Green** (gblargg@gmail.com).
- [100thCoin/AccuracyCoin](https://github.com/100thCoin/AccuracyCoin) —
  the all-in-one accuracy ROM.

## What's next

- APU audio output (sample generation is implemented; an audio device / sink is not yet wired)
- Battery-backed save (PRG-RAM) persistence
- The subsystem-scale / expansion-audio mappers (MMC5; the Konami VRC and Namco 163
  audio families — deferred until an audio-output path exists to mix their extra channels)
- Remaining AccuracyCoin accuracy quirks (see [`docs/accuracycoin_outcome.md`](docs/accuracycoin_outcome.md))
