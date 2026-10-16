//! Battle-plan name labels. Compose three variants, then lay the first that
//! fits along the arrow in the `arrowText` font. Draw with the `mapname` effect
//! after the ribbons.
//!
//! Composition, in the viewer's own language:
//!
//! * `$ORDERNAME$` is the order name plus a separator for move, invasion and
//!   paradrop orders, and empty for lines. Name parts are a localized prefix,
//!   stored unique text or localized name (else localized first and second
//!   parts), then a localized postfix, separated with spaces.
//! * Count assigned members except the display group's HQ member, which is
//!   excluded only while the group has a leader.
//! * full: `MAP_ARROWS_LOCALIZATION` with `$DIVISIONS$` (`MAP_ARROWS_DIVISIONS_COUNT`
//!   or, for a paradrop, `MAP_ARROWS_PARATROOPERS_COUNT`, with `$DIV$`) and
//!   `$GROUP$` (the stored group name); with no count an army shows
//!   `MAP_ARROWS_LOCALIZATION_EMPTY` and an army group `$ORDERNAME$` and its name.
//!   middle: the count string or `MAP_ARROWS_LOCALIZATION_EMPTY`; short: the
//!   `_SHORT` keys. The keys are the executable's literals.
//! * only the full variant appends the invasion/paradrop transport count,
//!   including zero, when the arrow's control count is strictly above
//!   `ARROW_EXT_TEXT_POINTS_LIMIT`. The invasion
//!   supplier reads the order's allocated convoy count; the paradrop supplier
//!   counts eligible transport wings at its source base.
//!
//! Placement: use the longest run of stored controls whose x keeps one
//! direction, read left to right. Scale the glyph run by 0.05 * text_size world
//! units per font pixel. A variant fits when its width is at most the path
//! arc length. Centre it on the path with text_offset; each glyph centre follows
//! the arc, turned to the mean of the current and previous segment and centred
//! across the line. No variant fitting draws no text.

use installed_content::localisation::{Argument, Localisation};

use super::fixed::{self, Point};
use viewer_foundation::map::text::{self, Font, GlyphMetrics};

/// World units per font pixel at font size 1.
const FONT_PIXEL: f32 = 0.05;
/// Initial minimum and maximum for the bounds.
const BOUND: f32 = 9999.0;
/// Run-merge ratio.
const RUN_MERGE: f32 = 0.25;

/// The exe's label keys and substitution names.
const KEY_FULL: &str = "MAP_ARROWS_LOCALIZATION";
const KEY_EMPTY: &str = "MAP_ARROWS_LOCALIZATION_EMPTY";
const KEY_EMPTY_SHORT: &str = "MAP_ARROWS_LOCALIZATION_EMPTY_SHORT";
const KEY_DIVISIONS: &str = "MAP_ARROWS_DIVISIONS_COUNT";
const KEY_DIVISIONS_SHORT: &str = "MAP_ARROWS_DIVISIONS_COUNT_SHORT";
const KEY_PARATROOPERS: &str = "MAP_ARROWS_PARATROOPERS_COUNT";
const KEY_INVASION_TRANSPORTS: &str = "MAP_ARROWS_INVASION_TRANSPORTS_COUNT";
const KEY_PARADROP_TRANSPORTS: &str = "MAP_ARROWS_PARADROP_TRANSPORTS_COUNT";
const ORDER_NAME_SEPARATOR: &str = " - ";

/// What label building reads from the content.
#[derive(Clone, Copy)]
pub struct LabelInputs<'a> {
    /// The `arrowText` font (`maparrows.txt`), loaded with the map icons.
    pub font: &'a Font,
    pub localisation: &'a Localisation,
}

/// Recorded name strings of one order.
#[derive(Clone, Copy, Debug, Default)]
pub struct OrderName<'a> {
    pub prefix: &'a [u8],
    pub name: &'a [u8],
    pub first: &'a [u8],
    pub second: &'a [u8],
    pub unique: &'a [u8],
    pub postfix: &'a [u8],
}

/// The three label variants, in the order the layout tries them.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Variant {
    Full,
    Middle,
    Short,
}

impl Variant {
    const TRIED: [Self; 3] = [Self::Full, Self::Middle, Self::Short];
}

