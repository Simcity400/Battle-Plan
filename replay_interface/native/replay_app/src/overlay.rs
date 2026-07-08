//! In-window overlay: date/status text in a real game font + the scrub
//! bar. Shared by the interactive window loop and the headless dump path
//! so verification stays screen-free.
//!
//! Font selection contract: the real clock font is tried first. If no
//! game root was supplied, or the real font fails to load, the former TGA
//! proxy font is used and logged as such. If both paths fail, the overlay
//! draws no text; the scrub bar (which is not a font) still draws.

use crate::d3d::Gfx;
use crate::font;
use crate::text::{QuadRenderer, SolidRect, TextRenderer};
use crate::Result;
use map_viewer::GameFileResolver;
use std::collections::BTreeSet;

// PROXY: overlay text placement is a replay-specific choice, not taken
// from a the game .gui layout yet. Real parity requires laying the readout out
// from an extracted layout (e.g. game_assets/gui/ingame_clock.json places
// time_text inside clock_window anchored LOWER_RIGHT).
const TEXT_MARGIN_X: f32 = 8.0;
const TEXT_MARGIN_Y: f32 = 8.0;
const LINE_GAP: f32 = 4.0;

// PROXY: the scrub bar is a replay-specific control with no the game
// counterpart; its geometry and colors are ours, not a parity claim.
const SCRUB_MARGIN_X: f32 = 12.0;
const SCRUB_BOTTOM_MARGIN: f32 = 14.0;
const SCRUB_TRACK_HEIGHT: f32 = 8.0;
const SCRUB_MARKER_WIDTH: f32 = 3.0;
const SCRUB_MARKER_OVERHANG: f32 = 3.0;
/// Extra pixels around the track that still count as a click on it.
const SCRUB_HIT_PAD: f32 = 6.0;
/// Per-hour ticks are only cheap for short recordings; beyond this the
/// bar draws without ticks.
const MAX_SCRUB_TICKS: usize = 512;
// PROXY: scrub bar colors are replay-specific (see block comment above).
const SCRUB_TRACK_COLOR: u32 = 0xA020_2020;
const SCRUB_TICK_COLOR: u32 = 0x5AFF_FFFF;
const SCRUB_MARKER_COLOR: u32 = 0xF0FF_FFFF;

/// Scrub-bar track rectangle (x, y, width, height), or `None` when the
/// window is too small to hold one.
pub fn scrub_track_rect(screen_w: f32, screen_h: f32) -> Option<(f32, f32, f32, f32)> {
    let width = screen_w - 2.0 * SCRUB_MARGIN_X;
    let y = screen_h - SCRUB_BOTTOM_MARGIN - SCRUB_TRACK_HEIGHT;
    if width < 16.0 || y < 0.0 {
        return None;
    }
    Some((SCRUB_MARGIN_X, y, width, SCRUB_TRACK_HEIGHT))
}

/// Hit-test a click against the scrub bar; returns the 0..=1 fraction
/// along the track when hit.
pub fn scrub_fraction_at(x: f32, y: f32, screen_w: f32, screen_h: f32) -> Option<f32> {
    let (track_x, track_y, track_w, track_h) = scrub_track_rect(screen_w, screen_h)?;
    if y < track_y - SCRUB_HIT_PAD || y > track_y + track_h + SCRUB_HIT_PAD {
        return None;
    }
    if x < track_x - SCRUB_HIT_PAD || x > track_x + track_w + SCRUB_HIT_PAD {
        return None;
    }
    Some(((x - track_x) / track_w).clamp(0.0, 1.0))
}

/// Fraction for an ongoing drag: x only, clamped to the track (standard
/// drag behavior keeps seeking when the cursor leaves the bar).
pub fn scrub_drag_fraction(x: f32, screen_w: f32, screen_h: f32) -> Option<f32> {
    let (track_x, _, track_w, _) = scrub_track_rect(screen_w, screen_h)?;
    Some(((x - track_x) / track_w).clamp(0.0, 1.0))
}

