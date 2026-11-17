//! gntop's UI state and key handling, independent of the terminal and the
//! network so every interaction is unit-testable.

use crate::model::{ClusterSnapshot, JobView};
use crate::view::Window;
use crossterm::event::{KeyCode, KeyEvent, KeyEventKind, KeyModifiers};

/// Controller connection status shown in the header.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Conn {
    /// No poll has completed yet.
    Connecting,
    Connected,
    /// The last poll failed; the poller keeps retrying.
    Disconnected(String),
}

/// What the poller hands the UI after each attempt.
#[derive(Clone, Debug, PartialEq)]
pub enum Update {
    Snapshot(ClusterSnapshot),
    Error(String),
}

/// A key, decoupled from crossterm so tests can drive the app directly.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Key {
    Char(char),
    Left,
    Right,
    Enter,
    Esc,
    Backspace,
    CtrlC,
    Other,
}

impl Key {
    /// Maps a crossterm key event; key releases map to [`Key::Other`].
    pub fn from_event(ev: KeyEvent) -> Key {
        if ev.kind == KeyEventKind::Release {
            return Key::Other;
        }
        match ev.code {
            KeyCode::Char('c') if ev.modifiers.contains(KeyModifiers::CONTROL) => Key::CtrlC,
            KeyCode::Char(c) => Key::Char(c),
            KeyCode::Left => Key::Left,
            KeyCode::Right => Key::Right,
            KeyCode::Enter => Key::Enter,
            KeyCode::Esc => Key::Esc,
            KeyCode::Backspace => Key::Backspace,
            _ => Key::Other,
        }
    }
}

/// The whole UI state.
#[derive(Clone, Debug)]
pub struct App {
    /// Controller address (header display).
    pub addr: String,
    /// Latest successful poll; kept (as stale data) while disconnected.
    pub snapshot: Option<ClusterSnapshot>,
    pub conn: Conn,
    pub window: Window,
    /// Polling suspended with `p`.
    pub paused: bool,
    /// Applied user filter (`/`); empty shows every user.
    pub filter: String,
    /// Filter being typed, while the `/` prompt is open.
    pub input: Option<String>,
    /// Wall clock used for the "now" cursor, epoch seconds.
    pub now: u64,
    pub quit: bool,
}

impl App {
    pub fn new(addr: impl Into<String>, window: Window, now: u64) -> Self {
        App {
            addr: addr.into(),
            snapshot: None,
            conn: Conn::Connecting,
            window,
            paused: false,
            filter: String::new(),
            input: None,
            now,
            quit: false,
        }
    }

    /// Applies a poll result.
    pub fn apply(&mut self, update: Update) {
        match update {
            Update::Snapshot(s) => {
                self.snapshot = Some(s);
                self.conn = Conn::Connected;
            }
            Update::Error(e) => self.conn = Conn::Disconnected(e),
        }
    }

    /// Jobs passing the user filter.
    pub fn visible_jobs(&self) -> impl Iterator<Item = &JobView> + Clone {
        self.snapshot
            .iter()
            .flat_map(|s| s.jobs.iter())
            .filter(move |j| self.filter.is_empty() || j.user == self.filter)
    }

