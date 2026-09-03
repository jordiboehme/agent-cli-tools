//! GNU coreutils `parse_duration` semantics: a strtod-style floating point
//! number with an optional single-letter suffix.

use std::time::Duration;

/// How long a timer runs. `Never` is the GNU meaning of a zero duration
/// ("a duration of 0 disables the associated timeout"); `After` is a span,
/// clamped to `Duration::MAX` for infinity and overflow.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Timeout {
    Never,
    After(Duration),
}

/// Parse a GNU duration such as `10`, `1.5s`, `2m`, `1h`, `0.5d` or `inf`.
/// Returns `None` for anything GNU reports as an invalid time interval.
pub fn parse_duration(text: &str) -> Option<Timeout> {
    let (value, consumed, nonzero_digits) = strtod(text)?;
    let multiplier = match &text[consumed..] {
        "" | "s" => 1.0,
        "m" => 60.0,
        "h" => 3600.0,
        "d" => 86400.0,
        _ => return None,
    };
    // GNU rejects with `!(0 <= s)`, which catches negatives and NaN alike.
    if value.is_nan() || value < 0.0 {
        return None;
    }
    let seconds = value * multiplier;
    if seconds == 0.0 {
        // strtod underflow on a nonzero mantissa is still a positive span to
        // GNU, and its timer rounds that up to one nanosecond.
        return Some(if nonzero_digits {
            Timeout::After(Duration::from_nanos(1))
        } else {
            Timeout::Never
        });
    }
    Some(Timeout::After(match Duration::try_from_secs_f64(seconds) {
        Ok(d) if d.is_zero() => Duration::from_nanos(1),
        Ok(d) => d,
        Err(_) => Duration::MAX,
    }))
}

/// A strtod work-alike: skips leading whitespace, accepts an optional sign,
/// decimal and hexadecimal floats, `inf`, `infinity` and `nan` in any case,
/// and stops at the first byte that cannot continue the number. Returns the
/// value, the number of bytes consumed and whether the mantissa contained a
/// nonzero digit. `None` when no number was found at all.
fn strtod(text: &str) -> Option<(f64, usize, bool)> {
    let bytes = text.as_bytes();
    let mut i = 0;
    while i < bytes.len() && matches!(bytes[i], b' ' | b'\t' | b'\n' | b'\x0b' | b'\x0c' | b'\r') {
        i += 1;
    }
    let mut sign = 1.0;
    if i < bytes.len() && (bytes[i] == b'+' || bytes[i] == b'-') {
        if bytes[i] == b'-' {
            sign = -1.0;
        }
        i += 1;
    }
    let lower = text[i..].to_ascii_lowercase();
    if lower.starts_with("infinity") {
        return Some((sign * f64::INFINITY, i + 8, true));
    }
    if lower.starts_with("inf") {
        return Some((sign * f64::INFINITY, i + 3, true));
    }
    if lower.starts_with("nan") {
        return Some((f64::NAN, i + 3, true));
    }
    let hex = if lower.starts_with("0x") {
        parse_hex(bytes, i + 2)
    } else {
        None
    };
    hex.or_else(|| parse_decimal(bytes, i))
        .map(|(value, end, nonzero)| (sign * value, end, nonzero))
}

fn parse_decimal(bytes: &[u8], start: usize) -> Option<(f64, usize, bool)> {
    let mut i = start;
    let mut number = String::new();
    let mut digits = 0;
    let mut nonzero = false;
    while i < bytes.len() && bytes[i].is_ascii_digit() {
        nonzero |= bytes[i] != b'0';
        number.push(bytes[i] as char);
        digits += 1;
        i += 1;
    }
    if i < bytes.len() && bytes[i] == b'.' {
        let mut j = i + 1;
        let mut fraction = String::from(".");
        while j < bytes.len() && bytes[j].is_ascii_digit() {
            nonzero |= bytes[j] != b'0';
            fraction.push(bytes[j] as char);
            digits += 1;
            j += 1;
        }
        if digits > 0 {
            number.push_str(&fraction);
            i = j;
        }
    }
    if digits == 0 {
        return None;
    }
    if i < bytes.len() && (bytes[i] == b'e' || bytes[i] == b'E') {
        let mut j = i + 1;
        let mut exponent = String::from("e");
        if j < bytes.len() && (bytes[j] == b'+' || bytes[j] == b'-') {
            exponent.push(bytes[j] as char);
            j += 1;
        }
        let first_digit = j;
        while j < bytes.len() && bytes[j].is_ascii_digit() {
            exponent.push(bytes[j] as char);
            j += 1;
        }
        if j > first_digit {
            number.push_str(&exponent);
            i = j;
        }
    }
    let value: f64 = number.parse().ok()?;
    Some((value, i, nonzero))
}