/// Track fraction of hour `index` among `len` recorded hours. Ticks and
/// the marker are index-spaced (recorded hours, not wall-time-uniform).
pub fn fraction_for_index(index: usize, len: usize) -> f32 {
    if len < 2 {
        return 0.0;
    }
    (index.min(len - 1)) as f32 / (len - 1) as f32
}

/// Inverse of [`fraction_for_index`]: nearest recorded-hour index.
pub fn index_for_fraction(fraction: f32, len: usize) -> usize {
    if len < 2 {
        return 0;
    }
    let scaled = fraction.clamp(0.0, 1.0) * (len - 1) as f32;
    (scaled.round() as usize).min(len - 1)
}

/// Overlay renderer state. Construction never fails the app: each broken
/// piece is reported once to stderr and simply not drawn.
pub struct Overlay {
    quads: Option<QuadRenderer>,
    text: Option<TextRenderer>,
    /// Missing-glyph characters already reported (skip + record contract).
    missing_reported: BTreeSet<char>,
}

impl Overlay {
    /// Build the overlay. `game_resolver` is `--game` / `BATTLE_PLAN_VANILLA`; when
    /// absent, font selection falls back to the local vanilla proxy path.
    pub fn create(gfx: &Gfx, game_resolver: Option<&GameFileResolver>) -> Overlay {
        let quads = match QuadRenderer::new(gfx) {
            Ok(quads) => Some(quads),
            Err(err) => {
                eprintln!("replay_app: overlay disabled: {err}");
                None
            }
        };
        let text = match font::load_overlay_font(game_resolver) {
            Ok(selection) => {
                let font_name = selection.font_name;
                let kind = match selection.kind {
                    font::OverlayFontKind::Real => "real",
                    font::OverlayFontKind::Proxy => "proxy fallback",
                };
                match TextRenderer::new(gfx, &selection.font) {
                    Ok(renderer) => {
                        match &selection.fallback_reason {
                            Some(reason) => eprintln!(
                                "replay_app: overlay text active: font {font_name:?} \
                                 ({kind}) from {}; {reason}",
                                selection.root.display()
                            ),
                            None => eprintln!(
                                "replay_app: overlay text active: font {font_name:?} \
                                 ({kind}) from {}",
                                selection.root.display()
                            ),
                        }
                        Some(renderer)
                    }
                    Err(err) => {
                        eprintln!(
                            "replay_app: overlay text disabled: failed to upload active font \
                             {font_name:?} from {}: {err}",
                            selection.root.display()
                        );
                        None
                    }
                }
            }
            Err(err) => {
                eprintln!("replay_app: overlay text disabled: {err}");
                None
            }
        };
        Overlay {
            quads,
            text,
            missing_reported: BTreeSet::new(),
        }
    }

    /// Whether game-font text will actually be drawn.
    pub fn has_text(&self) -> bool {
        self.text.is_some()
    }