    /// Handles one key press.
    pub fn on_key(&mut self, key: Key) {
        if key == Key::CtrlC {
            self.quit = true;
            return;
        }
        if let Some(buf) = self.input.as_mut() {
            match key {
                Key::Char(c) => buf.push(c),
                Key::Backspace => {
                    buf.pop();
                }
                Key::Enter => {
                    self.filter = self.input.take().unwrap_or_default().trim().to_string();
                }
                Key::Esc => self.input = None,
                _ => {}
            }
            return;
        }
        match key {
            Key::Char('q') | Key::Esc => self.quit = true,
            Key::Char('p') => self.paused = !self.paused,
            Key::Char('+') | Key::Char('=') => self.window.zoom_in(),
            Key::Char('-') | Key::Char('_') => self.window.zoom_out(),
            Key::Left | Key::Char('h') => self.window.pan_left(),
            Key::Right | Key::Char('l') => self.window.pan_right(),
            Key::Char('0') => self.window.reset(),
            Key::Char('/') => self.input = Some(self.filter.clone()),
            _ => {}
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::JobState;

    fn app() -> App {
        App::new("http://ctl:7631", Window::default(), 1000)
    }

    #[test]
    fn quit_keys() {
        for k in [Key::Char('q'), Key::Esc, Key::CtrlC] {
            let mut a = app();
            a.on_key(k);
            assert!(a.quit, "{k:?} should quit");
        }
    }

    #[test]
    fn pause_toggles_and_window_keys_drive_the_window() {
        let mut a = app();
        a.on_key(Key::Char('p'));
        assert!(a.paused);
        a.on_key(Key::Char('p'));
        assert!(!a.paused);
        a.on_key(Key::Char('+'));
        assert_eq!(a.window.span(), Window::default().span() / 2);
        a.on_key(Key::Right);
        assert!(a.window.offset > 0);
        a.on_key(Key::Char('0'));
        assert_eq!(a.window, Window::default());
        a.on_key(Key::Char('-'));
        a.on_key(Key::Left);
        assert_eq!(a.window.span(), Window::default().span() * 2);
        assert!(a.window.offset < 0);
    }

    #[test]
    fn slash_opens_a_filter_prompt_that_esc_cancels_without_quitting() {
        let mut a = app();
        a.on_key(Key::Char('/'));
        for c in "bobx".chars() {
            a.on_key(Key::Char(c));
        }
        a.on_key(Key::Backspace);
        assert_eq!(a.input.as_deref(), Some("bob"));
        // 'q' is text while typing, not quit.
        a.on_key(Key::Char('q'));
        a.on_key(Key::Backspace);
        a.on_key(Key::Enter);
        assert_eq!(a.filter, "bob");
        assert!(a.input.is_none() && !a.quit);
        a.on_key(Key::Char('/'));
        a.on_key(Key::Esc);
        assert!(!a.quit && a.input.is_none());
        assert_eq!(a.filter, "bob");
    }

    #[test]
    fn filter_limits_visible_jobs_by_user() {
        let mut a = app();
        let job = |id, user: &str| JobView {
            id,
            name: String::new(),
            user: user.into(),
            state: JobState::Pending,
            priority: 0,
            submit_time: 0,
            cores: 1,
            planned_start: None,
            planned_dur: 1,
            nodes: vec![],
        };
        a.apply(Update::Snapshot(ClusterSnapshot {
            jobs: vec![job(1, "alice"), job(2, "bob")],
            ..Default::default()
        }));
        assert_eq!(a.visible_jobs().count(), 2);
        a.filter = "bob".into();
        let ids: Vec<u64> = a.visible_jobs().map(|j| j.id).collect();
        assert_eq!(ids, vec![2]);
    }

    #[test]
    fn errors_mark_disconnected_but_keep_the_last_snapshot() {
        let mut a = app();
        assert_eq!(a.conn, Conn::Connecting);
        a.apply(Update::Snapshot(ClusterSnapshot::default()));
        assert_eq!(a.conn, Conn::Connected);
        a.apply(Update::Error("connection refused".into()));
        assert_eq!(a.conn, Conn::Disconnected("connection refused".into()));
        assert!(a.snapshot.is_some());
    }

    #[test]
    fn crossterm_events_map_to_keys() {
        let ev = KeyEvent::new(KeyCode::Char('c'), KeyModifiers::CONTROL);
        assert_eq!(Key::from_event(ev), Key::CtrlC);
        let ev = KeyEvent::new(KeyCode::Left, KeyModifiers::NONE);
        assert_eq!(Key::from_event(ev), Key::Left);
        let mut ev = KeyEvent::new(KeyCode::Char('q'), KeyModifiers::NONE);
        assert_eq!(Key::from_event(ev), Key::Char('q'));
        ev.kind = KeyEventKind::Release;
        assert_eq!(Key::from_event(ev), Key::Other);
    }
}
