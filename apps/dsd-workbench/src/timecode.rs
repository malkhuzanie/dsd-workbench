//! Formatting and parsing of editable time stamps.

/// `m:ss.mmm`, or `h:mm:ss.mmm` from one hour up.
pub fn format(seconds: f64) -> String {
    let total_ms = (seconds.max(0.0) * 1000.0).round() as u64;
    let (ms, total_s) = (total_ms % 1000, total_ms / 1000);
    let (s, total_m) = (total_s % 60, total_s / 60);
    let (m, h) = (total_m % 60, total_m / 60);
    if h > 0 {
        format!("{h}:{m:02}:{s:02}.{ms:03}")
    } else {
        format!("{m}:{s:02}.{ms:03}")
    }
}

/// Accepts `90`, `90.5`, `1:30`, `1:30.25` and `1:02:03.5`.
pub fn parse(text: &str) -> Option<f64> {
    let parts: Vec<&str> = text.trim().split(':').collect();
    if parts.len() > 3 {
        return None;
    }
    let (last, head) = parts.split_last()?;
    let seconds = parse_seconds(last)?;

    let mut units = Vec::with_capacity(head.len());
    for part in head {
        if part.is_empty() || !part.bytes().all(|b| b.is_ascii_digit()) {
            return None;
        }
        units.push(part.parse::<u64>().ok()?);
    }
    if !units.is_empty() && seconds >= 60.0 {
        return None;
    }
    if units.len() == 2 && units[1] >= 60 {
        return None;
    }

    let minutes = units
        .iter()
        .try_fold(0u64, |acc, v| acc.checked_mul(60)?.checked_add(*v))?;
    Some(minutes as f64 * 60.0 + seconds)
}

fn parse_seconds(text: &str) -> Option<f64> {
    let valid = !text.is_empty()
        && text.bytes().all(|b| b.is_ascii_digit() || b == b'.')
        && text.bytes().filter(|b| *b == b'.').count() <= 1;
    if valid { text.parse().ok() } else { None }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn formats() {
        assert_eq!(format(0.0), "0:00.000");
        assert_eq!(format(90.5), "1:30.500");
        assert_eq!(format(3723.25), "1:02:03.250");
    }

    #[test]
    fn parses() {
        assert_eq!(parse("90"), Some(90.0));
        assert_eq!(parse(" 1:30.5 "), Some(90.5));
        assert_eq!(parse("1:02:03"), Some(3723.0));
    }

    #[test]
    fn rejects_garbage() {
        for bad in [
            "", "abc", "-1", "1:75", "1:60:00", "1::2", "1:2:3:4", "1e3", ".",
        ] {
            assert_eq!(parse(bad), None, "{bad:?}");
        }
    }
}
