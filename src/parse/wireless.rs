//! Parser for cmd 0x25 Wireless diagnostics response.
//!
//! Payload = 3 bytes per pump: situation(1) + LQI(1 hex digit) + RSSI(1 hex digit).
//! Some firmwares emit the situation char in lowercase ('r' instead of 'R').

use serde::Serialize;

use super::ParseError;

#[derive(Serialize, Debug, Clone)]
pub struct WirelessParsed {
    pub links: Vec<WirelessLink>,
    pub any_fault: bool,
}

#[derive(Serialize, Debug, Clone)]
pub struct WirelessLink {
    pub side_index: usize,
    pub situation: String,        // letter as device returned it
    pub situation_label: String,
    pub lqi: u8,                  // 0..=15
    pub rssi: u8,                 // 0..=15
}

fn situation_label(c: u8) -> &'static str {
    match c.to_ascii_uppercase() {
        b'R' => "responding",
        b'F' => "fault",
        b'N' => "not_configured",
        _ => "unknown",
    }
}

pub fn parse(payload: &[u8]) -> Result<WirelessParsed, ParseError> {
    if payload.len() % 3 != 0 {
        return Err(ParseError::new(format!(
            "wireless payload length {} is not a multiple of 3",
            payload.len()
        )));
    }

    let mut links = Vec::with_capacity(payload.len() / 3);
    for (i, chunk) in payload.chunks_exact(3).enumerate() {
        let situation_byte = chunk[0];
        let lqi = parse_hex_nibble(chunk[1])
            .ok_or_else(|| ParseError::new(format!("LQI {:?} not hex", chunk[1] as char)))?;
        let rssi = parse_hex_nibble(chunk[2])
            .ok_or_else(|| ParseError::new(format!("RSSI {:?} not hex", chunk[2] as char)))?;
        links.push(WirelessLink {
            side_index: i,
            situation: (situation_byte as char).to_string(),
            situation_label: situation_label(situation_byte).to_string(),
            lqi,
            rssi,
        });
    }

    let any_fault = links
        .iter()
        .any(|l| l.situation.eq_ignore_ascii_case("F"));
    Ok(WirelessParsed { links, any_fault })
}

fn parse_hex_nibble(b: u8) -> Option<u8> {
    match b {
        b'0'..=b'9' => Some(b - b'0'),
        b'a'..=b'f' => Some(b - b'a' + 10),
        b'A'..=b'F' => Some(b - b'A' + 10),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_dt214_example() {
        // DT214 §3.7.5: RX >!000525r0E34 — one pump, lowercase 'r'
        // Payload after stripping index "25": "r0E"
        let parsed = parse(b"r0E").unwrap();
        assert_eq!(parsed.links.len(), 1);
        assert_eq!(parsed.links[0].situation, "r");
        assert_eq!(parsed.links[0].situation_label, "responding");
        assert_eq!(parsed.links[0].lqi, 0);
        assert_eq!(parsed.links[0].rssi, 14);
        assert!(!parsed.any_fault);
    }

    #[test]
    fn handles_multiple_pumps() {
        let parsed = parse(b"R0FRABF00").unwrap();
        assert_eq!(parsed.links.len(), 3);
        assert_eq!(parsed.links[2].situation, "F");
        assert!(parsed.any_fault);
    }

    #[test]
    fn rejects_misaligned_body() {
        assert!(parse(b"R01").is_ok());
        assert!(parse(b"R012").is_err());
    }

    #[test]
    fn rejects_non_hex_lqi() {
        assert!(parse(b"RZ0").is_err());
    }
}
