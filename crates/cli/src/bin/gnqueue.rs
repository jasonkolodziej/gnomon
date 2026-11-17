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

/// START = planned/actual start (epoch s, 0 = not placed yet); TIME = limit
/// (s); CPUS = per node; NODES = gang size; NODELIST = placed nodes, as in
/// squeue (empty while pending).
fn header() -> String {
    format!(
        "{:>8} {:>10} {:>12} {:>16} {:>5} {:>5} {:>8} {:>12} {}",
        "JOBID", "USER", "STATE", "NAME", "CPUS", "NODES", "TIME", "START", "NODELIST"
    )
}

fn row(j: &JobInfo) -> String {
    let cpus = j.request.map(|r| r.cores).unwrap_or(0);
    format!(
        "{:>8} {:>10} {:>12} {:>16} {:>5} {:>5} {:>8} {:>12} {}",
        j.id,
        j.user,
        j.state,
        j.name,
        cpus,
        j.num_nodes.max(1),
        j.walltime_secs,
        j.planned_start,
        j.nodes.join(",")
    )
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
    println!("{}", header());
    for j in &r.jobs {
        println!("{}", row(j));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rows_show_node_count_and_nodelist() {
        let gang = JobInfo {
            id: 7,
            name: "mpi".into(),
            user: "alice".into(),
            state: "RUNNING".into(),
            request: Some(Resources {
                cores: 8,
                mem_mb: 0,
                gpus: 0,
            }),
            walltime_secs: 600,
            planned_start: 100,
            nodes: vec!["n1".into(), "n2".into()],
            num_nodes: 2,
            ..Default::default()
        };
        let line = row(&gang);
        let cols: Vec<&str> = line.split_whitespace().collect();
        let header = header();
        let head: Vec<&str> = header.split_whitespace().collect();
        assert_eq!(cols.len(), head.len());
        let col = |name: &str| cols[head.iter().position(|h| *h == name).unwrap()];
        assert_eq!(col("NODES"), "2");
        assert_eq!(col("CPUS"), "8");
        assert_eq!(col("NODELIST"), "n1,n2");
        // Records from an older controller (num_nodes unset) show one node.
        let old = JobInfo {
            num_nodes: 0,
            nodes: vec![],
            ..gang
        };
        assert!(row(&old).contains(" 1 "));
    }
}
