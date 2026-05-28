//! Response frame parsing for `>!CCCCXX...KK` frames.
//!
//! Defensive: refuses lengths above [`MAX_DATA_LEN`], verifies checksum,
//! and validates that the echoed command index matches what we asked for.

use super::checksum;

/// Hard cap on the declared CCCC data length. Device responses we expect:
///   - status:      ~16 chars (≤ 16 nozzles)
///   - device_info: ~158 chars max per CLAUDE.md (full template ~ 182 incl framing)
///   - clock:       ~14 chars
///   - diagnostics: ~24 chars
///   - wireless:    ~48 chars
/// 512 leaves generous slack while refusing pathological lengths.
pub const MAX_DATA_LEN: usize = 512;

#[derive(Debug, Clone)]
pub struct ResponseFrame {
    /// Echoed command index from the response's first data byte pair.
    pub command_index: u8,
    /// Payload AFTER the 2-char command-index prefix.
    pub payload: Vec<u8>,
    /// Full frame as received, including `>` delimiter and checksum.
    pub raw: Vec<u8>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum FrameError {
    TooShort,
    MissingDelimiter,
    NotAResponse,
    BadLengthField,
    LengthExceedsMax,
    Truncated,
    BadChecksum,
    BadCommandIndex,
    CommandIndexMismatch { expected: u8, actual: u8 },
}

impl std::fmt::Display for FrameError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            FrameError::TooShort => write!(f, "frame too short"),
            FrameError::MissingDelimiter => write!(f, "no '>' delimiter"),
            FrameError::NotAResponse => write!(f, "not a response frame (P != '!')"),
            FrameError::BadLengthField => write!(f, "CCCC length field is not valid hex"),
            FrameError::LengthExceedsMax => write!(f, "declared length exceeds {} bytes", MAX_DATA_LEN),
            FrameError::Truncated => write!(f, "frame truncated"),
            FrameError::BadChecksum => write!(f, "checksum mismatch"),
            FrameError::BadCommandIndex => write!(f, "command index is not valid hex"),
            FrameError::CommandIndexMismatch { expected, actual } => {
                write!(f, "echoed command index 0x{:02X} does not match queried 0x{:02X}", actual, expected)
            }
        }
    }
}

impl std::error::Error for FrameError {}

/// Parse a complete response frame from `buf`. `buf` must start with `>`.
/// Trailing bytes beyond the frame are ignored. The caller is responsible
/// for byte-level framing (skipping garbage before `>` and reading exactly
/// the right number of bytes — see [`crate::client`]).
pub fn parse(buf: &[u8], expected_cmd_index: u8) -> Result<ResponseFrame, FrameError> {
    // Minimum: > ! C C C C X X K K = 10 bytes
    if buf.len() < 10 {
        return Err(FrameError::TooShort);
    }
    if buf[0] != b'>' {
        return Err(FrameError::MissingDelimiter);
    }
    if buf[1] != b'!' {
        return Err(FrameError::NotAResponse);
    }

    let cccc = std::str::from_utf8(&buf[2..6]).map_err(|_| FrameError::BadLengthField)?;
    let data_len = usize::from_str_radix(cccc, 16).map_err(|_| FrameError::BadLengthField)?;
    if data_len > MAX_DATA_LEN {
        return Err(FrameError::LengthExceedsMax);
    }
    if data_len < 2 {
        // Must at least contain the command-index echo.
        return Err(FrameError::BadCommandIndex);
    }

    let total = 1 /* > */ + 1 /* ! */ + 4 /* CCCC */ + data_len + 2 /* KK */;
    if buf.len() < total {
        return Err(FrameError::Truncated);
    }

    // Checksum covers from `!` through the last data byte (everything after `>`,
    // up to but not including the checksum chars).
    let body_start = 1;
    let body_end = 6 + data_len;
    let body = &buf[body_start..body_end];
    let expected_cksum = [buf[body_end], buf[body_end + 1]];
    if !checksum::verify(body, expected_cksum) {
        return Err(FrameError::BadChecksum);
    }

    let data_start = 6;
    let cmd_hex = std::str::from_utf8(&buf[data_start..data_start + 2])
        .map_err(|_| FrameError::BadCommandIndex)?;
    let cmd_idx = u8::from_str_radix(cmd_hex, 16).map_err(|_| FrameError::BadCommandIndex)?;
    if cmd_idx != expected_cmd_index {
        return Err(FrameError::CommandIndexMismatch {
            expected: expected_cmd_index,
            actual: cmd_idx,
        });
    }

    let payload = buf[data_start + 2..body_end].to_vec();
    Ok(ResponseFrame {
        command_index: cmd_idx,
        payload,
        raw: buf[..total].to_vec(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Live device info response from CLAUDE.md (2026-05-26 reading).
    const LIVE_DEVICE_INFO: &[u8] = b">!009E12B01.00 F08.03 22/07/19 0 12,84 2 0113 3-00010427 17/01/17 26/05/26 00:26:28:11:04:27 192.168.025.091;00/00/00 Fc  00000000;c900HDNN;000.000.000.000;00000;D;FB";

    #[test]
    fn parses_live_device_info_frame() {
        let f = parse(LIVE_DEVICE_INFO, 0x12).expect("must parse");
        assert_eq!(f.command_index, 0x12);
        // Declared length 0x009E = 158 bytes. The 158 bytes of data start with "12" (the echoed index).
        assert_eq!(f.payload.len(), 158 - 2);
        // First payload byte should be 'B' (start of "B01.00" boot-loader version).
        assert_eq!(f.payload[0], b'B');
    }

    #[test]
    fn rejects_wrong_expected_index() {
        let err = parse(LIVE_DEVICE_INFO, 0x01).unwrap_err();
        assert!(matches!(err, FrameError::CommandIndexMismatch { expected: 0x01, actual: 0x12 }));
    }

    #[test]
    fn rejects_truncated_frame() {
        let truncated = &LIVE_DEVICE_INFO[..LIVE_DEVICE_INFO.len() - 5];
        assert_eq!(parse(truncated, 0x12).unwrap_err(), FrameError::Truncated);
    }

    #[test]
    fn rejects_bad_checksum() {
        let mut bad = LIVE_DEVICE_INFO.to_vec();
        let last = bad.len() - 1;
        bad[last] = b'0'; // was 'B' in "FB"
        assert_eq!(parse(&bad, 0x12).unwrap_err(), FrameError::BadChecksum);
    }

    #[test]
    fn rejects_missing_delimiter() {
        let mut bad = LIVE_DEVICE_INFO.to_vec();
        bad[0] = b'?';
        assert_eq!(parse(&bad, 0x12).unwrap_err(), FrameError::MissingDelimiter);
    }

    #[test]
    fn rejects_query_frame_as_response() {
        let query = b">?00021264";
        assert_eq!(parse(query, 0x12).unwrap_err(), FrameError::NotAResponse);
    }

    #[test]
    fn rejects_oversized_length_field() {
        // CCCC = "FFFF" = 65535, way past MAX_DATA_LEN
        let mut bad = Vec::from(&b">!FFFF12"[..]);
        bad.extend_from_slice(&[b'A'; 100]);
        bad.extend_from_slice(b"XX");
        assert_eq!(parse(&bad, 0x12).unwrap_err(), FrameError::LengthExceedsMax);
    }

    #[test]
    fn rejects_non_hex_length_field() {
        let bad = b">!ZZZZ1264";
        assert_eq!(parse(bad, 0x12).unwrap_err(), FrameError::BadLengthField);
    }
}
