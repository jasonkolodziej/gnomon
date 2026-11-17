//! Three replicated controllers over real TCP on localhost: writes from any
//! replica commit everywhere, plan read sets are revalidated identically,
//! the cluster survives losing its leader, and a restarted replica catches
//! up from its own disk plus the new leader's log.

use gnomon_consensus::replica::{self, Peers, Replica};
use gnomon_core::{Event, Job, JobId, JobState, ManualClock, Resources, Walltime};
use gnomon_fsm::Fsm;
use std::net::SocketAddr;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;
use tokio::net::TcpListener;

const NODES: u64 = 3;

fn dir(tag: &str, id: u64) -> PathBuf {
    let d = std::env::temp_dir().join(format!("gnomon-repl-{tag}-{}-{id}", std::process::id()));
    if id == 0 {
        let _ = std::fs::remove_dir_all(&d);
    }
    d
}

fn fingerprint(fsm: &Fsm) -> String {
    fsm.read(|st| format!("{st:?}"))
}

fn clock() -> Arc<ManualClock> {
    Arc::new(ManualClock::new(1_000))
}

struct Cluster {
    tag: &'static str,
    peers: Peers,
    replicas: Vec<Option<Replica>>,
}

impl Cluster {
    async fn start(tag: &'static str) -> Cluster {
        let mut listeners = Vec::new();
        let mut peers = Peers::new();
        for id in 1..=NODES {
            let _ = std::fs::remove_dir_all(dir(tag, id));
            let l = TcpListener::bind("127.0.0.1:0").await.unwrap();
            peers.insert(id, l.local_addr().unwrap().to_string());
            listeners.push(l);
        }
        let mut replicas = Vec::new();
        for (i, l) in listeners.into_iter().enumerate() {
            let id = i as u64 + 1;
            let r = replica::start(id, &peers, &dir(tag, id), l, clock())
                .await
                .unwrap();
            replicas.push(Some(r));
        }
        Cluster {
            tag,
            peers,
            replicas,
        }
    }

    fn live(&self) -> impl Iterator<Item = &Replica> {
        self.replicas.iter().flatten()
    }

    fn get(&self, id: u64) -> &Replica {
        self.replicas[(id - 1) as usize].as_ref().unwrap()
    }

    async fn leader(&self) -> u64 {
        for _ in 0..200 {
            if let Some(r) = self.live().find(|r| r.is_leader()) {
                return r.id;
            }
            tokio::time::sleep(Duration::from_millis(50)).await;
        }
        panic!("no leader elected");
    }

    fn follower_of(&self, leader: u64) -> u64 {
        self.live().map(|r| r.id).find(|&id| id != leader).unwrap()
    }

    /// Wait until every live replica has applied the same index; return
    /// their (equal) fingerprint.
    async fn converged(&self) -> String {
        for _ in 0..200 {
            let idx: Vec<u64> = self.live().map(|r| r.fsm.index()).collect();
            if idx.windows(2).all(|w| w[0] == w[1]) {
                let fps: Vec<String> = self.live().map(|r| fingerprint(&r.fsm)).collect();
                if fps.windows(2).all(|w| w[0] == w[1]) {
                    return fps[0].clone();
                }
            }
            tokio::time::sleep(Duration::from_millis(50)).await;
        }
        let idx: Vec<u64> = self.live().map(|r| r.fsm.index()).collect();
        panic!("replicas did not converge: indexes {idx:?}");
    }

    async fn stop(&mut self, id: u64) {
        let r = self.replicas[(id - 1) as usize].take().unwrap();
        r.stop().await;
    }

    async fn restart(&mut self, id: u64) {
        let addr: SocketAddr = self.peers[&id].parse().unwrap();
        let l = TcpListener::bind(addr).await.unwrap();
        let r = replica::start(id, &self.peers, &dir(self.tag, id), l, clock())
            .await
            .unwrap();
        self.replicas[(id - 1) as usize] = Some(r);
    }
}

/// Write through a replica's FSM from a blocking thread (the scheduler
/// threads' situation).
async fn write(fsm: &Arc<Fsm>, ev: Event) -> gnomon_fsm::Applied {
    let fsm = fsm.clone();
    tokio::task::spawn_blocking(move || fsm.try_apply(ev).expect("committed"))
        .await
        .unwrap()
}