/// `start` points just past the `0x` prefix. Returns `None` when no hex
/// digit follows, so the caller falls back to parsing the leading `0`.
fn parse_hex(bytes: &[u8], start: usize) -> Option<(f64, usize, bool)> {
    let mut i = start;
    let mut value = 0.0f64;
    let mut scale: i32 = 0;
    let mut digits = 0;
    let mut nonzero = false;
    while i < bytes.len() && bytes[i].is_ascii_hexdigit() {
        let d = hex_digit(bytes[i]);
        nonzero |= d != 0.0;
        value = value * 16.0 + d;
        digits += 1;
        i += 1;
    }
    if i < bytes.len() && bytes[i] == b'.' {
        let mut j = i + 1;
        let mut fraction_digits = 0;
        while j < bytes.len() && bytes[j].is_ascii_hexdigit() {
            let d = hex_digit(bytes[j]);
            nonzero |= d != 0.0;
            value = value * 16.0 + d;
            scale -= 4;
            fraction_digits += 1;
            j += 1;
        }
        if digits + fraction_digits > 0 {
            digits += fraction_digits;
            i = j;
        }
    }
    if digits == 0 {
        return None;
    }
    if i < bytes.len() && (bytes[i] == b'p' || bytes[i] == b'P') {
        let mut j = i + 1;
        let mut negative = false;
        if j < bytes.len() && (bytes[j] == b'+' || bytes[j] == b'-') {
            negative = bytes[j] == b'-';
            j += 1;
        }
        let first_digit = j;
        let mut exponent: i32 = 0;
        while j < bytes.len() && bytes[j].is_ascii_digit() {
            exponent = exponent
                .saturating_mul(10)
                .saturating_add(i32::from(bytes[j] - b'0'));
            j += 1;
        }
        if j > first_digit {
            scale = scale.saturating_add(if negative { -exponent } else { exponent });
            i = j;
        }
    }
    Some((value * 2f64.powi(scale), i, nonzero))
}

fn hex_digit(byte: u8) -> f64 {
    f64::from(
        (byte as char)
            .to_digit(16)
            .expect("caller checked is_ascii_hexdigit"),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    fn secs(text: &str) -> f64 {
        match parse_duration(text) {
            Some(Timeout::After(d)) => d.as_secs_f64(),
            other => panic!("{text:?} parsed as {other:?}"),
        }
    }

    #[test]
    fn plain_seconds_and_fractions() {
        assert_eq!(secs("10"), 10.0);
        assert_eq!(secs("1.5"), 1.5);
        assert_eq!(secs(".1"), 0.1);
        assert_eq!(secs("5."), 5.0);
        assert_eq!(secs("1e3"), 1000.0);
        assert_eq!(secs("1e-3"), 0.001);
        assert_eq!(secs("  7"), 7.0);
        assert_eq!(secs("+2"), 2.0);
    }

    #[test]
    fn suffixes_multiply() {
        assert_eq!(secs("2s"), 2.0);
        assert_eq!(secs("1m"), 60.0);
        assert_eq!(secs("0.5h"), 1800.0);
        assert_eq!(secs("1d"), 86400.0);
        assert_eq!(secs("1e3s"), 1000.0);
    }

    #[test]
    fn hexadecimal_like_strtod() {
        assert_eq!(secs("0x10"), 16.0);
        assert_eq!(secs("0x1d"), 29.0);
        assert_eq!(secs("0x1p0d"), 86400.0);
        assert_eq!(secs("0x.8"), 0.5);
    }

    #[test]
    fn zero_disables() {
        assert_eq!(parse_duration("0"), Some(Timeout::Never));
        assert_eq!(parse_duration("-0"), Some(Timeout::Never));
        assert_eq!(parse_duration("0s"), Some(Timeout::Never));
        assert_eq!(parse_duration("0e5"), Some(Timeout::Never));
    }

    #[test]
    fn infinity_never_fires_but_is_valid() {
        assert_eq!(parse_duration("inf"), Some(Timeout::After(Duration::MAX)));
        assert_eq!(
            parse_duration("INFINITY"),
            Some(Timeout::After(Duration::MAX))
        );
        assert_eq!(parse_duration("1e400"), Some(Timeout::After(Duration::MAX)));
    }

    #[test]
    fn underflow_rounds_up_to_a_nanosecond() {
        assert_eq!(
            parse_duration("1e-10000"),
            Some(Timeout::After(Duration::from_nanos(1)))
        );
        assert_eq!(
            parse_duration("1e-12"),
            Some(Timeout::After(Duration::from_nanos(1)))
        );
    }

    #[test]
    fn rejects_what_gnu_rejects() {
        for bad in [
            "", "abc", "-1", " -0.1", "nan", "NaN", "1ss", "5 s", "42D", "1x", ".", "e5", "+",
            "0x", "1e", "-", "1-",
        ] {
            assert_eq!(parse_duration(bad), None, "{bad:?} should be invalid");
        }
    }
}
