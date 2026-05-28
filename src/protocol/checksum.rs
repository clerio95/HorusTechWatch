//! DT214 checksum: sum of ASCII bytes from `P` to the last data byte,
//! take the low byte (drop the most-significant byte), format as
//! two uppercase ASCII hex digits.

pub fn compute(body: &[u8]) -> [u8; 2] {
    let sum: u32 = body.iter().map(|&b| b as u32).sum();
    let low = (sum & 0xFF) as u8;
    let hi = HEX_UPPER[(low >> 4) as usize];
    let lo = HEX_UPPER[(low & 0x0F) as usize];
    [hi, lo]
}

pub fn verify(body: &[u8], expected: [u8; 2]) -> bool {
    compute(body) == expected
}

const HEX_UPPER: &[u8; 16] = b"0123456789ABCDEF";

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn canonical_status_query() {
        // CLAUDE.md: >?00020162  →  checksum covers ?000201 = "62"
        assert_eq!(&compute(b"?000201"), b"62");
    }

    #[test]
    fn canonical_clock_query() {
        // CLAUDE.md: >?00020B73
        assert_eq!(&compute(b"?00020B"), b"73");
    }

    #[test]
    fn canonical_device_info_query() {
        // CLAUDE.md: >?00021264
        assert_eq!(&compute(b"?000212"), b"64");
    }

    #[test]
    fn canonical_diagnostics_query() {
        // CLAUDE.md: >?00041B5AEC
        assert_eq!(&compute(b"?00041B5A"), b"EC");
    }

    #[test]
    fn canonical_wireless_query() {
        // CLAUDE.md: >?00022568
        assert_eq!(&compute(b"?000225"), b"68");
    }

    #[test]
    fn verify_accepts_correct_checksum() {
        assert!(verify(b"?000201", *b"62"));
    }

    #[test]
    fn verify_rejects_wrong_checksum() {
        assert!(!verify(b"?000201", *b"00"));
        assert!(!verify(b"?000201", *b"63"));
    }
}