/// Localize a recorded name token or a `MAP_ARROWS_*` label key. A key absent
/// from the content uses its own bytes; the error names what cannot be shown.
fn localise(
    localisation: &Localisation,
    key: &[u8],
    arguments: &[(&str, Argument<'_>)],
) -> Result<String, String> {
    localisation
        .localise_with(key, arguments)
        .map_err(|refusal| refusal.to_string())
}

/// Use the unique name as stored; localize the other recorded name tokens
/// with no arguments.
pub fn order_name(localisation: &Localisation, name: &OrderName<'_>) -> Result<String, String> {
    let mut out = String::new();
    if !name.prefix.is_empty() {
        out.push_str(&localise(localisation, name.prefix, &[])?);
        out.push(' ');
    }
    if !name.unique.is_empty() {
        out.push_str(
            std::str::from_utf8(name.unique)
                .map_err(|_| "the order's unique name is not UTF-8".to_owned())?,
        );
    } else if !name.name.is_empty() {
        out.push_str(&localise(localisation, name.name, &[])?);
    } else {
        out.push_str(&localise(localisation, name.first, &[])?);
        out.push(' ');
        out.push_str(&localise(localisation, name.second, &[])?);
    }
    if !name.postfix.is_empty() {
        out.push(' ');
        out.push_str(&localise(localisation, name.postfix, &[])?);
    }
    Ok(out)
}

/// Operands for three label variants. Layout builds each only when tried,
/// so a full label that fits does not localize shorter variants.
#[derive(Clone, Copy)]
pub struct Composer<'a> {
    pub localisation: &'a Localisation,
    pub order_type: i32,
    pub name: OrderName<'a>,
    /// The display group's recorded name, copied as stored.
    pub group_name: &'a [u8],
    pub army_group: bool,
    /// Assigned members that are not the display group's HQ unit.
    pub count: usize,
}

impl Composer<'_> {
    /// One variant's localized text: `DIV` is an integer (type 5); `DIVISIONS`,
    /// `GROUP` and `ORDERNAME` are strings (type 1). The error names what cannot be shown.
    pub fn text(&self, variant: Variant) -> Result<String, String> {
        let localisation = self.localisation;
        let counted = |key: &str| {
            localise(
                localisation,
                key.as_bytes(),
                &[("DIV", Argument::Int32(self.count as i32))],
            )
        };
        let counting = self.count > 0;
        match variant {
            Variant::Middle if counting => counted(KEY_DIVISIONS),
            Variant::Middle => counted(KEY_EMPTY),
            Variant::Short if counting => counted(KEY_DIVISIONS_SHORT),
            Variant::Short => counted(KEY_EMPTY_SHORT),
            Variant::Full => {
                // Each operand is made only on the branch that uses it.
                let order_name = || -> Result<String, String> {
                    Ok(
                        if matches!(
                            self.order_type,
                            super::TYPE_MOVE | super::TYPE_INVASION | super::TYPE_PARADROP
                        ) {
                            format!(
                                "{}{ORDER_NAME_SEPARATOR}",
                                order_name(localisation, &self.name)?
                            )
                        } else {
                            String::new()
                        },
                    )
                };
                let group_name = || {
                    std::str::from_utf8(self.group_name)
                        .map_err(|_| "the order's display group name is not UTF-8".to_owned())
                };
                if !counting {
                    if self.army_group {
                        Ok(format!("{}{}", order_name()?, group_name()?))
                    } else {
                        localise(localisation, KEY_EMPTY.as_bytes(), &[])
                    }
                } else {
                    let divisions = counted(if self.order_type == super::TYPE_PARADROP {
                        KEY_PARATROOPERS
                    } else {
                        KEY_DIVISIONS
                    })?;
                    localise(
                        localisation,
                        KEY_FULL.as_bytes(),
                        &[
                            ("DIVISIONS", Argument::Text(&divisions)),
                            ("GROUP", Argument::Text(group_name()?)),
                            ("ORDERNAME", Argument::Text(&order_name()?)),
                        ],
                    )
                }
            }
        }
    }

    /// One variant as map text draws it: [`Composer::text`], refused by name
    /// when it holds a control map text does not draw.
    pub fn drawn(&self, variant: Variant) -> Result<String, String> {
        let text = self.text(variant)?;
        drawable(text)
    }

    /// The extended full label. Read its simulation operands only when this
    /// variant needs the suffix. Bind the supplier's signed i32 under `TR`
    /// without clamping a negative value or omitting zero.
    pub fn drawn_with_transports(
        &self,
        variant: Variant,
        controls: usize,
        extended_limit: i32,
        transports: &dyn Fn() -> Result<i32, String>,
    ) -> Result<String, String> {
        if variant != Variant::Full || controls as i64 <= i64::from(extended_limit) {
            return self.drawn(variant);
        }
        let key = match self.order_type {
            super::TYPE_INVASION => KEY_INVASION_TRANSPORTS,
            super::TYPE_PARADROP => KEY_PARADROP_TRANSPORTS,
            _ => return self.drawn(variant),
        };
        let mut text = self.text(variant)?;
        text.push_str(&localise(
            self.localisation,
            key.as_bytes(),
            &[("TR", Argument::Int32(transports()?))],
        )?);
        drawable(text)
    }
}

