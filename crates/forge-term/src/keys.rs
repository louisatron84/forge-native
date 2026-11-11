//! What a key press sends to the program in a terminal (the bytes xterm
//! sends: control characters, escape sequences for arrows and function keys).

/// Bytes for a key that isn't plain text, or None to let typed text through.
/// `key` is the key's name as GPUI gives it ("a", "enter", "up", "f5"...).
pub fn key_bytes(key: &str, ctrl: bool, alt: bool, shift: bool, app_cursor: bool) -> Option<Vec<u8>> {
    // Modifier code used in CSI sequences: 1 + shift + 2*alt + 4*ctrl.
    let modifier = 1 + u8::from(shift) + 2 * u8::from(alt) + 4 * u8::from(ctrl);
    let csi = |final_byte: char| -> Vec<u8> {
        if modifier > 1 {
            format!("\x1b[1;{modifier}{final_byte}").into_bytes()
        } else if app_cursor {
            format!("\x1bO{final_byte}").into_bytes()
        } else {
            format!("\x1b[{final_byte}").into_bytes()
        }
    };
    let tilde = |code: u8| -> Vec<u8> {
        if modifier > 1 {
            format!("\x1b[{code};{modifier}~").into_bytes()
        } else {
            format!("\x1b[{code}~").into_bytes()
        }
    };
    let with_alt = |bytes: Vec<u8>| if alt { [vec![0x1b], bytes].concat() } else { bytes };
    let out = match key {
        "enter" => with_alt(vec![b'\r']),
        "backspace" => with_alt(if ctrl { vec![0x08] } else { vec![0x7f] }),
        "tab" => {
            if shift {
                b"\x1b[Z".to_vec()
            } else {
                vec![b'\t']
            }
        }
        "escape" => vec![0x1b],
        "up" => csi('A'),
        "down" => csi('B'),
        "right" => csi('C'),
        "left" => csi('D'),
        "home" => csi('H'),
        "end" => csi('F'),
        "insert" => tilde(2),
        "delete" => tilde(3),
        "pageup" => tilde(5),
        "pagedown" => tilde(6),
        "f1" | "f2" | "f3" | "f4" => {
            let c = match key {
                "f1" => 'P',
                "f2" => 'Q',
                "f3" => 'R',
                _ => 'S',
            };
            if modifier > 1 {
                format!("\x1b[1;{modifier}{c}").into_bytes()
            } else {
                format!("\x1bO{c}").into_bytes()
            }
        }
        k if k.len() >= 2 && k.starts_with('f') && k[1..].parse::<u8>().is_ok() => {
            let n: u8 = k[1..].parse().unwrap_or(0);
            let code = match n {
                5 => 15,
                6 => 17,
                7 => 18,
                8 => 19,
                9 => 20,
                10 => 21,
                11 => 23,
                12 => 24,
                _ => return None,
            };
            tilde(code)
        }
        "space" if ctrl => vec![0],
        k if ctrl && k.chars().count() == 1 => {
            let c = k.chars().next().unwrap_or(' ').to_ascii_lowercase();
            let byte = match c {
                'a'..='z' => c as u8 - b'a' + 1,
                '[' | '3' => 0x1b,
                '\\' | '4' => 0x1c,
                ']' | '5' => 0x1d,
                '^' | '6' => 0x1e,
                '_' | '-' | '7' => 0x1f,
                '@' | '2' => 0,
                '8' => 0x7f,
                _ => return None,
            };
            with_alt(vec![byte])
        }
        // Alt+letter: ESC then the letter (typed text otherwise comes as text).
        k if alt && !ctrl && k.chars().count() == 1 => {
            let c = k.chars().next().unwrap_or(' ');
            let c = if shift { c.to_ascii_uppercase() } else { c };
            let mut out = vec![0x1b];
            let mut buf = [0u8; 4];
            out.extend_from_slice(c.encode_utf8(&mut buf).as_bytes());
            out
        }
        _ => return None,
    };
    Some(out)
}

/// Pasted text as the program should get it (bracketed when it asks for that;
/// line breaks as Enter presses).
pub fn paste_bytes(text: &str, bracketed: bool) -> Vec<u8> {
    let body = text.replace("\r\n", "\r").replace('\n', "\r");
    if bracketed {
        // A paste can't end the bracket early.
        let body = body.replace("\x1b[201~", "");
        format!("\x1b[200~{body}\x1b[201~").into_bytes()
    } else {
        body.into_bytes()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn k(key: &str) -> Option<Vec<u8>> {
        key_bytes(key, false, false, false, false)
    }

    #[test]
    fn control_keys() {
        assert_eq!(k("enter"), Some(b"\r".to_vec()));
        assert_eq!(k("backspace"), Some(vec![0x7f]));
        assert_eq!(k("tab"), Some(b"\t".to_vec()));
        assert_eq!(key_bytes("tab", false, false, true, false), Some(b"\x1b[Z".to_vec()));
        assert_eq!(k("escape"), Some(vec![0x1b]));
        assert_eq!(key_bytes("c", true, false, false, false), Some(vec![3]));
        assert_eq!(key_bytes("d", true, false, false, false), Some(vec![4]));
        assert_eq!(key_bytes("[", true, false, false, false), Some(vec![0x1b]));
        assert_eq!(key_bytes("space", true, false, false, false), Some(vec![0]));
    }

    #[test]
    fn arrows_follow_the_cursor_key_mode_and_modifiers() {
        assert_eq!(k("up"), Some(b"\x1b[A".to_vec()));
        assert_eq!(key_bytes("up", false, false, false, true), Some(b"\x1bOA".to_vec()));
        assert_eq!(key_bytes("left", true, false, false, false), Some(b"\x1b[1;5D".to_vec()));
        assert_eq!(key_bytes("right", false, false, true, true), Some(b"\x1b[1;2C".to_vec()));
        assert_eq!(k("home"), Some(b"\x1b[H".to_vec()));
    }

    #[test]
    fn editing_and_function_keys() {
        assert_eq!(k("delete"), Some(b"\x1b[3~".to_vec()));
        assert_eq!(k("pageup"), Some(b"\x1b[5~".to_vec()));
        assert_eq!(k("f1"), Some(b"\x1bOP".to_vec()));
        assert_eq!(k("f5"), Some(b"\x1b[15~".to_vec()));
        assert_eq!(k("f12"), Some(b"\x1b[24~".to_vec()));
        assert_eq!(key_bytes("delete", true, false, false, false), Some(b"\x1b[3;5~".to_vec()));
    }

    #[test]
    fn plain_text_is_left_to_typing_and_alt_prefixes_escape() {
        assert_eq!(k("a"), None);
        assert_eq!(key_bytes("b", false, true, false, false), Some(b"\x1bb".to_vec()));
        assert_eq!(key_bytes("enter", false, true, false, false), Some(b"\x1b\r".to_vec()));
    }

    #[test]
    fn pastes_are_bracketed_when_asked_and_newlines_become_enter() {
        assert_eq!(paste_bytes("a\nb", false), b"a\rb".to_vec());
        assert_eq!(paste_bytes("x\r\ny", true), b"\x1b[200~x\ry\x1b[201~".to_vec());
        assert_eq!(paste_bytes("evil\x1b[201~", true), b"\x1b[200~evil\x1b[201~".to_vec());
    }
}
