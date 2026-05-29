//! State document plus atomic publish to disk.
//!
//! The accumulator owns the in-memory "last successful" values for each
//! command and is updated once per poll cycle. After each update, the
//! caller serializes the snapshot and writes it via [`write_atomic`].

use std::fs;
use std::io::Write;
use std::path::Path;

use chrono::{DateTime, SecondsFormat, Utc};
use serde::Serialize;
use serde_json::Value;

use crate::protocol::Command;

/// What the consumer fetches each cycle.
#[derive(Serialize, Debug, Clone)]
pub struct StateSnapshot {
    pub poll_at: String,
    pub device: DeviceRef,
    pub reachable: bool,
    pub last_error: Option<String>,
    pub consecutive_failures: u32,
    pub commands: CommandsBlock,
}

#[derive(Serialize, Debug, Clone)]
pub struct DeviceRef {
    pub ip: String,
    pub port: u16,
}

#[derive(Serialize, Debug, Clone, Default)]
pub struct CommandsBlock {
    pub status: CommandState,
    pub clock: CommandState,
    pub device_info: CommandState,
    pub diagnostics: CommandState,
    pub wireless: CommandState,
}

#[derive(Serialize, Debug, Clone, Default)]
pub struct CommandState {
    pub ok: bool,
    pub raw: Option<String>,
    pub frame_error: Option<String>,
    pub parse_error: Option<String>,
    pub parsed: Option<Value>,
    pub last_success_at: Option<String>,
    pub last_success: Option<Value>,
}

impl CommandState {
    /// Update this command slot with a fresh poll result. On success, the
    /// `last_success_*` fields advance; on failure, they retain their old
    /// values so the consumer sees how stale the latest good reading is.
    pub fn update(&mut self, result: CommandPollResult, now: &DateTime<Utc>) {
        match result {
            CommandPollResult::Ok { raw, parsed } => {
                self.ok = true;
                self.raw = Some(raw);
                self.frame_error = None;
                self.parse_error = None;
                self.parsed = Some(parsed.clone());
                self.last_success_at = Some(now.to_rfc3339_opts(SecondsFormat::Secs, true));
                self.last_success = Some(parsed);
            }
            CommandPollResult::ParseFailed { raw, error } => {
                self.ok = false;
                self.raw = Some(raw);
                self.frame_error = None;
                self.parse_error = Some(error);
                self.parsed = None;
                // last_success_* unchanged
            }
            CommandPollResult::FrameFailed { error } => {
                self.ok = false;
                self.raw = None;
                self.frame_error = Some(error);
                self.parse_error = None;
                self.parsed = None;
            }
        }
    }
}

pub enum CommandPollResult {
    Ok { raw: String, parsed: Value },
    ParseFailed { raw: String, error: String },
    FrameFailed { error: String },
}

/// Accumulator that keeps last-good state across polls and serializes the
/// full snapshot on demand.
pub struct StateAccumulator {
    pub device: DeviceRef,
    pub commands: CommandsBlock,
    pub consecutive_failures: u32,
}

impl StateAccumulator {
    pub fn new(ip: String, port: u16) -> Self {
        Self {
            device: DeviceRef { ip, port },
            commands: CommandsBlock::default(),
            consecutive_failures: 0,
        }
    }

    pub fn slot_mut(&mut self, cmd: Command) -> &mut CommandState {
        match cmd {
            Command::Status => &mut self.commands.status,
            Command::Clock => &mut self.commands.clock,
            Command::DeviceInfo => &mut self.commands.device_info,
            Command::Diagnostics => &mut self.commands.diagnostics,
            Command::Wireless => &mut self.commands.wireless,
        }
    }

    pub fn snapshot(&self, poll_at: &DateTime<Utc>, reachable: bool, last_error: Option<String>) -> StateSnapshot {
        StateSnapshot {
            poll_at: poll_at.to_rfc3339_opts(SecondsFormat::Secs, true),
            device: self.device.clone(),
            reachable,
            last_error,
            consecutive_failures: self.consecutive_failures,
            commands: self.commands.clone(),
        }
    }
}

/// Alarm-relevant subset of a poll cycle — noise stripped.
#[derive(Serialize, Debug)]
pub struct HealthSnapshot {
    pub poll_at: String,
    pub reachable: bool,
    pub last_error: Option<String>,
    pub consecutive_failures: u32,
    pub status: HealthStatus,
    pub battery: HealthBattery,
    pub clock: HealthClock,
    pub diagnostics: HealthDiagnostics,
    pub wireless: HealthWireless,
}

