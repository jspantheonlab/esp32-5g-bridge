//! UCS2 hex encoding as used by `AT+CSCS="UCS2"` (UTF-16BE code units, 4 hex digits each).

use std::fmt::Write;

/// Encodes text as UCS2 hex. Returns the hex string and the number of UTF-16 code units
/// (which is what the 70-unit single-SMS limit counts).
pub fn utf8_to_ucs2_hex(text: &str) -> (String, usize) {
    let mut hex = String::with_capacity(text.len() * 4);
    let mut units = 0;
    for unit in text.encode_utf16() {
        let _ = write!(hex, "{unit:04X}");
        units += 1;
    }
    (hex, units)
}

/// Decodes UCS2 hex. Returns `None` if the input isn't hex UCS2 (e.g. the modem sent plain text).
pub fn ucs2_hex_to_utf8(hex: &str) -> Option<String> {
    if hex.is_empty() || hex.len() % 4 != 0 || !hex.bytes().all(|b| b.is_ascii_hexdigit()) {
        return None;
    }
    let units: Vec<u16> =
        (0..hex.len()).step_by(4).map(|i| u16::from_str_radix(&hex[i..i + 4], 16).ok()).collect::<Option<_>>()?;
    Some(String::from_utf16_lossy(&units))
}

/// True if the text only uses characters that map 1:1 into the GSM 7-bit default alphabet
/// via `AT+CSCS="IRA"`.
pub fn is_gsm_safe(text: &str) -> bool {
    const EXTRA: &str = " !\"#$%&'()*+,-./:;<=>?@_\r\n";
    text.chars().all(|c| c.is_ascii_alphanumeric() || EXTRA.contains(c))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn round_trips_emoji_and_accents() {
        let (hex, units) = utf8_to_ucs2_hex("Hi 👋 café");
        assert_eq!(hex, "004800690020D83DDC4B002000630061006600E9");
        assert_eq!(units, 10);
        assert_eq!(ucs2_hex_to_utf8(&hex).as_deref(), Some("Hi 👋 café"));
    }

    #[test]
    fn decodes_numbers() {
        assert_eq!(ucs2_hex_to_utf8("002B0031").as_deref(), Some("+1"));
    }

    #[test]
    fn rejects_non_hex() {
        assert_eq!(ucs2_hex_to_utf8("24/10/25,12:00:00+32"), None);
        assert_eq!(ucs2_hex_to_utf8("+123"), None);
        assert_eq!(ucs2_hex_to_utf8("ABC"), None);
        assert_eq!(ucs2_hex_to_utf8(""), None);
    }

    #[test]
    fn gsm_safety() {
        assert!(is_gsm_safe("Hello, world! @home_1"));
        assert!(!is_gsm_safe("café"));
        assert!(!is_gsm_safe("a[b]"));
        assert!(!is_gsm_safe("~"));
    }
}
