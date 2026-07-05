use std::collections::HashMap;
use std::fs;
use std::path::Path;

use serde::Deserialize;
use winit::event::VirtualKeyCode;

pub const BUTTON_A: u8 = 1 << 0;
pub const BUTTON_B: u8 = 1 << 1;
pub const BUTTON_SELECT: u8 = 1 << 2;
pub const BUTTON_START: u8 = 1 << 3;
pub const BUTTON_UP: u8 = 1 << 4;
pub const BUTTON_DOWN: u8 = 1 << 5;
pub const BUTTON_LEFT: u8 = 1 << 6;
pub const BUTTON_RIGHT: u8 = 1 << 7;

#[derive(Deserialize)]
struct RawKeyBindings {
    up: VirtualKeyCode,
    down: VirtualKeyCode,
    left: VirtualKeyCode,
    right: VirtualKeyCode,
    a: VirtualKeyCode,
    b: VirtualKeyCode,
    select: VirtualKeyCode,
    start: VirtualKeyCode,
}

#[derive(Deserialize)]
struct RawConfig {
    player1: RawKeyBindings,
    player2: RawKeyBindings,
}

impl RawKeyBindings {
    fn into_pairs(self, port: usize) -> [(VirtualKeyCode, (usize, u8)); 8] {
        [
            (self.up, (port, BUTTON_UP)),
            (self.down, (port, BUTTON_DOWN)),
            (self.left, (port, BUTTON_LEFT)),
            (self.right, (port, BUTTON_RIGHT)),
            (self.a, (port, BUTTON_A)),
            (self.b, (port, BUTTON_B)),
            (self.select, (port, BUTTON_SELECT)),
            (self.start, (port, BUTTON_START)),
        ]
    }
}

/// Maps keyboard keys to (controller port, button bit) pairs. Built either
/// from a parsed `RawConfig` or the hardcoded default (see `Default` impl).
pub struct KeyMap {
    bindings: HashMap<VirtualKeyCode, (usize, u8)>,
}

impl KeyMap {
    fn from_raw(raw: RawConfig) -> Self {
        let mut bindings = HashMap::new();
        for (key, value) in raw.player1.into_pairs(0) {
            bindings.insert(key, value);
        }
        for (key, value) in raw.player2.into_pairs(1) {
            bindings.insert(key, value);
        }
        Self { bindings }
    }

    /// Reads and parses `path` as a keybindings TOML file. On any failure
    /// (missing file, unreadable, parse error), prints a warning to stderr
    /// naming the path and reason, and falls back to `KeyMap::default()`.
    /// No partial merging: the file must fully parse or the whole thing is
    /// discarded in favor of the default.
    pub fn load(path: &Path) -> Self {
        let text = match fs::read_to_string(path) {
            Ok(t) => t,
            Err(e) => {
                eprintln!(
                    "warning: could not read {}: {e} — using default keybindings",
                    path.display()
                );
                return Self::default();
            }
        };
        match toml::from_str::<RawConfig>(&text) {
            Ok(raw) => Self::from_raw(raw),
            Err(e) => {
                eprintln!(
                    "warning: invalid keybindings file {}: {e} — using default keybindings",
                    path.display()
                );
                Self::default()
            }
        }
    }

    /// Looks up which (port, button bit) a keycode is bound to, if any.
    pub fn on_key(&self, keycode: VirtualKeyCode) -> Option<(usize, u8)> {
        self.bindings.get(&keycode).copied()
    }
}

impl Default for KeyMap {
    fn default() -> Self {
        Self::from_raw(RawConfig {
            player1: RawKeyBindings {
                up: VirtualKeyCode::Up,
                down: VirtualKeyCode::Down,
                left: VirtualKeyCode::Left,
                right: VirtualKeyCode::Right,
                b: VirtualKeyCode::Z,
                a: VirtualKeyCode::X,
                select: VirtualKeyCode::A,
                start: VirtualKeyCode::S,
            },
            player2: RawKeyBindings {
                up: VirtualKeyCode::I,
                down: VirtualKeyCode::K,
                left: VirtualKeyCode::J,
                right: VirtualKeyCode::L,
                b: VirtualKeyCode::N,
                a: VirtualKeyCode::M,
                select: VirtualKeyCode::Comma,
                start: VirtualKeyCode::Period,
            },
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn valid_toml_parses_expected_bindings() {
        let toml_text = r#"
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
        "#;
        let raw: RawConfig = toml::from_str(toml_text).expect("valid toml must parse");
        let map = KeyMap::from_raw(raw);
        assert_eq!(map.on_key(VirtualKeyCode::Up), Some((0, BUTTON_UP)));
        assert_eq!(map.on_key(VirtualKeyCode::L), Some((1, BUTTON_RIGHT)));
        assert_eq!(map.on_key(VirtualKeyCode::Period), Some((1, BUTTON_START)));
        assert_eq!(map.on_key(VirtualKeyCode::Q), None);
    }

    #[test]
    fn missing_file_falls_back_to_default() {
        let map = KeyMap::load(Path::new("/nonexistent/path/keybindings.toml"));
        assert_eq!(map.on_key(VirtualKeyCode::Up), Some((0, BUTTON_UP)));
        assert_eq!(map.on_key(VirtualKeyCode::I), Some((1, BUTTON_UP)));
    }

    #[test]
    fn invalid_toml_falls_back_to_default() {
        let path = std::env::temp_dir().join("nes_emu_test_invalid_keybindings.toml");
        fs::write(&path, "not valid toml [[[").unwrap();
        let map = KeyMap::load(&path);
        assert_eq!(map.on_key(VirtualKeyCode::Up), Some((0, BUTTON_UP)));
        let _ = fs::remove_file(&path);
    }

    #[test]
    fn default_map_has_no_collisions_between_players() {
        let map = KeyMap::default();
        let p1_keys = [
            VirtualKeyCode::Up,
            VirtualKeyCode::Down,
            VirtualKeyCode::Left,
            VirtualKeyCode::Right,
            VirtualKeyCode::Z,
            VirtualKeyCode::X,
            VirtualKeyCode::A,
            VirtualKeyCode::S,
        ];
        for key in p1_keys {
            assert_eq!(map.on_key(key).unwrap().0, 0);
        }
        let p2_keys = [
            VirtualKeyCode::I,
            VirtualKeyCode::K,
            VirtualKeyCode::J,
            VirtualKeyCode::L,
            VirtualKeyCode::N,
            VirtualKeyCode::M,
            VirtualKeyCode::Comma,
            VirtualKeyCode::Period,
        ];
        for key in p2_keys {
            assert_eq!(map.on_key(key).unwrap().0, 1);
        }
    }
}
