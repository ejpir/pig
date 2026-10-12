//! Numeric POSIX five-field cron. Croner does calendar matching; we deliberately
//! do timezone conversion ourselves (its gap snapping policy is not ours).
use anyhow::{Result, ensure};
use chrono::{DateTime, Duration, LocalResult, TimeZone, Timelike, Utc};
use chrono_tz::Tz;
use croner::Cron;
use std::str::FromStr;

pub struct Calendar {
    cron: Cron,
    timezone: Tz,
}
impl Calendar {
    pub fn parse(expression: &str, timezone: &str) -> Result<Self> {
        ensure!(
            expression.len() <= 256 && expression.split_whitespace().count() == 5,
            "Use numeric standard five-field cron (minute hour day month weekday)"
        );
        ensure!(
            expression
                .bytes()
                .all(|b| b.is_ascii_digit() || b"*,-/ \t".contains(&b)),
            "Cron accepts only numeric fields, *, ranges, lists and steps"
        );
        Ok(Self {
            cron: Cron::from_str(expression)?,
            timezone: timezone.parse::<Tz>()?,
        })
    }
    pub fn next(&self, after_ms: u64) -> Result<u64> {
        let after = DateTime::<Utc>::from_timestamp_millis(i64::try_from(after_ms)?)
            .ok_or_else(|| anyhow::anyhow!("Invalid clock"))?;
        // A fictitious UTC calendar lets the maintained parser find local wall
        // times without applying its own DST gap/repeat policy.
        let mut cursor = after
            .with_timezone(&self.timezone)
            .naive_local()
            .with_nanosecond(0)
            .ok_or_else(|| anyhow::anyhow!("Invalid local clock"))?
            .and_utc();
        let end = cursor + Duration::days(366 * 8);
        for _ in 0..10000 {
            let candidate = self.cron.find_next_occurrence(&cursor, false)?;
            ensure!(
                candidate <= end,
                "Cron has no occurrence in the next eight years"
            );
            cursor = candidate;
            let utc = match self.timezone.from_local_datetime(&candidate.naive_utc()) {
                LocalResult::Single(time) => time.with_timezone(&Utc),
                LocalResult::Ambiguous(first, second) => first.min(second).with_timezone(&Utc),
                LocalResult::None => continue, // nonexistent local minute: skip, never snap
            };
            if utc > after {
                return Ok(u64::try_from(utc.timestamp_millis())?);
            }
        }
        anyhow::bail!("Cron timezone search exceeded its bounded limit")
    }
    pub fn preview(&self, after_ms: u64, count: u8) -> Result<Vec<u64>> {
        ensure!(
            (1..=20).contains(&count),
            "Preview count must be 1 through 20"
        );
        let mut cursor = after_ms;
        (0..count)
            .map(|_| {
                cursor = self.next(cursor)?;
                Ok(cursor)
            })
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    pub fn ms(s: &str) -> u64 {
        DateTime::parse_from_rfc3339(s).unwrap().timestamp_millis() as u64
    }
    #[test]
    fn numeric_validation_and_posix_day_or_weekday() {
        for invalid in [
            "@daily",
            "0 0 * * MON",
            "0 0 0 * * *",
            "60 * * * *",
            "*/0 * * * *",
            "0 0 L * *",
            "0 0 ? * *",
        ] {
            assert!(Calendar::parse(invalid, "UTC").is_err(), "{invalid}");
        }
        assert!(Calendar::parse("0 0 * * *", "+02:00").is_err());
        assert!(Calendar::parse("0 0 * * *", "Not/AZone").is_err());
        let calendar = Calendar::parse("0 0 15 * 1", "UTC").unwrap();
        assert_eq!(
            calendar.next(ms("2026-10-12T01:00:00Z")).unwrap(),
            ms("2026-10-15T00:00:00Z")
        );
        assert_eq!(
            Calendar::parse("*/15 9-10 * * 1-5", "UTC")
                .unwrap()
                .next(ms("2026-10-12T09:01:00Z"))
                .unwrap(),
            ms("2026-10-12T09:15:00Z")
        );
    }
    #[test]
    fn dst_skips_gap_and_uses_only_first_repeated_minute() {
        let daily = Calendar::parse("30 2 * * *", "Europe/Berlin").unwrap();
        assert_eq!(
            daily.next(ms("2026-03-28T02:00:00Z")).unwrap(),
            ms("2026-03-30T00:30:00Z")
        );
        let first = ms("2026-10-25T00:30:00Z");
        assert_eq!(daily.next(ms("2026-10-24T02:00:00Z")).unwrap(), first);
        assert_eq!(daily.next(first).unwrap(), ms("2026-10-26T01:30:00Z"));
        assert_eq!(
            daily.next(ms("2026-10-25T01:01:00Z")).unwrap(),
            ms("2026-10-26T01:30:00Z")
        );
        let interval = Calendar::parse("*/15 * * * *", "Europe/Berlin").unwrap();
        assert_eq!(
            interval.next(ms("2026-10-25T00:45:00Z")).unwrap(),
            ms("2026-10-25T02:00:00Z")
        );
    }
    #[test]
    fn nonexistent_full_day_and_leap_year_and_ms_boundary() {
        let samoa = Calendar::parse("0 12 * * *", "Pacific/Apia").unwrap();
        assert_eq!(
            samoa.next(ms("2011-12-29T22:00:00Z")).unwrap(),
            ms("2011-12-30T22:00:00Z")
        );
        let leap = Calendar::parse("0 0 29 2 *", "UTC").unwrap();
        assert_eq!(
            leap.next(ms("2025-01-01T00:00:00Z")).unwrap(),
            ms("2028-02-29T00:00:00Z")
        );
        let every = Calendar::parse("* * * * *", "UTC").unwrap();
        assert_eq!(
            every.next(ms("2026-10-12T08:00:00.001Z")).unwrap(),
            ms("2026-10-12T08:01:00Z")
        );
        assert!(every.preview(0, 0).is_err());
    }
}
