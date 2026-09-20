use crate::lsn::lag_bytes;

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum Severity {
    Ok,
    Warning,
    Critical,
}

impl Severity {
    pub fn label(&self) -> &'static str {
        match self {
            Severity::Ok => "OK",
            Severity::Warning => "WARN",
            Severity::Critical => "CRIT",
        }
    }
}

/// One row of `pg_stat_replication`, already decoded into Rust types.
/// `replay_lsn` and `replay_lag_seconds` are `Option` because both can
/// genuinely be `NULL` — `replay_lsn` before a brand new replica has
/// replayed anything at all, and `replay_lag_seconds` whenever the
/// standby hasn't sent a timing feedback message recently enough for
/// Postgres to compute it (this is normal, not an error condition).
#[derive(Debug, Clone)]
pub struct ReplicaStatus {
    pub application_name: String,
    pub client_addr: Option<String>,
    pub state: String,
    pub sent_lsn: Option<u64>,
    pub replay_lsn: Option<u64>,
    pub replay_lag_seconds: Option<f64>,
}

#[derive(Debug, Clone, Copy)]
pub struct Thresholds {
    pub warn_bytes: u64,
    pub critical_bytes: u64,
    pub warn_seconds: f64,
    pub critical_seconds: f64,
}

impl Default for Thresholds {
    fn default() -> Self {
        Thresholds {
            warn_bytes: 16 * 1024 * 1024,      // 16 MiB
            critical_bytes: 128 * 1024 * 1024, // 128 MiB
            warn_seconds: 30.0,
            critical_seconds: 300.0,
        }
    }
}

pub struct Finding {
    pub lag_bytes: Option<u64>,
    pub severity: Severity,
    pub reasons: Vec<String>,
}

/// Evaluates one replica's lag against `current_wal_lsn` (the
/// primary's own current WAL insert position at the moment of the
/// check) and the given thresholds.
///
/// Byte lag and time lag are independent signals and either can trip
/// the verdict on its own — a replica can be caught up on wall-clock
/// time but still measurably behind in bytes right after a burst of
/// writes, or vice versa if it's a slow network link with steady but
/// delayed replay.
pub fn evaluate(current_wal_lsn: u64, replica: &ReplicaStatus, thresholds: &Thresholds) -> Finding {
    let mut severity = Severity::Ok;
    let mut reasons = Vec::new();

    let bytes = replica.replay_lsn.map(|r| lag_bytes(current_wal_lsn, r));

    if let Some(b) = bytes {
        if b >= thresholds.critical_bytes {
            reasons.push(format!(
                "{} behind in WAL bytes (>= {} critical threshold)",
                crate::format::format_bytes(b),
                crate::format::format_bytes(thresholds.critical_bytes)
            ));
            severity = severity.max(Severity::Critical);
        } else if b >= thresholds.warn_bytes {
            reasons.push(format!(
                "{} behind in WAL bytes (>= {} warning threshold)",
                crate::format::format_bytes(b),
                crate::format::format_bytes(thresholds.warn_bytes)
            ));
            severity = severity.max(Severity::Warning);
        }
    }

    if let Some(s) = replica.replay_lag_seconds {
        if s >= thresholds.critical_seconds {
            reasons.push(format!(
                "{:.1}s replay lag (>= {:.1}s critical threshold)",
                s, thresholds.critical_seconds
            ));
            severity = severity.max(Severity::Critical);
        } else if s >= thresholds.warn_seconds {
            reasons.push(format!(
                "{:.1}s replay lag (>= {:.1}s warning threshold)",
                s, thresholds.warn_seconds
            ));
            severity = severity.max(Severity::Warning);
        }
    }

    Finding {
        lag_bytes: bytes,
        severity,
        reasons,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::lsn::parse_lsn;

    fn replica_at(lsn: &str) -> ReplicaStatus {
        ReplicaStatus {
            application_name: "standby1".to_string(),
            client_addr: Some("10.0.0.2".to_string()),
            state: "streaming".to_string(),
            sent_lsn: Some(parse_lsn(lsn).unwrap()),
            replay_lsn: Some(parse_lsn(lsn).unwrap()),
            replay_lag_seconds: None,
        }
    }

    #[test]
    fn caught_up_replica_is_ok() {
        let current = parse_lsn("0/1000").unwrap();
        let replica = replica_at("0/1000");
        let finding = evaluate(current, &replica, &Thresholds::default());
        assert_eq!(finding.severity, Severity::Ok);
        assert_eq!(finding.lag_bytes, Some(0));
        assert!(finding.reasons.is_empty());
    }

    #[test]
    fn small_byte_lag_under_warn_threshold_is_ok() {
        let current = parse_lsn("0/1000000").unwrap();
        let replica = replica_at("0/FF0000"); // ~64KB behind, well under 16MiB
        let finding = evaluate(current, &replica, &Thresholds::default());
        assert_eq!(finding.severity, Severity::Ok);
    }

    #[test]
    fn byte_lag_above_warn_threshold_warns() {
        let thresholds = Thresholds {
            warn_bytes: 100,
            critical_bytes: 10_000,
            ..Thresholds::default()
        };
        let current = parse_lsn("0/200").unwrap();
        let replica = replica_at("0/50"); // 0x1B0 = 432 bytes behind
        let finding = evaluate(current, &replica, &thresholds);
        assert_eq!(finding.severity, Severity::Warning);
        assert_eq!(finding.reasons.len(), 1);
    }

    #[test]
    fn byte_lag_above_critical_threshold_is_critical() {
        let thresholds = Thresholds {
            warn_bytes: 10,
            critical_bytes: 100,
            ..Thresholds::default()
        };
        let current = parse_lsn("0/1000").unwrap();
        let replica = replica_at("0/10");
        let finding = evaluate(current, &replica, &thresholds);
        assert_eq!(finding.severity, Severity::Critical);
    }

    #[test]
    fn time_lag_independently_trips_severity() {
        let thresholds = Thresholds::default();
        let current = parse_lsn("0/1000").unwrap();
        let mut replica = replica_at("0/1000"); // zero byte lag
        replica.replay_lag_seconds = Some(400.0); // above critical_seconds
        let finding = evaluate(current, &replica, &thresholds);
        assert_eq!(finding.severity, Severity::Critical);
        assert_eq!(finding.reasons.len(), 1);
        assert!(finding.reasons[0].contains("replay lag"));
    }

    #[test]
    fn both_byte_and_time_lag_can_fire_together() {
        let thresholds = Thresholds {
            warn_bytes: 10,
            critical_bytes: 100,
            warn_seconds: 1.0,
            critical_seconds: 10.0,
        };
        let current = parse_lsn("0/1000").unwrap();
        let mut replica = replica_at("0/10");
        replica.replay_lag_seconds = Some(20.0);
        let finding = evaluate(current, &replica, &thresholds);
        assert_eq!(finding.severity, Severity::Critical);
        assert_eq!(finding.reasons.len(), 2);
    }

    #[test]
    fn null_replay_lsn_reports_no_byte_lag_rather_than_erroring() {
        let current = parse_lsn("0/1000").unwrap();
        let mut replica = replica_at("0/1000");
        replica.replay_lsn = None;
        let finding = evaluate(current, &replica, &Thresholds::default());
        assert_eq!(finding.lag_bytes, None);
        assert_eq!(finding.severity, Severity::Ok);
    }

    #[test]
    fn severity_ordering() {
        assert!(Severity::Critical > Severity::Warning);
        assert!(Severity::Warning > Severity::Ok);
    }
}
