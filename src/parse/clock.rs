//! Parser for cmd 0x0B Clock response.
//!
//! Payload = 14 ASCII digits: `YYMMDDddHHNNSS` where dd = day-of-week
//! (01 = Sunday). Year is 2 digits — convention is 20YY.

use chrono::{DateTime, NaiveDate, NaiveDateTime, Utc};
use serde::Serialize;

use super::ParseError;

#[derive(Serialize, Debug, Clone)]
pub struct ClockParsed {
    /// Device clock as ISO 8601 (naive — device has no TZ info).
    pub device: String,
    /// Local clock at the moment the response was received (UTC).
    pub local: String,
    /// Drift = device clock seconds - local clock seconds.
    pub drift_seconds: i64,
    /// 1 = Sunday, 2 = Monday, ... per DT214 convention.
    pub day_of_week: u8,
}

pub fn parse_with_local(payload: &[u8], local: DateTime<Utc>) -> Result<ClockParsed, ParseError> {
    if payload.len() != 14 {
        return Err(ParseError::new(format!(
            "clock payload must be 14 bytes, got {}",
            payload.len()
        )));
    }
    let s = std::str::from_utf8(payload).map_err(|_| ParseError::new("clock payload not ASCII"))?;
    let yy: i32 = s[0..2].parse().map_err(|_| ParseError::new("bad year"))?;
    let mm: u32 = s[2..4].parse().map_err(|_| ParseError::new("bad month"))?;
    let dd: u32 = s[4..6].parse().map_err(|_| ParseError::new("bad day"))?;
    let dow: u8 = s[6..8].parse().map_err(|_| ParseError::new("bad day-of-week"))?;
    let hh: u32 = s[8..10].parse().map_err(|_| ParseError::new("bad hour"))?;
    let nn: u32 = s[10..12].parse().map_err(|_| ParseError::new("bad minute"))?;
    let ss: u32 = s[12..14].parse().map_err(|_| ParseError::new("bad second"))?;

    let year = 2000 + yy;
    let device_naive = NaiveDate::from_ymd_opt(year, mm, dd)
        .and_then(|d| d.and_hms_opt(hh, nn, ss))
        .ok_or_else(|| ParseError::new(format!("invalid date {:04}-{:02}-{:02} {:02}:{:02}:{:02}", year, mm, dd, hh, nn, ss)))?;

    let local_naive: NaiveDateTime = local.naive_utc();
    let drift = (device_naive - local_naive).num_seconds();

    Ok(ClockParsed {
        device: device_naive.format("%Y-%m-%dT%H:%M:%S").to_string(),
        local: local.format("%Y-%m-%dT%H:%M:%SZ").to_string(),
        drift_seconds: drift,
        day_of_week: dow,
    })
}

pub fn parse(payload: &[u8]) -> Result<ClockParsed, ParseError> {
    parse_with_local(payload, Utc::now())
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::TimeZone;

    #[test]
    fn parses_dt214_example() {
        // DT214 §3.6.2 example RX: >!00100B1207270615492224
        // payload = "1207270615492224"... wait that includes "1B" or trailing checksum
        // Actually after stripping index "0B", payload should be "1207270615492224"
        // which is 16 chars — but spec says 14 chars payload after index.
        // The trailing "24" is the checksum KK. So payload is "12072706154922" = 14 chars.
        let local = Utc.with_ymd_and_hms(2012, 7, 27, 15, 49, 22).unwrap();
        let parsed = parse_with_local(b"12072706154922", local).unwrap();
        assert_eq!(parsed.device, "2012-07-27T15:49:22");
        assert_eq!(parsed.drift_seconds, 0);
        assert_eq!(parsed.day_of_week, 6);
    }

    #[test]
    fn computes_drift() {
        let local = Utc.with_ymd_and_hms(2026, 5, 27, 14, 0, 0).unwrap();
        // Device clock is 30 seconds ahead
        let parsed = parse_with_local(b"26052704140030", local).unwrap();
        assert_eq!(parsed.drift_seconds, 30);
    }

    #[test]
    fn rejects_wrong_length() {
        assert!(parse(b"123").is_err());
        assert!(parse(b"123456789012345").is_err());
    }

    #[test]
    fn rejects_invalid_date() {
        assert!(parse(b"26130106120000").is_err()); // month 13
        assert!(parse(b"26023206120000").is_err()); // feb 32
    }
}
