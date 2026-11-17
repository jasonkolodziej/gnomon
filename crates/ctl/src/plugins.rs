//! Controller policy configuration: declarative policy and WASM plugins
//! (ADR 0002, 0003, 0004).
//!
//! | Flag | Environment | Plugs into |
//! |---|---|---|
//! | `--admission-plugin` | `GNOMON_ADMISSION_PLUGIN` | submit: accept, reject or rewrite |
//! | `--admission-fail open\|closed` | `GNOMON_ADMISSION_FAIL` | what a failed admission call does |
//! | `--priority-plugin` | `GNOMON_PRIORITY_PLUGIN` | submit: priority |
//! | `--policy-config` | `GNOMON_POLICY_CONFIG` | submit: declarative priority / fallback |
//! | `--shadow-priority-plugin` | `GNOMON_SHADOW_PRIORITY_PLUGIN` | submit: compared, never applied |
//! | `--estimator-plugin` | `GNOMON_ESTIMATOR_PLUGIN` | planning: runtime estimate |
//! | `--node-score-plugin` | `GNOMON_NODE_SCORE_PLUGIN` | planning: node choice |
//! | `--node-score-candidates N\|all` | `GNOMON_NODE_SCORE_CANDIDATES` | nodes offered per call (default 64) |
//! | `--plugin-fuel`, `--plugin-memory-mb` | `GNOMON_PLUGIN_FUEL`, `GNOMON_PLUGIN_MEMORY_MB` | every call |
//!
//! A plugin or config that does not load stops startup: every replica must
//! run the same policy.

use super::flag;
use gnomon_api::SubmitPolicy;
use gnomon_core::Job;
use gnomon_policy::{
    AdmissionHook, AdmissionPlugin, EstimatorPlugin, FailMode, NodeScorePlugin, PluginLimits,
    PolicyConfig, PriorityHook, PriorityPlugin, ScoredPolicy, ShadowOutcome, ShadowPriority,
    WasmEstimator, DEFAULT_MAX_CANDIDATES,
};
use gnomon_sched::{BackfillPolicy, Policy};
use std::sync::atomic::Ordering;
use std::sync::Arc;

/// Reservation window of the controller's backfill policy.
pub const WINDOW_SECS: u64 = 48 * 3600;

/// What the controller runs: the submit handler's policy (`None` keeps
/// client priorities and accepts everything) and the scheduling policy.
pub struct Plugins {
    pub submit: Option<Arc<dyn SubmitPolicy>>,
    pub policy: Arc<dyn Policy>,
    /// One line per configured piece, for the startup log.
    pub summary: Vec<String>,
}

