/// A UI-neutral key press for translating into terminal input bytes.
#[derive(Clone, Debug, Default)]
pub struct KeyInput<'a> {
    /// Key name in GPUI's vocabulary: "a", "enter", "left", "f1", ...
    pub key: &'a str,
    /// Text the key produces with the current layout, when any.
    pub key_char: Option<&'a str>,
    pub ctrl: bool,
    pub alt: bool,
    pub shift: bool,
    pub app_cursor: bool,
    /// Treat Option/Alt as Meta (ESC prefix). When false, Option composes
    /// characters through the keyboard layout (needed for `[ ] { } | @ ~`
    /// on many European layouts) and those arrive as text input instead.
    pub option_as_meta: bool,
    /// The alternate screen is active (full-screen TUI).
    pub alt_screen: bool,
}

fn modifier_param(input: &KeyInput) -> Option<u8> {
    let value = 1 + (input.shift as u8) + 2 * (input.alt as u8) + 4 * (input.ctrl as u8);
    (value > 1).then_some(value)
}

/// Translate a key press into the bytes a terminal application expects.
/// Returns `None` for keys that are not control input: printable text is
/// delivered through the platform text input path (IME, dead keys).
pub fn key_to_bytes(input: &KeyInput) -> Option<Vec<u8>> {
    let modifier = modifier_param(input);
    let csi = |final_char: char| -> Vec<u8> {
        match modifier {
            Some(m) => format!("\x1b[1;{m}{final_char}").into_bytes(),
            None if input.app_cursor => format!("\x1bO{final_char}").into_bytes(),
            None => format!("\x1b[{final_char}").into_bytes(),
        }
    };
    let tilde = |code: u8| -> Vec<u8> {
        match modifier {
            Some(m) => format!("\x1b[{code};{m}~").into_bytes(),
            None => format!("\x1b[{code}~").into_bytes(),
        }
    };
    let bytes = match input.key {
        "enter" => {
            if input.alt {
                b"\x1b\r".to_vec()
            } else {
                b"\r".to_vec()
            }
        }
        "tab" if input.shift => b"\x1b[Z".to_vec(),
        "tab" => b"\t".to_vec(),
        "backspace" => {
            if input.alt {
                b"\x1b\x7f".to_vec()
            } else if input.ctrl {
                b"\x08".to_vec()
            } else {
                b"\x7f".to_vec()
            }
        }
        "escape" => b"\x1b".to_vec(),
        "up" => csi('A'),
        "down" => csi('B'),
        // Shells expect word motion as ESC b / ESC f (readline, zle).
        "right" if input.alt && !input.ctrl && !input.shift && !input.alt_screen => {
            b"\x1bf".to_vec()
        }
        "left" if input.alt && !input.ctrl && !input.shift && !input.alt_screen => {
            b"\x1bb".to_vec()
        }
        "right" => csi('C'),
        "left" => csi('D'),
        "home" => csi('H'),
        "end" => csi('F'),
        "insert" => tilde(2),
        "delete" => tilde(3),
        "pageup" => tilde(5),
        "pagedown" => tilde(6),
        "f1" => b"\x1bOP".to_vec(),
        "f2" => b"\x1bOQ".to_vec(),
        "f3" => b"\x1bOR".to_vec(),
        "f4" => b"\x1bOS".to_vec(),
        "f5" => tilde(15),
        "f6" => tilde(17),
        "f7" => tilde(18),
        "f8" => tilde(19),
        "f9" => tilde(20),
        "f10" => tilde(21),
        "f11" => tilde(23),
        "f12" => tilde(24),
        "space" if input.ctrl => vec![0],
        key if input.ctrl && key.len() == 1 => {
            let c = key.as_bytes()[0].to_ascii_lowercase();
            let code = match c {
                b'a'..=b'z' => c - b'a' + 1,
                b'[' | b'3' => 0x1b,
                b'\\' | b'4' => 0x1c,
                b']' | b'5' => 0x1d,
                b'6' => 0x1e,
                b'/' | b'7' | b'-' => 0x1f,
                b'@' | b'2' => 0,
                _ => return None,
            };
            if input.alt {
                vec![0x1b, code]
            } else {
                vec![code]
            }
        }
        "space" if input.alt && input.option_as_meta => b"\x1b ".to_vec(),
        key if input.alt && input.option_as_meta && key.chars().count() == 1 => {
            // Meta: ESC followed by the key (shifted when Shift is held).
            let text = if input.shift {
                key.to_uppercase()
            } else {
                key.to_string()
            };
            let mut bytes = vec![0x1b];
            bytes.extend_from_slice(text.as_bytes());
            bytes
        }
        // Printable text, including Option-composed characters, arrives
        // through the platform text input path instead.
        _ => return None,
    };
    Some(bytes)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn key(key: &str) -> KeyInput<'_> {
        KeyInput {
            key,
            ..Default::default()
        }
    }

    #[test]
    fn translates_common_keys() {
        assert_eq!(key_to_bytes(&key("enter")).unwrap(), b"\r");
        assert_eq!(
            key_to_bytes(&KeyInput {
                ctrl: true,
                ..key("c")
            })
            .unwrap(),
            vec![3]
        );
        assert_eq!(key_to_bytes(&key("up")).unwrap(), b"\x1b[A");
        assert_eq!(
            key_to_bytes(&KeyInput {
                app_cursor: true,
                ..key("up")
            })
            .unwrap(),
            b"\x1bOA"
        );
        assert_eq!(
            key_to_bytes(&KeyInput {
                ctrl: true,
                ..key("right")
            })
            .unwrap(),
            b"\x1b[1;5C"
        );
        let text = |key_char, alt| KeyInput {
            key_char: Some(key_char),
            alt,
            ..key("8")
        };
        assert!(
            key_to_bytes(&text("å", false)).is_none(),
            "text goes through IME"
        );
        assert!(
            key_to_bytes(&text("[", true)).is_none(),
            "Option composes ["
        );
        let meta = |name| KeyInput {
            alt: true,
            option_as_meta: true,
            ..key(name)
        };
        assert_eq!(key_to_bytes(&meta("b")).unwrap(), b"\x1bb");
        let word_left = KeyInput {
            alt: true,
            ..key("left")
        };
        assert_eq!(key_to_bytes(&word_left).unwrap(), b"\x1bb");
        let tui_left = KeyInput {
            alt_screen: true,
            ..word_left
        };
        assert_eq!(key_to_bytes(&tui_left).unwrap(), b"\x1b[1;3D");
        let back_tab = KeyInput {
            shift: true,
            ..key("tab")
        };
        assert_eq!(key_to_bytes(&back_tab).unwrap(), b"\x1b[Z");
        assert!(key_to_bytes(&key("shift")).is_none());
    }
}
