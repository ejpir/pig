use pi_core::clock::{parse_timestamp, short_age};

#[test]
fn parses_pi_iso_timestamps_to_epoch_milliseconds() {
    assert_eq!(parse_timestamp("1970-01-01T00:00:00.000Z"), Some(0));
    assert_eq!(
        parse_timestamp("2026-09-28T09:41:00.000Z"),
        Some(1_790_588_460_000)
    );
    assert_eq!(
        parse_timestamp("2024-02-29T23:59:59.5Z"),
        Some(1_709_251_199_500)
    );
    assert_eq!(
        parse_timestamp("2026-09-28T11:41:00+02:00"),
        parse_timestamp("2026-09-28T09:41:00Z")
    );
    assert_eq!(
        parse_timestamp("2026-09-28T09:41:00.123456Z"),
        Some(1_790_588_460_123)
    );
}

#[test]
fn rejects_malformed_timestamps() {
    for text in [
        "",
        "2026-09-28",
        "2026-09-28 09:41:00Z",
        "2026-13-28T09:41:00Z",
        "2026-09-28T24:00:00Z",
        "2026-09-28T09:41:60Z",
        "2026-09-28T09:41:00",
        "2026-09-28T09:41:00.Z",
        "2026-09-28T09:41:00+0200",
        "20x6-09-28T09:41:00Z",
        "1969-12-31T23:59:59Z",
    ] {
        assert_eq!(parse_timestamp(text), None, "{text:?}");
    }
}

#[test]
fn short_ages_round_down_and_never_go_negative() {
    let now = 1_790_590_440_000;
    let ago = |minutes: u64| short_age(now - minutes * 60_000, now);
    assert_eq!(short_age(now + 5_000, now), "now");
    assert_eq!(ago(0), "now");
    assert_eq!(ago(12), "12m");
    assert_eq!(ago(125), "2h");
    assert_eq!(ago(23 * 60 + 59), "23h");
    assert_eq!(ago(24 * 60), "1d");
    assert_eq!(ago(13 * 1440), "13d");
    assert_eq!(ago(20 * 1440), "2w");
    assert_eq!(ago(90 * 1440), "3mo");
    assert_eq!(ago(800 * 1440), "2y");
}

#[test]
fn sessions_say_when_in_words_and_dates() {
    use pi_core::clock::{date_time, when};
    let now = parse_timestamp("2026-09-29T18:00:00Z").unwrap();
    let at = |text| parse_timestamp(text).unwrap();
    assert_eq!(when(at("2026-09-29T17:48:00Z"), now), "12 min ago");
    assert_eq!(when(at("2026-09-29T16:00:00Z"), now), "2 h ago");
    assert_eq!(when(at("2026-09-28T23:00:00Z"), now), "yesterday");
    assert_eq!(when(at("2026-09-27T10:00:00Z"), now), "Sun");
    assert_eq!(when(at("2026-09-21T20:14:00Z"), now), "Sep 21");
    assert_eq!(when(at("2025-12-31T20:14:00Z"), now), "Dec 31, 2025");
    assert_eq!(date_time(at("2026-09-21T20:14:59Z")), "Sep 21 · 20:14");
    assert_eq!(date_time(at("2024-02-29T00:05:00Z")), "Feb 29 · 00:05");
}