/// Load everything the flags and environment ask for.
pub fn load(args: &[String], env: impl Fn(&str) -> Option<String>) -> Result<Plugins, String> {
    let get = |name: &str, var: &str| flag(args, name, env(var));
    let limits = limits(args, &env)?;
    let mut summary = Vec::new();

    let admission = match get("admission-plugin", "GNOMON_ADMISSION_PLUGIN")? {
        Some(path) => {
            let fail: FailMode = get("admission-fail", "GNOMON_ADMISSION_FAIL")?
                .as_deref()
                .unwrap_or("open")
                .parse()
                .map_err(|e| format!("--admission-fail: {e}"))?;
            let p = AdmissionPlugin::from_file(&path, limits)
                .map_err(|e| format!("--admission-plugin {path}: {e}"))?;
            summary.push(format!("admission plugin {path} (fail {fail:?})"));
            Some(AdmissionHook::new(p, fail))
        }
        None => None,
    };

    let priority_path = get("priority-plugin", "GNOMON_PRIORITY_PLUGIN")?;
    let config = get("policy-config", "GNOMON_POLICY_CONFIG")?;
    let priority = if priority_path.is_some() || config.is_some() {
        let fallback = match &config {
            Some(path) => {
                let text = std::fs::read_to_string(path)
                    .map_err(|e| format!("--policy-config {path}: {e}"))?;
                PolicyConfig::from_json(&text)
                    .map_err(|e| format!("--policy-config {path}: {e}"))?
            }
            None => PolicyConfig::default(),
        };
        let plugin = match &priority_path {
            Some(path) => Some(
                PriorityPlugin::from_file(path, limits)
                    .map_err(|e| format!("--priority-plugin {path}: {e}"))?,
            ),
            None => None,
        };
        summary.push(match &priority_path {
            Some(path) => format!("priority plugin {path} (declarative fallback)"),
            None => "declarative priority".into(),
        });
        Some(PriorityHook::new(plugin, fallback))
    } else {
        None
    };

    let shadow = match get("shadow-priority-plugin", "GNOMON_SHADOW_PRIORITY_PLUGIN")? {
        Some(path) => {
            let p = PriorityPlugin::from_file(&path, limits)
                .map_err(|e| format!("--shadow-priority-plugin {path}: {e}"))?;
            summary.push(format!(
                "shadow priority plugin {path} (compared, not applied)"
            ));
            Some(ShadowPriority::new(p))
        }
        None => None,
    };

    let estimator = match get("estimator-plugin", "GNOMON_ESTIMATOR_PLUGIN")? {
        Some(path) => {
            let p = EstimatorPlugin::from_file(&path, limits)
                .map_err(|e| format!("--estimator-plugin {path}: {e}"))?;
            summary.push(format!("estimator plugin {path} (walltime fallback)"));
            Some(Arc::new(WasmEstimator::new(p)))
        }
        None => None,
    };
    let node_score = match get("node-score-plugin", "GNOMON_NODE_SCORE_PLUGIN")? {
        Some(path) => {
            let p = NodeScorePlugin::from_file(&path, limits)
                .map_err(|e| format!("--node-score-plugin {path}: {e}"))?;
            let cap = match get("node-score-candidates", "GNOMON_NODE_SCORE_CANDIDATES")? {
                None => Some(DEFAULT_MAX_CANDIDATES),
                Some(v) if v == "all" => None,
                Some(v) => match v.parse::<usize>() {
                    Ok(n) if n > 0 => Some(n),
                    _ => return Err(format!("--node-score-candidates '{v}' is not N or all")),
                },
            };
            let shown = cap.map_or("all".to_string(), |n| n.to_string());
            summary.push(format!(
                "node-score plugin {path} ({shown} candidates per call, default placement fallback)"
            ));
            Some((p, cap))
        }
        None => None,
    };

    let backfill = match estimator {
        Some(e) => BackfillPolicy::with_estimator(WINDOW_SECS, e),
        None => BackfillPolicy::new(WINDOW_SECS),
    };
    let policy: Arc<dyn Policy> = match node_score {
        Some((p, cap)) => {
            Arc::new(ScoredPolicy::new(Arc::new(backfill), p).with_max_candidates(cap))
        }
        None => Arc::new(backfill),
    };
    let submit = (admission.is_some() || priority.is_some() || shadow.is_some()).then(|| {
        Arc::new(SubmitChain {
            admission,
            priority,
            shadow,
        }) as Arc<dyn SubmitPolicy>
    });
    Ok(Plugins {
        submit,
        policy,
        summary,
    })
}

fn limits(args: &[String], env: &impl Fn(&str) -> Option<String>) -> Result<PluginLimits, String> {
    let defaults = PluginLimits::default();
    let number = |name: &str, var: &str, default: u64| -> Result<u64, String> {
        match flag(args, name, env(var))? {
            None => Ok(default),
            Some(v) => match v.parse::<u64>() {
                Ok(n) if n > 0 => Ok(n),
                _ => Err(format!("--{name} '{v}' is not a positive number")),
            },
        }
    };
    Ok(PluginLimits {
        fuel: number("plugin-fuel", "GNOMON_PLUGIN_FUEL", defaults.fuel)?,
        max_memory_bytes: number(
            "plugin-memory-mb",
            "GNOMON_PLUGIN_MEMORY_MB",
            (defaults.max_memory_bytes >> 20) as u64,
        )?
        .checked_mul(1 << 20)
        .and_then(|b| usize::try_from(b).ok())
        .ok_or("--plugin-memory-mb is too large")?,
    })
}

/// The submit handler's policy: admission (which may reject or rewrite),
/// then priority, then the shadow comparison. Plugin failures are logged
/// with running counts; the fallbacks decide.
struct SubmitChain {
    admission: Option<AdmissionHook>,
    priority: Option<PriorityHook>,
    shadow: Option<ShadowPriority>,
}