fn drawable(text: String) -> Result<String, String> {
    match text::unsupported_control(&text) {
        Some(control) => Err(format!("it holds {control}, which map text does not draw")),
        None => Ok(text),
    }
}

/// A font the layout can read: metrics and each glyph's page.
pub trait LabelFont: GlyphMetrics {
    fn page(&self, ch: char) -> Option<usize>;
}

impl LabelFont for Font {
    fn page(&self, ch: char) -> Option<usize> {
        self.page_of(ch)
    }
}

/// One glyph of the centred run: its quad in the run's own frame (x along,
/// z up), its atlas rectangle and page.
#[derive(Clone, Copy, Debug, PartialEq)]
struct RunGlyph {
    left: f32,
    right: f32,
    bottom: f32,
    top: f32,
    uv: [f32; 4],
    page: usize,
}

/// For one line, start the pen at minus half the integer run width. Skip a
/// character the font lacks and break kerning; a glyph without height advances
/// but emits nothing.
fn centred_run(font: &dyn LabelFont, text: &str, size: f32) -> Vec<RunGlyph> {
    let chars: Vec<char> = text::strip_color_codes(text).chars().collect();
    let mut width = 0.0f32;
    let mut previous = None;
    for &ch in &chars {
        let Some(glyph) = font.glyph(ch) else {
            previous = None;
            continue;
        };
        if let Some(p) = previous {
            width += font.kerning(p, ch);
        }
        width += glyph.xadvance;
        previous = Some(ch);
    }
    let scale = FONT_PIXEL * size;
    let line = font.line_height();
    let mut pen = -((width as i32) / 2) as f32;
    let mut previous = None;
    let mut out = Vec::with_capacity(chars.len());
    for &ch in &chars {
        let Some(glyph) = font.glyph(ch) else {
            previous = None;
            continue;
        };
        if let Some(p) = previous {
            pen += font.kerning(p, ch);
        }
        previous = Some(ch);
        let left = pen + glyph.xoffset;
        pen += glyph.xadvance;
        if glyph.height <= 0.0 {
            continue;
        }
        let Some(page) = font.page(ch) else { continue };
        out.push(RunGlyph {
            left: left * scale,
            right: (left + glyph.width) * scale,
            bottom: (line - glyph.yoffset - glyph.height) * scale,
            top: (line - glyph.yoffset) * scale,
            uv: glyph.uv,
            page,
        });
    }
    out
}

/// Split the stored controls except the last into runs of one x direction,
/// merge small runs into their neighbours, and read the longest left to right.
#[cfg(test)]
pub fn text_path(controls: &[Point]) -> Vec<[f32; 2]> {
    indexed_text_path(controls).0
}

/// Label-path points and their stored-control indices, passed as sample
/// indices to the arrow height callback.
type IndexedPath = (Vec<[f32; 2]>, Vec<u32>);

/// Runs of one x direction: each point with its control index, and whether
/// the run's x increases.
type Runs = Vec<(Vec<([f32; 2], u32)>, bool)>;

