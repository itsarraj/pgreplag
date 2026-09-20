use std::process::ExitCode;

use clap::Parser;
use pgreplag::format::format_bytes;
use pgreplag::lag::{evaluate, Severity, Thresholds};
use pgreplag::lsn::format_lsn;
use pgreplag::query::{current_wal_lsn, fetch_replicas};
use sqlx::postgres::PgPoolOptions;

#[derive(Parser)]
#[command(
    name = "pgreplag",
    about = "Reports live Postgres physical replication lag in bytes and seconds"
)]
struct Cli {
    /// Connection string for the primary.
    database_url: String,

    #[arg(long, default_value_t = 16 * 1024 * 1024)]
    warn_bytes: u64,

    #[arg(long, default_value_t = 128 * 1024 * 1024)]
    critical_bytes: u64,

    #[arg(long, default_value_t = 30.0)]
    warn_seconds: f64,

    #[arg(long, default_value_t = 300.0)]
    critical_seconds: f64,
}

#[tokio::main]
async fn main() -> ExitCode {
    let cli = Cli::parse();

    let pool = match PgPoolOptions::new()
        .max_connections(2)
        .connect(&cli.database_url)
        .await
    {
        Ok(p) => p,
        Err(e) => {
            eprintln!("pgreplag: could not connect: {e:#}");
            return ExitCode::FAILURE;
        }
    };

    let current = match current_wal_lsn(&pool).await {
        Ok(lsn) => lsn,
        Err(e) => {
            eprintln!(
                "pgreplag: could not read pg_current_wal_lsn(): {e:#}\n\
                 (this fails if you pointed pgreplag at a standby instead of the primary -\n\
                 pg_current_wal_lsn() only works on a server that isn't itself in recovery)"
            );
            return ExitCode::FAILURE;
        }
    };

    let replicas = match fetch_replicas(&pool).await {
        Ok(r) => r,
        Err(e) => {
            eprintln!("pgreplag: could not read pg_stat_replication: {e:#}");
            return ExitCode::FAILURE;
        }
    };

    println!("primary WAL position: {}", format_lsn(current));

    if replicas.is_empty() {
        println!("no replicas connected (pg_stat_replication is empty)");
        return ExitCode::SUCCESS;
    }

    let thresholds = Thresholds {
        warn_bytes: cli.warn_bytes,
        critical_bytes: cli.critical_bytes,
        warn_seconds: cli.warn_seconds,
        critical_seconds: cli.critical_seconds,
    };

    let mut worst = Severity::Ok;
    for replica in &replicas {
        let finding = evaluate(current, replica, &thresholds);
        let lag_str = finding
            .lag_bytes
            .map(format_bytes)
            .unwrap_or_else(|| "unknown (no replay_lsn yet)".to_string());
        println!(
            "[{}] {} ({}) state={} lag={}",
            finding.severity.label(),
            replica.application_name,
            replica.client_addr.as_deref().unwrap_or("unix socket"),
            replica.state,
            lag_str,
        );
        for reason in &finding.reasons {
            println!("       - {reason}");
        }
        if finding.severity > worst {
            worst = finding.severity;
        }
    }

    if worst == Severity::Critical {
        ExitCode::FAILURE
    } else {
        ExitCode::SUCCESS
    }
}
