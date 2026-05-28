//! TOML configuration loader with hardcoded safety enforcement.
//!
//! The 30-second poll-interval floor and the 5-second socket timeout are
//! NOT configurable — they are project-wide invariants. This module rejects
//! any TOML that tries to set a lower poll interval.

use std::path::PathBuf;

use serde::Deserialize;

/// Minimum allowed poll interval, in seconds. Hardcoded per CLAUDE.md;
/// values below this are rejected at config load time.
pub const POLL_INTERVAL_FLOOR_SECS: u64 = 30;

#[derive(Debug, Clone)]
pub struct Config {
    pub device_ip: String,
    pub device_port: u16,
    pub poll_interval_secs: u64,
    pub state_file: PathBuf,
    pub audit_log_dir: PathBuf,
}

#[derive(Deserialize, Debug)]
struct RawConfig {
    device: RawDevice,
    poll: RawPoll,
    output: RawOutput,
    audit: RawAudit,
}

#[derive(Deserialize, Debug)]
struct RawDevice {
    ip: String,
    port: u16,
}

#[derive(Deserialize, Debug)]
struct RawPoll {
    interval_seconds: u64,
}

#[derive(Deserialize, Debug)]
struct RawOutput {
    state_file: String,
}

#[derive(Deserialize, Debug)]
struct RawAudit {
    log_dir: String,
}

#[derive(Debug)]
pub enum ConfigError {
    Io(std::io::Error),
    Parse(String),
    Invalid(String),
}

impl std::fmt::Display for ConfigError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ConfigError::Io(e) => write!(f, "config I/O error: {}", e),
            ConfigError::Parse(s) => write!(f, "config parse error: {}", s),
            ConfigError::Invalid(s) => write!(f, "invalid config: {}", s),
        }
    }
}

impl std::error::Error for ConfigError {}

pub fn load_from_file(path: &std::path::Path) -> Result<Config, ConfigError> {
    let text = std::fs::read_to_string(path).map_err(ConfigError::Io)?;
    let raw: RawConfig = toml::from_str(&text).map_err(|e| ConfigError::Parse(e.to_string()))?;
    validate(raw)
}

fn validate(raw: RawConfig) -> Result<Config, ConfigError> {
    if raw.device.ip.trim().is_empty() {
        return Err(ConfigError::Invalid("device.ip is empty".into()));
    }
    if raw.device.port == 0 {
        return Err(ConfigError::Invalid("device.port must be > 0".into()));
    }
    if raw.poll.interval_seconds < POLL_INTERVAL_FLOOR_SECS {
        return Err(ConfigError::Invalid(format!(
            "poll.interval_seconds = {} is below the hardcoded floor of {} seconds. \
             This floor is a project-wide safety invariant and cannot be lowered via config.",
            raw.poll.interval_seconds, POLL_INTERVAL_FLOOR_SECS
        )));
    }
    if raw.output.state_file.trim().is_empty() {
        return Err(ConfigError::Invalid("output.state_file is empty".into()));
    }
    if raw.audit.log_dir.trim().is_empty() {
        return Err(ConfigError::Invalid("audit.log_dir is empty".into()));
    }
    Ok(Config {
        device_ip: raw.device.ip,
        device_port: raw.device.port,
        poll_interval_secs: raw.poll.interval_seconds,
        state_file: PathBuf::from(raw.output.state_file),
        audit_log_dir: PathBuf::from(raw.audit.log_dir),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn parse(s: &str) -> Result<Config, ConfigError> {
        let raw: RawConfig = toml::from_str(s).map_err(|e| ConfigError::Parse(e.to_string()))?;
        validate(raw)
    }

    #[test]
    fn accepts_valid_config() {
        let cfg = parse(
            r#"
[device]
ip = "192.168.25.91"
port = 2001

[poll]
interval_seconds = 900

[output]
state_file = "/tmp/state.json"

[audit]
log_dir = "/tmp/logs"
"#,
        )
        .unwrap();
        assert_eq!(cfg.device_port, 2001);
        assert_eq!(cfg.poll_interval_secs, 900);
    }

    #[test]
    fn rejects_poll_interval_below_floor() {
        let err = parse(
            r#"
[device]
ip = "192.168.25.91"
port = 2001
[poll]
interval_seconds = 10
[output]
state_file = "x"
[audit]
log_dir = "y"
"#,
        )
        .unwrap_err();
        match err {
            ConfigError::Invalid(s) => assert!(s.contains("30")),
            _ => panic!("expected Invalid"),
        }
    }

    #[test]
    fn accepts_exact_floor() {
        let cfg = parse(
            r#"
[device]
ip = "192.168.25.91"
port = 2001
[poll]
interval_seconds = 30
[output]
state_file = "x"
[audit]
log_dir = "y"
"#,
        )
        .unwrap();
        assert_eq!(cfg.poll_interval_secs, 30);
    }

    #[test]
    fn rejects_empty_ip() {
        assert!(parse(
            r#"
[device]
ip = ""
port = 2001
[poll]
interval_seconds = 60
[output]
state_file = "x"
[audit]
log_dir = "y"
"#,
        )
        .is_err());
    }

    #[test]
    fn rejects_port_zero() {
        assert!(parse(
            r#"
[device]
ip = "1.2.3.4"
port = 0
[poll]
interval_seconds = 60
[output]
state_file = "x"
[audit]
log_dir = "y"
"#,
        )
        .is_err());
    }
}
