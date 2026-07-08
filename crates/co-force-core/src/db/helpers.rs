//! Utility helpers shared across database modules.

use chrono::{DateTime, NaiveDateTime, Utc};

/// Parses a SQLite TIMESTAMP string into `DateTime<Utc>`.
///
/// SQLite stores timestamps as text in the format `YYYY-MM-DD HH:MM:SS`
/// (from `CURRENT_TIMESTAMP`) or RFC 3339 format. This function handles both.
pub fn parse_datetime(s: &str) -> Option<DateTime<Utc>> {
    // Try RFC 3339 first (from explicit inserts)
    if let Ok(dt) = DateTime::parse_from_rfc3339(s) {
        return Some(dt.with_timezone(&Utc));
    }
    // Try SQLite's CURRENT_TIMESTAMP format: "2024-01-01 12:00:00"
    if let Ok(naive) = NaiveDateTime::parse_from_str(s, "%Y-%m-%d %H:%M:%S") {
        return Some(naive.and_utc());
    }
    None
}

/// Reads an optional timestamp column from a rusqlite row as `Option<DateTime<Utc>>`.
pub fn get_optional_datetime(
    row: &rusqlite::Row<'_>,
    idx: usize,
) -> Result<Option<DateTime<Utc>>, rusqlite::Error> {
    let raw: Option<String> = row.get(idx)?;
    Ok(raw.and_then(|s| parse_datetime(&s)))
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::Datelike;

    #[test]
    fn test_parse_sqlite_format() {
        let dt = parse_datetime("2026-07-08 12:00:00").unwrap();
        assert_eq!(dt.year(), 2026);
        assert_eq!(dt.month(), 7);
    }

    #[test]
    fn test_parse_rfc3339_format() {
        let dt = parse_datetime("2026-07-08T12:00:00+00:00").unwrap();
        assert_eq!(dt.year(), 2026);
    }

    #[test]
    fn test_parse_invalid() {
        assert!(parse_datetime("not-a-date").is_none());
    }
}
