//! Native combat tooltip presentation; no tooltip text or layout is recorded.
//!
//! `1402A4080` wraps through `1422861E0`, then gives `CTextBox` its text with
//! the final argument set. `14228EF60` supplies zero height to `CTextSprite`,
//! so the authored `maxHeight` does not truncate tooltips. The first wrapping
//! pass subtracts the horizontal border twice before `142285B10` subtracts
//! it twice again (instructions `1422862D4..142286317`, `142285B58..61`).
//! `14228C8D0` then measures the resulting lines and adds a border on every
//! side. For the shipped ToolTip this means wrapping at 328 pixels, an
//! 18-pixel text inset, and a background fitted to the resulting text.
use super::{Assets, Chunk, Drawn, Painter, invalid, layout, window_layer};
use crate::map::{names::GlyphMetrics, text};
use installed_content::defines::Defines;
use std::{
    io,
    time::{Duration, Instant},
};

/// `NInterface.TOOLTIP_SCREEN_{LEFT,TOP,RIGHT,BOTTOM}_OFFSET_{X,Y}` and the
/// `NGraphics.TOOLTIP_{SHOW,DELAYED,HIDE}_DELAY` seconds `1402A4080` reads.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct TooltipDefines {
    offsets: [i32; 4],
    pub(super) timing: [f32; 3],
}

impl TooltipDefines {
    pub(crate) fn read(defines: &Defines) -> io::Result<Self> {
        let offset = |name: &str| defines.int32("NInterface", name);
        // The game keeps each delay as a float (14331D158, 14331D1E8,
        // 14331D25C) and only compares it (1402A4080): any value is a delay,
        // a negative one included.
        let delay =
            |name: &str| -> io::Result<f32> { Ok(defines.number("NGraphics", name)? as f32) };
        Ok(Self {
            offsets: [
                offset("TOOLTIP_SCREEN_LEFT_OFFSET_X")?,
                offset("TOOLTIP_SCREEN_TOP_OFFSET_Y")?,
                offset("TOOLTIP_SCREEN_RIGHT_OFFSET_X")?,
                offset("TOOLTIP_SCREEN_BOTTOM_OFFSET_Y")?,
            ],
            timing: [
                delay("TOOLTIP_SHOW_DELAY")?,
                delay("TOOLTIP_DELAYED_DELAY")?,
                delay("TOOLTIP_HIDE_DELAY")?,
            ],
        })
    }
}

/// Per-viewer tooltip clocks, corresponding to the controller's presentation
/// clocks in `1402A4080`. A widget rectangle identifies the current binding:
/// refreshing its simulation values does not restart the delayed-body timer.
///
/// The show and hide timers are deadlines on the presentation clock, as the
/// game keeps them (14331730C, 143317308): seconds since the clock started
/// (ReadPresentationClockSeconds 142238C90 accumulates from its start), as a
/// float, with a deadline at or below 0 meaning unset. The viewer's clock
/// starts at its first frame.
#[derive(Default)]
pub(super) struct HoverState {
    target: Option<[f32; 4]>,
    entered: Option<Instant>,
    delayed_pending: bool,
    /// The presentation clock's start.
    clock: Option<Instant>,
    /// The show deadline (14331730C); 0 or below is unset.
    show_deadline: f32,
    /// The hide deadline (143317308); 0 or below is unset.
    hide_deadline: f32,
    /// Text waits for the show deadline.
    showing: bool,
    /// The visible tooltip waits for the hide deadline.
    hiding: bool,
    visible: bool,
    text: String,
    /// The next frame's text, kept to reuse its buffer.
    candidate: String,
    point: [f32; 2],
}

