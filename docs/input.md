# Controller Input

Keyboard input is mapped to both NES controller ports via a TOML config
file, `keybindings.toml`, which is seeded next to the built binary (e.g.
`target/debug/keybindings.toml` or `target/release/keybindings.toml`) the
first time you `cargo build`. Edit that file to remap keys; rebuilding
afterward will not overwrite your edits (see `build.rs`).

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
| Select save-state slot | 0–9 (not configurable) |
| Save state (active slot) | F5 (not configurable) |
| Load state (active slot) | F9 (not configurable) |

Number keys 0–9 pick one of ten save-state slots (shown in the window
title); F5/F9 quick-save/load the active slot's file next to the loaded ROM
(`<rom>.state` for slot 0, `<rom>.stateN` for slots 1–9). The **File → Save
State... / Load State...** menu items open a file dialog to save to / load
from any path instead. A save state records the full machine state but not the
ROM itself, so it can only be loaded with the same ROM open. See
`docs/savestate.md`.

`fps_toggle` lives in an optional `[app]` section:

```toml
[app]
fps_toggle = "KeyF"
```

If `[app]` is missing entirely (e.g. an older `keybindings.toml` written
before this option existed), it defaults to `F` — same fallback tolerance
as a missing file, just scoped to this one section instead of the whole
file.

## Fallback behavior

If `keybindings.toml` is missing, unreadable, or fails to parse (either
section missing, a key misspelled, etc.), the emulator prints a warning to
stderr naming the file and reason, and runs with the hardcoded default
keymap above — the same values as the shipped file, so behavior is
identical either way. There's no partial merging: an invalid file falls
back entirely rather than mixing in per-field defaults.

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
