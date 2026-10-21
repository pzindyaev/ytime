use std::time::Duration;

use crate::config::RoundingMode;

/// Yandex Tracker counts a worklog "day" as one workday (8 hours by default,
/// like Jira): a `P1D` entry shows as 8h in Tracker's reports, not 24h.
/// A "week" is five workdays. All day/week conversions in the app use this.
pub const WORK_HOURS_PER_DAY: f64 = 8.0;

/// Rounds `elapsed` to a multiple of the configured unit.
///
/// With `Up` any non-zero interval becomes at least one unit — by default
/// (30m) a 1-minute run logs as 30m. `Nearest`/`Down` may round to zero,
/// in which case the caller treats it as "nothing to log".
pub fn round_duration(elapsed: Duration, unit_minutes: u32, mode: RoundingMode) -> Duration {
    let unit = u64::from(unit_minutes) * 60;
    if unit == 0 {
        return elapsed;
    }
    let secs = elapsed.as_secs();
    let units = match mode {
        RoundingMode::Up => secs.div_ceil(unit),
        // exact halves round up
        RoundingMode::Nearest => (secs + unit / 2) / unit,
        RoundingMode::Down => secs / unit,
    };
    Duration::from_secs(units * unit)
}

/// `Duration` → ISO-8601 in pure time form (`PT1H30M`, `PT26H`).
/// Days are deliberately never emitted: Tracker reads `P1D` as one 8-hour
/// workday, so an unambiguous hours/minutes form avoids surprises.
pub fn format_iso_duration(d: Duration) -> String {
    let total = d.as_secs();
    let h = total / 3600;
    let m = (total % 3600) / 60;
    let s = total % 60;
    let mut out = String::from("PT");
    if h > 0 {
        out.push_str(&format!("{h}H"));
    }
    if m > 0 {
        out.push_str(&format!("{m}M"));
    }
    if s > 0 {
        out.push_str(&format!("{s}S"));
    }
    if h == 0 && m == 0 && s == 0 {
        out.push_str("0M");
    }
    out
}

/// Parses ISO-8601 durations (`PT1H30M`, `P1DT2H`, `P3W`, fractional values).
/// Calendar units convert to work time: 1d = 8h, 1w = 5d.
pub fn parse_iso_duration(s: &str) -> Option<Duration> {
    let s = s.trim();
    let mut chars = s.chars().peekable();
    if chars.next()? != 'P' {
        return None;
    }
    let mut in_time = false;
    let mut seconds: f64 = 0.0;
    let mut num = String::new();
    for c in chars.by_ref() {
        if c.is_ascii_digit() || c == '.' || c == ',' {
            num.push(if c == ',' { '.' } else { c });
            continue;
        }
        if c == 'T' || c == 't' {
            if !num.is_empty() {
                return None;
            }
            in_time = true;
            continue;
        }
        let value: f64 = num.parse().ok()?;
        num.clear();
        let workday = WORK_HOURS_PER_DAY * 3600.0;
        let mult = match (c.to_ascii_uppercase(), in_time) {
            ('Y', false) => 365.25 * workday,
            ('M', false) => 30.0 * workday,
            ('W', false) => 5.0 * workday,
            ('D', false) => workday,
            ('H', true) => 3600.0,
            ('M', true) => 60.0,
            ('S', true) => 1.0,
            _ => return None,
        };
        seconds += value * mult;
    }
    if !num.is_empty() {
        return None; // trailing number without a unit
    }
    Some(Duration::from_secs_f64(seconds))
}

/// Parses user-typed durations: `1h30m`, `45m`, `1.5h`, `1d2h`, `1:30` (h:mm).
/// A bare number means hours.
pub fn parse_duration_input(s: &str) -> Option<Duration> {
    let s = s.trim().to_lowercase();
    if s.is_empty() {
        return None;
    }
    if let Some(rest) = s.strip_prefix('t') {
        // tolerate "t1h30m" — users reach for the ISO prefix occasionally
        return parse_duration_input(rest);
    }
    if s.contains(':') {
        let parts: Vec<&str> = s.split(':').collect();
        if parts.len() == 2 || parts.len() == 3 {
            let h: f64 = parts[0].trim().parse().ok()?;
            let m: f64 = parts[1].trim().parse().ok()?;
            let sec: f64 = if parts.len() == 3 {
                parts[2].trim().parse().ok()?
            } else {
                0.0
            };
            if m >= 60.0 || sec >= 60.0 {
                return None;
            }
            return Some(Duration::from_secs_f64(h * 3600.0 + m * 60.0 + sec));
        }
        return None;
    }
    let mut seconds: f64 = 0.0;
    let mut num = String::new();
    let mut saw_any = false;
    let mut saw_unit = false;
    for c in s.chars() {
        if c.is_ascii_digit() || c == '.' || c == ' ' {
            if c != ' ' {
                num.push(c);
            } else if !num.is_empty() {
                num.push(c); // keep inner spaces, they separate tokens
            }
            continue;
        }
        let token: String = num.trim().chars().filter(|c| *c != ' ').collect();
        num.clear();
        let value: f64 = if token.is_empty() {
            return None;
        } else {
            token.parse().ok()?
        };
        saw_any = true;
        saw_unit = true;
        let mult = match c {
            // "1d" typed by the user means one workday, as in Tracker
            'd' => WORK_HOURS_PER_DAY * 3600.0,
            'h' => 3600.0,
            'm' => 60.0,
            's' => 1.0,
            _ => return None,
        };
        seconds += value * mult;
    }
    if !num.trim().is_empty() {
        // a lone number is hours; after a unit ("1h30") it means minutes
        let value: f64 = num.trim().parse().ok()?;
        seconds += value * if saw_unit { 60.0 } else { 3600.0 };
        saw_any = true;
    }
    if !saw_any {
        return None;
    }
    Some(Duration::from_secs_f64(seconds))
}

