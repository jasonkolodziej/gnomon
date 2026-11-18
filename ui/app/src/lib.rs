//! Gnomon's browser UI: a terminal-styled Rune app that shows everything the
//! scheduler does, live, from `gnomon-web` (JSON + Server-Sent Events).
//!
//! | Key | View |
//! |---|---|
//! | `1` timeline | each node's planner: running spans, backfill reservations, the queue |
//! | `2` nodes | agents, capacity in use, ranks, the kill channel |
//! | `3` queue | jobs (gnqueue, live), job detail and cancel, a new-job form |
//! | `4` cluster | the Raft controller group and replication progress |
//! | `5` policy | the WASM plugin pipeline (admission, priority, shadow, estimator, node-score) |
//! | `6` events | the state machine's event feed |
//! | `7` lab | policies compared on one workload with the production scheduling code |
//!
//! `?demo` in the URL shows built-in sample data without a server.

pub mod fixture;
pub mod net;
pub mod state;
pub mod theme;
mod views;

use rune::{AppSpec, Key, KeyEvent, Tx};
use state::{State, Tab};

fn on_key(s: &mut State, e: &KeyEvent, _tx: &mut Tx) -> bool {
    match &e.key {
        Key::Text(c)
            if c.len() == 1 && !(e.mods.cmd || e.mods.ctrl || e.mods.alt || e.mods.logo) =>
        {
            let ch = c.chars().next().unwrap_or(' ');
            if let Some(d) = ch.to_digit(10) {
                if (1..=7).contains(&d) {
                    s.tab = Tab::ALL[d as usize - 1];
                    return true;
                }
            }
            if ch == 'x' {
                if let Some(id) = s.selected {
                    if s.job(id).is_some_and(|j| j.is_active()) {
                        net::cancel(s, id);
                        return true;
                    }
                }
            }
            false
        }
        Key::Up => {
            s.select_step(-1);
            true
        }
        Key::Down => {
            s.select_step(1);
            true
        }
        Key::Escape => {
            s.toast = None;
            s.selected = None;
            true
        }
        _ => false,
    }
}

/// Sample data on a given tab: `?demo`, previews and screenshots.
pub fn demo_state(tab: Tab) -> State {
    let mut s = State {
        demo: true,
        tab,
        selected: Some(41),
        ..State::default()
    };
    s.apply_view(fixture::view());
    s.push_events(fixture::events());
    if tab == Tab::Lab {
        s.lab_report = Some(fixture::lab_report(&s.lab));
    }
    if tab == Tab::Queue {
        s.selected = Some(48);
    }
    s
}

fn scene(tab: Tab) -> impl Fn(&mut State) {
    move |s: &mut State| *s = demo_state(tab)
}

/// `AppSpec::scenes` takes plain fns.
const SCENES: &[(&str, fn(&mut State))] = &[
    ("timeline", |s| scene(Tab::Timeline)(s)),
    ("nodes", |s| scene(Tab::Nodes)(s)),
    ("queue", |s| scene(Tab::Queue)(s)),
    ("cluster", |s| scene(Tab::Cluster)(s)),
    ("policy", |s| scene(Tab::Policy)(s)),
    ("events", |s| scene(Tab::Events)(s)),
    ("lab", |s| scene(Tab::Lab)(s)),
];

pub fn spec() -> AppSpec<State> {
    let mut spec = AppSpec::new("gnomon", (1440.0, 900.0), State::default, views::root);
    spec.on_key = Some(on_key);
    spec.scenes = SCENES;
    // The brand fonts, bundled, on every target: the browser and the native
    // snapshots draw the same faces.
    spec.fonts = theme::FONTS;
    spec.families = (theme::MONO_FAMILY, theme::MONO_FAMILY);
    spec
}

#[cfg_attr(target_arch = "wasm32", wasm_bindgen::prelude::wasm_bindgen(start))]
pub fn start() {
    #[cfg(target_arch = "wasm32")]
    {
        let mut spec = spec();
        if net::demo_requested() {
            // `?demo=queue` opens that tab.
            let tab = web_sys::window()
                .and_then(|w| w.location().search().ok())
                .and_then(|q| {
                    Tab::ALL
                        .into_iter()
                        .find(|t| q.contains(&format!("demo={}", t.name())))
                })
                .unwrap_or(Tab::Timeline);
            spec.init = match tab {
                Tab::Timeline => || demo_state(Tab::Timeline),
                Tab::Nodes => || demo_state(Tab::Nodes),
                Tab::Queue => || demo_state(Tab::Queue),
                Tab::Cluster => || demo_state(Tab::Cluster),
                Tab::Policy => || demo_state(Tab::Policy),
                Tab::Events => || demo_state(Tab::Events),
                Tab::Lab => || demo_state(Tab::Lab),
            };
            if let Err(e) = rune::App::new(spec).try_run() {
                web_sys::console::error_1(&e.into());
            }
            return;
        }
        let app = rune::App::new(spec);
        net::connect(app.handle());
        if let Err(e) = app.try_run() {
            web_sys::console::error_1(&e.into());
        }
    }
    #[cfg(not(target_arch = "wasm32"))]
    rune::run(spec());
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn demo_state_covers_every_view() {
        for tab in Tab::ALL {
            let s = demo_state(tab);
            let v = s.view.as_ref().unwrap();
            assert!(v.cluster.replicated && v.policy.plugins.len() == 5);
            assert!(
                v.jobs
                    .iter()
                    .any(|j| j.num_nodes > 1 && j.state == "RUNNING")
            );
            assert!(v.nodes.iter().any(|n| !n.kill_pending.is_empty()));
            assert!(!s.events.is_empty());
            assert!(!s.writable(), "demo data never writes");
        }
        assert!(demo_state(Tab::Lab).lab_report.is_some());
    }
}
