//! Parser for cmd 0x01 Status response.
//!
//! Payload = one ASCII byte per configured nozzle slot, up to 99.

use serde::Serialize;

use super::ParseError;

#[derive(Serialize, Debug, Clone)]
pub struct StatusParsed {
    pub nozzles: Vec<NozzleStatus>,
    pub any_fault: bool,
    pub any_generic_error: bool,
}

#[derive(Serialize, Debug, Clone)]
pub struct NozzleStatus {
    pub index: usize,
    pub code: String, // one-char string, easier for JSON consumers than `char`
    pub state: NozzleState,
}

#[derive(Serialize, Debug, Clone, Copy, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum NozzleState {
    Blocked,
    Free,
    Fueling,
    Fault,
    Waiting,
    Ready,
    Busy,
    GenericError,
    NotConfigured,
    Unknown,
}

impl NozzleState {
    fn classify(c: u8) -> Self {
        match c {
            b'B' => NozzleState::Blocked,
            b'L' => NozzleState::Free,
            b'A' => NozzleState::Fueling,
            b'F' => NozzleState::Fault,
            b'E' => NozzleState::Waiting,
            b'P' => NozzleState::Ready,
            b'#' => NozzleState::Busy,
            b'!' => NozzleState::GenericError,
            b' ' => NozzleState::NotConfigured,
            _ => NozzleState::Unknown,
        }
    }
}

pub fn parse(payload: &[u8]) -> Result<StatusParsed, ParseError> {
    if payload.len() > 99 {
        return Err(ParseError::new(format!(
            "status payload has {} bytes, max 99",
            payload.len()
        )));
    }

    let nozzles: Vec<NozzleStatus> = payload
        .iter()
        .enumerate()
        .map(|(i, &b)| NozzleStatus {
            index: i,
            code: (b as char).to_string(),
            state: NozzleState::classify(b),
        })
        .collect();

    let any_fault = nozzles.iter().any(|n| n.state == NozzleState::Fault);
    let any_generic_error = nozzles.iter().any(|n| n.state == NozzleState::GenericError);

    Ok(StatusParsed { nozzles, any_fault, any_generic_error })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_dt214_example() {
        // From DT214 §3.4.1: 8 nozzles, "AALB P A" + trailing... wait,
        // example payload is "AALB P A" (8 chars).
        // index 1,2,8 fueling; 3 free; 4 blocked; 5,7 not configured; 6 ready.
        let parsed = parse(b"AALB P A").unwrap();
        assert_eq!(parsed.nozzles.len(), 8);
        assert_eq!(parsed.nozzles[0].state, NozzleState::Fueling);   // A
        assert_eq!(parsed.nozzles[1].state, NozzleState::Fueling);   // A
        assert_eq!(parsed.nozzles[2].state, NozzleState::Free);      // L
        assert_eq!(parsed.nozzles[3].state, NozzleState::Blocked);   // B
        assert_eq!(parsed.nozzles[4].state, NozzleState::NotConfigured); // ' '
        assert_eq!(parsed.nozzles[5].state, NozzleState::Ready);     // P
        assert_eq!(parsed.nozzles[6].state, NozzleState::NotConfigured); // ' '
        assert_eq!(parsed.nozzles[7].state, NozzleState::Fueling);   // A
        assert!(!parsed.any_fault);
        assert!(!parsed.any_generic_error);
    }

    #[test]
    fn detects_fault() {
        let parsed = parse(b"LFL").unwrap();
        assert!(parsed.any_fault);
    }

    #[test]
    fn detects_generic_error() {
        let parsed = parse(b"L!L").unwrap();
        assert!(parsed.any_generic_error);
    }

    #[test]
    fn empty_payload_is_zero_nozzles() {
        let parsed = parse(b"").unwrap();
        assert_eq!(parsed.nozzles.len(), 0);
    }

    #[test]
    fn rejects_oversized_payload() {
        let huge = vec![b'L'; 100];
        assert!(parse(&huge).is_err());
    }

    #[test]
    fn unknown_chars_become_unknown_state() {
        let parsed = parse(b"X").unwrap();
        assert_eq!(parsed.nozzles[0].state, NozzleState::Unknown);
    }
}
