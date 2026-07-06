use std::collections::HashMap;
use std::fs;
use std::path::Path;

use serde::Deserialize;
use tao::keyboard::KeyCode;

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
    up: KeyCode,
    down: KeyCode,
    left: KeyCode,
    right: KeyCode,
    a: KeyCode,
    b: KeyCode,
    select: KeyCode,
    start: KeyCode,
}

#[derive(Deserialize)]
struct RawAppConfig {
    fps_toggle: KeyCode,
}

impl Default for RawAppConfig {
    fn default() -> Self {
        Self {
            fps_toggle: KeyCode::KeyF,
        }
    }
}

#[derive(Deserialize)]
struct RawConfig {
    player1: RawKeyBindings,
    player2: RawKeyBindings,
    #[serde(default)]
    app: RawAppConfig,
}

impl RawKeyBindings {
    fn into_pairs(self, port: usize) -> [(KeyCode, (usize, u8)); 8] {
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
    bindings: HashMap<KeyCode, (usize, u8)>,
    fps_toggle: KeyCode,
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
        Self {
            bindings,
            fps_toggle: raw.app.fps_toggle,
        }
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
    pub fn on_key(&self, keycode: KeyCode) -> Option<(usize, u8)> {
        self.bindings.get(&keycode).copied()
    }

    /// True if `keycode` is the configured FPS-overlay toggle key (paired
    /// with Ctrl by the caller — this only checks the letter).
    pub fn is_fps_toggle(&self, keycode: KeyCode) -> bool {
        keycode == self.fps_toggle
    }
}

impl Default for KeyMap {
    fn default() -> Self {
        Self::from_raw(RawConfig {
            player1: RawKeyBindings {
                up: KeyCode::KeyW,
                down: KeyCode::KeyS,
                left: KeyCode::KeyA,
                right: KeyCode::KeyD,
                b: KeyCode::KeyZ,
                a: KeyCode::KeyX,
                select: KeyCode::KeyQ,
                start: KeyCode::KeyE,
            },
            player2: RawKeyBindings {
                up: KeyCode::KeyI,
                down: KeyCode::KeyK,
                left: KeyCode::KeyJ,
                right: KeyCode::KeyL,
                b: KeyCode::KeyN,
                a: KeyCode::KeyM,
                select: KeyCode::Comma,
                start: KeyCode::Period,
            },
            app: RawAppConfig::default(),
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
            up = "ArrowUp"
            down = "ArrowDown"
            left = "ArrowLeft"
            right = "ArrowRight"
            b = "KeyZ"
            a = "KeyX"
            select = "KeyA"
            start = "KeyS"

            [player2]
            up = "KeyI"
            down = "KeyK"
            left = "KeyJ"
            right = "KeyL"
            b = "KeyN"
            a = "KeyM"
            select = "Comma"
            start = "Period"
        "#;
        let raw: RawConfig = toml::from_str(toml_text).expect("valid toml must parse");
        let map = KeyMap::from_raw(raw);
        assert_eq!(map.on_key(KeyCode::ArrowUp), Some((0, BUTTON_UP)));
        assert_eq!(map.on_key(KeyCode::KeyL), Some((1, BUTTON_RIGHT)));
        assert_eq!(map.on_key(KeyCode::Period), Some((1, BUTTON_START)));
        assert_eq!(map.on_key(KeyCode::KeyQ), None);
    }

    #[test]
    fn missing_file_falls_back_to_default() {
        let map = KeyMap::load(Path::new("/nonexistent/path/keybindings.toml"));
        assert_eq!(map.on_key(KeyCode::KeyW), Some((0, BUTTON_UP)));
        assert_eq!(map.on_key(KeyCode::KeyI), Some((1, BUTTON_UP)));
    }

    #[test]
    fn invalid_toml_falls_back_to_default() {
        let path = std::env::temp_dir().join("nes_emu_test_invalid_keybindings.toml");
        fs::write(&path, "not valid toml [[[").unwrap();
        let map = KeyMap::load(&path);
        assert_eq!(map.on_key(KeyCode::KeyW), Some((0, BUTTON_UP)));
        let _ = fs::remove_file(&path);
    }

    #[test]
    fn default_map_has_no_collisions_between_players() {
        let map = KeyMap::default();
        let p1_keys = [
            KeyCode::KeyW,
            KeyCode::KeyS,
            KeyCode::KeyA,
            KeyCode::KeyD,
            KeyCode::KeyZ,
            KeyCode::KeyX,
            KeyCode::KeyQ,
            KeyCode::KeyE,
        ];
        for key in p1_keys {
            assert_eq!(map.on_key(key).unwrap().0, 0);
        }
        let p2_keys = [
            KeyCode::KeyI,
            KeyCode::KeyK,
            KeyCode::KeyJ,
            KeyCode::KeyL,
            KeyCode::KeyN,
            KeyCode::KeyM,
            KeyCode::Comma,
            KeyCode::Period,
        ];
        for key in p2_keys {
            assert_eq!(map.on_key(key).unwrap().0, 1);
        }
    }

    #[test]
    fn missing_app_section_falls_back_to_f_toggle() {
        let toml_text = r#"
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
        "#;
        let raw: RawConfig = toml::from_str(toml_text).expect("valid toml must parse");
        let map = KeyMap::from_raw(raw);
        assert!(map.is_fps_toggle(KeyCode::KeyF));
        assert!(!map.is_fps_toggle(KeyCode::KeyG));
    }

    #[test]
    fn explicit_app_section_overrides_toggle_key() {
        let toml_text = r#"
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

            [app]
            fps_toggle = "KeyG"
        "#;
        let raw: RawConfig = toml::from_str(toml_text).expect("valid toml must parse");
        let map = KeyMap::from_raw(raw);
        assert!(map.is_fps_toggle(KeyCode::KeyG));
        assert!(!map.is_fps_toggle(KeyCode::KeyF));
    }

    #[test]
    fn default_map_has_f_as_fps_toggle() {
        let map = KeyMap::default();
        assert!(map.is_fps_toggle(KeyCode::KeyF));
    }
}
