mod audit;
mod client;
mod config;
mod parse;
mod protocol;
mod state;

use std::path::Path;
use std::time::{Duration, Instant};

use chrono::{DateTime, SecondsFormat, Utc};
use serde_json::Value;

use audit::{AuditEvent, AuditLog};
use client::Connection;
use config::Config;
use protocol::Command;
use state::{CommandPollResult, StateAccumulator};

fn main() {
    let config_path = std::env::args()
        .nth(1)
        .unwrap_or_else(|| "config.toml".to_string());

    let cfg = match config::load_from_file(Path::new(&config_path)) {
        Ok(c) => c,
        Err(e) => {
            eprintln!("FATAL: {}", e);
            std::process::exit(2);
        }
    };

    let audit = AuditLog::new(&cfg.audit_log_dir);
    if let Err(e) = audit.prepare() {
        eprintln!("FATAL: cannot create audit log directory {}: {}", cfg.audit_log_dir.display(), e);
        std::process::exit(2);
    }

    let startup_detail = format!(
        "ip={} port={} interval={}s state_file={}",
        cfg.device_ip,
        cfg.device_port,
        cfg.poll_interval_secs,
        cfg.state_file.display()
    );
    log_audit(
        &audit,
        AuditEvent {
            at: now_iso(),
            event: "startup",
            detail: Some(&startup_detail),
            error: None,
            command: None,
        },
    );
    eprintln!("horustechwatch started: {}", startup_detail);

    let mut acc = StateAccumulator::new(cfg.device_ip.clone(), cfg.device_port);
    let interval = Duration::from_secs(cfg.poll_interval_secs);

    loop {
        let cycle_start = Instant::now();
        let poll_at = Utc::now();
        run_one_cycle(&cfg, &audit, &mut acc, &poll_at);
        sleep_to_next_interval(cycle_start, interval);
    }
}

fn run_one_cycle(cfg: &Config, audit: &AuditLog, acc: &mut StateAccumulator, poll_at: &DateTime<Utc>) {
    // Step 1: verify the output directory is reachable / writable. If not,
    // skip the device poll entirely — we have nowhere to publish the result.
    if let Err(e) = state::check_output_writable(&cfg.state_file) {
        let msg = e.to_string();
        log_audit(
            audit,
            AuditEvent {
                at: poll_at.to_rfc3339_opts(SecondsFormat::Secs, true),
                event: "output_unreachable",
                detail: None,
                error: Some(&msg),
                command: None,
            },
        );
        eprintln!("[{}] output unreachable, skipping poll: {}", poll_at, msg);
        return;
    }

    // Step 2: open TCP connection to the device.
    let mut conn = match Connection::connect(&cfg.device_ip, cfg.device_port) {
        Ok(c) => c,
        Err(e) => {
            let msg = e.to_string();
            acc.consecutive_failures = acc.consecutive_failures.saturating_add(1);
            // Mark every command as frame-failed so consumers see consistent staleness.
            for cmd in Command::ALL {
                acc.slot_mut(cmd).update(
                    CommandPollResult::FrameFailed { error: msg.clone() },
                    poll_at,
                );
            }
            let snap = acc.snapshot(poll_at, false, Some(msg.clone()));
            publish_snapshot(cfg, audit, poll_at, &snap);
            log_audit(
                audit,
                AuditEvent {
                    at: poll_at.to_rfc3339_opts(SecondsFormat::Secs, true),
                    event: "connect_failed",
                    detail: None,
                    error: Some(&msg),
                    command: None,
                },
            );
            eprintln!("[{}] connect failed: {}", poll_at, msg);
            return;
        }
    };

    // Step 3: run all 5 queries serially.
    let mut any_failed = false;
    let mut first_error: Option<String> = None;
    for cmd in Command::ALL {
        match conn.query(cmd) {
            Ok(frame) => {
                let raw = String::from_utf8_lossy(&frame.raw).into_owned();
                match dispatch_parse(cmd, &frame.payload, poll_at) {
                    Ok(parsed) => acc.slot_mut(cmd).update(
                        CommandPollResult::Ok { raw, parsed },
                        poll_at,
                    ),
                    Err(e) => {
                        any_failed = true;
                        if first_error.is_none() {
                            first_error = Some(format!("{}: {}", cmd.name(), e));
                        }
                        log_audit(
                            audit,
                            AuditEvent {
                                at: poll_at.to_rfc3339_opts(SecondsFormat::Secs, true),
                                event: "parse_failed",
                                detail: None,
                                error: Some(&e),
                                command: Some(cmd.name()),
                            },
                        );
                        acc.slot_mut(cmd).update(
                            CommandPollResult::ParseFailed { raw, error: e },
                            poll_at,
                        );
                    }
                }
            }
            Err(e) => {
                any_failed = true;
                let msg = e.to_string();
                if first_error.is_none() {
                    first_error = Some(format!("{}: {}", cmd.name(), msg));
                }
                log_audit(
                    audit,
                    AuditEvent {
                        at: poll_at.to_rfc3339_opts(SecondsFormat::Secs, true),
                        event: "frame_failed",
                        detail: None,
                        error: Some(&msg),
                        command: Some(cmd.name()),
                    },
                );
                acc.slot_mut(cmd).update(
                    CommandPollResult::FrameFailed { error: msg },
                    poll_at,
                );
            }
        }
    }

    // Step 4: update overall failure counter + publish snapshot.
    if any_failed {
        acc.consecutive_failures = acc.consecutive_failures.saturating_add(1);
    } else {
        acc.consecutive_failures = 0;
    }
    let snap = acc.snapshot(poll_at, true, first_error.clone());
    publish_snapshot(cfg, audit, poll_at, &snap);

    let event = if any_failed { "poll_partial" } else { "poll_ok" };
    log_audit(
        audit,
        AuditEvent {
            at: poll_at.to_rfc3339_opts(SecondsFormat::Secs, true),
            event,
            detail: None,
            error: first_error.as_deref(),
            command: None,
        },
    );
    if any_failed {
        eprintln!("[{}] poll partial: {}", poll_at, first_error.as_deref().unwrap_or(""));
    } else {
        eprintln!("[{}] poll ok", poll_at);
    }
}