impl HoverState {
    /// Delays are `[show, delayed-body, hide]` seconds. Call this on every
    /// requested frame, including frames without a hit, so hiding can finish.
    /// An already visible tooltip updates immediately when its binding changes;
    /// the show delay applies only while the CTextBox is hidden in the game.
    pub(super) fn shown(
        &mut self,
        hit: Option<(&str, Option<&str>, [f32; 4])>,
        point: Option<[f32; 2]>,
        now: Instant,
        timing: [f32; 3],
    ) -> Option<(&str, [f32; 2])> {
        let clock = *self.clock.get_or_insert(now);
        let seconds = now.saturating_duration_since(clock).as_secs_f32();
        if let Some(point) = point {
            self.point = point;
        }
        // 1402A4080 tests the stored byte lengths, then appends those bytes.
        // Spaces and authored leading newlines are part of the tooltip.
        let hit = point.and(hit).filter(|(text, delayed, _)| {
            !text.is_empty() || delayed.is_some_and(|s| !s.is_empty())
        });
        let mut candidate = std::mem::take(&mut self.candidate);
        candidate.clear();
        if let Some((text, delayed, rect)) = hit {
            if self.target != Some(rect) {
                self.target = Some(rect);
                self.entered = Some(now);
            }
            candidate.push_str(text);
            let delayed = delayed.filter(|text| !text.is_empty());
            self.delayed_pending = delayed.is_some() && !past(self.entered, now, timing[1]);
            if !self.delayed_pending
                && let Some(delayed) = delayed
            {
                if !candidate.is_empty() {
                    candidate.push_str("\n--------------\n");
                }
                candidate.push_str(delayed);
            }
        } else {
            self.target = None;
            self.entered = None;
            self.delayed_pending = false;
        }
        if candidate.is_empty() {
            self.show_deadline = 0.0;
            self.showing = false;
            self.hiding = self.visible;
            if self.visible {
                // 1402A5C0B sets an unset deadline; 1402A5C3B hides once
                // `0 < deadline < now`.
                if self.hide_deadline <= 0.0 {
                    self.hide_deadline = seconds + timing[2];
                }
                if 0.0 < self.hide_deadline && self.hide_deadline < seconds {
                    self.visible = false;
                    self.hiding = false;
                    self.hide_deadline = 0.0;
                    self.text.clear();
                }
            }
            self.candidate = candidate;
        } else {
            self.hide_deadline = 0.0;
            self.hiding = false;
            self.candidate = std::mem::replace(&mut self.text, candidate);
            if !self.visible {
                self.showing = true;
                // 1402A5AF1 sets an unset deadline; 1402A5B1C shows unless
                // `deadline <= 0 || now <= deadline`.
                if self.show_deadline <= 0.0 {
                    self.show_deadline = seconds + timing[0];
                }
                if shows(self.show_deadline, seconds) {
                    self.visible = true;
                    self.showing = false;
                    self.show_deadline = 0.0;
                }
            }
        }
        self.visible.then_some((self.text.as_str(), self.point))
    }

    /// Request another frame at the next transition even with a stationary
    /// pointer. The native comparisons are strict (`deadline < now`), so a
    /// deadline already reached schedules the following clock quantum. An
    /// unset deadline (at or below 0) is set again on each frame, and holds
    /// once the clock passes the negated delay.
    pub(super) fn next_change(&self, now: Instant, timing: [f32; 3]) -> Option<Duration> {
        let seconds = self.clock.map_or(0.0, |clock| {
            now.saturating_duration_since(clock).as_secs_f32()
        });
        let deadline = |pending: bool, deadline: f32, delay: f32| {
            pending.then(|| {
                if deadline > 0.0 {
                    deadline - seconds
                } else {
                    -delay - seconds
                }
            })
        };
        [
            deadline(self.showing, self.show_deadline, timing[0]),
            self.entered
                .filter(|_| self.delayed_pending)
                .map(|start| timing[1] - now.saturating_duration_since(start).as_secs_f32()),
            deadline(self.hiding, self.hide_deadline, timing[2]),
        ]
        .into_iter()
        .flatten()
        .filter_map(|left| {
            // Only the interval left is a duration: none left is the next
            // quantum; a NaN or infinite one never passes (the comparisons
            // are false), so it schedules nothing.
            if !left.is_finite() {
                return None;
            }
            Duration::try_from_secs_f32(left.max(0.0))
                .ok()?
                .checked_add(Duration::from_nanos(1))
        })
        .min()
    }
}