/// Human duration: `1d 2h 05m`, where a day is one workday (as Tracker
/// displays it) — used in calendar/review.
pub fn format_human(d: Duration) -> String {
    let total = d.as_secs();
    let day = (WORK_HOURS_PER_DAY * 3600.0) as u64;
    let days = total / day;
    let h = (total % day) / 3600;
    let m = (total % 3600) / 60;
    let mut parts = Vec::new();
    if days > 0 {
        parts.push(format!("{days}d"));
    }
    if h > 0 {
        parts.push(format!("{h}h"));
    }
    if m > 0 || parts.is_empty() {
        parts.push(format!("{m:02}m"));
    }
    parts.join(" ")
}

/// Elapsed wall-clock time: `1:23:45` (hours unbounded).
pub fn format_hms(d: Duration) -> String {
    let total = d.as_secs();
    let h = total / 3600;
    let m = (total % 3600) / 60;
    let s = total % 60;
    format!("{h}:{m:02}:{s:02}")
}

/// The maximum sensible single worklog entry (31 days).
pub const MAX_ENTRY: Duration = Duration::from_secs(31 * 86_400);

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::RoundingMode::*;

    fn d(secs: u64) -> Duration {
        Duration::from_secs(secs)
    }

    #[test]
    fn rounding_up() {
        // the user's rule: >= 1 minute of tracking logs a full unit
        assert_eq!(round_duration(d(60), 30, Up), d(1800));
        assert_eq!(round_duration(d(1800), 30, Up), d(1800));
        assert_eq!(round_duration(d(1801), 30, Up), d(3600));
        assert_eq!(round_duration(d(1), 30, Up), d(1800));
        assert_eq!(round_duration(d(0), 30, Up), d(0));
    }

    #[test]
    fn rounding_nearest_and_down() {
        assert_eq!(round_duration(d(12 * 60), 5, Nearest), d(10 * 60));
        assert_eq!(round_duration(d(13 * 60), 5, Nearest), d(15 * 60));
        assert_eq!(round_duration(d(30), 5, Nearest), d(0));
        assert_eq!(round_duration(d(29 * 60), 30, Down), d(0));
        assert_eq!(round_duration(d(45 * 60), 30, Down), d(30 * 60));
    }

    #[test]
    fn iso_roundtrip() {
        for secs in [60u64, 1800, 5400, 86_400 + 7200, 36 * 3600, 45] {
            let iso = format_iso_duration(d(secs));
            assert_eq!(parse_iso_duration(&iso), Some(d(secs)), "case {iso}");
        }
        assert_eq!(format_iso_duration(d(5400)), "PT1H30M");
        // pure time form — days are never emitted
        assert_eq!(format_iso_duration(d(90000)), "PT25H");
        // regression: a Tracker "day" is one 8h workday, not 24h
        assert_eq!(parse_iso_duration("P1D"), Some(d(8 * 3600)));
        assert_eq!(parse_iso_duration("P1DT2H"), Some(d(10 * 3600)));
        assert_eq!(parse_iso_duration("P3W"), Some(d(3 * 5 * 8 * 3600)));
        assert_eq!(parse_iso_duration("PT0.5H"), Some(d(1800)));
        assert_eq!(parse_iso_duration("garbage"), None);
    }

    #[test]
    fn user_durations() {
        assert_eq!(parse_duration_input("1h30m"), Some(d(5400)));
        assert_eq!(parse_duration_input("45m"), Some(d(2700)));
        assert_eq!(parse_duration_input("1.5h"), Some(d(5400)));
        assert_eq!(parse_duration_input("2"), Some(d(7200)));
        // "1d" typed by the user is one workday
        assert_eq!(parse_duration_input("1d"), Some(d(8 * 3600)));
        assert_eq!(parse_duration_input("1d 2h"), Some(d(10 * 3600)));
        assert_eq!(parse_duration_input("1d2h"), Some(d(10 * 3600)));
        assert_eq!(parse_duration_input("1:30"), Some(d(5400)));
        assert_eq!(parse_duration_input("0:45"), Some(d(2700)));
        assert_eq!(parse_duration_input("1h 30m"), Some(d(5400)));
        assert_eq!(parse_duration_input("1h30"), Some(d(5400))); // "1h30" idiom
        assert_eq!(parse_duration_input("2 30"), None);
        assert_eq!(parse_duration_input(""), None);
        assert_eq!(parse_duration_input("abc"), None);
        assert_eq!(parse_duration_input("1x"), None);
    }

    #[test]
    fn human_formats() {
        assert_eq!(format_human(d(5400)), "1h 30m");
        // days in the human form are workdays, matching Tracker
        assert_eq!(format_human(d(8 * 3600)), "1d");
        assert_eq!(format_human(d(93600)), "3d 2h"); // 26h
        assert_eq!(format_human(d(0)), "00m");
        assert_eq!(format_hms(d(5025)), "1:23:45");
    }
}
