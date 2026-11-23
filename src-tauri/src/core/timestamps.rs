use time::format_description::well_known::Rfc3339;
use time::macros::format_description;
use time::{OffsetDateTime, UtcOffset};

pub fn parse(value: &str) -> Result<OffsetDateTime, time::error::Parse> {
    OffsetDateTime::parse(value, &Rfc3339).map(|timestamp| timestamp.to_offset(UtcOffset::UTC))
}

/// Keep the UTC offset and fractional precision used by existing SQLite records.
pub fn format(value: &OffsetDateTime) -> String {
    let utc = value.to_offset(UtcOffset::UTC);
    let datetime = utc
        .format(&format_description!(
            "[year]-[month]-[day]T[hour]:[minute]:[second]"
        ))
        .expect("UTC datetime format");
    let fraction = match utc.nanosecond() {
        0 => String::new(),
        value if value % 1_000_000 == 0 => format!(".{:03}", value / 1_000_000),
        value if value % 1_000 == 0 => format!(".{:06}", value / 1_000),
        value => format!(".{value:09}"),
    };
    format!("{datetime}{fraction}+00:00")
}

pub fn now() -> String {
    format(&OffsetDateTime::now_utc())
}

pub fn unix_millis() -> i64 {
    (OffsetDateTime::now_utc()
        .unix_timestamp_nanos()
        .div_euclid(1_000_000)) as i64
}

pub(crate) fn unix_nanos() -> i64 {
    i64::try_from(OffsetDateTime::now_utc().unix_timestamp_nanos()).unwrap_or_default()
}

pub(crate) fn format_seconds(value: OffsetDateTime) -> String {
    value
        .to_offset(UtcOffset::UTC)
        .format(&format_description!(
            "[year]-[month]-[day]T[hour]:[minute]:[second]Z"
        ))
        .expect("UTC datetime format")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn preserves_stored_utc_dates_and_their_fractional_precision() {
        for stored in [
            "2026-11-21T12:00:00+00:00",
            "2026-11-21T12:00:00.010+00:00",
            "2026-11-21T12:00:00.010100+00:00",
            "2026-11-21T12:00:00.010100010+00:00",
        ] {
            assert_eq!(format(&parse(stored).unwrap()), stored);
        }
        let timestamp = parse("2026-11-21T13:00:00.010+01:00").unwrap();
        assert_eq!(format(&timestamp), "2026-11-21T12:00:00.010+00:00");
        assert_eq!(format_seconds(timestamp), "2026-11-21T12:00:00Z");
    }
}
