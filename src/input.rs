use std::collections::HashMap;
use std::fs;
use std::path::Path;

use serde::Deserialize;
use winit::keyboard::KeyCode;

use crate::app::NUM_SLOTS;

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

fn default_fps_toggle() -> KeyCode {
    KeyCode::KeyF
}

fn default_save_state() -> KeyCode {
    KeyCode::F5
}

fn default_load_state() -> KeyCode {
    KeyCode::F9
}

fn default_pause() -> KeyCode {
    KeyCode::KeyP
}

fn default_speed_up() -> KeyCode {
    KeyCode::Equal
}

fn default_slow_down() -> KeyCode {
    KeyCode::Minus
}

fn default_normal_speed() -> KeyCode {
    KeyCode::Backspace
}

fn default_slots() -> [KeyCode; NUM_SLOTS] {
    [
        KeyCode::Digit0,
        KeyCode::Digit1,
        KeyCode::Digit2,
        KeyCode::Digit3,
        KeyCode::Digit4,
        KeyCode::Digit5,
        KeyCode::Digit6,
        KeyCode::Digit7,
        KeyCode::Digit8,
        KeyCode::Digit9,
    ]
}

#[derive(Deserialize)]
struct RawAppConfig {
    /// FPS-overlay toggle (paired with Ctrl). Default: F.
    #[serde(default = "default_fps_toggle")]
    fps_toggle: KeyCode,
    /// Quick-save the active slot. Default: F5.
    #[serde(default = "default_save_state")]
    save_state: KeyCode,
    /// Quick-load the active slot. Default: F9.
    #[serde(default = "default_load_state")]
    load_state: KeyCode,
    /// Freeze/unfreeze emulation. Default: P.
    #[serde(default = "default_pause")]
    pause: KeyCode,
    /// Step one entry faster through the speed list (fast forward). Default: =.
    #[serde(default = "default_speed_up")]
    speed_up: KeyCode,
    /// Step one entry slower through the speed list (slow motion). Default: -.
    #[serde(default = "default_slow_down")]
    slow_down: KeyCode,
    /// Jump back to 1x (normal) speed. Default: Backspace.
    #[serde(default = "default_normal_speed")]
    normal_speed: KeyCode,
    /// Keys selecting save-state slots 0..NUM_SLOTS, in order. Default: the
    /// number-row keys Digit0..Digit9. Must list exactly `NUM_SLOTS` keys.
    #[serde(default = "default_slots")]
    slots: [KeyCode; NUM_SLOTS],
}