impl SubmitPolicy for SubmitChain {
    fn prepare(&self, job: &mut Job, now: u64) -> Result<(), String> {
        if let Some(a) = &self.admission {
            let (outcome, err) = a.admit(job, now);
            if let Some(e) = err {
                eprintln!(
                    "gnomon-ctl: admission plugin failed for job {} ({} of {} calls); \
                     job {} (fail {:?}): {e}",
                    job.id.0,
                    a.failures.load(Ordering::Relaxed),
                    a.calls.load(Ordering::Relaxed),
                    if outcome.is_ok() {
                        "accepted"
                    } else {
                        "rejected"
                    },
                    a.fail,
                );
            }
            outcome?;
        }
        if let Some(p) = &self.priority {
            let (priority, err) = p.priority(job, now);
            if let Some(e) = err {
                eprintln!(
                    "gnomon-ctl: priority plugin failed for job {} ({} of {} calls); \
                     using declarative priority {priority}: {e}",
                    job.id.0,
                    p.failures.load(Ordering::Relaxed),
                    p.calls.load(Ordering::Relaxed),
                );
            }
            job.priority = priority;
        }
        if let Some(s) = &self.shadow {
            let outcome = s.compare(job, now);
            let n = s.compared.load(Ordering::Relaxed);
            // Log the first comparisons, then at powers of two.
            if outcome != ShadowOutcome::Same && (n <= 10 || n.is_power_of_two()) {
                eprintln!(
                    "gnomon-ctl: shadow priority on job {} ({}): {outcome:?}; \
                     agreement {:.1}% over {n} submissions",
                    job.id.0,
                    job.priority,
                    s.agreement() * 100.0
                );
            }
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use gnomon_api::pb::{scheduler_server::Scheduler, GetJobRequest, SubmitJobRequest};
    use gnomon_core::{JobId, Resources, Walltime};
    use gnomon_fsm::Fsm;
    use gnomon_policy::fixtures;
    use gnomon_sched::EvalBroker;
    use std::path::PathBuf;
    use std::time::Duration;

    fn args(v: &[&str]) -> Vec<String> {
        v.iter().map(|s| s.to_string()).collect()
    }

    /// Writes the fixtures to a temporary directory, removed on drop.
    struct Files(PathBuf);
    impl Files {
        fn new(tag: &str) -> Self {
            let dir = std::env::temp_dir().join(format!("gnomon-ctl-{tag}-{}", std::process::id()));
            std::fs::create_dir_all(&dir).unwrap();
            for (name, wat) in [
                ("priority.wat", fixtures::strings()),
                ("trap.wat", fixtures::trap()),
                ("admission.wat", fixtures::admission_rules()),
                ("estimator.wat", fixtures::half_walltime()),
                ("score.wat", fixtures::most_free_cores()),
            ] {
                std::fs::write(dir.join(name), wat).unwrap();
            }
            std::fs::write(dir.join("policy.json"), POLICY_JSON).unwrap();
            Files(dir)
        }
        fn path(&self, name: &str) -> String {
            self.0.join(name).to_str().unwrap().to_string()
        }
    }
    impl Drop for Files {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }

    const POLICY_JSON: &str =
        r#"{"weight_age":1,"weight_jobsize":1,"weight_qos":0,"qos_tiers":[],"partitions":[]}"#;

    fn none(_: &str) -> Option<String> {
        None
    }

    #[test]
    fn nothing_configured_keeps_client_priorities_and_plain_backfill() {
        let p = load(&[], none).unwrap();
        assert!(p.submit.is_none());
        assert_eq!(p.policy.name(), "backfill/walltime");
        assert!(p.summary.is_empty());
    }

    #[test]
    fn every_plugin_loads_from_flags_or_environment() {
        let f = Files::new("load");
        let all = args(&[
            "--admission-plugin",
            &f.path("admission.wat"),
            "--admission-fail",
            "closed",
            "--priority-plugin",
            &f.path("priority.wat"),
            "--policy-config",
            &f.path("policy.json"),
            "--shadow-priority-plugin",
            &f.path("priority.wat"),
            "--estimator-plugin",
            &f.path("estimator.wat"),
            "--node-score-plugin",
            &f.path("score.wat"),
        ]);
        let p = load(&all, none).unwrap();
        assert!(p.submit.is_some());
        assert_eq!(p.policy.name(), "backfill/wasm+node-score");
        assert_eq!(p.summary.len(), 5);

        let est = f.path("estimator.wat");
        let env = |k: &str| (k == "GNOMON_ESTIMATOR_PLUGIN").then(|| est.clone());
        assert_eq!(load(&[], env).unwrap().policy.name(), "backfill/wasm");
        let cfg = load(&args(&["--policy-config", &f.path("policy.json")]), none).unwrap();
        assert_eq!(cfg.summary, ["declarative priority"]);
    }

    #[test]
    fn bad_configuration_stops_startup() {
        let f = Files::new("bad");
        let p = f.path("priority.wat");
        let a = f.path("admission.wat");
        let s = f.path("score.wat");
        assert!(load(
            &args(&["--node-score-plugin", &s, "--node-score-candidates", "all"]),
            none
        )
        .is_ok());
        for bad in [
            vec!["--priority-plugin", "/nonexistent.wasm"],
            vec!["--policy-config", "/nonexistent.json"],
            vec!["--priority-plugin", &p, "--plugin-fuel", "0"],
            vec!["--priority-plugin", &p, "--plugin-memory-mb", "lots"],
            vec!["--priority-plugin"],
            vec!["--admission-plugin", &a, "--admission-fail", "maybe"],
            // A component for the wrong world.
            vec!["--admission-plugin", &p],
            vec!["--estimator-plugin", &a],
            vec!["--node-score-plugin", &p],
            vec!["--shadow-priority-plugin", &a],
            vec!["--node-score-plugin", &s, "--node-score-candidates", "0"],
            vec!["--node-score-plugin", &s, "--node-score-candidates", "some"],
        ] {
            assert!(load(&args(&bad), none).is_err(), "{bad:?}");
        }
    }

    fn svc(submit: Option<Arc<dyn SubmitPolicy>>) -> gnomon_api::SchedulerSvc {
        gnomon_api::SchedulerSvc {
            fsm: Arc::new(Fsm::new()),
            broker: Arc::new(EvalBroker::new()),
            submit_policy: submit,
        }
    }

    /// Submit `cores` x `nodes` (with `gpus`); the stored job, or the error.
    #[allow(clippy::result_large_err)] // tonic's own error type
    async fn submit(
        s: &gnomon_api::SchedulerSvc,
        cores: u32,
        gpus: u32,
        nodes: u32,
    ) -> Result<gnomon_api::pb::JobInfo, tonic::Status> {
        let req = SubmitJobRequest {
            name: "j".into(),
            user: "u".into(),
            request: Some(gnomon_api::pb::Resources {
                cores,
                mem_mb: 0,
                gpus,
            }),
            walltime_secs: 60,
            priority: 1,
            nodes,
            ..Default::default()
        };
        let id = s
            .submit_job(tonic::Request::new(req))
            .await?
            .into_inner()
            .job_id;
        let get = GetJobRequest {
            job_id: id,
            wait_index: 0,
        };
        let job = s.get_job(tonic::Request::new(get)).await.unwrap();
        Ok(job.into_inner().job.unwrap())
    }

    #[tokio::test]
    async fn submissions_are_admitted_then_prioritized() {
        let f = Files::new("submit");
        let p = load(
            &args(&[
                "--admission-plugin",
                &f.path("admission.wat"),
                "--priority-plugin",
                &f.path("priority.wat"),
                "--shadow-priority-plugin",
                &f.path("trap.wat"),
            ]),
            none,
        )
        .unwrap();
        let s = svc(p.submit);
        // strings: user "u" (117) * 1000 + partition length.
        let plain = submit(&s, 4, 0, 2).await.unwrap();
        assert_eq!(
            (plain.partition.as_str(), plain.priority),
            ("normal", 117_006)
        );
        // Rewritten to partition "gpu" first, so the priority sees "gpu".
        let gpu = submit(&s, 4, 1, 2).await.unwrap();
        assert_eq!((gpu.partition.as_str(), gpu.priority), ("gpu", 117_003));
        let err = submit(&s, 64, 0, 2).await.unwrap_err();
        assert_eq!(err.code(), tonic::Code::PermissionDenied);
        assert!(err.message().contains("too many cores"));
    }

    #[tokio::test]
    async fn failed_plugins_fall_back() {
        let f = Files::new("fallback");
        // A trapping priority plugin: the declarative priority, weight_jobsize
        // 100 * (2 nodes * 4 cores), not the client's 1.
        let p = load(&args(&["--priority-plugin", &f.path("trap.wat")]), none).unwrap();
        assert_eq!(submit(&svc(p.submit), 4, 0, 2).await.unwrap().priority, 800);
        // A trapping admission plugin: open accepts, closed rejects.
        let trap = f.path("trap.wat");
        std::fs::write(&trap, fixtures::admission_trap()).unwrap();
        let open = load(&args(&["--admission-plugin", &trap]), none).unwrap();
        assert!(submit(&svc(open.submit), 1, 0, 1).await.is_ok());
        let closed = args(&["--admission-plugin", &trap, "--admission-fail", "closed"]);
        let closed = load(&closed, none).unwrap();
        assert!(submit(&svc(closed.submit), 1, 0, 1).await.is_err());
    }

    #[test]
    fn the_scheduling_policy_uses_estimates_and_scores() {
        let f = Files::new("sched");
        let all = args(&[
            "--estimator-plugin",
            &f.path("estimator.wat"),
            "--node-score-plugin",
            &f.path("score.wat"),
        ]);
        let p = load(&all, none).unwrap();
        let fsm = Fsm::new();
        for n in ["a", "b"] {
            fsm.apply(gnomon_core::Event::NodeRegistered {
                name: n.into(),
                capacity: Resources {
                    cores: 8,
                    mem_mb: 0,
                    gpus: 0,
                },
            });
        }
        let job = Job::new(
            JobId(1),
            "j",
            "u",
            Resources {
                cores: 2,
                mem_mb: 0,
                gpus: 0,
            },
            Walltime(Duration::from_secs(3600)),
        );
        let plan = p.policy.schedule(&fsm.snapshot(), &job);
        match plan.decision {
            gnomon_sched::Decision::Start { dur, .. } => assert_eq!(dur, Some(1800)),
            d => panic!("{d:?}"),
        }
    }
}