    /// Draw the overlay over the already-rendered map: two text lines at
    /// the top-left and the scrub bar at the bottom. `hour_index` /
    /// `hour_count` position the marker among the recorded hours.
    pub fn draw(
        &mut self,
        gfx: &Gfx,
        line1: &str,
        line2: &str,
        hour_index: usize,
        hour_count: usize,
    ) -> Result<()> {
        let screen_w = gfx.width as f32;
        let screen_h = gfx.height as f32;

        if let (Some(quads), Some((track_x, track_y, track_w, track_h))) =
            (&self.quads, scrub_track_rect(screen_w, screen_h))
        {
            let mut rects = vec![SolidRect {
                x: track_x,
                y: track_y,
                width: track_w,
                height: track_h,
                color: SCRUB_TRACK_COLOR,
            }];
            // Measured-hour ticks: one per recorded hour, only while cheap.
            if (2..=MAX_SCRUB_TICKS).contains(&hour_count) {
                for index in 0..hour_count {
                    let fraction = fraction_for_index(index, hour_count);
                    rects.push(SolidRect {
                        x: track_x + fraction * (track_w - 1.0),
                        y: track_y + 1.0,
                        width: 1.0,
                        height: track_h - 2.0,
                        color: SCRUB_TICK_COLOR,
                    });
                }
            }
            let marker_fraction = fraction_for_index(hour_index, hour_count);
            rects.push(SolidRect {
                x: track_x + marker_fraction * (track_w - SCRUB_MARKER_WIDTH),
                y: track_y - SCRUB_MARKER_OVERHANG,
                width: SCRUB_MARKER_WIDTH,
                height: track_h + 2.0 * SCRUB_MARKER_OVERHANG,
                color: SCRUB_MARKER_COLOR,
            });
            quads.draw_rects(gfx, &rects)?;
        }

        if let Some(text) = &self.text {
            // Grounded: the font's own fonts.json color tints the real
            // white-alpha glyph pages, matching the game's scheme.
            let color = text.font().default_color;
            let line_height = text.font().line_height.max(0) as f32;
            let mut missing = text.draw_text(gfx, TEXT_MARGIN_X, TEXT_MARGIN_Y, line1, color)?;
            missing.extend(text.draw_text(
                gfx,
                TEXT_MARGIN_X,
                TEXT_MARGIN_Y + line_height + LINE_GAP,
                line2,
                color,
            )?);
            for ch in missing {
                if self.missing_reported.insert(ch) {
                    eprintln!(
                        "replay_app: font {:?} has no glyph for {ch:?}; skipped (no substitute)",
                        text.font().name
                    );
                }
            }
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const W: f32 = 1280.0;
    const H: f32 = 720.0;

    #[test]
    fn fraction_and_index_round_trip() {
        let len = 49;
        for index in 0..len {
            let fraction = fraction_for_index(index, len);
            assert_eq!(index_for_fraction(fraction, len), index, "index {index}");
        }
    }

    #[test]
    fn fraction_handles_degenerate_lengths() {
        assert_eq!(fraction_for_index(0, 0), 0.0);
        assert_eq!(fraction_for_index(0, 1), 0.0);
        assert_eq!(fraction_for_index(5, 3), 1.0);
        assert_eq!(index_for_fraction(0.7, 0), 0);
        assert_eq!(index_for_fraction(0.7, 1), 0);
    }

    #[test]
    fn index_for_fraction_clamps_out_of_range() {
        assert_eq!(index_for_fraction(-0.5, 10), 0);
        assert_eq!(index_for_fraction(1.5, 10), 9);
    }

    #[test]
    fn scrub_hit_inside_track_returns_fraction() {
        let (x, y, w, h) = scrub_track_rect(W, H).expect("track exists");
        let mid = scrub_fraction_at(x + w / 2.0, y + h / 2.0, W, H).expect("hit");
        assert!((mid - 0.5).abs() < 0.01, "{mid}");
        let start = scrub_fraction_at(x, y, W, H).expect("hit");
        assert_eq!(start, 0.0);
        let end = scrub_fraction_at(x + w, y + h, W, H).expect("hit");
        assert_eq!(end, 1.0);
    }

    #[test]
    fn scrub_hit_outside_track_returns_none() {
        let (x, y, _, _) = scrub_track_rect(W, H).expect("track exists");
        assert!(scrub_fraction_at(x, y - SCRUB_HIT_PAD - 1.0, W, H).is_none());
        assert!(scrub_fraction_at(x, H / 2.0, W, H).is_none());
    }

    #[test]
    fn drag_fraction_clamps_x_to_track() {
        assert_eq!(scrub_drag_fraction(-100.0, W, H), Some(0.0));
        assert_eq!(scrub_drag_fraction(W + 100.0, W, H), Some(1.0));
    }

    #[test]
    fn tiny_windows_have_no_scrub_bar() {
        assert!(scrub_track_rect(20.0, 10.0).is_none());
        assert!(scrub_fraction_at(5.0, 5.0, 20.0, 10.0).is_none());
    }
}