/// [`text_path`] with each point's control index.
fn indexed_text_path(controls: &[Point]) -> IndexedPath {
    if controls.len() < 2 {
        return (Vec::new(), Vec::new());
    }
    let mut runs: Runs = Vec::new();
    for index in 0..controls.len() - 1 {
        let increasing = controls[index + 1][0] > controls[index][0];
        if runs.last().map(|run| run.1) != Some(increasing) {
            runs.push((Vec::new(), increasing));
        }
        runs.last_mut()
            .expect("a run was pushed")
            .0
            .push((fixed::to_world(controls[index]), index as u32));
    }
    loop {
        let mut merged = false;
        let mut j = 0;
        while j < runs.len() {
            let len = runs[j].0.len() as f32;
            let keep = j < 1
                || (runs[j - 1].1 != runs[j].1 && runs[j - 1].0.len() as f32 * RUN_MERGE <= len);
            if keep {
                if j + 1 < runs.len()
                    && (runs[j + 1].1 == runs[j].1 || len < runs[j + 1].0.len() as f32 * RUN_MERGE)
                {
                    let mut points = runs.remove(j).0;
                    points.append(&mut runs[j].0);
                    runs[j].0 = points;
                    merged = true;
                    continue;
                }
            } else {
                let run = runs.remove(j);
                runs[j - 1].0.extend(run.0);
                merged = true;
                continue;
            }
            j += 1;
        }
        if !merged {
            break;
        }
    }
    let mut best: Option<(usize, f32)> = None;
    for (index, (points, _)) in runs.iter().enumerate() {
        let length: f32 = points.windows(2).map(|w| span(w[0].0, w[1].0)).sum();
        if length > best.map_or(0.0, |b| b.1) {
            best = Some((index, length));
        }
    }
    let Some((index, _)) = best else {
        return (Vec::new(), Vec::new());
    };
    let (mut points, increasing) = runs.swap_remove(index);
    if !increasing {
        points.reverse();
    }
    points.into_iter().unzip()
}

/// Retry with the stored controls except the last, reversed when they run
/// west overall.
fn retry_path(controls: &[Point]) -> IndexedPath {
    if controls.len() < 2 {
        return (Vec::new(), Vec::new());
    }
    let mut points: Vec<([f32; 2], u32)> = controls[..controls.len() - 1]
        .iter()
        .enumerate()
        .map(|(index, &p)| (fixed::to_world(p), index as u32))
        .collect();
    let dx: f32 = controls
        .windows(2)
        .take(controls.len() - 1)
        .map(|w| (w[1][0] - w[0][0]) as f32 / fixed::SCALE as f32)
        .sum();
    if dx < 0.0 {
        points.reverse();
    }
    points.into_iter().unzip()
}

fn span(a: [f32; 2], b: [f32; 2]) -> f32 {
    ((b[0] - a[0]).powi(2) + (b[1] - a[1]).powi(2)).sqrt()
}

fn normalized([x, z]: [f32; 2]) -> [f32; 2] {
    let length = (x * x + z * z).sqrt();
    if length > 0.0 {
        [x / length, z / length]
    } else {
        [x, z]
    }
}

/// One placed glyph, in the corner order the name pass draws: top-left,
/// top-right, bottom-right, bottom-left in the glyph's own frame.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct LabelGlyph {
    pub corners: [[f32; 3]; 4],
    pub uv: [f32; 4],
    pub page: usize,
}

/// A laid-out label: the variant that fitted and its glyphs.
#[derive(Clone, Debug, PartialEq)]
pub struct PlacedLabel {
    pub text: String,
    pub glyphs: Vec<LabelGlyph>,
}

enum Fit {
    Placed(Vec<LabelGlyph>),
    /// The run is wider than the path.
    TooWide,
    /// A glyph centre fell past the path's end.
    PastEnd,
}

