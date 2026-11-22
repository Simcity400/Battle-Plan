
use content_reader_localisation::{localisation::Argument, localisation::Localisation};

use super::fixed::{self, Point};
use viewer_native_gui::text::{self, Font, GlyphMetrics};

const FONT_PIXEL: f32 = 0.05;
const BOUND: f32 = 9999.0;
const RUN_MERGE: f32 = 0.25;

const KEY_FULL: &str = "MAP_ARROWS_LOCALIZATION";
const KEY_EMPTY: &str = "MAP_ARROWS_LOCALIZATION_EMPTY";
const KEY_EMPTY_SHORT: &str = "MAP_ARROWS_LOCALIZATION_EMPTY_SHORT";
const KEY_DIVISIONS: &str = "MAP_ARROWS_DIVISIONS_COUNT";
const KEY_DIVISIONS_SHORT: &str = "MAP_ARROWS_DIVISIONS_COUNT_SHORT";
const KEY_PARATROOPERS: &str = "MAP_ARROWS_PARATROOPERS_COUNT";
const KEY_INVASION_TRANSPORTS: &str = "MAP_ARROWS_INVASION_TRANSPORTS_COUNT";
const KEY_PARADROP_TRANSPORTS: &str = "MAP_ARROWS_PARADROP_TRANSPORTS_COUNT";
const ORDER_NAME_SEPARATOR: &str = " - ";

#[derive(Clone, Copy)]
pub struct LabelInputs<'a> {
    pub font: &'a Font,
    pub localisation: &'a Localisation,
}

#[derive(Clone, Copy, Debug, Default)]
pub struct OrderName<'a> {
    pub prefix: &'a [u8],
    pub name: &'a [u8],
    pub first: &'a [u8],
    pub second: &'a [u8],
    pub unique: &'a [u8],
    pub postfix: &'a [u8],
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Variant {
    Full,
    Middle,
    Short,
}

impl Variant {
    const TRIED: [Self; 3] = [Self::Full, Self::Middle, Self::Short];
}

fn localise(
    localisation: &Localisation,
    key: &[u8],
    arguments: &[(&str, Argument<'_>)],
) -> Result<String, String> {
    localisation
        .localise_with(key, arguments)
        .map_err(|refusal| refusal.to_string())
}

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

#[derive(Clone, Copy)]
pub struct Composer<'a> {
    pub localisation: &'a Localisation,
    pub order_type: i32,
    pub name: OrderName<'a>,
    pub group_name: &'a [u8],
    pub army_group: bool,
    pub count: usize,
}

impl Composer<'_> {
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

    pub fn drawn(&self, variant: Variant) -> Result<String, String> {
        let text = self.text(variant)?;
        drawable(text)
    }

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

pub trait LabelFont: GlyphMetrics {
    fn page(&self, ch: char) -> Option<usize>;
}

impl LabelFont for Font {
    fn page(&self, ch: char) -> Option<usize> {
        self.page_of(ch)
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
struct RunGlyph {
    left: f32,
    right: f32,
    bottom: f32,
    top: f32,
    uv: [f32; 4],
    page: usize,
    color: Option<[f32; 4]>,
}

fn centred_run(font: &dyn LabelFont, text: &str, size: f32) -> Vec<RunGlyph> {
    let chars = text::color_runs(text, |code| font.text_color(code));
    let mut width = 0.0f32;
    let mut previous = None;
    for item in &chars {
        if item.reset_kerning {
            previous = None;
        }
        let Some(glyph) = font.glyph(item.ch) else {
            previous = None;
            continue;
        };
        if let Some(p) = previous {
            width += font.kerning(p, item.ch);
        }
        width += glyph.xadvance;
        previous = Some(item.ch);
    }
    let scale = FONT_PIXEL * size;
    let line = font.line_height();
    let mut pen = -((width as i32) / 2) as f32;
    let mut previous = None;
    let mut out = Vec::with_capacity(chars.len());
    for item in &chars {
        if item.reset_kerning {
            previous = None;
        }
        let Some(glyph) = font.glyph(item.ch) else {
            previous = None;
            continue;
        };
        if let Some(p) = previous {
            pen += font.kerning(p, item.ch);
        }
        previous = Some(item.ch);
        let left = pen + glyph.xoffset;
        pen += glyph.xadvance;
        if glyph.height <= 0.0 {
            continue;
        }
        let Some(page) = font.page(item.ch) else {
            continue;
        };
        out.push(RunGlyph {
            left: left * scale,
            right: (left + glyph.width) * scale,
            bottom: (line - glyph.yoffset - glyph.height) * scale,
            top: (line - glyph.yoffset) * scale,
            uv: glyph.uv,
            page,
            color: item.color,
        });
    }
    out
}

#[cfg(test)]
pub fn text_path(controls: &[Point]) -> Vec<[f32; 2]> {
    indexed_text_path(controls).0
}

type IndexedPath = (Vec<[f32; 2]>, Vec<u32>);

type Runs = Vec<(Vec<([f32; 2], u32)>, bool)>;

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

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct LabelGlyph {
    pub corners: [[f32; 3]; 4],
    pub uv: [f32; 4],
    pub page: usize,
    pub color: Option<[f32; 4]>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct PlacedLabel {
    pub text: String,
    pub glyphs: Vec<LabelGlyph>,
}

enum Fit {
    Placed(Vec<LabelGlyph>),
    TooWide,
    PastEnd,
}

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
            color: glyph.color,
        });
    }
    Fit::Placed(out)
}

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
