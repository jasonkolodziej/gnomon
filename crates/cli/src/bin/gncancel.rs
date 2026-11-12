//! scancel-compatible cancellation.
use clap::Parser;
use gnomon_api::pb::*;
mod common {
    include!("../common.rs");
}

#[derive(Parser)]
struct Args {
    job_id: u64,
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let a = Args::parse();
    let mut client = common::connect().await?;
    client
        .cancel_job(CancelJobRequest {
            job_id: a.job_id,
            user: std::env::var("USER").unwrap_or_default(),
        })
        .await?;
    Ok(()) // scancel prints nothing on success
}
