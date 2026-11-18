//! The brand palette (assets/logo/BRAND.md) and the terminal look: one monospace
//! family everywhere, hairline rules, square-ish panels.

use rune::color::hex;
use rune::tok::TextStyle;
use rune::{Color, El, text};

fn c(s: &str) -> Color {
    hex(s).expect("valid hex colour")
}

/// Ink: the page.
pub fn ink() -> Color {
    c("#0B1220")
}
/// Panels sit on the page itself: Ink, set apart by hairline rules (`line`),
/// as in a terminal. Slate is the one raised tone.
pub fn panel() -> Color {
    ink()
}
/// Slate: raised surfaces, selection.
pub fn slate() -> Color {
    c("#1E293B")
}
/// Hairline rules and borders.
pub fn line() -> Color {
    c("#334155")
}
pub fn dim() -> Color {
    c("#64748B")
}
pub fn muted() -> Color {
    c("#94A3B8")
}
/// Paper: primary text.
pub fn paper() -> Color {
    c("#F8FAFC")
}
/// Signal Teal: the accent (and, per the brand, the backfilled span).
pub fn teal() -> Color {
    c("#14B8A6")
}
/// Teal Light: accent text on dark.
pub fn teal_light() -> Color {
    c("#5EEAD4")
}
pub fn amber() -> Color {
    c("#F59E0B")
}
pub fn sky() -> Color {
    c("#38BDF8")
}
pub fn red() -> Color {
    c("#F43F5E")
}
pub fn violet() -> Color {
    c("#A78BFA")
}

/// Job state colours: gntop's mapping, on the brand palette.
pub fn state_color(state: &str) -> Color {
    match state {
        "RUNNING" => teal(),
        "RESERVED" => amber(),
        "PENDING" => sky(),
        "FAILED" => red(),
        "CANCELLED" => violet(),
        _ => dim(),
    }
}

/// Short state tags, as in gnqueue/gntop.
pub fn state_tag(state: &str) -> &'static str {
    match state {
        "RUNNING" => "RUN ",
        "RESERVED" => "RSV ",
        "PENDING" => "PEND",
        "COMPLETED" => "DONE",
        "FAILED" => "FAIL",
        "CANCELLED" => "CANC",
        _ => "????",
    }
}

/// The brand's type (assets/logo/BRAND.md): JetBrains Mono Medium for the UI,
/// Space Grotesk SemiBold for the wordmark. Both are bundled (`assets/fonts`).
pub const MONO_FAMILY: &str = "JetBrains Mono";
pub const WORDMARK_FAMILY: &str = "Space Grotesk";

pub static FONTS: &[&[u8]] = &[
    include_bytes!("../../assets/fonts/JetBrainsMono-Medium.ttf"),
    include_bytes!("../../assets/fonts/JetBrainsMono-Bold.ttf"),
    include_bytes!("../../assets/fonts/SpaceGrotesk-SemiBold.ttf"),
];

pub const fn mono(size: f32) -> TextStyle {
    TextStyle::new(size, 500.0).mono()
}

pub const fn mono_b(size: f32) -> TextStyle {
    TextStyle::new(size, 700.0).mono()
}

/// The wordmark: Space Grotesk 600, lowercase, tracked −2%.
pub fn wordmark(size: f32) -> TextStyle {
    TextStyle::new(size, 600.0)
        .family(rune::tok::Family::Named(WORDMARK_FAMILY))
        .tracking(-0.02 * size)
}

/// Body text size.
pub const BODY: f32 = 12.5;
pub const SMALL: f32 = 11.0;

/// Monospace text in `color`.
pub fn t(s: impl Into<String>, color: Color) -> El {
    text(s).font(mono(BODY)).color(color)
}

pub fn ts(s: impl Into<String>, color: Color) -> El {
    text(s).font(mono(SMALL)).color(color)
}

pub fn tb(s: impl Into<String>, color: Color) -> El {
    text(s).font(mono_b(BODY)).color(color)
}

/// A section caption: `── TITLE ──────`.
pub fn caption(title: &str) -> El {
    rune::row()
        .w_full()
        .center_y()
        .gap(8.0)
        .child(
            text(format!("── {}", title.to_uppercase()))
                .font(mono_b(SMALL).tracking(1.2))
                .color(teal_light()),
        )
        .child(rune::el().grow(1.0).h(1.0).fill(line()))
}

/// A bordered panel.
pub fn panel_box() -> El {
    rune::col()
        .fill(panel())
        .border(1.0, line())
        .radius(4.0)
        .padding(12.0)
        .gap(8.0)
}

/// `[██████░░░░]` for `used` of `cap`, `cells` wide.
pub fn bar(used: f64, cap: f64, cells: usize) -> String {
    let frac = if cap > 0.0 {
        (used / cap).clamp(0.0, 1.0)
    } else {
        0.0
    };
    let full = (frac * cells as f64).round() as usize;
    format!("{}{}", "█".repeat(full), "░".repeat(cells - full))
}

/// `HH:MM:SS` (UTC) of a Unix time.
pub fn clock(t: u64) -> String {
    let s = t % 86_400;
    format!("{:02}:{:02}:{:02}", s / 3600, s % 3600 / 60, s % 60)
}

/// `HH:MM` (UTC).
pub fn hhmm(t: u64) -> String {
    let s = t % 86_400;
    format!("{:02}:{:02}", s / 3600, s % 3600 / 60)
}

/// `1h05m`, `12m`, `45s`.
pub fn dur(secs: u64) -> String {
    match secs {
        s if s >= 3600 => format!("{}h{:02}m", s / 3600, s % 3600 / 60),
        s if s >= 60 => format!("{}m", s / 60),
        s => format!("{s}s"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bars_and_times_render_like_a_terminal() {
        assert_eq!(bar(4.0, 8.0, 8), "████░░░░");
        assert_eq!(bar(9.0, 8.0, 4), "████", "clamped");
        assert_eq!(bar(1.0, 0.0, 3), "░░░");
        assert_eq!(clock(3_725), "01:02:05");
        assert_eq!(dur(3_900), "1h05m");
        assert_eq!(dur(125), "2m");
        assert_eq!(state_tag("RUNNING"), "RUN ");
    }
}