/// 1402A4080's delayed-body test, `delay < now - start`: a negative delay
/// passes at once, a NaN never.
fn past(start: Option<Instant>, now: Instant, delay: f32) -> bool {
    start.is_some_and(|start| delay < now.saturating_duration_since(start).as_secs_f32())
}

/// 1402A4080's show test (1402A5B1C): it skips while `deadline <= 0` or
/// `now <= deadline`, so a NaN deadline shows at once.
fn shows(deadline: f32, now: f32) -> bool {
    use std::cmp::Ordering::{Equal, Less};
    !matches!(deadline.partial_cmp(&0.0), Some(Less | Equal))
        && !matches!(now.partial_cmp(&deadline), Some(Less | Equal))
}

struct Atom {
    chunk: Chunk,
    advance: f32,
    kerning: f32,
    height: f32,
    ends_word: bool,
}
impl Atom {
    fn step(&self) -> f32 {
        self.advance + self.kerning
    }

    fn is_space(&self) -> bool {
        matches!(&self.chunk, Chunk::Text(value, _) if value == " ")
    }
}

struct Line {
    atoms: Vec<Atom>,
    width: f32,
    height: f32,
}
impl Line {
    fn new(mut atoms: Vec<Atom>, line_height: f32) -> Self {
        // The later draw pass sees the inserted newline and starts a new
        // glyph run even when the first pass split an overlong word.
        if let Some(first) = atoms.first_mut() {
            first.kerning = 0.0;
        }
        let width = atoms.iter().map(Atom::step).sum::<f32>()
            - atoms
                .last()
                .filter(|atom| atom.ends_word)
                .map_or(0.0, Atom::step);
        let height = atoms
            .iter()
            .fold(line_height, |height, atom| height.max(atom.height));
        Self {
            atoms,
            width,
            height,
        }
    }
}

/// `142285B10` wraps when a glyph reaches the width, moves the unfinished
/// word to the next line, and breaks an overlong word at the current glyph.
/// `142287190` carries leading spaces onto automatically wrapped lines.
fn wrap(atoms: Vec<Atom>, width: f32, line_height: f32) -> Vec<Line> {
    let indent: Vec<_> = atoms.iter().take_while(|atom| atom.is_space()).collect();
    let indent_width: f32 = indent.iter().map(|atom| atom.step()).sum();
    let indent = if indent_width < width {
        indent
            .iter()
            .map(|atom| match &atom.chunk {
                Chunk::Text(_, color) => (*color, atom.advance, atom.height),
                _ => unreachable!("leading spaces are text"),
            })
            .collect::<Vec<_>>()
    } else {
        Vec::new()
    };
    let mut lines = Vec::new();
    let mut finished = Vec::new();
    let mut word = Vec::new();
    let (mut line_width, mut word_width) = (0.0_f32, 0.0_f32);
    for atom in atoms {
        let step = atom.step();
        if (!atom.ends_word || word_width == 0.0) && width <= line_width + step {
            if line_width == word_width {
                finished.append(&mut word);
                word_width = 0.0;
            }
            lines.push(Line::new(std::mem::take(&mut finished), line_height));
            for &(color, advance, height) in &indent {
                finished.push(Atom {
                    chunk: Chunk::Text(" ".into(), color),
                    advance,
                    kerning: 0.0,
                    height,
                    ends_word: true,
                });
                word_width += advance;
            }
            finished.append(&mut word);
            line_width = word_width;
        }
        line_width += step;
        word_width += step;
        let ends_word = atom.ends_word;
        word.push(atom);
        if ends_word {
            finished.append(&mut word);
            word_width = 0.0;
        }
    }
    finished.append(&mut word);
    lines.push(Line::new(finished, line_height));
    lines
}

