//! Android key codes as GPUI key names.
//!
//! Text from the on-screen keyboard arrives through the input connection, not
//! as keys; keys come from hardware keyboards and from the few editing keys an
//! on-screen keyboard sends (delete, enter, arrows).

const LETTERS: [&str; 26] = [
    "a", "b", "c", "d", "e", "f", "g", "h", "i", "j", "k", "l", "m", "n", "o", "p", "q", "r", "s",
    "t", "u", "v", "w", "x", "y", "z",
];
const DIGITS: [&str; 10] = ["0", "1", "2", "3", "4", "5", "6", "7", "8", "9"];
const FUNCTION_KEYS: [&str; 12] = [
    "f1", "f2", "f3", "f4", "f5", "f6", "f7", "f8", "f9", "f10", "f11", "f12",
];

/// GPUI's name for a key (`android.view.KeyEvent.KEYCODE_*`), or `None` for
/// keys the app should not see, such as volume, media and modifier keys.
pub(crate) fn key_name(keycode: u32) -> Option<&'static str> {
    Some(match keycode {
        7..=16 => DIGITS[(keycode - 7) as usize],
        19 => "up",
        20 => "down",
        21 => "left",
        22 => "right",
        29..=54 => LETTERS[(keycode - 29) as usize],
        55 => ",",
        56 => ".",
        61 => "tab",
        62 => "space",
        66 | 160 => "enter",
        67 => "backspace",
        68 => "`",
        69 => "-",
        70 => "=",
        71 => "[",
        72 => "]",
        73 => "\\",
        74 => ";",
        75 => "'",
        76 => "/",
        77 => "@",
        81 => "+",
        92 => "pageup",
        93 => "pagedown",
        111 => "escape",
        112 => "delete",
        122 => "home",
        123 => "end",
        124 => "insert",
        131..=142 => FUNCTION_KEYS[(keycode - 131) as usize],
        _ => return None,
    })
}

/// Whether the key types its character when no action handles it. Named keys
/// such as enter and tab only run actions, as on other platforms.
pub(crate) fn types_text(name: &str) -> bool {
    name == "space" || name.chars().count() == 1
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn editing_keys_and_characters_have_gpui_names() {
        assert_eq!(key_name(67), Some("backspace"));
        assert_eq!(key_name(112), Some("delete"));
        assert_eq!(key_name(66), Some("enter"));
        assert_eq!(key_name(160), Some("enter"), "keypad enter");
        assert_eq!(key_name(29), Some("a"));
        assert_eq!(key_name(54), Some("z"));
        assert_eq!(key_name(7), Some("0"));
        assert_eq!(key_name(142), Some("f12"));
        assert_eq!(key_name(21), Some("left"));
    }

    #[test]
    fn system_keys_stay_with_android() {
        // Volume up and down, power, media play/pause, back, left shift.
        for keycode in [24, 25, 26, 85, 4, 59] {
            assert_eq!(key_name(keycode), None, "keycode {keycode}");
        }
    }

    #[test]
    fn only_character_keys_type() {
        assert!(types_text("a") && types_text(",") && types_text("space"));
        assert!(!types_text("enter") && !types_text("backspace") && !types_text("f1"));
    }
}
