use anyhow::{Context, Result};
use sqlx::PgPool;

use crate::lag::ReplicaStatus;
use crate::lsn::parse_lsn;

/// Reads the primary's current WAL insert position. Only meaningful
/// (and only succeeds) against a real primary — a standby has no WAL
/// of its own to report here, and Postgres itself will error with
/// "recursive query is not supported" style text; the actual error
/// text from Postgres for this case is "recovery is in progress" (from
/// `pg_current_wal_lsn`'s doc: "cannot be executed during recovery"),
/// which is re-wrapped with a clearer hint in `main.rs`.
pub async fn current_wal_lsn(pool: &PgPool) -> Result<u64> {
    let (text,): (String,) = sqlx::query_as("SELECT pg_current_wal_lsn()::text")
        .fetch_one(pool)
        .await
        .context("reading pg_current_wal_lsn()")?;
    parse_lsn(&text)
}

/// Raw column tuple as returned by the `pg_stat_replication` query,
/// before reshaping into `ReplicaStatus`.
type ReplicationRow = (
    String,         // application_name
    Option<String>, // client_addr (may be NULL for a unix-socket connection)
    String,         // state
    Option<String>, // sent_lsn
    Option<String>, // replay_lsn
    Option<f64>,    // replay_lag in seconds
);

/// Reads every currently-connected replica's status from
/// `pg_stat_replication`. Returns an empty vec, not an error, when
/// nothing is connected — "no replicas" is a completely valid (if
/// perhaps unexpected) state for a Postgres server to be in, not a
/// failure of this query.
pub async fn fetch_replicas(pool: &PgPool) -> Result<Vec<ReplicaStatus>> {
    let rows: Vec<ReplicationRow> = sqlx::query_as(
        r#"
        SELECT
            application_name,
            client_addr::text,
            state,
            sent_lsn::text,
            replay_lsn::text,
            EXTRACT(EPOCH FROM replay_lag)
        FROM pg_stat_replication
        "#,
    )
    .fetch_all(pool)
    .await
    .context("reading pg_stat_replication")?;

    rows.into_iter()
        .map(
            |(application_name, client_addr, state, sent_lsn, replay_lsn, replay_lag_seconds)| {
                Ok(ReplicaStatus {
                    application_name,
                    client_addr,
                    state,
                    sent_lsn: sent_lsn.as_deref().map(parse_lsn).transpose()?,
                    replay_lsn: replay_lsn.as_deref().map(parse_lsn).transpose()?,
                    replay_lag_seconds,
                })
            },
        )
        .collect()
}
