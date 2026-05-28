//! Parser for cmd 0x1B Diagnostics response (control byte 0x5A only).
//!
//! Payload starts with the echoed control bytes "5A", then 5 chars per
//! configured pump side: situation(1) + pump_status(2) + pump_type(2 hex).

use serde::Serialize;

use super::ParseError;

#[derive(Serialize, Debug, Clone)]
pub struct DiagnosticsParsed {
    pub control: String,
    pub pumps: Vec<PumpDiagnostic>,
    pub any_fault: bool,
}

#[derive(Serialize, Debug, Clone)]
pub struct PumpDiagnostic {
    pub side_index: usize,
    pub situation: String,         // R/F/?/!/N/0
    pub situation_label: String,
    pub pump_status: String,       // 2-char status code, kept as raw string
    pub pump_type_hex: String,     // 2 hex digits
}

fn situation_label(c: u8) -> &'static str {
    match c {
        b'R' => "responding",
        b'F' => "fault",
        b'?' => "unknown_type",
        b'!' => "unauthorized_type",
        b'N' => "not_configured",
        b'0' => "no_pump",
        _ => "unknown",
    }
}

pub fn parse(payload: &[u8]) -> Result<DiagnosticsParsed, ParseError> {
    if payload.len() < 2 {
        return Err(ParseError::new("diagnostics payload missing control byte"));
    }
    let control = std::str::from_utf8(&payload[0..2])
        .map_err(|_| ParseError::new("control not ASCII"))?
        .to_string();
    if control != "5A" {
        return Err(ParseError::new(format!(
            "expected control 5A (we only ever send 5A), got {}",
            control
        )));
    }

    let rest = &payload[2..];
    if rest.len() % 5 != 0 {
        return Err(ParseError::new(format!(
            "diagnostics body length {} is not a multiple of 5",
            rest.len()
        )));
    }

    let mut pumps = Vec::with_capacity(rest.len() / 5);
    for (i, chunk) in rest.chunks_exact(5).enumerate() {
        let situation_byte = chunk[0];
        let pump_status = std::str::from_utf8(&chunk[1..3])
            .map_err(|_| ParseError::new("pump_status not ASCII"))?
            .to_string();
        let pump_type = std::str::from_utf8(&chunk[3..5])
            .map_err(|_| ParseError::new("pump_type not ASCII"))?
            .to_string();
        // Validate pump_type is hex without consuming the string.
        u8::from_str_radix(&pump_type, 16)
            .map_err(|_| ParseError::new(format!("pump_type {:?} not hex", pump_type)))?;
        pumps.push(PumpDiagnostic {
            side_index: i,
            situation: (situation_byte as char).to_string(),
            situation_label: situation_label(situation_byte).to_string(),
            pump_status,
            pump_type_hex: pump_type,
        });
    }

    let any_fault = pumps.iter().any(|p| p.situation == "F");
    Ok(DiagnosticsParsed { control, pumps, any_fault })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_dt214_example() {
        // DT214 §3.7.2: RX >!001D1B5AR2007F0007N0000N0000F00012A
        // After stripping index "1B" and frame, payload = "5AR2007F0007N0000N0000F0001"
        let parsed = parse(b"5AR2007F0007N0000N0000F0001").unwrap();
        assert_eq!(parsed.control, "5A");
        assert_eq!(parsed.pumps.len(), 5);

        assert_eq!(parsed.pumps[0].situation, "R");
        assert_eq!(parsed.pumps[0].pump_status, "20");
        assert_eq!(parsed.pumps[0].pump_type_hex, "07");

        assert_eq!(parsed.pumps[1].situation, "F");
        assert_eq!(parsed.pumps[2].situation, "N");
        assert_eq!(parsed.pumps[3].situation, "N");
        assert_eq!(parsed.pumps[4].situation, "F");

        assert!(parsed.any_fault);
    }

    #[test]
    fn rejects_non_5a_control() {
        assert!(parse(b"5B0102").is_err());
    }

    #[test]
    fn rejects_misaligned_body() {
        assert!(parse(b"5AR200").is_err()); // 4 bytes after control, not multiple of 5
    }

    #[test]
    fn empty_pumps_ok() {
        let parsed = parse(b"5A").unwrap();
        assert_eq!(parsed.pumps.len(), 0);
        assert!(!parsed.any_fault);
    }
}
