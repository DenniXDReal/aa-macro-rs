//! Keys as hardware scan codes (set 1). Roblox reads scan codes, so `SendInput`
//! is always fed `KEYEVENTF_SCANCODE` events built from these.

use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct Key {
    pub scan: u16,
    /// Extended key (arrows, right ctrl, ...): sent with `KEYEVENTF_EXTENDEDKEY`.
    pub extended: bool,
}

impl Key {
    pub const fn new(scan: u16) -> Self {
        Self {
            scan,
            extended: false,
        }
    }
    pub const fn ext(scan: u16) -> Self {
        Self {
            scan,
            extended: true,
        }
    }

    /// Parse names like `q`, `5`, `space`, `enter`, `esc`, `f11`, `shift`, `up`.
    pub fn parse(name: &str) -> Option<Key> {
        let n = name.trim().to_ascii_lowercase();
        let k = match n.as_str() {
            "esc" | "escape" => Key::new(0x01),
            "1" => Key::new(0x02),
            "2" => Key::new(0x03),
            "3" => Key::new(0x04),
            "4" => Key::new(0x05),
            "5" => Key::new(0x06),
            "6" => Key::new(0x07),
            "7" => Key::new(0x08),
            "8" => Key::new(0x09),
            "9" => Key::new(0x0A),
            "0" => Key::new(0x0B),
            "-" | "minus" => Key::new(0x0C),
            "=" | "equals" => Key::new(0x0D),
            "backspace" | "bs" => Key::new(0x0E),
            "tab" => Key::new(0x0F),
            "q" => Key::new(0x10),
            "w" => Key::new(0x11),
            "e" => Key::new(0x12),
            "r" => Key::new(0x13),
            "t" => Key::new(0x14),
            "y" => Key::new(0x15),
            "u" => Key::new(0x16),
            "i" => Key::new(0x17),
            "o" => Key::new(0x18),
            "p" => Key::new(0x19),
            "enter" | "return" => Key::new(0x1C),
            "/" | "slash" => Key::new(0x35),
            "ctrl" | "lctrl" | "control" => Key::new(0x1D),
            "a" => Key::new(0x1E),
            "s" => Key::new(0x1F),
            "d" => Key::new(0x20),
            "f" => Key::new(0x21),
            "g" => Key::new(0x22),
            "h" => Key::new(0x23),
            "j" => Key::new(0x24),
            "k" => Key::new(0x25),
            "l" => Key::new(0x26),
            "shift" | "lshift" => Key::new(0x2A),
            "z" => Key::new(0x2C),
            "x" => Key::new(0x2D),
            "c" => Key::new(0x2E),
            "v" => Key::new(0x2F),
            "b" => Key::new(0x30),
            "n" => Key::new(0x31),
            "m" => Key::new(0x32),
            "rshift" => Key::new(0x36),
            "alt" | "lalt" => Key::new(0x38),
            "space" => Key::new(0x39),
            "f1" => Key::new(0x3B),
            "f2" => Key::new(0x3C),
            "f3" => Key::new(0x3D),
            "f4" => Key::new(0x3E),
            "f5" => Key::new(0x3F),
            "f6" => Key::new(0x40),
            "f7" => Key::new(0x41),
            "f8" => Key::new(0x42),
            "f9" => Key::new(0x43),
            "f10" => Key::new(0x44),
            "f11" => Key::new(0x57),
            "f12" => Key::new(0x58),
            "rctrl" => Key::ext(0x1D),
            "ralt" => Key::ext(0x38),
            "up" => Key::ext(0x48),
            "left" => Key::ext(0x4B),
            "right" => Key::ext(0x4D),
            "down" => Key::ext(0x50),
            _ => return None,
        };
        Some(k)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_common_keys() {
        assert_eq!(Key::parse("Q"), Some(Key::new(0x10)));
        assert_eq!(Key::parse("1"), Some(Key::new(0x02)));
        assert_eq!(Key::parse("space"), Some(Key::new(0x39)));
        assert_eq!(Key::parse("f11"), Some(Key::new(0x57)));
        assert_eq!(Key::parse("up"), Some(Key::ext(0x48)));
        assert_eq!(Key::parse("nope"), None);
    }

    #[test]
    fn all_letters_and_digits_parse() {
        for c in ('a'..='z').chain('0'..='9') {
            assert!(Key::parse(&c.to_string()).is_some(), "missing {c}");
        }
    }
}
