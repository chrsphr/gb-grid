//! gb-grid-ingest CLI: Rust ingester for the GB grid BMRS data pipeline.

use anyhow::Result;
use clap::{Parser, Subcommand};

use gb_grid_ingest::config::Config;
use gb_grid_ingest::{constraints, ingest, materialize, scheduler};

#[derive(Parser)]
#[command(
    name = "gb-grid-ingest",
    version,
    about = "GB power grid energy database ingester (Rust)."
)]
struct Cli {
    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand)]
enum Command {
    /// Start the always-on async scheduler.
    Run,
    /// Backfill historical data into the database.
    Backfill {
        #[arg(long, help = "YYYY-MM-DD inclusive", value_name = "YYYY-MM-DD")]
        from: String,
        #[arg(long, help = "YYYY-MM-DD inclusive", value_name = "YYYY-MM-DD")]
        to: String,
        #[arg(
            short,
            long = "dataset",
            value_name = "DATASET",
            help = "Repeatable. Default: all."
        )]
        dataset: Vec<String>,
    },
    /// Recompute bmu_dispatch (only BMUs with BOA acceptances in the window).
    MaterializeDispatch {
        #[arg(long, help = "YYYY-MM-DD inclusive", value_name = "YYYY-MM-DD")]
        from: String,
        #[arg(long, help = "YYYY-MM-DD inclusive", value_name = "YYYY-MM-DD")]
        to: String,
        #[arg(
            short,
            long = "bmu",
            value_name = "BMU",
            help = "Repeatable. Default: all BMUs with PN data."
        )]
        bmu: Vec<String>,
        #[arg(short, long = "workers", help = "Worker task count. Default: ncpu-1.")]
        workers: Option<usize>,
    },
    /// Roll bmu_dispatch + b1610 up to per-BMU daily MWh totals.
    MaterializeDispatchDaily {
        #[arg(long, help = "YYYY-MM-DD inclusive", value_name = "YYYY-MM-DD")]
        from: String,
        #[arg(long, help = "YYYY-MM-DD inclusive", value_name = "YYYY-MM-DD")]
        to: String,
    },
    /// Refresh all continuous aggregates over a date window.
    RefreshCaggs {
        #[arg(long, help = "YYYY-MM-DD inclusive", value_name = "YYYY-MM-DD")]
        from: String,
        #[arg(long, help = "YYYY-MM-DD inclusive", value_name = "YYYY-MM-DD")]
        to: String,
    },
    /// Download and upsert the NESO day-ahead constraint flows CSV.
    RefreshConstraints,
    /// Print row counts and latest timestamps per table.
    Status,
}

#[tokio::main]
async fn main() -> Result<()> {
    let cli = Cli::parse();
    let cfg = Config::from_env()?;

    match cli.command {
        Command::Run => scheduler::run_scheduler(cfg).await,
        Command::Backfill { from, to, dataset } => {
            ingest::cmd_backfill(&cfg, &from, &to, &dataset).await?;
            println!("done.");
            Ok(())
        }
        Command::MaterializeDispatch {
            from,
            to,
            bmu,
            workers,
        } => {
            materialize::cmd_materialize_dispatch(&cfg, &from, &to, &bmu, workers).await
        }
        Command::MaterializeDispatchDaily { from, to } => {
            materialize::cmd_materialize_dispatch_daily(&cfg, &from, &to).await
        }
        Command::RefreshCaggs { from, to } => scheduler::cmd_refresh_caggs(&cfg, &from, &to).await,
        Command::RefreshConstraints => {
            let n = constraints::cmd_refresh_constraints(&cfg).await?;
            println!("wrote {n} rows");
            Ok(())
        }
        Command::Status => scheduler::cmd_status(&cfg).await,
    }
}