fn dispatch_parse(cmd: Command, payload: &[u8], poll_at: &DateTime<Utc>) -> Result<Value, String> {
    let to_value = |r: Result<serde_json::Value, String>| r;
    match cmd {
        Command::Status => to_value(
            parse::status::parse(payload)
                .map_err(|e| e.to_string())
                .and_then(|v| serde_json::to_value(v).map_err(|e| e.to_string())),
        ),
        Command::Clock => to_value(
            parse::clock::parse_with_local(payload, *poll_at)
                .map_err(|e| e.to_string())
                .and_then(|v| serde_json::to_value(v).map_err(|e| e.to_string())),
        ),
        Command::DeviceInfo => to_value(
            parse::device_info::parse(payload)
                .map_err(|e| e.to_string())
                .and_then(|v| serde_json::to_value(v).map_err(|e| e.to_string())),
        ),
        Command::Diagnostics => to_value(
            parse::diagnostics::parse(payload)
                .map_err(|e| e.to_string())
                .and_then(|v| serde_json::to_value(v).map_err(|e| e.to_string())),
        ),
        Command::Wireless => to_value(
            parse::wireless::parse(payload)
                .map_err(|e| e.to_string())
                .and_then(|v| serde_json::to_value(v).map_err(|e| e.to_string())),
        ),
    }
}

fn publish_snapshot(cfg: &Config, audit: &AuditLog, poll_at: &DateTime<Utc>, snap: &state::StateSnapshot) {
    if let Err(e) = state::write_atomic(&cfg.state_file, snap) {
        let msg = e.to_string();
        log_audit(
            audit,
            AuditEvent {
                at: poll_at.to_rfc3339_opts(SecondsFormat::Secs, true),
                event: "publish_failed",
                detail: None,
                error: Some(&msg),
                command: None,
            },
        );
        eprintln!("[{}] state.json publish failed: {}", poll_at, msg);
    }
}

fn sleep_to_next_interval(cycle_start: Instant, interval: Duration) {
    let elapsed = cycle_start.elapsed();
    if interval > elapsed {
        std::thread::sleep(interval - elapsed);
    }
    // else: poll took longer than the interval — fire next cycle immediately.
}

fn log_audit(audit: &AuditLog, event: AuditEvent<'_>) {
    if let Err(e) = audit.append(&Utc::now(), event) {
        eprintln!("audit log write failed: {}", e);
    }
}

fn now_iso() -> String {
    Utc::now().to_rfc3339_opts(SecondsFormat::Secs, true)
}
