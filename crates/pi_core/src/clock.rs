//! RFC 3339 timestamps from pi's session list, without a date-time dependency.

/// Milliseconds since the Unix epoch, for timestamps like pi's `modified`
/// (`2026-09-28T09:41:00.000Z`). Numeric offsets are accepted; other forms are not.
pub fn parse_timestamp(text: &str) -> Option<u64> {
    let bytes = text.as_bytes();
    if bytes.len() < 20 || bytes[4] != b'-' || bytes[7] != b'-' || !matches!(bytes[10], b'T' | b't')
    {
        return None;
    }
    let number = |range: std::ops::Range<usize>| -> Option<i64> {
        let digits = text.get(range)?;
        digits
            .bytes()
            .all(|byte| byte.is_ascii_digit())
            .then(|| digits.parse().ok())?
    };
    let (year, month, day) = (number(0..4)?, number(5..7)?, number(8..10)?);
    if bytes[13] != b':' || bytes[16] != b':' {
        return None;
    }
    let (hour, minute, second) = (number(11..13)?, number(14..16)?, number(17..19)?);
    if !(1..=12).contains(&month) || !(1..=31).contains(&day) || hour > 23 || minute > 59 {
        return None;
    }
    // Leap seconds are rare enough that pi never writes them; reject rather than guess.
    if second > 59 {
        return None;
    }
    let mut rest = &text[19..];
    let mut millis = 0;
    if let Some(fraction) = rest.strip_prefix('.') {
        let digits = fraction.bytes().take_while(u8::is_ascii_digit).count();
        if digits == 0 {
            return None;
        }
        // Keep millisecond precision: ".5" is 500 ms, ".123456" is 123 ms.
        let padded = format!("{:0<3}", &fraction[..digits.min(3)]);
        millis = padded.parse::<i64>().ok()?;
        rest = &fraction[digits..];
    }
    let offset_minutes = match rest {
        "Z" | "z" => 0,
        _ => {
            let sign = match rest.as_bytes().first()? {
                b'+' => 1,
                b'-' => -1,
                _ => return None,
            };
            let offset = rest.get(1..)?;
            if offset.len() != 5 || offset.as_bytes()[2] != b':' {
                return None;
            }
            let hours: i64 = offset[..2].parse().ok()?;
            let minutes: i64 = offset[3..].parse().ok()?;
            sign * (hours * 60 + minutes)
        }
    };
    let days = days_from_civil(year, month, day);
    let seconds = days * 86_400 + hour * 3600 + minute * 60 + second - offset_minutes * 60;
    u64::try_from(seconds * 1000 + millis).ok()
}

/// Howard Hinnant's `days_from_civil`: days since 1970-01-01 in the proleptic Gregorian calendar.
fn days_from_civil(year: i64, month: i64, day: i64) -> i64 {
    let year = if month <= 2 { year - 1 } else { year };
    let era = year.div_euclid(400);
    let year_of_era = year - era * 400;
    let day_of_year = (153 * ((month + 9) % 12) + 2) / 5 + day - 1;
    let day_of_era = year_of_era * 365 + year_of_era / 4 - year_of_era / 100 + day_of_year;
    era * 146_097 + day_of_era - 719_468
}

/// A compact age for sidebar rows: `now`, `12m`, `5h`, `3d`, `2w`, `4mo`, `1y`.
pub fn short_age(then_ms: u64, now_ms: u64) -> String {
    let minutes = now_ms.saturating_sub(then_ms) / 60_000;
    let (hours, days) = (minutes / 60, minutes / 1440);
    match () {
        _ if minutes < 1 => "now".into(),
        _ if hours < 1 => format!("{minutes}m"),
        _ if days < 1 => format!("{hours}h"),
        _ if days < 14 => format!("{days}d"),
        _ if days < 60 => format!("{}w", days / 7),
        _ if days < 365 => format!("{}mo", days / 30),
        _ => format!("{}y", days / 365),
    }
}

const MONTHS: [&str; 12] = [
    "Jan", "Feb", "Mar", "Apr", "May", "Jun", "Jul", "Aug", "Sep", "Oct", "Nov", "Dec",
];
const WEEKDAYS: [&str; 7] = ["Sun", "Mon", "Tue", "Wed", "Thu", "Fri", "Sat"];

/// Year, month (1-12) and day for days since 1970-01-01 (Howard Hinnant's algorithm).
fn civil(days: i64) -> (i64, u32, u32) {
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let day_of_era = z.rem_euclid(146_097);
    let year_of_era =
        (day_of_era - day_of_era / 1460 + day_of_era / 36_524 - day_of_era / 146_096) / 365;
    let day_of_year = day_of_era - (365 * year_of_era + year_of_era / 4 - year_of_era / 100);
    let mp = (5 * day_of_year + 2) / 153;
    let day = (day_of_year - (153 * mp + 2) / 5 + 1) as u32;
    let month = if mp < 10 { mp + 3 } else { mp - 9 } as u32;
    (year_of_era + era * 400 + i64::from(month <= 2), month, day)
}

/// `Sep 21 · 20:14`, in UTC like the rest of the app's times.
pub fn date_time(ms: u64) -> String {
    let (days, minutes) = ((ms / 86_400_000) as i64, ms / 60_000 % 1440);
    let (_, month, day) = civil(days);
    format!(
        "{} {day} · {:02}:{:02}",
        MONTHS[month as usize - 1],
        minutes / 60,
        minutes % 60
    )
}

/// When, as a list of sessions says it: `12 min ago`, `2 h ago`, `yesterday`, a
/// weekday within the week, then `Sep 26`, with the year once it is not this one.
pub fn when(then_ms: u64, now_ms: u64) -> String {
    let minutes = now_ms.saturating_sub(then_ms) / 60_000;
    let (then_day, today) = ((then_ms / 86_400_000) as i64, (now_ms / 86_400_000) as i64);
    match today - then_day {
        _ if minutes < 1 => "now".into(),
        0 if minutes < 60 => format!("{minutes} min ago"),
        0 => format!("{} h ago", minutes / 60),
        1 => "yesterday".into(),
        2..=6 => WEEKDAYS[(then_day + 4).rem_euclid(7) as usize].into(),
        _ => {
            let (year, month, day) = civil(then_day);
            let (this_year, _, _) = civil(today);
            if year == this_year {
                format!("{} {day}", MONTHS[month as usize - 1])
            } else {
                format!("{} {day}, {year}", MONTHS[month as usize - 1])
            }
        }
    }
}