/// `height` receives the world point and segment control index over the
/// control count, passed to the arrow height callback.
fn fit(
    run: &[RunGlyph],
    (path, indices): (&[[f32; 2]], &[u32]),
    controls: usize,
    text_offset: f32,
    height: &dyn Fn(f32, f32, f32) -> f32,
) -> Fit {
    let (mut min_x, mut max_x, mut min_z, mut max_z) = (BOUND, -BOUND, BOUND, -BOUND);
    for glyph in run {
        min_x = min_x.min(glyph.left).min(glyph.right);
        max_x = max_x.max(glyph.left).max(glyph.right);
        min_z = min_z.min(glyph.bottom).min(glyph.top);
        max_z = max_z.max(glyph.bottom).max(glyph.top);
    }
    let segments: Vec<f32> = path.windows(2).map(|w| span(w[0], w[1])).collect();
    let total: f32 = segments.iter().sum();
    let half_height = (max_z - min_z) * 0.5;
    let width = max_x - min_x;
    if total < width {
        return Fit::TooWide;
    }
    let mut along = total * 0.5 - width * 0.5 + text_offset;
    let mut previous_centre = 0.0;
    let mut out = Vec::with_capacity(run.len());
    for (index, glyph) in run.iter().enumerate() {
        let centre = (glyph.right - glyph.left) * 0.5 + glyph.left;
        if index > 0 {
            along += centre - previous_centre;
        }
        previous_centre = centre;
        let mut cumulative = 0.0;
        let mut found = None;
        for (k, &length) in segments.iter().enumerate() {
            if length <= 0.0 {
                continue;
            }
            cumulative += length;
            if cumulative >= along {
                found = Some((k, length));
                break;
            }
        }
        let Some((k, length)) = found else {
            return Fit::PastEnd;
        };
        let mut direction = [path[k + 1][0] - path[k][0], path[k + 1][1] - path[k][1]];
        if k > 0 {
            direction = [
                (direction[0] + path[k][0] - path[k - 1][0]) * 0.5,
                (direction[1] + path[k][1] - path[k - 1][1]) * 0.5,
            ];
        }
        let direction = normalized(direction);
        let into = along - (cumulative - length);
        let anchor = [
            path[k][0] + direction[0] * into,
            path[k][1] + direction[1] * into,
        ];
        let normal = normalized([-direction[1], direction[0]]);
        let place = |x: f32, z: f32| {
            let across = (z - min_z) - half_height;
            let ahead = x - centre;
            let px = normal[0] * across + anchor[0] + direction[0] * ahead;
            let pz = normal[1] * across + anchor[1] + direction[1] * ahead;
            let t = indices[k] as f32 / controls.max(1) as f32;
            [px, height(px, pz, t), pz]
        };
        out.push(LabelGlyph {
            corners: [
                place(glyph.left, glyph.top),
                place(glyph.right, glyph.top),
                place(glyph.right, glyph.bottom),
                place(glyph.left, glyph.bottom),
            ],
            uv: glyph.uv,
            page: glyph.page,
        });
    }
    Fit::Placed(out)
}

/// Use the first variant that fits, trying the retry path once per variant
/// when a glyph runs past the end.
///
/// `variant` builds a variant's text or says why it cannot be drawn. Build each
/// only when layout reaches it: a variant after the first that fits cannot
/// refuse the label. Compute the text path once per label and the retry path
/// at most once.
pub fn place(
    variant: &dyn Fn(Variant) -> Result<String, String>,
    controls: &[Point],
    font: &dyn LabelFont,
    text_size: f32,
    text_offset: f32,
    height: &dyn Fn(f32, f32, f32) -> f32,
) -> Result<Option<PlacedLabel>, String> {
    let path = indexed_text_path(controls);
    if path.0.len() < 2 {
        return Ok(None);
    }
    let full = variant(Variant::Full)?;
    if full.is_empty() {
        return Ok(None);
    }
    let mut retry: Option<IndexedPath> = None;
    for tried in Variant::TRIED {
        let other = (tried != Variant::Full)
            .then(|| variant(tried))
            .transpose()?;
        if other.as_ref().is_some_and(|text| *text == full) {
            continue;
        }
        let run = centred_run(font, other.as_deref().unwrap_or(&full), text_size);
        if run.is_empty() {
            continue;
        }
        let mut retried = false;
        loop {
            let along = if retried {
                retry.get_or_insert_with(|| retry_path(controls))
            } else {
                &path
            };
            match fit(
                &run,
                (&along.0, &along.1),
                controls.len(),
                text_offset,
                height,
            ) {
                Fit::Placed(glyphs) => {
                    return Ok(Some(PlacedLabel {
                        text: other.unwrap_or(full),
                        glyphs,
                    }));
                }
                Fit::TooWide => break,
                Fit::PastEnd if retried => break,
                Fit::PastEnd => retried = true,
            }
        }
    }
    Ok(None)
}

#[cfg(test)]
mod tests;
