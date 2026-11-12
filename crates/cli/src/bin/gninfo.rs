//! sinfo-compatible node listing.
use gnomon_api::pb::*;
mod common {
    include!("../common.rs");
}

fn node_state(alive: bool, capacity: &Resources, allocated: &Resources) -> &'static str {
    if !alive {
        "down*"
    } else if allocated.cores == 0 && allocated.gpus == 0 && allocated.mem_mb == 0 {
        "idle"
    } else if allocated.cores >= capacity.cores
        && allocated.gpus >= capacity.gpus
        && allocated.mem_mb >= capacity.mem_mb
    {
        "alloc"
    } else {
        "mix"
    }
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
        let allocated = n.allocated.unwrap_or_default();
        println!(
            "{:>16} {:>8} {:>8} {:>10} {:>6}",
            n.name,
            c.cores,
            c.gpus,
            c.mem_mb,
            node_state(n.alive, &c, &allocated)
        );
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn resources(cores: u32, mem_mb: u64, gpus: u32) -> Resources {
        Resources {
            cores,
            mem_mb,
            gpus,
        }
    }

    #[test]
    fn node_state_reflects_liveness_and_allocation() {
        let capacity = resources(8, 16_384, 2);

        assert_eq!(node_state(true, &capacity, &resources(0, 0, 0)), "idle");
        assert_eq!(node_state(true, &capacity, &resources(4, 8_192, 1)), "mix");
        assert_eq!(
            node_state(true, &capacity, &resources(8, 16_384, 2)),
            "alloc"
        );
        assert_eq!(
            node_state(false, &capacity, &resources(4, 8_192, 1)),
            "down*"
        );
    }
}
