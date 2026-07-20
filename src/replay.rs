use crate::input::{
    BUTTON_A, BUTTON_B, BUTTON_DOWN, BUTTON_LEFT, BUTTON_RIGHT, BUTTON_SELECT, BUTTON_START,
    BUTTON_UP,
};

/// One playback frame decoded from an FM2 movie: any reset command plus
/// both controller ports' button state in the emulator's native bit layout
/// (see `crate::input::BUTTON_*`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Fm2Frame {
    pub soft_reset: bool,
    pub hard_reset: bool,
    pub controllers: [u8; 2],
}

/// A parsed FM2 movie: a flat list of per-frame input records. Only
/// 2-controller (non-fourscore) gamepad movies are supported — see
/// `Fm2Movie::parse`.
#[derive(Debug)]
pub struct Fm2Movie {
    frames: Vec<Fm2Frame>,
}

/// FM2's fixed per-controller field order: Right, Left, Down, Up, sTart,
/// Select, B, A. https://fceux.com/web/FM2.html
const JOYPAD_BITS: [u8; 8] = [
    BUTTON_RIGHT,
    BUTTON_LEFT,
    BUTTON_DOWN,
    BUTTON_UP,
    BUTTON_START,
    BUTTON_SELECT,
    BUTTON_B,
    BUTTON_A,
];

fn parse_joypad_field(field: &str, line_no: usize) -> Result<u8, String> {
    let chars: Vec<char> = field.chars().collect();
    if chars.len() != 8 {
        return Err(format!(
            "line {line_no}: joypad field {field:?} must be 8 characters, got {}",
            chars.len()
        ));
    }
    let mut buttons = 0u8;
    for (i, ch) in chars.iter().enumerate() {
        if *ch != '.' {
            buttons |= JOYPAD_BITS[i];
        }
    }
    Ok(buttons)
}

fn parse_data_line(line: &str, line_no: usize) -> Result<Fm2Frame, String> {
    let parts: Vec<&str> = line.split('|').collect();
    if parts.len() < 4 {
        return Err(format!(
            "line {line_no}: expected `|cmd|joy0|joy1|`, got {line:?}"
        ));
    }
    let command: u8 = parts[1]
        .trim()
        .parse()
        .map_err(|e| format!("line {line_no}: invalid command byte {:?}: {e}", parts[1]))?;
    let controllers = [
        parse_joypad_field(parts[2], line_no)?,
        parse_joypad_field(parts[3], line_no)?,
    ];
    Ok(Fm2Frame {
        soft_reset: command & 0x01 != 0,
        hard_reset: command & 0x02 != 0,
        controllers,
    })
}

impl Fm2Movie {
    /// Parses FM2 text-format movie data. Rejects `fourscore` movies (the
    /// bus only exposes 2 controller ports) and any malformed data line.
    pub fn parse(text: &str) -> Result<Self, String> {
        for line in text.lines() {
            if line.starts_with('|') {
                break;
            }
            if let Some(value) = line.strip_prefix("fourscore ")
                && value.trim() != "0"
            {
                return Err(
                    "fourscore movies are not supported (only 2-controller movies are)".to_string(),
                );
            }
        }

        let mut frames = Vec::new();
        for (i, line) in text.lines().enumerate() {
            if !line.starts_with('|') {
                continue;
            }
            frames.push(parse_data_line(line, i + 1)?);
        }
        Ok(Self { frames })
    }
}

/// Steps through a parsed `Fm2Movie` one frame at a time.
pub struct Fm2Player {
    movie: Fm2Movie,
    index: usize,
}

impl Fm2Player {
    /// Wraps `movie`, starting playback at its first frame.
    pub fn new(movie: Fm2Movie) -> Self {
        Self { movie, index: 0 }
    }

    /// Returns the next frame and advances the cursor, or `None` once the
    /// movie is exhausted.
    pub fn next_frame(&mut self) -> Option<Fm2Frame> {
        let frame = self.movie.frames.get(self.index).copied()?;
        self.index += 1;
        Some(frame)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_button_presses_in_rldutsba_order() {
        let text = "fourscore 0\n|0|R.......|.......A||\n";
        let movie = Fm2Movie::parse(text).unwrap();
        assert_eq!(movie.frames.len(), 1);
        assert_eq!(movie.frames[0].controllers[0], BUTTON_RIGHT);
        assert_eq!(movie.frames[0].controllers[1], BUTTON_A);
        assert!(!movie.frames[0].soft_reset);
        assert!(!movie.frames[0].hard_reset);
    }

    #[test]
    fn command_byte_bit0_sets_soft_reset_bit1_sets_hard_reset() {
        let text = "fourscore 0\n\
                     |1|........|........||\n\
                     |2|........|........||\n\
                     |3|........|........||\n";
        let movie = Fm2Movie::parse(text).unwrap();
        assert_eq!(movie.frames.len(), 3);
        assert!(movie.frames[0].soft_reset && !movie.frames[0].hard_reset);
        assert!(!movie.frames[1].soft_reset && movie.frames[1].hard_reset);
        assert!(movie.frames[2].soft_reset && movie.frames[2].hard_reset);
    }

    #[test]
    fn fourscore_movies_are_rejected() {
        let text = "fourscore 1\n|0|........|........|........|........||\n";
        let err = Fm2Movie::parse(text).unwrap_err();
        assert!(
            err.contains("fourscore"),
            "error must mention fourscore: {err}"
        );
    }

    #[test]
    fn header_only_input_parses_to_zero_frames() {
        let text = "version 3\nfourscore 0\nport0 1\nport1 1\n";
        let movie = Fm2Movie::parse(text).unwrap();
        assert_eq!(movie.frames.len(), 0);
    }

    #[test]
    fn malformed_joypad_field_length_errors() {
        let text = "fourscore 0\n|0|SHORT|........||\n";
        assert!(Fm2Movie::parse(text).is_err());
    }

    #[test]
    fn non_numeric_command_byte_errors() {
        let text = "fourscore 0\n|X|........|........||\n";
        assert!(Fm2Movie::parse(text).is_err());
    }

    #[test]
    fn fm2_player_returns_frames_in_order_then_none() {
        let text = "fourscore 0\n|1|........|........||\n|0|........|........||\n";
        let movie = Fm2Movie::parse(text).unwrap();
        let mut player = Fm2Player::new(movie);
        assert!(player.next_frame().unwrap().soft_reset);
        assert!(!player.next_frame().unwrap().soft_reset);
        assert!(player.next_frame().is_none());
    }
}
