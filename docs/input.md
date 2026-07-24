# Controller Input

Keyboard input is mapped to both NES controller ports via a TOML config
file, `keybindings.toml`.

**Which file is read depends on the build:**

- **Development (`cargo run`, debug builds):** the source file
  `assets/keybindings.toml` in the repo is read directly. Edit it and the
  change applies on the next `cargo run` — no rebuild dance, no stale copy.
- **Release / installed (`cargo run --release`, a shipped binary):** the
  copy sitting next to the executable is read. `build.rs` seeds that copy
  from `assets/keybindings.toml` the first time you build and never
  overwrites it afterward, so your customizations survive upgrades.
- **Override:** set `$NES_EMU_KEYBINDINGS` to an explicit path to force that
  file in either build.

(The previous behavior — debug builds also reading the seeded
`target/debug/keybindings.toml` — meant edits to the tracked
`assets/keybindings.toml` never took effect, because the seed copy was
created once and never refreshed. Debug builds now read the source directly
to avoid that trap.)

## File format

```toml
[player1]
up = "KeyW"
down = "KeyS"
left = "KeyA"
right = "KeyD"
b = "KeyZ"
a = "KeyX"
select = "KeyQ"
start = "KeyE"

[player2]
up = "KeyI"
down = "KeyK"
left = "KeyJ"
right = "KeyL"
b = "KeyN"
a = "KeyM"
select = "Comma"
start = "Period"
```

Both `[player1]` and `[player2]` sections are required, each with all 8
keys. Key values are `winit::keyboard::KeyCode` variant names — see
[winit's `KeyCode` docs](https://docs.rs/winit/latest/winit/keyboard/enum.KeyCode.html)
for the full list (letters are `"KeyA"`.."KeyZ"`, digits are `"Digit0"`.."Digit9"`;
arrow keys are `"ArrowUp"`/`"ArrowDown"`/`"ArrowLeft"`/`"ArrowRight"`).

## Defaults

| Button | P1 key | P2 key |
|---|---|---|
| Up | KeyW | KeyI |
| Down | KeyS | KeyK |
| Left | KeyA | KeyJ |
| Right | KeyD | KeyL |
| B | KeyZ | KeyN |
| A | KeyX | KeyM |
| Select | KeyQ | Comma |
| Start | KeyE | Period |

## App shortcuts

| Action | Combo |
|---|---|
| Toggle FPS overlay | Ctrl+`fps_toggle` (default: Ctrl+F) |
| Open ROM | Ctrl+O (not configurable) |
| Select save-state slot | `slots` (default: 0–9) |
| Save state (active slot) | `save_state` (default: F5) |
| Load state (active slot) | `load_state` (default: F9) |
| Pause / resume | `pause` (default: P) |
| Speed up (fast forward) | `speed_up` (default: `=`) |
| Slow down (slow motion) | `slow_down` (default: `-`) |
| Reset to normal speed | `normal_speed` (default: Backspace) |

`pause` freezes emulation (the last frame stays on screen, the window keeps
responding); pressing it again resumes. `speed_up`/`slow_down` step one entry
through the emulation-speed list —
`1/32×, 1/16×, 1/8×, 1/4×, 1/2×, 1×, 1.5×, 2×, 3×, 4×, 8×` — clamping at both
ends, and `normal_speed` jumps straight back to `1×`. The current speed (when
not `1×`) and a `paused` marker are shown in the window title. See
[`docs/usage.md`](usage.md#emulation-speed).

The `slots` keys pick one of ten save-state slots (shown in the window
title); `save_state`/`load_state` quick-save/load the active slot's file next
to the loaded ROM (`<rom>.state` for slot 0, `<rom>.stateN` for slots 1–9).
The **File → Save State... / Load State...** menu items open a file dialog to
save to / load from any path instead. A save state records the full machine
state but not the ROM itself, so it can only be loaded with the same ROM open.
See `docs/savestate.md`.

App shortcuts live in an optional `[app]` section:

```toml
[app]
fps_toggle = "KeyF"
save_state = "F5"
load_state = "F9"
pause = "KeyP"
speed_up = "Equal"
slow_down = "Minus"
normal_speed = "Backspace"
# Exactly 10 keys, selecting slots 0..9 in order.
slots = [
    "Digit0", "Digit1", "Digit2", "Digit3", "Digit4",
    "Digit5", "Digit6", "Digit7", "Digit8", "Digit9",
]
```

Each field in `[app]` has an independent default, so an older
`keybindings.toml` written before one of these options existed (or one that
omits `[app]` entirely) still works — every unspecified app key falls back to
the default shown above. `slots`, if present, must list exactly ten keys, or
the file fails to parse.

## Fallback behavior

If `keybindings.toml` is missing, unreadable, or fails to parse (a required
`[player1]`/`[player2]` section missing, a key misspelled, a `slots` array of
the wrong length, etc.), the emulator prints a warning to stderr naming the
file and reason, and runs with the hardcoded default keymap above — the same
values as the shipped file, so behavior is identical either way. There's no
partial merging at the file level: an invalid file falls back entirely. Within
a present-and-valid `[app]` section, however, each key defaults independently
(see above).

## Implementation

- `src/input.rs` — `KeyMap`, the button bit constants, and TOML parsing.
- `src/bus.rs` — `Bus::set_controller_state` (byte→bus) and the
  `$4016`/`$4017` shift-register read logic (bus→CPU); see "Controller
  Strobe / Read" in `docs/bus.md`.
- `src/app.rs` — `App::set_controller_buttons`, the passthrough into the bus.
- `src/main.rs` — loads the `KeyMap` at startup and updates per-key button
  state on every `WindowEvent::KeyboardInput`.
- `build.rs` — seeds `keybindings.toml` next to the binary from
  `assets/keybindings.toml`, without overwriting an existing file.
