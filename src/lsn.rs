//! Parsing and arithmetic for Postgres's `pg_lsn` type.
//!
//! A log sequence number is textually `"<hi>/<lo>"`, each half an
//! unsigned hex number, together forming a 64-bit byte offset into the
//! write-ahead log stream (`hi` is the high 32 bits, `lo` the low 32
//! bits — see the Postgres source's `pg_lsn_in`/`pg_lsn_out`). Since
//! it's a plain byte offset, the difference between two LSNs *is* the
//! number of WAL bytes between them, which is exactly what "replication
//! lag in bytes" means: how many bytes of WAL the primary has generated
//! that a given replica hasn't replayed yet.

use anyhow::{anyhow, Result};

/// Parses a Postgres `pg_lsn` string like `"16/B374D848"` into its
/// 64-bit integer value.
pub fn parse_lsn(s: &str) -> Result<u64> {
    let (hi, lo) = s
        .trim()
        .split_once('/')
        .ok_or_else(|| anyhow!("'{s}' is not a valid LSN (expected HEX/HEX)"))?;
    let hi =
        u64::from_str_radix(hi, 16).map_err(|e| anyhow!("'{s}' has an invalid high half: {e}"))?;
    let lo =
        u64::from_str_radix(lo, 16).map_err(|e| anyhow!("'{s}' has an invalid low half: {e}"))?;
    if hi > u32::MAX as u64 {
        return Err(anyhow!("'{s}' has a high half wider than 32 bits"));
    }
    if lo > u32::MAX as u64 {
        return Err(anyhow!("'{s}' has a low half wider than 32 bits"));
    }
    Ok((hi << 32) | lo)
}

/// Renders a 64-bit LSN value back to Postgres's own `"HEX/HEX"` text
/// form (uppercase hex, no leading zeros on either half — matching
/// `pg_lsn_out`).
pub fn format_lsn(value: u64) -> String {
    format!("{:X}/{:X}", value >> 32, value & 0xFFFF_FFFF)
}

/// Bytes of WAL between `ahead` and `behind`. Saturates at zero rather
/// than underflowing/panicking if `behind` is somehow ahead of `ahead`
/// (e.g. a stats snapshot race where the replica's own reported
/// position momentarily looks newer than the primary's) — reporting
/// "0 bytes behind" is the safe, sane answer for that case, not a
/// crash.
pub fn lag_bytes(ahead: u64, behind: u64) -> u64 {
    ahead.saturating_sub(behind)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_lsn_basic() {
        assert_eq!(parse_lsn("0/0").unwrap(), 0);
        assert_eq!(parse_lsn("0/1").unwrap(), 1);
        assert_eq!(parse_lsn("1/0").unwrap(), 0x1_0000_0000);
    }

    #[test]
    fn parse_lsn_realistic_value() {
        // A real-looking LSN as Postgres actually prints it.
        assert_eq!(
            parse_lsn("16/B374D848").unwrap(),
            (0x16u64 << 32) | 0xB374D848
        );
    }

    #[test]
    fn parse_lsn_lowercase_hex_accepted() {
        assert_eq!(
            parse_lsn("16/b374d848").unwrap(),
            (0x16u64 << 32) | 0xB374D848
        );
    }

    #[test]
    fn parse_lsn_rejects_missing_slash() {
        assert!(parse_lsn("16B374D848").is_err());
    }

    #[test]
    fn parse_lsn_rejects_non_hex() {
        assert!(parse_lsn("zz/11").is_err());
        assert!(parse_lsn("11/zz").is_err());
    }

    #[test]
    fn parse_lsn_rejects_overflowing_half() {
        assert!(parse_lsn("100000000/0").is_err()); // hi > u32::MAX
        assert!(parse_lsn("0/100000000").is_err()); // lo > u32::MAX
    }

    #[test]
    fn format_lsn_roundtrips() {
        for s in ["0/0", "16/B374D848", "FFFFFFFF/FFFFFFFF", "A/1"] {
            let v = parse_lsn(s).unwrap();
            let back = format_lsn(v);
            assert_eq!(parse_lsn(&back).unwrap(), v, "roundtrip mismatch for {s}");
        }
    }

    #[test]
    fn format_lsn_matches_postgres_style() {
        assert_eq!(format_lsn(0), "0/0");
        assert_eq!(format_lsn((0x16u64 << 32) | 0xB374D848), "16/B374D848");
    }

    #[test]
    fn lag_bytes_basic() {
        let a = parse_lsn("0/1000").unwrap();
        let b = parse_lsn("0/100").unwrap();
        assert_eq!(lag_bytes(a, b), 0xF00);
    }

    #[test]
    fn lag_bytes_zero_when_equal() {
        let a = parse_lsn("5/AAAA").unwrap();
        assert_eq!(lag_bytes(a, a), 0);
    }

    #[test]
    fn lag_bytes_saturates_instead_of_underflowing() {
        let ahead = parse_lsn("0/100").unwrap();
        let behind = parse_lsn("0/200").unwrap();
        assert_eq!(lag_bytes(ahead, behind), 0);
    }

    #[test]
    fn lag_bytes_crosses_a_segment_boundary() {
        // Crossing from one "hi" segment into the next is exactly the
        // case that would break a naive "just diff the low halves"
        // implementation.
        let ahead = parse_lsn("1/00000010").unwrap();
        let behind = parse_lsn("0/FFFFFFF0").unwrap();
        assert_eq!(lag_bytes(ahead, behind), 0x20);
    }
}