fn node(name: &str) -> Event {
    Event::NodeRegistered {
        name: name.into(),
        capacity: Resources {
            cores: 4,
            mem_mb: 0,
            gpus: 0,
        },
    }
}

fn submit(id: u64) -> Event {
    Event::JobSubmitted(Job::new(
        JobId(id),
        "j",
        "u",
        Resources {
            cores: 4,
            mem_mb: 0,
            gpus: 0,
        },
        Walltime(Duration::from_secs(600)),
    ))
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn replicas_converge_forward_writes_and_survive_leader_failover() {
    let mut c = Cluster::start("failover").await;
    let leader = c.leader().await;
    let follower = c.follower_of(leader);

    // A follower's write is forwarded, committed, and visible to it at once.
    let a = write(&c.get(follower).fsm, node("n0")).await;
    assert!(a.accepted);
    assert_eq!(c.get(follower).fsm.index(), a.index, "read your writes");
    assert!(write(&c.get(leader).fsm, submit(1)).await.accepted);
    // From an async task too (API handlers run on runtime workers).
    let fsm = c.get(follower).fsm.clone();
    tokio::spawn(async move { assert!(fsm.apply_checked(submit(2)).accepted) })
        .await
        .unwrap();
    c.converged().await;

    // Two plans computed against the same read set for the one 4-core node:
    // the first commits, the second conflicts. Every replica revalidates the
    // logged read set against identical state, so all agree.
    let read_set = c.get(follower).fsm.read(|s| s.graph.versions(&[1]));
    let start = |job| Event::JobStarted {
        job: JobId(job),
        vertices: vec![1],
        start: 1_000,
        dur: None,
    };
    let fsm = c.get(follower).fsm.clone();
    let (first, second) = tokio::task::spawn_blocking(move || {
        (
            fsm.apply_with_expect(start(1), &read_set),
            fsm.apply_with_expect(start(2), &read_set),
        )
    })
    .await
    .unwrap();
    assert!(first.accepted && !second.accepted);
    c.converged().await;
    for r in c.live() {
        assert_eq!(r.fsm.read(|s| s.jobs[&JobId(1)].state), JobState::Running);
        assert_eq!(r.fsm.read(|s| s.jobs[&JobId(2)].state), JobState::Pending);
    }

    // Duplicate job ids are rejected on every replica (the API retries).
    assert!(!write(&c.get(leader).fsm, submit(1)).await.accepted);

    // Lose the leader: the survivors elect a new one and keep committing.
    c.stop(leader).await;
    let new_leader = c.leader().await;
    assert_ne!(new_leader, leader);
    let survivor = c.follower_of(new_leader);
    assert!(
        write(&c.get(survivor).fsm, Event::JobCancelled(JobId(2)))
            .await
            .accepted
    );
    assert!(write(&c.get(new_leader).fsm, submit(3)).await.accepted);
    let two = c.converged().await;
    assert!(two.contains("Cancelled"));

    // The old leader restarts from its own disk and catches up.
    c.restart(leader).await;
    let three = c.converged().await;
    assert_eq!(three, two, "the restarted replica matches the survivors");
    for r in c.live() {
        assert_eq!(r.fsm.next_job_id(), JobId(4));
    }
    for id in 1..=NODES {
        c.stop(id).await;
        let _ = std::fs::remove_dir_all(dir(c.tag, id));
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn writes_fail_cleanly_without_a_quorum() {
    let mut c = Cluster::start("quorum").await;
    let leader = c.leader().await;
    let follower = c.follower_of(leader);
    assert!(write(&c.get(follower).fsm, node("n0")).await.accepted);
    // Two of three down: no majority, so writes must fail, not hang or fork.
    let lone = c.follower_of(leader);
    for id in (1..=NODES).filter(|&id| id != lone) {
        c.stop(id).await;
    }
    let fsm = c.get(lone).fsm.clone();
    let before = fsm.index();
    let err = tokio::task::spawn_blocking(move || fsm.try_apply(node("n1")))
        .await
        .unwrap();
    assert!(err.is_err(), "a minority must not commit");
    assert_eq!(c.get(lone).fsm.index(), before);
    for id in 1..=NODES {
        if c.replicas[(id - 1) as usize].is_some() {
            c.stop(id).await;
        }
        let _ = std::fs::remove_dir_all(dir(c.tag, id));
    }
}
