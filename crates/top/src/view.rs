//! Time-window state machine: zoom (`+`/`-`), pan (`←`/`→`) and reset (`0`)
//! over the `[now - history, now + horizon]` axis.

/// Narrowest window `+` can reach (seconds, history + horizon).
pub const MIN_SPAN: u64 = 60;
/// Widest window `-` can reach: two weeks of plan.
pub const MAX_SPAN: u64 = 14 * 24 * 3600;

/// The visible time window, relative to `now`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Window {
    /// Seconds shown before `now` (at zero pan).
    pub history: u64,
    /// Seconds shown after `now` (at zero pan).
    pub horizon: u64,
    /// Pan offset in seconds (positive = later).
    pub offset: i64,
    initial: (u64, u64),
}

impl Default for Window {
    /// The documented default: ten minutes of history, two hours of plan.
    fn default() -> Self {
        Window::new(600, 7200)
    }
}

impl Window {
    /// A window with the given history and horizon; also what `0` resets to.
    /// Degenerate inputs are widened to [`MIN_SPAN`].
    pub fn new(history: u64, horizon: u64) -> Self {
        let horizon = if history + horizon < MIN_SPAN {
            MIN_SPAN - history
        } else {
            horizon
        };
        Window {
            history,
            horizon,
            offset: 0,
            initial: (history, horizon),
        }
    }

    /// Total visible seconds.
    pub fn span(&self) -> u64 {
        self.history + self.horizon
    }

    /// Absolute `[start, end)` bounds for the given `now`.
    pub fn bounds(&self, now: u64) -> (i64, i64) {
        let now = now as i64;
        (
            now - self.history as i64 + self.offset,
            now + self.horizon as i64 + self.offset,
        )
    }

    /// Halves the window (keeping `now`'s relative position), down to
    /// [`MIN_SPAN`].
    pub fn zoom_in(&mut self) {
        if self.span() / 2 >= MIN_SPAN {
            self.scale(1, 2);
        }
    }

    /// Doubles the window, up to [`MAX_SPAN`].
    pub fn zoom_out(&mut self) {
        if self.span() * 2 <= MAX_SPAN {
            self.scale(2, 1);
        }
    }

    fn scale(&mut self, num: u64, den: u64) {
        self.history = self.history * num / den;
        self.horizon = self.horizon * num / den;
        self.offset = self.offset * num as i64 / den as i64;
    }

    /// Moves a quarter window earlier.
    pub fn pan_left(&mut self) {
        self.offset -= (self.span() / 4).max(1) as i64;
    }

    /// Moves a quarter window later.
    pub fn pan_right(&mut self) {
        self.offset += (self.span() / 4).max(1) as i64;
    }

    /// Back to the initial history/horizon with no pan.
    pub fn reset(&mut self) {
        *self = Window::new(self.initial.0, self.initial.1);
    }
}

/// Parses a duration such as `90`, `90s`, `10m`, `2h`, `1d` or `500ms` into
/// milliseconds.
pub fn parse_duration_ms(s: &str) -> Result<u64, String> {
    let s = s.trim();
    let split = s.find(|c: char| !c.is_ascii_digit()).unwrap_or(s.len());
    let (num, unit) = s.split_at(split);
    let n: u64 = num
        .parse()
        .map_err(|_| format!("invalid duration {s:?}: expected e.g. 30s, 10m, 2h"))?;
    let mult = match unit {
        "ms" => 1,
        "" | "s" => 1000,
        "m" => 60_000,
        "h" => 3_600_000,
        "d" => 86_400_000,
        _ => return Err(format!("invalid duration unit {unit:?} in {s:?}")),
    };
    n.checked_mul(mult)
        .ok_or_else(|| format!("duration {s:?} overflows"))
}

/// [`parse_duration_ms`] rounded down to whole seconds.
pub fn parse_duration_secs(s: &str) -> Result<u64, String> {
    parse_duration_ms(s).map(|ms| ms / 1000)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_window_is_ten_minutes_back_two_hours_ahead() {
        let w = Window::default();
        assert_eq!(w.bounds(10_000), (10_000 - 600, 10_000 + 7200));
    }

    #[test]
    fn zoom_halves_and_doubles_within_limits() {
        let mut w = Window::new(600, 7200);
        w.zoom_in();
        assert_eq!((w.history, w.horizon), (300, 3600));
        w.zoom_out();
        w.zoom_out();
        assert_eq!((w.history, w.horizon), (1200, 14_400));
        for _ in 0..40 {
            w.zoom_in();
        }
        assert!(w.span() >= MIN_SPAN && w.span() < 2 * MIN_SPAN);
        for _ in 0..40 {
            w.zoom_out();
        }
        assert!(w.span() <= MAX_SPAN && w.span() * 2 > MAX_SPAN);
    }

    #[test]
    fn pan_moves_a_quarter_window_and_zoom_scales_the_pan() {
        let mut w = Window::new(600, 7200);
        w.pan_right();
        assert_eq!(w.offset, 1950);
        w.pan_left();
        w.pan_left();
        assert_eq!(w.offset, -1950);
        w.zoom_in();
        assert_eq!(w.offset, -975);
        assert_eq!(w.bounds(10_000), (10_000 - 300 - 975, 10_000 + 3600 - 975));
    }

    #[test]
    fn reset_restores_the_initial_window() {
        let mut w = Window::new(60, 600);
        w.zoom_out();
        w.pan_right();
        w.reset();
        assert_eq!(w, Window::new(60, 600));
        assert_eq!(w.offset, 0);
    }

    #[test]
    fn degenerate_window_is_widened() {
        assert_eq!(Window::new(0, 0).span(), MIN_SPAN);
    }

    #[test]
    fn durations_parse_with_units() {
        assert_eq!(parse_duration_ms("500ms"), Ok(500));
        assert_eq!(parse_duration_ms("2"), Ok(2000));
        assert_eq!(parse_duration_secs("10m"), Ok(600));
        assert_eq!(parse_duration_secs("2h"), Ok(7200));
        assert_eq!(parse_duration_secs("1d"), Ok(86_400));
        assert!(parse_duration_ms("ten").is_err());
        assert!(parse_duration_ms("5y").is_err());
        assert!(parse_duration_ms("").is_err());
    }
}
