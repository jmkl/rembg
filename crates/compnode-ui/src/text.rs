//! Headless text helpers. Real glyph metrics belong to the host; these are
//! deliberately simple estimates so layout and tests do not need a font.

/// Average advance as a fraction of the font size.
pub const AVG_ADVANCE: f32 = 0.55;

pub fn text_width(s: &str, size: f32) -> f32 {
    s.chars().count() as f32 * size * AVG_ADVANCE
}

/// Shorten `s` with an ellipsis so it fits in `max_w` (estimated).
pub fn ellipsize(s: &str, max_w: f32, size: f32) -> String {
    if text_width(s, size) <= max_w {
        return s.to_string();
    }
    let fit = (max_w / (size * AVG_ADVANCE)).floor() as i64 - 1;
    if fit <= 0 {
        return String::new();
    }
    let mut out: String = s.chars().take(fit as usize).collect();
    out.push('\u{2026}');
    out
}

/// Up to three decimals, trailing zeros trimmed (`2.5`, `3`, `0.125`).
pub fn format_float(v: f32) -> String {
    let s = format!("{v:.3}");
    let s = s.trim_end_matches('0').trim_end_matches('.');
    if s.is_empty() || s == "-" || s == "-0" {
        "0".to_string()
    } else {
        s.to_string()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ellipsize_fits_or_truncates() {
        assert_eq!(ellipsize("abc", 100.0, 10.0), "abc");
        let e = ellipsize("abcdefghijklmnop", 40.0, 10.0);
        assert!(e.ends_with('\u{2026}'));
        assert!(text_width(&e, 10.0) <= 40.0 + 5.5);
        assert_eq!(ellipsize("abcdef", 3.0, 10.0), "");
    }

    #[test]
    fn float_formatting() {
        assert_eq!(format_float(2.5), "2.5");
        assert_eq!(format_float(3.0), "3");
        assert_eq!(format_float(0.125), "0.125");
        assert_eq!(format_float(-0.0), "0");
        assert_eq!(format_float(-1.5), "-1.5");
    }
}
