//! xterm mouse reporting: X10/normal, UTF-8 (1005) and SGR (1006) encodings.

use crate::{GridPos, TermModeFlags};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MouseButton {
    Left,
    Middle,
    Right,
    WheelUp,
    WheelDown,
    /// Motion without a pressed button (any-motion tracking).
    None,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct MouseModifiers {
    pub shift: bool,
    pub alt: bool,
    pub ctrl: bool,
}

fn button_code(button: MouseButton) -> u8 {
    match button {
        MouseButton::Left => 0,
        MouseButton::Middle => 1,
        MouseButton::Right => 2,
        MouseButton::None => 3,
        MouseButton::WheelUp => 64,
        MouseButton::WheelDown => 65,
    }
}

pub(crate) fn encode(
    button: MouseButton,
    pressed: bool,
    motion: bool,
    pos: GridPos,
    mods: MouseModifiers,
    mode: TermModeFlags,
) -> Option<Vec<u8>> {
    let mut code = button_code(button);
    if motion {
        code += 32;
    }
    if mods.shift {
        code += 4;
    }
    if mods.alt {
        code += 8;
    }
    if mods.ctrl {
        code += 16;
    }
    let (x, y) = (pos.column + 1, pos.line + 1);

    if mode.sgr_mouse() {
        let suffix = if pressed { 'M' } else { 'm' };
        return Some(format!("\x1b[<{code};{x};{y}{suffix}").into_bytes());
    }

    // Legacy encodings cannot express which button was released.
    if !pressed {
        code = 3 + (code & !3);
    }
    let mut bytes = b"\x1b[M".to_vec();
    bytes.push(32 + code);
    if mode.utf8_mouse() {
        for value in [x, y] {
            let ch = char::from_u32(32 + value as u32)?;
            let mut buf = [0u8; 4];
            bytes.extend_from_slice(ch.encode_utf8(&mut buf).as_bytes());
        }
    } else {
        // Coordinates above 223 cannot be encoded in a single byte.
        if x > 223 || y > 223 {
            return None;
        }
        bytes.push(32 + x as u8);
        bytes.push(32 + y as u8);
    }
    Some(bytes)
}

#[cfg(test)]
mod tests {
    use super::*;
    use alacritty_terminal::term::TermMode;

    fn pos(column: usize, line: usize) -> GridPos {
        GridPos {
            line,
            column,
            right_half: false,
        }
    }

    #[test]
    fn encodes_sgr_and_legacy_reports() {
        let sgr = TermModeFlags(TermMode::MOUSE_REPORT_CLICK | TermMode::SGR_MOUSE);
        let mods = MouseModifiers::default();
        assert_eq!(
            encode(MouseButton::Left, true, false, pos(4, 9), mods, sgr).unwrap(),
            b"\x1b[<0;5;10M"
        );
        assert_eq!(
            encode(MouseButton::Left, false, false, pos(4, 9), mods, sgr).unwrap(),
            b"\x1b[<0;5;10m"
        );
        assert_eq!(
            encode(MouseButton::WheelUp, true, false, pos(0, 0), mods, sgr).unwrap(),
            b"\x1b[<64;1;1M"
        );
        let ctrl = MouseModifiers { ctrl: true, ..mods };
        assert_eq!(
            encode(MouseButton::Left, true, true, pos(0, 0), ctrl, sgr).unwrap(),
            b"\x1b[<48;1;1M"
        );

        let legacy = TermModeFlags(TermMode::MOUSE_REPORT_CLICK);
        assert_eq!(
            encode(MouseButton::Right, true, false, pos(0, 0), mods, legacy).unwrap(),
            vec![0x1b, b'[', b'M', 34, 33, 33]
        );
        assert_eq!(
            encode(MouseButton::Right, false, false, pos(0, 0), mods, legacy).unwrap(),
            vec![0x1b, b'[', b'M', 35, 33, 33]
        );
        assert!(encode(MouseButton::Left, true, false, pos(300, 0), mods, legacy).is_none());
    }
}