#[derive(Serialize, Debug)]
pub struct HealthStatus {
    pub ok: bool,
    pub any_fault: bool,
    pub any_generic_error: bool,
}

#[derive(Serialize, Debug)]
pub struct HealthBattery {
    pub ok: bool,
    pub level: Option<i64>,
    pub level_label: Option<String>,
    pub voltage_raw: Option<String>,
    pub hw_status: Option<String>,
    pub hw_label: Option<String>,
}

#[derive(Serialize, Debug)]
pub struct HealthClock {
    pub ok: bool,
    pub drift_seconds: Option<i64>,
}

#[derive(Serialize, Debug)]
pub struct HealthDiagnostics {
    pub ok: bool,
    pub any_fault: bool,
}

#[derive(Serialize, Debug)]
pub struct HealthWireless {
    pub ok: bool,
    pub any_fault: bool,
}

impl StateSnapshot {
    pub fn to_health(&self) -> HealthSnapshot {
        let di = self.commands.device_info.parsed.as_ref();
        HealthSnapshot {
            poll_at: self.poll_at.clone(),
            reachable: self.reachable,
            last_error: self.last_error.clone(),
            consecutive_failures: self.consecutive_failures,
            status: HealthStatus {
                ok: self.commands.status.ok,
                any_fault: self.commands.status.parsed
                    .as_ref()
                    .and_then(|v| v["any_fault"].as_bool())
                    .unwrap_or(false),
                any_generic_error: self.commands.status.parsed
                    .as_ref()
                    .and_then(|v| v["any_generic_error"].as_bool())
                    .unwrap_or(false),
            },
            battery: HealthBattery {
                ok: self.commands.device_info.ok,
                level: di.and_then(|v| v["battery_level"].as_i64()),
                level_label: di.and_then(|v| v["battery_level_label"].as_str()).map(str::to_owned),
                voltage_raw: di.and_then(|v| v["battery_voltage_raw"].as_str()).map(str::to_owned),
                hw_status: di.and_then(|v| v["battery_hw_status"].as_str()).map(str::to_owned),
                hw_label: di.and_then(|v| v["battery_hw_label"].as_str()).map(str::to_owned),
            },
            clock: HealthClock {
                ok: self.commands.clock.ok,
                drift_seconds: self.commands.clock.parsed
                    .as_ref()
                    .and_then(|v| v["drift_seconds"].as_i64()),
            },
            diagnostics: HealthDiagnostics {
                ok: self.commands.diagnostics.ok,
                any_fault: self.commands.diagnostics.parsed
                    .as_ref()
                    .and_then(|v| v["any_fault"].as_bool())
                    .unwrap_or(false),
            },
            wireless: HealthWireless {
                ok: self.commands.wireless.ok,
                any_fault: self.commands.wireless.parsed
                    .as_ref()
                    .and_then(|v| v["any_fault"].as_bool())
                    .unwrap_or(false),
            },
        }
    }
}

/// Atomic publish: write to `<path>.tmp`, fsync, then rename over `<path>`.
/// On a successful return, a reader doing a single open+read of `path` will
/// see either the previous file contents or the new ones — never a partial.
pub fn write_atomic<T: serde::Serialize>(path: &Path, value: &T) -> std::io::Result<()> {
    let bytes = serde_json::to_vec_pretty(value)
        .map_err(|e| std::io::Error::new(std::io::ErrorKind::Other, e))?;

    let tmp_path = match path.extension() {
        Some(ext) => {
            let mut s = path.as_os_str().to_owned();
            s.push(".tmp");
            // The above doesn't always behave on Windows; use a sibling instead.
            let _ = ext;
            path.with_extension(format!(
                "{}.tmp",
                path.extension().map(|e| e.to_string_lossy().into_owned()).unwrap_or_default()
            ))
        }
        None => {
            let mut buf = path.as_os_str().to_owned();
            buf.push(".tmp");
            std::path::PathBuf::from(buf)
        }
    };

    {
        let mut f = fs::File::create(&tmp_path)?;
        f.write_all(&bytes)?;
        f.write_all(b"\n")?;
        f.sync_all()?;
    }

    // std::fs::rename on Linux is atomic; on Windows it uses MoveFileEx
    // with MOVEFILE_REPLACE_EXISTING which is best-effort atomic. Either
    // way, no reader can see a half-written file.
    fs::rename(&tmp_path, path)
}