impl Default for RawAppConfig {
    fn default() -> Self {
        Self {
            fps_toggle: default_fps_toggle(),
            save_state: default_save_state(),
            load_state: default_load_state(),
            pause: default_pause(),
            speed_up: default_speed_up(),
            slow_down: default_slow_down(),
            normal_speed: default_normal_speed(),
            slots: default_slots(),
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
    save_state: KeyCode,
    load_state: KeyCode,
    pause: KeyCode,
    speed_up: KeyCode,
    slow_down: KeyCode,
    normal_speed: KeyCode,
    slots: [KeyCode; NUM_SLOTS],
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
            save_state: raw.app.save_state,
            load_state: raw.app.load_state,
            pause: raw.app.pause,
            speed_up: raw.app.speed_up,
            slow_down: raw.app.slow_down,
            normal_speed: raw.app.normal_speed,
            slots: raw.app.slots,
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

    /// True if `keycode` is the configured save-state (quick-save) key.
    pub fn is_save_state(&self, keycode: KeyCode) -> bool {
        keycode == self.save_state
    }

    /// True if `keycode` is the configured load-state (quick-load) key.
    pub fn is_load_state(&self, keycode: KeyCode) -> bool {
        keycode == self.load_state
    }

    /// True if `keycode` is the configured pause (freeze/unfreeze) key.
    pub fn is_pause(&self, keycode: KeyCode) -> bool {
        keycode == self.pause
    }

    /// True if `keycode` is the configured "speed up" (fast-forward) key.
    pub fn is_speed_up(&self, keycode: KeyCode) -> bool {
        keycode == self.speed_up
    }

    /// True if `keycode` is the configured "slow down" (slow-motion) key.
    pub fn is_slow_down(&self, keycode: KeyCode) -> bool {
        keycode == self.slow_down
    }

    /// True if `keycode` is the configured "normal speed" (reset to 1x) key.
    pub fn is_normal_speed(&self, keycode: KeyCode) -> bool {
        keycode == self.normal_speed
    }

    /// The save-state slot `keycode` selects (0..NUM_SLOTS), or `None` if it
    /// isn't a configured slot key.
    pub fn slot_for_key(&self, keycode: KeyCode) -> Option<u8> {
        self.slots
            .iter()
            .position(|&k| k == keycode)
            .map(|i| i as u8)
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
mod speed_key_tests {
    use super::*;

    #[test]
    fn default_map_has_speed_control_keys() {
        let map = KeyMap::default();
        assert!(map.is_pause(KeyCode::KeyP));
        assert!(map.is_speed_up(KeyCode::Equal));
        assert!(map.is_slow_down(KeyCode::Minus));
        assert!(map.is_normal_speed(KeyCode::Backspace));
    }

    #[test]
    fn missing_app_section_defaults_speed_keys() {
        let players = r#"
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
        let raw: RawConfig = toml::from_str(players).expect("valid toml must parse");
        let map = KeyMap::from_raw(raw);
        assert!(map.is_pause(KeyCode::KeyP));
        assert!(map.is_speed_up(KeyCode::Equal));
        assert!(map.is_slow_down(KeyCode::Minus));
        assert!(map.is_normal_speed(KeyCode::Backspace));
    }

    #[test]
    fn app_section_overrides_speed_keys() {
        let text = r#"
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
            pause = "Enter"
            speed_up = "BracketRight"
            slow_down = "BracketLeft"
            normal_speed = "Backslash"
        "#;
        let raw: RawConfig = toml::from_str(text).expect("valid toml must parse");
        let map = KeyMap::from_raw(raw);
        assert!(map.is_pause(KeyCode::Enter));
        assert!(map.is_speed_up(KeyCode::BracketRight));
        assert!(map.is_slow_down(KeyCode::BracketLeft));
        assert!(map.is_normal_speed(KeyCode::Backslash));
        // Defaults no longer apply once overridden.
        assert!(!map.is_pause(KeyCode::KeyP));
        assert!(!map.is_speed_up(KeyCode::Equal));
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

    /// The two required player sections, shared by the `[app]`-focused tests.
    const PLAYERS: &str = r#"
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

    fn map_from_app(app_section: &str) -> KeyMap {
        let text = format!("{PLAYERS}\n{app_section}");
        let raw: RawConfig = toml::from_str(&text).expect("valid toml must parse");
        KeyMap::from_raw(raw)
    }

    #[test]
    fn default_map_has_save_load_and_slot_keys() {
        let map = KeyMap::default();
        assert!(map.is_save_state(KeyCode::F5));
        assert!(map.is_load_state(KeyCode::F9));
        assert_eq!(map.slot_for_key(KeyCode::Digit0), Some(0));
        assert_eq!(map.slot_for_key(KeyCode::Digit9), Some(9));
        assert_eq!(map.slot_for_key(KeyCode::KeyP), None);
    }

    #[test]
    fn missing_app_section_defaults_save_load_and_slots() {
        // No `[app]` at all -> RawAppConfig::default() supplies every app key.
        let raw: RawConfig = toml::from_str(PLAYERS).expect("valid toml must parse");
        let map = KeyMap::from_raw(raw);
        assert!(map.is_save_state(KeyCode::F5));
        assert!(map.is_load_state(KeyCode::F9));
        assert_eq!(map.slot_for_key(KeyCode::Digit3), Some(3));
    }

    #[test]
    fn app_section_overrides_save_load_and_slot_keys() {
        let map = map_from_app(
            r#"
            [app]
            save_state = "F2"
            load_state = "F4"
            slots = ["Numpad0", "Numpad1", "Numpad2", "Numpad3", "Numpad4",
                     "Numpad5", "Numpad6", "Numpad7", "Numpad8", "Numpad9"]
            "#,
        );
        assert!(map.is_save_state(KeyCode::F2));
        assert!(map.is_load_state(KeyCode::F4));
        assert!(!map.is_save_state(KeyCode::F5));
        assert_eq!(map.slot_for_key(KeyCode::Numpad3), Some(3));
        // The default number-row keys no longer select slots.
        assert_eq!(map.slot_for_key(KeyCode::Digit3), None);
    }

    #[test]
    fn app_section_fields_default_independently() {
        // Only `save_state` is set; the other app keys fall back to defaults.
        let map = map_from_app("[app]\nsave_state = \"F1\"");
        assert!(map.is_save_state(KeyCode::F1));
        assert!(map.is_load_state(KeyCode::F9)); // default
        assert!(map.is_fps_toggle(KeyCode::KeyF)); // default
        assert_eq!(map.slot_for_key(KeyCode::Digit5), Some(5)); // default
    }

    #[test]
    fn slots_with_wrong_count_is_a_parse_error() {
        // The array must contain exactly NUM_SLOTS keys.
        let text = format!("{PLAYERS}\n[app]\nslots = [\"Digit0\", \"Digit1\"]");
        assert!(toml::from_str::<RawConfig>(&text).is_err());
    }
}
