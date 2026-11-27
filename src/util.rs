//! Small formatting and parsing helpers shared by commands.

/// Formats seconds as `m:ss`. Zero means the length is unknown (live streams, missing data).
pub fn format_duration(seconds: u64) -> String {
    if seconds == 0 {
        return "Live / Unknown".into();
    }
    format!("{}:{:02}", seconds / 60, seconds % 60)
}

pub fn format_uptime(seconds: u64) -> String {
    format!("{}h {}m {}s", seconds / 3600, (seconds % 3600) / 60, seconds % 60)
}

/// Parses a seek target: `90`, `1:30` or `1:02:03`. Returns `None` for anything else.
pub fn parse_time(input: &str) -> Option<u64> {
    let input = input.trim();
    if input.is_empty() {
        return None;
    }
    let parts: Vec<&str> = input.split(':').collect();
    if parts.len() > 3 {
        return None;
    }
    let mut total: u64 = 0;
    for (i, part) in parts.iter().enumerate() {
        if part.is_empty() || !part.bytes().all(|b| b.is_ascii_digit()) {
            return None;
        }
        let value: u64 = part.parse().ok()?;
        // Minutes and seconds after the first field must be below 60.
        if i > 0 && value >= 60 {
            return None;
        }
        total = total.checked_mul(60)?.checked_add(value)?;
    }
    Some(total)
}

/// Truncates to at most `max` characters without splitting a UTF-8 character.
pub fn truncate_chars(s: &str, max: usize) -> &str {
    match s.char_indices().nth(max) {
        Some((idx, _)) => &s[..idx],
        None => s,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn duration_formatting() {
        assert_eq!(format_duration(0), "Live / Unknown");
        assert_eq!(format_duration(5), "0:05");
        assert_eq!(format_duration(213), "3:33");
        assert_eq!(format_duration(3725), "62:05");
    }

    #[test]
    fn uptime_formatting() {
        assert_eq!(format_uptime(3725), "1h 2m 5s");
    }

    #[test]
    fn parse_time_accepts_seconds_and_clock_forms() {
        assert_eq!(parse_time("90"), Some(90));
        assert_eq!(parse_time("0"), Some(0));
        assert_eq!(parse_time("1:45"), Some(105));
        assert_eq!(parse_time("01:02:03"), Some(3723));
        assert_eq!(parse_time(" 2:00 "), Some(120));
    }

    #[test]
    fn parse_time_rejects_bad_input() {
        for bad in ["", "abc", "-5", "1:", ":30", "1:60", "1:2:3:4", "1.5", "1:xx"] {
            assert_eq!(parse_time(bad), None, "{bad:?}");
        }
    }

    #[test]
    fn truncate_is_char_safe() {
        assert_eq!(truncate_chars("héllo", 2), "hé");
        assert_eq!(truncate_chars("abc", 10), "abc");
    }
}