/// Returns Ok(()) if the directory containing `path` exists and is writable
/// (verified by attempting to create + delete a probe file).
pub fn check_output_writable(path: &Path) -> std::io::Result<()> {
    let parent = path.parent().ok_or_else(|| {
        std::io::Error::new(std::io::ErrorKind::InvalidInput, "output path has no parent directory")
    })?;
    if parent.as_os_str().is_empty() {
        // path is just a filename in cwd — cwd is implicitly writable here
        return Ok(());
    }
    if !parent.exists() {
        return Err(std::io::Error::new(
            std::io::ErrorKind::NotFound,
            format!("output directory {} does not exist", parent.display()),
        ));
    }
    let probe = parent.join(".horustechwatch-probe");
    let mut f = fs::File::create(&probe)?;
    f.write_all(b"probe")?;
    drop(f);
    fs::remove_file(&probe)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::TimeZone;

    fn fake_now() -> DateTime<Utc> {
        Utc.with_ymd_and_hms(2026, 5, 27, 14, 0, 0).unwrap()
    }

    #[test]
    fn ok_update_advances_last_success() {
        let mut s = CommandState::default();
        let parsed = serde_json::json!({"battery_level": 0});
        s.update(
            CommandPollResult::Ok {
                raw: ">!...".into(),
                parsed: parsed.clone(),
            },
            &fake_now(),
        );
        assert!(s.ok);
        assert_eq!(s.last_success, Some(parsed));
        assert_eq!(s.last_success_at.as_deref(), Some("2026-05-27T14:00:00Z"));
    }

    #[test]
    fn parse_failure_preserves_last_success() {
        let mut s = CommandState::default();
        let good = serde_json::json!({"a": 1});
        s.update(CommandPollResult::Ok { raw: ">!OK".into(), parsed: good.clone() }, &fake_now());

        let later = Utc.with_ymd_and_hms(2026, 5, 27, 14, 15, 0).unwrap();
        s.update(
            CommandPollResult::ParseFailed { raw: ">!BAD".into(), error: "bad date".into() },
            &later,
        );
        assert!(!s.ok);
        assert_eq!(s.parse_error.as_deref(), Some("bad date"));
        // last_success carries forward
        assert_eq!(s.last_success, Some(good));
        assert_eq!(s.last_success_at.as_deref(), Some("2026-05-27T14:00:00Z"));
    }

    #[test]
    fn frame_failure_preserves_last_success() {
        let mut s = CommandState::default();
        let good = serde_json::json!({"a": 1});
        s.update(CommandPollResult::Ok { raw: ">!OK".into(), parsed: good.clone() }, &fake_now());

        s.update(CommandPollResult::FrameFailed { error: "timeout".into() }, &fake_now());
        assert!(!s.ok);
        assert!(s.raw.is_none());
        assert_eq!(s.frame_error.as_deref(), Some("timeout"));
        assert_eq!(s.last_success, Some(good));
    }

    #[test]
    fn atomic_write_round_trip() {
        let dir = tempdir();
        let path = dir.join("state.json");
        let acc = StateAccumulator::new("192.168.25.91".into(), 2001);
        let snap = acc.snapshot(&fake_now(), false, Some("device unreachable".into()));
        write_atomic(&path, &snap).unwrap();

        let bytes = std::fs::read(&path).unwrap();
        let parsed: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
        assert_eq!(parsed["device"]["ip"], "192.168.25.91");
        assert_eq!(parsed["device"]["port"], 2001);
        assert_eq!(parsed["reachable"], false);
        assert_eq!(parsed["last_error"], "device unreachable");

        // Trailing .tmp file must not linger.
        let tmp = dir.join("state.json.tmp");
        assert!(!tmp.exists(), "tmp file should have been renamed");
    }

    #[test]
    fn check_output_writable_detects_missing_dir() {
        let bogus = std::path::PathBuf::from("/no/such/directory/state.json");
        assert!(check_output_writable(&bogus).is_err());
    }

    #[test]
    fn check_output_writable_ok_in_existing_dir() {
        let dir = tempdir();
        let path = dir.join("state.json");
        check_output_writable(&path).unwrap();
    }

    fn tempdir() -> std::path::PathBuf {
        let mut p = std::env::temp_dir();
        let name = format!("horustechwatch-test-{}", std::process::id());
        p.push(name);
        let _ = std::fs::remove_dir_all(&p);
        std::fs::create_dir_all(&p).unwrap();
        p
    }
}
