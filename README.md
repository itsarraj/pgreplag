# pgreplag

Reports live Postgres physical replication lag in bytes and seconds from
`pg_stat_replication` — the query every Postgres DBA has typed by hand
at some point, as a proper CLI with configurable warn/critical
thresholds for a monitoring check.

## Usage

```bash
pgreplag "postgres://user@primary-host/dbname"
pgreplag "postgres://user@primary-host/dbname" --warn-bytes 16777216 --critical-bytes 134217728
```

Connects to what should be the **primary** and reads `pg_stat_replication`
(one row per connected replica) plus `pg_current_wal_lsn()`. Exit code
reflects the worst severity found across all replicas: `0` OK, `1` warn,
`2` critical — `cron`/monitoring-check friendly.

## What it computes

Byte lag is `pg_current_wal_lsn() - replay_lsn` for each replica — real
LSN (Log Sequence Number) arithmetic, not a string comparison (`LSN`s
are logically `<file>/<offset>` pairs, e.g. `2/A3F2C100`, and computing a
byte distance between two of them means real 64-bit arithmetic on the
combined value, not comparing the text). Time lag comes straight from
`replay_lag` (an `interval`) when Postgres reports one.

## Status: built and verified, including a real check against a live (non-replicated) Postgres

- **22 unit tests** (`cargo test --lib`) across `lsn` (12 — parsing and
  formatting the real `<hex>/<hex>` LSN text format, byte-distance
  arithmetic between two LSNs including across a file-segment boundary,
  and round-tripping a parsed LSN back to its original text form),
  `lag` (8 — severity classification against the warn/critical
  thresholds in both bytes and seconds, worst-of-both-dimensions
  selection), and `format` (2 — human-readable byte-size formatting).
- **Live-verified against a real local Postgres**: ran the actual
  compiled binary against a real scratch database with no replicas
  connected — it correctly printed the real, actually-current
  `pg_current_wal_lsn()` value (not a placeholder) and reported "no
  replicas connected (pg_stat_replication is empty)" with exit code `0`.
  Standing up genuine streaming replication (a second real standby
  server) is heavyweight infrastructure this pass didn't attempt — the
  no-replica path and all LSN/severity arithmetic are the parts
  genuinely exercised against something real; the "N replicas each with
  their own realistic lag" reporting path is verified only by unit tests
  against synthetic LSN/interval values, not a real streaming replica.
  Stated plainly rather than implied.

**Not done / deliberately deferred**: logical replication slots
(`pg_replication_slots`, a different lag concept — bytes of WAL retained
for a slot, not applied-vs-current lag for a streaming standby) — this
only covers physical streaming replication; and cascading replication
topologies (a replica of a replica) — this only ever queries the node
you point it at directly, it doesn't walk a multi-tier topology.
