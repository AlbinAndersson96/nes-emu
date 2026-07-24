# User Guide

How to build, run, and play with `nes-emu`. For the internals, see the
reference pages linked from [`Home.md`](Home.md).

## Requirements

- A recent **Rust toolchain** (stable; edition 2024). Install via [rustup](https://rustup.rs).
- A GPU or software rasterizer reachable by `wgpu` (Vulkan, Metal, DX12, or GL).
  WSL2 is handled automatically — see [Troubleshooting](#troubleshooting).

### System dependencies (Ubuntu/Debian)

```bash
sudo apt install pkg-config libgtk-3-dev libxkbcommon-x11-0
```

- `pkg-config` — used by the build to locate native libraries
- `libgtk-3-dev` — required by the native file dialog (`rfd` crate, `gtk3` feature)
- `libxkbcommon-x11-0` — required at runtime by `winit` for keyboard handling

On macOS and Windows no extra packages are needed beyond the Rust toolchain.

## Building

```bash
cargo build            # debug build (opt-level 1 — fast enough to hold 60 fps)
cargo build --release  # fully optimized build
```

The dev profile is deliberately set to `opt-level = 1`: a cycle-accurate core is
compute-heavy, and at `opt-level 0` the emulation loop alone can't hold the
16.639 ms NTSC frame budget, so games would run in slow motion. `--release` is a
touch faster still and is what you want for a shipped binary.

## Running a ROM

```bash
cargo run <rom.nes>              # load and run a ROM immediately
cargo run --release <rom.nes>    # same, fastest build
cargo run                        # start with no ROM; use File → Load ROM
```

You can also load a ROM at any time from the window's **File → Load ROM** menu
(or press **Ctrl+O**), which opens a native file picker. Only iNES (`.nes`)
images are supported; the mapper is chosen automatically from the header (see the
[supported mappers](#supported-mappers) list).

## The window

The window has three horizontal bands: the **menu bar** on top, the **NES
video** (256×240, scaled 2× to 512×480 by default) in the middle, and an
**info bar** along the bottom. The info bar always shows the live state:

- **FPS** — the measured frame rate (a 500 ms running average).
- **Speed** — the current emulation-speed multiplier (`1x`, `1/4x`, `8x`, …).
- **Slot** — the active save-state slot (0–9).
- **PAUSED** — shown only while emulation is paused.

Before a ROM is loaded the bar reads *No ROM loaded*. The same details also
appear in the window title; the info bar keeps them visible even when the title
is hidden or clipped. (The optional in-frame **FPS overlay**, toggled with
Ctrl+F, is separate — it draws the FPS reading directly onto the video for
screenshots, whereas the info bar is always present outside the picture.)

## Controls

Default keyboard bindings (fully remappable — see [`input.md`](input.md)):

| Button | Player 1 | Player 2 |
|--------|----------|----------|
| Up | W | I |
| Down | S | K |
| Left | A | J |
| Right | D | L |
| B | Z | N |
| A | X | M |
| Select | Q | , (comma) |
| Start | E | . (period) |

Bindings are read from a `keybindings.toml` file. In **debug** builds the repo's
`assets/keybindings.toml` is read directly (edit it, re-run, done). In
**release** builds a copy is seeded next to the binary on first build and never
overwritten, so your edits survive upgrades. Set `$NES_EMU_KEYBINDINGS` to force
a specific path. See [`input.md`](input.md) for the file format and full details.

### Application shortcuts

| Action | Key (default) |
|--------|---------------|
| Load ROM (open file dialog) | Ctrl+O |
| Toggle FPS overlay | Ctrl+F |
| Select save-state slot 0–9 | number keys 0–9 |
| Quick-save active slot | F5 |
| Quick-load active slot | F9 |
| Pause / resume | P |
| Speed up (fast forward) | `=` |
| Slow down (slow motion) | `-` |
| Reset to normal speed | Backspace |

The FPS overlay, slot keys, save/load keys, pause key, and speed keys are all
configurable in the `[app]` section of `keybindings.toml` (`fps_toggle`,
`slots`, `save_state`, `load_state`, `pause`, `speed_up`, `slow_down`,
`normal_speed`). Ctrl+O is fixed.

## Pausing

Press **P** to freeze the emulator and press it again to resume. While paused
the machine stops advancing entirely and the last rendered frame stays on
screen; the window still responds to input and menus, and the title shows a
`· paused` marker. The pause key is remappable via `pause` in the `[app]`
section of `keybindings.toml`.

## Emulation speed

The emulator can run slower or faster than real time. Press `-` to **slow down**
and `=` to **speed up** — each keypress steps one entry through the speed list:

`1/32× · 1/16× · 1/8× · 1/4× · 1/2× · 1× · 1.5× · 2× · 3× · 4× · 8×`

The list clamps at both ends (slowing past `1/32×` or speeding past `8×` does
nothing), and **Backspace** jumps straight back to `1×`. Whenever the speed
isn't `1×` it's shown in the window title (`nes-emu — <rom> · slot N · 2x`).

Speed is a pure wall-clock pacing change: the emulator runs one NES frame every
`frame_time ÷ multiplier` of real time, so slow motion stretches the interval
between frames and fast forward shrinks it. (There is no audio output yet, so
speed changes only affect video pacing.) At very high fast-forward the host may
not be able to emulate frames quickly enough to keep up, in which case it simply
runs as fast as it can. The keys are remappable via `[app]` in
`keybindings.toml`.

## Save states

A save state is a full snapshot of the running machine (CPU, PPU, APU, bus, and
the cartridge's mutable state) — independent of the game's own battery save.

- **Ten numbered slots.** Press a number key **0–9** to select the active slot;
  the current slot is shown in the window title (`nes-emu — <rom> · slot N`).
- **Quick save / load.** **F5** saves the active slot, **F9** loads it. Slot 0 is
  stored as `<rom>.state`, slots 1–9 as `<rom>.state1`…`<rom>.state9`, next to the
  ROM file.
- **Save/Load to any path.** **File → Save State… / Load State…** open a file
  dialog so you can save to or load from an arbitrary location.

A save state does **not** include the ROM itself, so it can only be loaded while
the same ROM is open. Full design details are in [`savestate.md`](savestate.md).

## Input replay (FM2)

The emulator can play back an FM2 input movie for deterministic replay:

```bash
cargo run <rom.nes> <replay.fm2>   # boot the ROM and play the movie
```

You can also load a movie at runtime from **File → Play Replay…**. This is handy
for regression-checking a run or reproducing a bug from a recorded input trace.

## Audio

All five APU channels and the frame counter are emulated and mixed to a sample
stream, but **no audio output device is wired yet** — you'll see accurate APU
state and pass the APU test ROMs, but hear nothing. Audio output is on the
roadmap (see the README's "What's next").

## Supported mappers

Fifteen iNES mappers are implemented; the header's mapper number selects one
automatically:

| # | Name | # | Name |
|---|------|---|------|
| 0 | NROM | 34 | BNROM / NINA-001 |
| 1 | MMC1 (SxROM) | 66 | GxROM |
| 2 | UxROM | 69 | Sunsoft FME-7 |
| 3 | CNROM | 71 | Camerica |
| 4 | MMC3 (+ scanline IRQ) | 87 | Jaleco CHR |
| 7 | AxROM | 206 | Namco 118 / DxROM |
| 9 | MMC2 | | |
| 10 | MMC4 | | |
| 11 | Color Dreams | | |

A ROM whose mapper number isn't in this list will fail to load with an error.

## Troubleshooting

- **Blank window / GPU error under WSL2.** Handled automatically:
  `maybe_configure_wsl2_gpu()` sets `WGPU_BACKEND=vulkan` and points
  `VK_ICD_FILENAMES` at the Mesa LLVMpipe software ICD when it detects WSL2. If
  you still hit issues, ensure Mesa's LLVMpipe ICD is installed.
- **Game runs in slow motion.** You're likely running an unoptimized build. Use
  `cargo run` (the dev profile is already `opt-level 1`) or `cargo run --release`.
- **Bad keybindings file.** If `keybindings.toml` is missing or fails to parse,
  the emulator prints a warning to stderr and falls back to the built-in defaults
  (identical to the shipped file), so it always starts.
- **"No such file" on load.** Only `.nes` (iNES) images are accepted; pass a path
  to a valid ROM.

## Running the tests

The ROM test suites are git submodules — fetch them first (not needed just to
build or run the emulator):

```bash
git submodule update --init --recursive
cargo test                 # run everything (includes the blargg ROM suites)
cargo test <name>          # run a single test by name
cargo test accuracycoin --release -- --ignored --nocapture   # the AccuracyCoin runner
```

See the README's "Test conformance" and [`accuracycoin_outcome.md`](accuracycoin_outcome.md)
for what passes.
