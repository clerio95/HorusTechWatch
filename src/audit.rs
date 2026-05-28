//! Append-only daily JSONL audit log.
//!
//! Format: one JSON line per event, stored in `<log_dir>/YYYY-MM-DD.jsonl`.
//! Survives the third party's separate consumption of `state.json` and gives
//! local forensic visibility into every poll cycle.

use std::fs::{self, OpenOptions};
use std::io::Write;
use std::path::{Path, PathBuf};

use chrono::{DateTime, SecondsFormat, Utc};
use serde::Serialize;

pub struct AuditLog {
    dir: PathBuf,
}

#[derive(Serialize, Debug)]
pub struct AuditEvent<'a> {
    pub at: String,
    pub event: &'a str,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub detail: Option<&'a str>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<&'a str>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub command: Option<&'a str>,
}

impl AuditLog {
    pub fn new(dir: impl Into<PathBuf>) -> Self {
        Self { dir: dir.into() }
    }

    /// Ensure `log_dir` exists. Called once at startup.
    pub fn prepare(&self) -> std::io::Result<()> {
        fs::create_dir_all(&self.dir)
    }

    /// Append `event` as a single JSON line. Errors are reported via the
    /// returned Result; callers typically log them to stderr and continue
    /// (the audit log is supportive, not load-bearing).
    pub fn append(&self, now: &DateTime<Utc>, event: AuditEvent<'_>) -> std::io::Result<()> {
        let file_path = self.file_path_for(now);
        let line = serde_json::to_string(&event)
            .map_err(|e| std::io::Error::new(std::io::ErrorKind::Other, e))?;
        let mut f = OpenOptions::new().create(true).append(true).open(&file_path)?;
        f.write_all(line.as_bytes())?;
        f.write_all(b"\n")?;
        Ok(())
    }

    fn file_path_for(&self, now: &DateTime<Utc>) -> PathBuf {
        let name = format!("{}.jsonl", now.format("%Y-%m-%d"));
        self.dir.join(name)
    }

    pub fn dir(&self) -> &Path {
        &self.dir
    }
}

/// Helper: build an event with the current time string.
pub fn event_now<'a>(name: &'a str, detail: Option<&'a str>) -> AuditEvent<'a> {
    AuditEvent {
        at: Utc::now().to_rfc3339_opts(SecondsFormat::Secs, true),
        event: name,
        detail,
        error: None,
        command: None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::TimeZone;

    fn tempdir() -> PathBuf {
        let mut p = std::env::temp_dir();
        p.push(format!("horustechwatch-audit-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&p);
        p
    }

    #[test]
    fn writes_one_line_per_event() {
        let dir = tempdir();
        let log = AuditLog::new(&dir);
        log.prepare().unwrap();
        let now = Utc.with_ymd_and_hms(2026, 5, 27, 14, 0, 0).unwrap();
        log.append(
            &now,
            AuditEvent {
                at: now.to_rfc3339_opts(SecondsFormat::Secs, true),
                event: "poll_ok",
                detail: None,
                error: None,
                command: Some("status"),
            },
        )
        .unwrap();
        log.append(
            &now,
            AuditEvent {
                at: now.to_rfc3339_opts(SecondsFormat::Secs, true),
                event: "poll_failed",
                detail: None,
                error: Some("timeout"),
                command: None,
            },
        )
        .unwrap();

        let path = dir.join("2026-05-27.jsonl");
        let body = std::fs::read_to_string(&path).unwrap();
        let lines: Vec<&str> = body.lines().collect();
        assert_eq!(lines.len(), 2);
        let l0: serde_json::Value = serde_json::from_str(lines[0]).unwrap();
        assert_eq!(l0["event"], "poll_ok");
        assert_eq!(l0["command"], "status");
        let l1: serde_json::Value = serde_json::from_str(lines[1]).unwrap();
        assert_eq!(l1["event"], "poll_failed");
        assert_eq!(l1["error"], "timeout");
    }

    #[test]
    fn file_path_rolls_with_date() {
        let log = AuditLog::new("/tmp/audit");
        let day1 = Utc.with_ymd_and_hms(2026, 5, 27, 23, 59, 59).unwrap();
        let day2 = Utc.with_ymd_and_hms(2026, 5, 28, 0, 0, 1).unwrap();
        assert_ne!(log.file_path_for(&day1), log.file_path_for(&day2));
    }
}
