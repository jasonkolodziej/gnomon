//! sinfo-compatible node listing.
use gnomon_api::pb::*;
mod common {
    include!("../common.rs");
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let mut client = common::connect().await?;
    let r = client.list_nodes(ListNodesRequest {}).await?.into_inner();
    println!(
        "{:>16} {:>8} {:>8} {:>10} {:>6}",
        "NODELIST", "CPUS", "GPUS", "MEM(MB)", "STATE"
    );
    for n in r.nodes {
        let c = n.capacity.unwrap_or_default();
        println!(
            "{:>16} {:>8} {:>8} {:>10} {:>6}",
            n.name,
            c.cores,
            c.gpus,
            c.mem_mb,
            if n.alive { "idle" } else { "down*" }
        );
    }
    Ok(())
}
