# Controller Input

Keyboard input is mapped to both NES controller ports via a TOML config
file, `keybindings.toml`, which is seeded next to the built binary (e.g.
`target/debug/keybindings.toml` or `target/release/keybindings.toml`) the
first time you `cargo build`. Edit that file to remap keys; rebuilding
afterward will not overwrite your edits (see `build.rs`).

## File format

```toml
[player1]
up = "Up"
down = "Down"
left = "Left"
right = "Right"
b = "Z"
a = "X"
select = "A"
start = "S"

[player2]
up = "I"
down = "K"
left = "J"
right = "L"
b = "N"
a = "M"
select = "Comma"
start = "Period"
```

Both `[player1]` and `[player2]` sections are required, each with all 8
keys. Key values are winit `VirtualKeyCode` variant names — see
[winit's `VirtualKeyCode` docs](https://docs.rs/winit/0.28/winit/event/enum.VirtualKeyCode.html)
for the full list (letters/digits are their own variants, e.g. `"Z"`,
`"Key1"`; arrow keys are `"Up"`/`"Down"`/`"Left"`/`"Right"`).

## Defaults

| Button | P1 key | P2 key |
|---|---|---|
| Up | Up | I |
| Down | Down | K |
| Left | Left | J |
| Right | Right | L |
| B | Z | N |
| A | X | M |
| Select | A | Comma |
| Start | S | Period |

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