/// Default, unanchored tooltip placement from `1402A4080`, in interface
/// pixels. Producer-specific relative anchors and its above-pointer flag are
/// not used by the land window.
fn position(point: [f32; 2], size: [f32; 2], extent: [f32; 2], offsets: [i32; 4]) -> [f32; 2] {
    let point = point.map(f32::trunc);
    let mut x = (point[0] + 21.0).max(offsets[0] as f32);
    let mut y = (point[1] + 24.0).max(offsets[1] as f32);
    if x > extent[0] - size[0] - offsets[2] as f32 {
        x = point[0] - size[0] - 20.0;
    }
    if y > extent[1] - size[1] - offsets[3] as f32 {
        y = point[1] - size[1] - 20.0;
    }
    [x.max(0.0), y.max(0.0)]
}

pub(super) fn draw(
    assets: &Assets,
    drawn: &mut Drawn,
    value: &str,
    point: [f32; 2],
    extent: [f32; 2],
) -> io::Result<()> {
    if value.is_empty() {
        return Ok(());
    }
    let Ok(defines) = &assets.tooltip_defines else {
        // The asset loader reports the unsupported definitions.
        return Ok(());
    };
    let node = assets
        .windows
        .get("ToolTip")
        .ok_or_else(|| invalid("missing ToolTip declaration"))?;
    let font = installed_content::gui::font(&node.items)
        .and_then(|name| assets.fonts.get(name))
        .ok_or_else(|| invalid("missing ToolTip font"))?;
    let sprite = assets
        .sprite("ToolTip")
        .ok_or_else(|| invalid("missing ToolTip background"))?;
    let border = sprite
        .border
        .ok_or_else(|| invalid("missing ToolTip borderSize"))?;
    let max_width = installed_content::gui::number(&node.items, "maxWidth")?
        .ok_or_else(|| invalid("missing ToolTip maxWidth"))? as f32;
    let width = max_width - 4.0 * border[0];
    if !width.is_finite() || width <= 0.0 || border.iter().any(|v| !v.is_finite() || *v < 0.0) {
        return Err(invalid("invalid ToolTip dimensions"));
    }
    if layout::boolean(&node.items, "fixedsize").map_err(invalid)? == Some(true) {
        return Err(invalid("fixed-size ToolTip content is unsupported"));
    }
    let mut painter = Painter {
        assets,
        drawn,
        hovers: false,
    };
    let mut color = font.color();
    let mut lines = Vec::new();
    for paragraph in value.split('\n') {
        let mut atoms = Vec::new();
        for chunk in painter.chunks(font, paragraph, &mut color) {
            // Even a zero-width colour marker resets the previous glyph in
            // 142285B10 after 1422894D0 consumes the formatting code.
            let mut previous = None;
            match chunk {
                Chunk::Text(run, color) => {
                    for ch in run.chars() {
                        let glyph = GlyphMetrics::glyph(font, ch);
                        let kerning = glyph
                            .as_ref()
                            .and(previous)
                            .map_or(0.0, |p| GlyphMetrics::kerning(font, p, ch));
                        atoms.push(Atom {
                            chunk: Chunk::Text(ch.to_string(), color),
                            advance: glyph.as_ref().map_or(0.0, |glyph| glyph.xadvance),
                            kerning,
                            height: font.line_height(),
                            ends_word: ch == ' '
                                || glyph.as_ref().is_some_and(|glyph| glyph.height == 0.0),
                        });
                        previous = glyph.map(|_| ch);
                    }
                }
                chunk => {
                    let height = match &chunk {
                        Chunk::Icon(sprite, _) => sprite.size[1],
                        _ => font.line_height(),
                    };
                    atoms.push(Atom {
                        advance: painter.length(font, &chunk),
                        chunk,
                        kerning: 0.0,
                        height,
                        ends_word: false,
                    });
                }
            }
        }
        lines.extend(wrap(atoms, width, font.line_height()));
    }
    let size = [
        lines
            .iter()
            .fold(0.0_f32, |width, line| width.max(line.width))
            + 2.0 * border[0],
        lines.iter().map(|line| line.height).sum::<f32>() + 2.0 * border[1],
    ];
    let at = position(point, size, extent, defines.offsets);
    painter
        .drawn
        .batch
        .at_layer(window_layer(painter.drawn.batch.quads.len()));
    painter.cornered(&sprite, 0, at, size, border, [1.0; 4]);
    let mut y = at[1] + border[1];
    for line in lines {
        let mut x = at[0] + border[0];
        for atom in line.atoms {
            x += atom.kerning;
            let top = y + (line.height - atom.height) * 0.5;
            painter
                .drawn
                .batch
                .at_layer(window_layer(painter.drawn.batch.quads.len()));
            match atom.chunk {
                Chunk::Text(value, color) => painter.drawn.batch.text(
                    &text::layout(font, &value, [x, top], text::Align::Left, None),
                    color,
                ),
                Chunk::Icon(sprite, frame) => {
                    painter.drawn.batch.scaled(
                        &sprite.sheet,
                        frame,
                        [x, top],
                        sprite.size,
                        [1.0; 4],
                    );
                }
                Chunk::Flag(key) => {
                    if let Some((texture, uv)) = assets.inline_flag(&key) {
                        painter.drawn.batch.cell(
                            texture,
                            [x, top + 5.0],
                            assets.inline_flag_size,
                            uv,
                            [1.0; 4],
                        );
                    }
                }
            }
            x += atom.advance;
        }
        y += line.height;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn text_atoms(value: &str) -> Vec<Atom> {
        value
            .chars()
            .map(|ch| Atom {
                chunk: Chunk::Text(ch.to_string(), [1.0; 4]),
                advance: 1.0,
                kerning: 0.0,
                height: 2.0,
                ends_word: ch == ' ',
            })
            .collect()
    }

    fn line_text(line: &Line) -> String {
        line.atoms
            .iter()
            .filter_map(|atom| match &atom.chunk {
                Chunk::Text(value, _) => Some(value.as_str()),
                _ => None,
            })
            .collect()
    }

    #[test]
    fn wraps_at_reaching_width_and_carries_a_pending_word() {
        let lines = wrap(text_atoms("aa bbb"), 6.0, 2.0);
        assert_eq!(
            lines.iter().map(line_text).collect::<Vec<_>>(),
            ["aa ", "bbb"]
        );
        assert_eq!(
            lines.iter().map(|line| line.width).collect::<Vec<_>>(),
            [2.0, 3.0]
        );
        let lines = wrap(text_atoms("aaaaa"), 4.0, 2.0);
        assert_eq!(
            lines.iter().map(line_text).collect::<Vec<_>>(),
            ["aaa", "aa"]
        );
    }

    #[test]
    fn wrapped_lines_preserve_paragraph_indentation() {
        let lines = wrap(text_atoms("  aa bb"), 6.0, 2.0);
        assert_eq!(
            lines.iter().map(line_text).collect::<Vec<_>>(),
            ["  aa ", "  bb"]
        );
    }

    #[test]
    fn tooltip_edge_reversal_uses_the_pointer_and_twenty_pixel_gap() {
        assert_eq!(
            position([100.9, 200.1], [300.0, 80.0], [1000.0, 600.0], [0; 4]),
            [121.0, 224.0]
        );
        assert_eq!(
            position([900.0, 550.0], [300.0, 80.0], [1000.0, 600.0], [0; 4]),
            [580.0, 450.0]
        );
        assert_eq!(
            position([5.0, 5.0], [300.0, 80.0], [100.0, 50.0], [0; 4]),
            [0.0, 0.0]
        );
    }

    const TIMING: [f32; 3] = [0.05, 1.0, 0.05];
    const RECT: [f32; 4] = [1.0, 2.0, 3.0, 4.0];
    const POINT: Option<[f32; 2]> = Some([2.0, 3.0]);

    #[test]
    fn hover_uses_strict_show_and_delayed_deadlines() {
        let start = Instant::now();
        let mut hover = HoverState::default();
        let hit = Some(("Header", Some("Body"), RECT));
        assert_eq!(hover.shown(hit, POINT, start, TIMING), None);
        assert!(hover.next_change(start, TIMING).is_some());
        assert_eq!(
            hover.shown(hit, POINT, start + Duration::from_millis(50), TIMING),
            None
        );
        assert_eq!(
            hover.shown(hit, POINT, start + Duration::from_millis(51), TIMING),
            Some(("Header", POINT.unwrap()))
        );
        assert_eq!(
            hover.shown(hit, POINT, start + Duration::from_secs(1), TIMING),
            Some(("Header", POINT.unwrap()))
        );
        assert_eq!(
            hover.shown(hit, POINT, start + Duration::from_millis(1001), TIMING),
            Some(("Header\n--------------\nBody", POINT.unwrap()))
        );
        assert_eq!(
            hover.next_change(start + Duration::from_millis(1001), TIMING),
            None
        );
    }

    #[test]
    fn hover_preserves_authored_whitespace_in_tooltip_parts() {
        // The clock runs a second before the hover, so a zero delay's
        // deadline is set (above 0).
        let clock = Instant::now();
        let start = clock + Duration::from_secs(1);
        let mut hover = HoverState::default();
        hover.shown(None, None, clock, [0.0; 3]);
        let hit = Some((" \n", Some(" x "), RECT));
        assert_eq!(hover.shown(hit, POINT, start, [0.0; 3]), None);
        assert_eq!(
            hover.shown(hit, POINT, start + Duration::from_millis(1), [0.0; 3]),
            Some((" \n\n--------------\n x ", POINT.unwrap()))
        );
    }

    #[test]
    fn hovering_new_values_keeps_clock_but_a_new_binding_restarts_delayed_text() {
        let start = Instant::now();
        let mut hover = HoverState::default();
        hover.shown(Some(("10", Some("details"), RECT)), POINT, start, TIMING);
        hover.shown(
            Some(("10", Some("details"), RECT)),
            POINT,
            start + Duration::from_millis(51),
            TIMING,
        );
        assert_eq!(
            hover.shown(
                Some(("11", Some("updated"), RECT)),
                POINT,
                start + Duration::from_millis(1001),
                TIMING
            ),
            Some(("11\n--------------\nupdated", POINT.unwrap()))
        );
        let next_rect = [5.0, 2.0, 3.0, 4.0];
        assert_eq!(
            hover.shown(
                Some(("New", Some("later"), next_rect)),
                POINT,
                start + Duration::from_millis(1100),
                TIMING
            ),
            Some(("New", POINT.unwrap()))
        );
    }

    #[test]
    fn hide_delay_retains_last_text_and_reentry_cancels_hiding() {
        let start = Instant::now();
        let mut hover = HoverState::default();
        let hit = Some(("Header", None, RECT));
        hover.shown(hit, POINT, start, TIMING);
        hover.shown(hit, POINT, start + Duration::from_millis(51), TIMING);
        assert_eq!(
            hover.shown(None, None, start + Duration::from_millis(100), TIMING),
            Some(("Header", POINT.unwrap()))
        );
        assert_eq!(
            hover.shown(None, None, start + Duration::from_millis(150), TIMING),
            Some(("Header", POINT.unwrap()))
        );
        assert_eq!(
            hover.shown(hit, POINT, start + Duration::from_millis(150), TIMING),
            Some(("Header", POINT.unwrap()))
        );
        assert_eq!(
            hover.next_change(start + Duration::from_millis(150), TIMING),
            None
        );
        hover.shown(None, None, start + Duration::from_millis(200), TIMING);
        assert_eq!(
            hover.shown(None, None, start + Duration::from_millis(251), TIMING),
            None
        );
    }

    /// The delays are the game's floats, only compared (1402A4080): a
    /// negative `TOOLTIP_DELAYED_DELAY` passes `delay < elapsed` at once, so
    /// the body joins on the first frame, and a negative show delay shows at
    /// once; neither disables the tooltip.
    #[test]
    fn negative_delays_follow_the_native_comparisons() {
        let defines = Defines::new().unwrap();
        defines
            .load(
                "common/defines/00_defines.lua",
                b"NDefines = {
                    NInterface = { TOOLTIP_SCREEN_LEFT_OFFSET_X = 0, TOOLTIP_SCREEN_TOP_OFFSET_Y = 0,
                        TOOLTIP_SCREEN_RIGHT_OFFSET_X = 0, TOOLTIP_SCREEN_BOTTOM_OFFSET_Y = 0 },
                    NGraphics = { TOOLTIP_SHOW_DELAY = -1, TOOLTIP_DELAYED_DELAY = -0.5, TOOLTIP_HIDE_DELAY = 0.05 } }",
            )
            .unwrap();
        let timing = TooltipDefines::read(&defines).unwrap().timing;
        assert_eq!(timing, [-1.0, -0.5, 0.05]);
        // Two seconds into the presentation clock the show deadline, one
        // second, is set and already passed.
        let clock = Instant::now();
        let start = clock + Duration::from_secs(2);
        let mut hover = HoverState::default();
        hover.shown(None, None, clock, timing);
        let hit = Some(("Header", Some("Body"), RECT));
        assert_eq!(
            hover.shown(hit, POINT, start, timing),
            Some(("Header\n--------------\nBody", POINT.unwrap()))
        );
        assert_eq!(hover.next_change(start, timing), None);
        // A NaN never passes `delay < elapsed`, so the body never joins, and
        // nothing is scheduled for it.
        let mut hover = HoverState::default();
        let nan = [0.0, f32::NAN, 0.05];
        hover.shown(None, None, clock, nan);
        hover.shown(hit, POINT, start, nan);
        assert_eq!(
            hover.shown(hit, POINT, start + Duration::from_secs(60), nan),
            Some(("Header", POINT.unwrap()))
        );
        assert_eq!(
            hover.next_change(start + Duration::from_secs(60), nan),
            None
        );
    }

    /// 1402A4080 keeps the show deadline on the presentation clock
    /// (142238C90) and reads one at or below 0 as unset, setting it again on
    /// each frame: with a show delay of -1, a hover in the clock's first
    /// second waits until the deadline `now - 1` rises above 0, then shows at
    /// once. The hide deadline follows the same rule.
    #[test]
    fn a_deadline_at_or_below_zero_on_the_presentation_clock_is_unset() {
        let clock = Instant::now();
        let at = |millis| clock + Duration::from_millis(millis);
        let timing = [-1.0, 1.0, -1.0];
        let mut hover = HoverState::default();
        let hit = Some(("Header", None, RECT));
        assert_eq!(hover.shown(hit, POINT, clock, timing), None);
        assert_eq!(hover.shown(hit, POINT, at(500), timing), None);
        // The deadline holds once the clock passes one second.
        let next = hover.next_change(at(500), timing).unwrap();
        assert!(
            next > Duration::from_millis(499) && next < Duration::from_millis(501),
            "{next:?}"
        );
        assert_eq!(
            hover.shown(hit, POINT, at(1100), timing),
            Some(("Header", POINT.unwrap()))
        );
        // The hide deadline, `now - 1`, is above 0 by now: it hides at once.
        assert_eq!(hover.shown(None, None, at(1200), timing), None);
        // In the clock's first second the hide waits instead.
        let timing = [0.05, 1.0, -1.0];
        let mut hover = HoverState::default();
        hover.shown(None, None, clock, timing);
        hover.shown(hit, POINT, at(100), timing);
        assert!(hover.shown(hit, POINT, at(200), timing).is_some());
        assert!(hover.shown(None, None, at(300), timing).is_some());
        assert_eq!(hover.shown(None, None, at(1100), timing), None);
    }

    #[test]
    fn delayed_only_tooltip_waits_for_body_then_show_delay_without_separator() {
        let start = Instant::now();
        let mut hover = HoverState::default();
        let hit = Some(("", Some("Details"), RECT));
        assert_eq!(hover.shown(hit, POINT, start, TIMING), None);
        assert_eq!(
            hover.shown(hit, POINT, start + Duration::from_millis(1001), TIMING),
            None
        );
        assert_eq!(
            hover.shown(hit, POINT, start + Duration::from_millis(1052), TIMING),
            Some(("Details", POINT.unwrap()))
        );
    }
}
