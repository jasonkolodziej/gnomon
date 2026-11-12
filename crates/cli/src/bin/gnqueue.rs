//! squeue-compatible job listing.
use clap::Parser;
use gnomon_api::pb::*;
mod common {
    include!("../common.rs");
}

#[derive(Parser)]
struct Args {
    #[arg(short, long, default_value = "")]
    user: String,
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let a = Args::parse();
    let mut client = common::connect().await?;
    let r = client
        .list_jobs(ListJobsRequest {
            user: a.user,
            state: String::new(),
        })
        .await?
        .into_inner();
    // START = planned/actual start (epoch s, 0 = not placed yet); TIME = limit (s).
    println!(
        "{:>8} {:>10} {:>12} {:>16} {:>5} {:>8} {:>12}",
        "JOBID", "USER", "STATE", "NAME", "CPUS", "TIME", "START"
    );
    for j in r.jobs {
        let cpus = j.request.map(|r| r.cores).unwrap_or(0);
        println!(
            "{:>8} {:>10} {:>12} {:>16} {:>5} {:>8} {:>12}",
            j.id, j.user, j.state, j.name, cpus, j.walltime_secs, j.planned_start
        );
    }
    Ok(())
}
