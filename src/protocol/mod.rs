//! Horustech DT214 frame protocol — query side only.
//!
//! SAFETY: the [`Command`] enum is the **only** way to construct a query frame.
//! No raw-byte send path exists. The variants in this enum are the only command
//! indices this binary will ever transmit. Adding a write-side command
//! (e.g. 0x06 increment, 0x02 fuel-delivery read, 0x1A config write) requires
//! editing this file — and is forbidden by project policy because we share the
//! port-2001 fuel-delivery read pointer with the live POS (DT432 §5.2).

pub mod checksum;
pub mod frame;

/// The complete set of commands this binary may send.
///
/// Health-check / read-only queries only. Anything not in this enum cannot be
/// serialized for transmission.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Command {
    Status,
    Clock,
    DeviceInfo,
    Diagnostics,
    Wireless,
}

impl Command {
    pub const ALL: [Command; 5] = [
        Command::Status,
        Command::Clock,
        Command::DeviceInfo,
        Command::Diagnostics,
        Command::Wireless,
    ];

    pub fn index(self) -> u8 {
        match self {
            Command::Status => 0x01,
            Command::Clock => 0x0B,
            Command::DeviceInfo => 0x12,
            Command::Diagnostics => 0x1B,
            Command::Wireless => 0x25,
        }
    }

    pub fn name(self) -> &'static str {
        match self {
            Command::Status => "status",
            Command::Clock => "clock",
            Command::DeviceInfo => "device_info",
            Command::Diagnostics => "diagnostics",
            Command::Wireless => "wireless",
        }
    }

    /// Extra data bytes after the command index. Diagnostics carries a
    /// pump-bitmap parameter (0x5A) per the canonical frame in CLAUDE.md.
    fn extra_data(self) -> &'static [u8] {
        match self {
            Command::Diagnostics => b"5A",
            _ => b"",
        }
    }

    /// Build the full wire bytes for the query frame:
    ///   `>` + `?` + CCCC + data + KK
    /// where data = `XX` (command index hex) + optional parameters.
    pub fn query_frame(self) -> Vec<u8> {
        let idx_hex = [HEX_UPPER[(self.index() >> 4) as usize], HEX_UPPER[(self.index() & 0x0F) as usize]];
        let extra = self.extra_data();

        let data_len = idx_hex.len() + extra.len();
        let cccc = format_u16_hex4(data_len as u16);

        // Body covered by checksum: P + CCCC + data
        let mut body = Vec::with_capacity(1 + 4 + data_len);
        body.push(b'?');
        body.extend_from_slice(&cccc);
        body.extend_from_slice(&idx_hex);
        body.extend_from_slice(extra);

        let cksum = checksum::compute(&body);

        let mut frame = Vec::with_capacity(1 + body.len() + 2);
        frame.push(b'>');
        frame.extend_from_slice(&body);
        frame.extend_from_slice(&cksum);
        frame
    }
}

fn format_u16_hex4(n: u16) -> [u8; 4] {
    [
        HEX_UPPER[((n >> 12) & 0x0F) as usize],
        HEX_UPPER[((n >> 8) & 0x0F) as usize],
        HEX_UPPER[((n >> 4) & 0x0F) as usize],
        HEX_UPPER[(n & 0x0F) as usize],
    ]
}

const HEX_UPPER: &[u8; 16] = b"0123456789ABCDEF";

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn allowlist_contains_only_safe_commands() {
        // If a write command (0x02, 0x06, 0x0A, 0x1A, etc.) ever appears here,
        // this test is the canary. Update with great care and re-read DT432 §5.2.
        let indices: Vec<u8> = Command::ALL.iter().map(|c| c.index()).collect();
        assert_eq!(indices, vec![0x01, 0x0B, 0x12, 0x1B, 0x25]);
    }

    #[test]
    fn status_query_matches_canonical_frame() {
        assert_eq!(Command::Status.query_frame(), b">?00020162".to_vec());
    }

    #[test]
    fn clock_query_matches_canonical_frame() {
        assert_eq!(Command::Clock.query_frame(), b">?00020B73".to_vec());
    }

    #[test]
    fn device_info_query_matches_canonical_frame() {
        assert_eq!(Command::DeviceInfo.query_frame(), b">?00021264".to_vec());
    }

    #[test]
    fn diagnostics_query_matches_canonical_frame() {
        assert_eq!(Command::Diagnostics.query_frame(), b">?00041B5AEC".to_vec());
    }

    #[test]
    fn wireless_query_matches_canonical_frame() {
        assert_eq!(Command::Wireless.query_frame(), b">?00022568".to_vec());
    }
}
