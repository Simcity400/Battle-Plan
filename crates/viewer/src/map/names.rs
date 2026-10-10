//! Country-name placement on the map.
//!
//! Glyph metrics arrive through [`GlyphMetrics`]. Font parsing, localisation
//! and GPU submission belong to callers. Public positions use the top-down
//! province bitmap; layout uses the bottom-up raster before truncation and
//! ordered Bresenham traversal. The caller converts quads to world x/z and
//! supplies terrain heights.
//!
//! For seam-crossing areas, the western half shifts by one map width. Sampling
//! wraps with `rem_euclid`, and the draw pass submits both seam copies.
use std::collections::{HashMap, HashSet};

/// Fraction of the fitted line the text fills.
pub const FILL: f32 = 0.9;
/// Half height and midpoint factor.
pub const HALF: f32 = 0.5;
/// Final scale multiplier applied after the width or height fit.
pub const FINAL_SCALE: f32 = 0.8;
/// Endpoint nudge step in map pixels for the three scored candidates.
pub const NUDGE: f32 = 3.0;
/// Bias of the axis choice towards horizontal.
pub const AXIS_BIAS: f32 = 1.2;
/// Converts the pixel scale into the font builder's design size units.
pub const FONT_SIZE_CONV: f32 = 20.0;
/// Fraction of the draw distance at which the per label fade starts.
pub const FADE_START: f32 = 0.7;
/// Most spaces the height limited branch inserts between every pair of characters.
pub const MAX_SPACES: usize = 19;
/// Weight the bend's run choice gives the run nearer the glyph.
pub const RUN_NEAR_BIAS: f32 = 1.4;

/// Upper bound on the points one rasterised line may produce.
///
/// A line across the vanilla map is about 5632 points. This only stops caller supplied degenerate
/// geometry from spinning, because [`place_name`] must never hang or panic on its input.
const MAX_LINE_POINTS: usize = 1 << 16;

/// Largest coordinate magnitude a fitted candidate may reach before it is discarded.
///
/// A near degenerate regression puts one endpoint at an astronomic y. Rasterising that is
/// pointless and would overflow the integer Bresenham, so such a candidate simply loses and the
/// other axis is used.
const MAX_COORD: f32 = 1.0e7;

/// Native finite-difference step for the cubic's tangent.
const TANGENT_STEP: f32 = 0.01;

/// One glyph's box in a bitmap font, in the font's own pixels at its native size.
///
/// The fields are a `hoi_mapfont4.fntchar` record: `xoffset` and `yoffset` are
/// measured from the top left of the line box, not from the baseline, and `xadvance` is the pen
/// step. `uv` is the atlas rectangle already normalised to `[0, 1]` as `[u0, v0, u1, v1]` with
/// `v0` the top edge, so this module never needs the atlas size.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct GlyphBox {
    /// Width of the glyph's atlas rectangle, in font pixels.
    pub width: f32,
    /// Height of the glyph's atlas rectangle, in font pixels.
    pub height: f32,
    /// Offset from the pen position to the rectangle's left edge, in font pixels.
    pub xoffset: f32,
    /// Offset from the line box top to the rectangle's top edge, in font pixels.
    pub yoffset: f32,
    /// Pen step after this glyph, in font pixels.
    pub xadvance: f32,
    /// Normalised atlas rectangle `[u0, v0, u1, v1]`.
    pub uv: [f32; 4],
}

/// Metrics of the bitmap font the names are built from.
///
/// The map font is `hoi_mapfont4`: `size = 84`, `lineHeight = 84`, `base = 70`, one 2048 x 1024
/// page, 575 characters. All lengths are in that native size's font pixels;
/// [`place_name`] multiplies them by the scale it computes to reach map pixels.
///
/// An implementation reports a missing character as `None`. Such a character is dropped from the
/// layout entirely rather than measured as zero, which keeps the measured width and the laid out
/// width equal.
pub trait GlyphMetrics {
    /// X advance of `ch` in font pixels, or `None` when the font has no such character.
    fn advance(&self, ch: char) -> Option<f32>;

    /// The font's `lineHeight`. This is the text height the engine's measure call returns for the
    /// single line names laid out here.
    fn line_height(&self) -> f32;

    /// The font's `base`: the baseline's distance below the top of the line box.
    fn base(&self) -> f32;

    /// The glyph box of `ch`, or `None` when the font has no such character.
    fn glyph(&self, ch: char) -> Option<GlyphBox>;

    /// Kerning between `a` and `b` in font pixels. `hoi_mapfont4` ships no kerning pairs, so the
    /// default is zero and the engine's measure and its layout agree with it.
    fn kerning(&self, a: char, b: char) -> f32 {
        let _ = (a, b);
        0.0
    }
}

/// The province id image the mask scan reads, in map pixel space.
#[derive(Clone, Copy, Debug)]
pub struct ProvinceMap<'a> {
    /// Image width in pixels.
    pub width: u32,
    /// Image height in pixels.
    pub height: u32,
    /// Row major province ids, `width * height` of them, starting at the top left.
    pub ids: &'a [u32],
}

impl ProvinceMap<'_> {
    /// True when the id slice matches the declared size and neither dimension is zero.
    fn is_well_formed(&self) -> bool {
        let expected = (self.width as usize).checked_mul(self.height as usize);
        self.width > 0 && self.height > 0 && expected == Some(self.ids.len())
    }

    /// The id at a pixel, wrapping x around the date line and rejecting y outside the image.
    fn id_at(&self, x: i32, y: i32) -> Option<u32> {
        if y < 0 || y >= self.height as i32 {
            return None;
        }
        let x = x.rem_euclid(self.width as i32) as usize;
        self.ids.get(y as usize * self.width as usize + x).copied()
    }
}

/// Inclusive pixel bounds of one province.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ProvinceBounds {
    /// Lowest x holding a pixel of this province.
    pub min_x: u32,
    /// Lowest y holding a pixel of this province.
    pub min_y: u32,
    /// Highest x holding a pixel of this province.
    pub max_x: u32,
    /// Highest y holding a pixel of this province.
    pub max_y: u32,
}

/// Bounding boxes of every province in the image, in one pass.
///
/// Cache province bounds once for many names and pass them in NameInput::bounds; place_name otherwise
/// rescans the full image per name.
pub fn scan_province_bounds(map: &ProvinceMap<'_>) -> HashMap<u32, ProvinceBounds> {
    let mut bounds: HashMap<u32, ProvinceBounds> = HashMap::new();
    if !map.is_well_formed() {
        return bounds;
    }
    for y in 0..map.height {
        let row = y as usize * map.width as usize;
        for x in 0..map.width {
            let id = map.ids[row + x as usize];
            bounds
                .entry(id)
                .and_modify(|b| {
                    b.min_x = b.min_x.min(x);
                    b.max_x = b.max_x.max(x);
                    b.min_y = b.min_y.min(y);
                    b.max_y = b.max_y.max(y);
                })
                .or_insert(ProvinceBounds {
                    min_x: x,
                    min_y: y,
                    max_x: x,
                    max_y: y,
                });
        }
    }
    bounds
}

/// One area's input to [`place_name`].
#[derive(Clone, Copy, Debug)]
pub struct NameInput<'a> {
    /// The resolved display string. [`place_name`] upper cases it itself because the engine does.
    pub text: &'a str,
    /// The province id image.
    pub provinces: ProvinceMap<'a>,
    /// The province ids making up this area. Ids absent from the image contribute nothing.
    pub area: &'a [u32],
    /// Province ids a fitted baseline may cross without ending its run: see [`best_run`]. The
    /// caller passes the map's sea and lake provinces.
    pub bridged: &'a HashSet<u32>,
    /// Per province bounds from [`scan_province_bounds`]. When `None` the bounds are recovered by
    /// scanning the whole image, which is correct but costs a full pass per name.
    pub bounds: Option<&'a HashMap<u32, ProvinceBounds>>,
}

/// One laid out glyph: two triangles worth of corners plus its atlas rectangle.
///
/// The engine emits six unindexed vertices per glyph, stride 20, `float3 POSITION + float2
/// TEXCOORD`. This keeps the four distinct corners instead; the pass expands them to
/// six in whatever winding its pipeline wants.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct GlyphQuad {
    /// The character this quad draws, after upper casing and space insertion.
    pub ch: char,
    /// Corners in map pixels, ordered top left, top right, bottom right, bottom left, where top
    /// and left are measured in the glyph's own frame along the bend curve.
    pub corners: [[f32; 2]; 4],
    /// Normalised atlas rectangle `[u0, v0, u1, v1]`, matching the corner order above.
    pub uv: [f32; 4],
}

/// A placed name in map pixel space.
#[derive(Clone, Debug, PartialEq)]
pub struct PlacedName {
    /// One quad per drawn glyph, in string order. Characters with no glyph box, or with an empty
    /// one such as the inserted spaces, advance the pen but produce no quad.
    pub glyphs: Vec<GlyphQuad>,
    /// Fitted straight baseline clipped to the area silhouette, as start/end. Glyphs follow the bend
    /// curve; the segment remains a placement diagnostic.
    pub baseline: ([f32; 2], [f32; 2]),
    /// The engine's font builder argument, `scale * FONT_SIZE_CONV`. This is not a
    /// map pixel length; [`PlacedName::scale`] is the multiplier the geometry above already used.
    pub font_size: f32,
    /// Axis aligned bounds of every glyph corner, `[min_x, min_y, max_x, max_y]` in map pixels.
    pub aabb: [f32; 4],
    /// Font pixels to map pixels, after `FINAL_SCALE`. Equals `font_size / FONT_SIZE_CONV`.
    pub scale: f32,
    /// How many spaces the height limited branch inserted between every pair of characters; zero
    /// when the fit was width limited.
    pub spaces_inserted: usize,
}

/// Per-label alpha of a country name, `Transp_OffsetX.x`.
///
/// The engine fades a label by `((D^2 - d^2) / (D^2 - 0.49 D^2)) * layer_alpha`, but the country
/// layer's `D` is `FLT_MAX`, so the camera distance `d` never changes the result.
pub fn label_alpha(layer_alpha: f32) -> f32 {
    let max_d2 = f32::MAX;
    let span = max_d2 - max_d2 * FADE_START * FADE_START;
    ((max_d2 / span) * layer_alpha).clamp(0.0, 1.0)
}

/// Places one area's name, reproducing steps 3 to 9 of algorithm C.
///
/// Returns `None` rather than panicking whenever the area cannot carry a name: a malformed image,
/// an empty area, an area whose provinces are absent from the image, a text with no character the
/// font knows, a regression with a zero denominator (a single pixel, or a perfectly axis aligned
/// one pixel line), or a baseline that clips to a single point.
///
/// The output is in map pixel space; see the module docs on who converts it.
pub fn place_name(input: &NameInput<'_>, metrics: &dyn GlyphMetrics) -> Option<PlacedName> {
    let map = input.provinces;
    if !map.is_well_formed() || input.area.is_empty() {
        return None;
    }
    let area: HashSet<u32> = input.area.iter().copied().collect();

    // Step 3: mask scan, with the date line wrap handled first.
    let scan = scan_area(&map, input.area, input.bounds)?;
    if scan.n <= 0 {
        return None;
    }

    // Step 4: the two ordinary least squares fits, dropped on a zero denominator.
    let (horizontal, vertical) = regressions(&scan)?;

    // Step 5: offset, rasterise, score, and clip each candidate to its best in area run.
    let (fit_h, fit_v) = fit_candidates(&map, &area, input.bridged, horizontal, vertical);

    // Step 6: the axis choice, biased towards horizontal.
    let mut fit = match (fit_h, fit_v) {
        (Some(h), Some(v)) => {
            if h.length_sq() * AXIS_BIAS <= v.length_sq() {
                v
            } else {
                h
            }
        }
        (Some(h), None) => h,
        (None, Some(v)) => v,
        (None, None) => return None,
    };
    if fit.end[0] < fit.start[0] {
        std::mem::swap(&mut fit.start, &mut fit.end);
    }
    let len = fit.length_sq().sqrt();
    if !len.is_finite() || len <= 0.0 {
        return None;
    }
    let right = [
        (fit.end[0] - fit.start[0]) / len,
        (fit.end[1] - fit.start[1]) / len,
    ];
    let up = [-right[1], right[0]];
    let mid = [
        (fit.start[0] + fit.end[0]) * 0.5,
        (fit.start[1] + fit.end[1]) * 0.5,
    ];

    // Step 7: scale, and the inserted spaces when the height limit is the tighter one.
    let text_height = metrics.line_height();
    if !text_height.is_finite() || text_height <= 0.0 {
        return None;
    }
    let chars: Vec<char> = input
        .text
        .to_uppercase()
        .chars()
        .filter(|ch| metrics.advance(*ch).is_some())
        .collect();
    if chars.is_empty() {
        return None;
    }
    let base_width = measure(&chars, metrics);
    if !base_width.is_finite() || base_width <= 0.0 {
        return None;
    }
    let width_scale = FILL * len / base_width;
    let height_scale = HALF * (scan.n as f32 / len) / text_height;

    let mut laid = chars.clone();
    let mut text_width = base_width;
    let mut spaces_inserted = 0;
    let scale = if height_scale < width_scale {
        if metrics.advance(' ').is_some() {
            let target = FILL * len / height_scale;
            for spaces in 1..=MAX_SPACES {
                let candidate = insert_spaces(&chars, spaces);
                let width = measure(&candidate, metrics);
                if target <= width {
                    break;
                }
                laid = candidate;
                text_width = width;
                spaces_inserted = spaces;
            }
        }
        height_scale
    } else {
        width_scale
    };
    let scale = scale * FINAL_SCALE;
    if !scale.is_finite() || !text_width.is_finite() || scale <= 0.0 {
        return None;
    }

    // Build a centred straight font run first. Pen origin uses integer division; vertical origin is
    // the line-box bottom.
    let half_width = (text_width as i32 / 2) as f32;
    let half_height = text_height * HALF * scale;
    let scaled = |pixels: f32| (pixels * 0.05) * (scale * FONT_SIZE_CONV);
    let origin = [mid[0] - half_height * up[0], mid[1] - half_height * up[1]];
    let mut pen = 0.0;
    let mut previous = None;
    let mut straight = Vec::new();
    let mut gaps = Vec::new();
    let mut space_run = false;
    for &ch in &laid {
        if let Some(p) = previous {
            pen += metrics.kerning(p, ch);
        }
        previous = Some(ch);
        let glyph = metrics.glyph(ch)?;
        let left_pixels = pen - half_width + glyph.xoffset;
        let left = scaled(left_pixels);
        pen += glyph.xadvance;
        if ch == ' ' || glyph.height <= 0.0 {
            space_run = true;
            continue;
        }
        let top = scaled(text_height - glyph.yoffset);
        let bottom = scaled(text_height - glyph.yoffset - glyph.height);
        let right_edge = scaled(left_pixels + glyph.width);
        let point = |x: f32, y: f32| [right[0] * x + up[0] * y, right[1] * x + up[1] * y];
        let quad = GlyphQuad {
            ch,
            corners: [
                point(left, top),
                point(right_edge, top),
                point(right_edge, bottom),
                point(left, bottom),
            ],
            uv: glyph.uv,
        };
        let gap = if space_run {
            straight.last().map_or(0.0, |prev: &GlyphQuad| {
                distance(quad.corners[3], prev.corners[2])
            })
        } else {
            0.0
        };
        gaps.push(gap);
        straight.push(quad);
        space_run = false;
    }
    let mut span = 0.0;
    for (index, quad) in straight.iter().enumerate() {
        span += gaps[index];
        let width = distance(quad.corners[3], quad.corners[2]);
        span += if index == 0 || index + 1 == straight.len() {
            width * HALF
        } else {
            width
        };
    }
    if straight.is_empty() || span <= 0.0 {
        return None;
    }

    // Probe beneath each quad centre. Endpoints mix medial and original centres; the middle control
    // averages medial samples in doubles.
    let centres: Vec<_> = straight
        .iter()
        .map(|q| {
            let centre = midpoint(q.corners[3], q.corners[1]);
            [centre[0] + origin[0], centre[1] + origin[1]]
        })
        .collect();
    let samples: Vec<_> = centres
        .iter()
        .map(|&centre| {
            medial_sample(
                &map,
                &area,
                input.bridged,
                centre,
                [fit.end[0] - fit.start[0], fit.end[1] - fit.start[1]],
                &scan,
            )
            .unwrap_or(centre)
        })
        .collect();
    let curve = Curve::through(&samples, &centres)?;
    let first_bottom = straight[0].corners[3];
    let first_half_height = distance(straight[0].corners[2], straight[0].corners[1]) * HALF;
    let mut progress = 0.0;
    let mut glyphs = Vec::with_capacity(straight.len());
    let mut aabb = [f32::MAX, f32::MAX, f32::MIN, f32::MIN];
    for (index, quad) in straight.iter().enumerate() {
        let half_width = distance(quad.corners[3], quad.corners[2]) * HALF;
        let height = distance(quad.corners[2], quad.corners[1]);
        progress += gaps[index];
        if index != 0 {
            progress += half_width;
        }
        let (point, tangent) = curve.at((progress / span).clamp(0.0, 1.0));
        progress += half_width;
        let delta = [
            quad.corners[3][0] - first_bottom[0],
            quad.corners[3][1] - first_bottom[1],
        ];
        let along = delta[0] * right[0] + delta[1] * right[1];
        let anchor = first_half_height
            - ((delta[0] - right[0] * along) * up[0] + (delta[1] - right[1] * along) * up[1]);
        let down = [tangent[1], -tangent[0]];
        let corner = |x: f32, y: f32| {
            [
                point[0] + tangent[0] * x + down[0] * y,
                point[1] + tangent[1] * x + down[1] * y,
            ]
        };
        let mut corners = [
            corner(-half_width, anchor - height),
            corner(half_width, anchor - height),
            corner(half_width, anchor),
            corner(-half_width, anchor),
        ];
        // The native algorithms above use the bottom-up province raster. The
        // public placement contract and heightmap consumer use bitmap pixels.
        for corner in &mut corners {
            corner[1] = map.height as f32 - 1.0 - corner[1];
            if !corner[0].is_finite() || !corner[1].is_finite() {
                return None;
            }
            aabb[0] = aabb[0].min(corner[0]);
            aabb[1] = aabb[1].min(corner[1]);
            aabb[2] = aabb[2].max(corner[0]);
            aabb[3] = aabb[3].max(corner[1]);
        }
        glyphs.push(GlyphQuad {
            ch: quad.ch,
            corners,
            uv: quad.uv,
        });
    }
    if glyphs.is_empty() {
        return None;
    }

    Some(PlacedName {
        glyphs,
        baseline: (
            [fit.start[0], map.height as f32 - 1.0 - fit.start[1]],
            [fit.end[0], map.height as f32 - 1.0 - fit.end[1]],
        ),
        font_size: scale * FONT_SIZE_CONV,
        aabb,
        scale,
        spaces_inserted,
    })
}

/// `spaces` U+0020 between every pair of characters.
fn insert_spaces(chars: &[char], spaces: usize) -> Vec<char> {
    let mut out = Vec::with_capacity(chars.len() + spaces * chars.len().saturating_sub(1));
    for (index, &ch) in chars.iter().enumerate() {
        if index > 0 {
            out.extend(std::iter::repeat_n(' ', spaces));
        }
        out.push(ch);
    }
    out
}

/// Width of a character run in font pixels, advances plus kerning.
fn measure(chars: &[char], metrics: &dyn GlyphMetrics) -> f32 {
    let mut width = 0.0;
    let mut previous: Option<char> = None;
    for &ch in chars {
        let Some(advance) = metrics.advance(ch) else {
            continue;
        };
        if let Some(p) = previous {
            width += metrics.kerning(p, ch);
        }
        width += advance;
        previous = Some(ch);
    }
    width
}

/// The accumulated mask statistics of one area.
#[derive(Clone, Copy, Debug)]
struct MaskScan {
    n: i64,
    sx: i64,
    sy: i64,
    sxy: i64,
    sxx: i64,
    syy: i64,
    min_x: f64,
    max_x: f64,
    min_y: f64,
    max_y: f64,
}

/// Walks the area's province bounding boxes and accumulates the sums the fits need.
///
/// Accumulate six integer sums in 64-bit precision, form covariance numerators, divide in f64 and
/// narrow slopes/means to f32.
fn scan_area(
    map: &ProvinceMap<'_>,
    area: &[u32],
    bounds: Option<&HashMap<u32, ProvinceBounds>>,
) -> Option<MaskScan> {
    let computed = match bounds {
        Some(_) => None,
        None => Some(scan_province_bounds(map)),
    };
    let bounds = bounds.or(computed.as_ref())?;

    let boxes: Vec<(u32, ProvinceBounds)> = area
        .iter()
        .filter_map(|id| bounds.get(id).map(|b| (*id, *b)))
        .collect();
    if boxes.is_empty() {
        return None;
    }

    // The date line pre pass: the engine compares the provinces' box origins and shifts the
    // western half east by one map width before accumulating.
    let min_origin = boxes.iter().map(|(_, b)| b.min_x).min()?;
    let max_origin = boxes.iter().map(|(_, b)| b.min_x).max()?;
    let half = map.width / 2;
    let wrap = max_origin - min_origin > half;

    let mut n = 0i64;
    let (mut sx, mut sy, mut sxy, mut sxx, mut syy) = (0i64, 0i64, 0i64, 0i64, 0i64);
    let (mut min_x, mut max_x) = (f64::MAX, f64::MIN);
    let (mut min_y, mut max_y) = (f64::MAX, f64::MIN);
    for (id, b) in &boxes {
        for y in b.min_y..=b.max_y {
            let row = y as usize * map.width as usize;
            for x in b.min_x..=b.max_x {
                if map.ids[row + x as usize] != *id {
                    continue;
                }
                let xs = if wrap && x < half {
                    (x + map.width) as f64
                } else {
                    x as f64
                };
                let ys = (map.height - 1 - y) as f64;
                n += 1;
                sx += xs as i64;
                sy += ys as i64;
                sxy += xs as i64 * ys as i64;
                sxx += xs as i64 * xs as i64;
                syy += ys as i64 * ys as i64;
                min_x = min_x.min(xs);
                max_x = max_x.max(xs);
                min_y = min_y.min(ys);
                max_y = max_y.max(ys);
            }
        }
    }
    if n <= 0 {
        return None;
    }
    Some(MaskScan {
        n,
        sx,
        sy,
        sxy,
        sxx,
        syy,
        min_x,
        max_x,
        min_y,
        max_y,
    })
}

/// Two fitted candidate segments, or None for a zero denominator.
#[allow(clippy::type_complexity)]
fn regressions(scan: &MaskScan) -> Option<(([f32; 2], [f32; 2]), ([f32; 2], [f32; 2]))> {
    let n = scan.n;
    let numerator = n
        .wrapping_mul(scan.sxy)
        .wrapping_sub(scan.sx.wrapping_mul(scan.sy));
    let denominator_h = n
        .wrapping_mul(scan.sxx)
        .wrapping_sub(scan.sx.wrapping_mul(scan.sx));
    let denominator_v = n
        .wrapping_mul(scan.syy)
        .wrapping_sub(scan.sy.wrapping_mul(scan.sy));
    if denominator_h == 0 || denominator_v == 0 {
        return None;
    }
    // The sums are 64-bit integers; the native conversion rounds
    // each mean and slope to f32 before forming intercepts and endpoints.
    let mean_x = (scan.sx as f64 / n as f64) as f32;
    let mean_y = (scan.sy as f64 / n as f64) as f32;
    let slope_h = (numerator as f64 / denominator_h as f64) as f32;
    let intercept_h = mean_y - slope_h * mean_x;
    let horizontal = (
        [scan.min_x as f32, slope_h * scan.min_x as f32 + intercept_h],
        [scan.max_x as f32, slope_h * scan.max_x as f32 + intercept_h],
    );
    let slope_v = (numerator as f64 / denominator_v as f64) as f32;
    let intercept_v = mean_x - slope_v * mean_y;
    let vertical = (
        [slope_v * scan.min_y as f32 + intercept_v, scan.min_y as f32],
        [slope_v * scan.max_y as f32 + intercept_v, scan.max_y as f32],
    );
    Some((horizontal, vertical))
}

/// A scored, clipped candidate baseline.
#[derive(Clone, Copy, Debug)]
struct Fitted {
    score: usize,
    start: [f32; 2],
    end: [f32; 2],
}

impl Fitted {
    fn length_sq(&self) -> f32 {
        let dx = self.end[0] - self.start[0];
        let dy = self.end[1] - self.start[1];
        dx * dx + dy * dy
    }
}

/// Score three sideways offsets of both candidates, rasterized with Bresenham and clipped to their
/// best in-area run.
///
/// Offset both endpoints perpendicular to the segment by three map pixels each way, retaining the
/// fitted candidate and two translated copies.
///
/// For nonzero offsets, reject the whole iteration if any endpoint of either candidate leaves the
/// image. Zero offset always runs. Keep only a run longer than one point that improves the incumbent.
fn fit_candidates(
    map: &ProvinceMap<'_>,
    area: &HashSet<u32>,
    bridged: &HashSet<u32>,
    horizontal: ([f32; 2], [f32; 2]),
    vertical: ([f32; 2], [f32; 2]),
) -> (Option<Fitted>, Option<Fitted>) {
    let mut best_h: Option<Fitted> = None;
    let mut best_v: Option<Fitted> = None;
    for k in [-1.0f32, 0.0, 1.0] {
        let direction_h = best_h.map_or(horizontal, |f| (f.start, f.end));
        let direction_v = best_v.map_or(vertical, |f| (f.start, f.end));
        let (Some(h), Some(v)) = (
            offset_segment(horizontal, direction_h, k),
            offset_segment(vertical, direction_v, k),
        ) else {
            continue;
        };
        if k != 0.0 && !(in_image(map, h) && in_image(map, v)) {
            continue;
        }
        consider(map, area, bridged, h, &mut best_h);
        consider(map, area, bridged, v, &mut best_v);
    }
    (best_h, best_v)
}

/// One segment translated sideways by `k * NUDGE` map pixels, or `None` when it is degenerate.
fn offset_segment(
    segment: ([f32; 2], [f32; 2]),
    direction: ([f32; 2], [f32; 2]),
    k: f32,
) -> Option<([f32; 2], [f32; 2])> {
    let (start, end) = segment;
    if k == 0.0 {
        return Some(segment);
    }
    let dx = direction.1[0] - direction.0[0];
    let dy = direction.1[1] - direction.0[1];
    let length = (dx * dx + dy * dy).sqrt();
    if !length.is_finite() || length <= 0.0 {
        return None;
    }
    let offset = [-((k * (dy / length)) * NUDGE), (k * (dx / length)) * NUDGE];
    Some((
        [start[0] + offset[0], start[1] + offset[1]],
        [end[0] + offset[0], end[1] + offset[1]],
    ))
}

/// True when both endpoints of a segment are inside the province image.
fn in_image(map: &ProvinceMap<'_>, segment: ([f32; 2], [f32; 2])) -> bool {
    let (width, height) = (map.width as f32, map.height as f32);
    [segment.0, segment.1]
        .iter()
        .all(|p| p[0] >= 0.0 && p[0] < width && p[1] >= 0.0 && p[1] < height)
}

/// Scores one offset segment and keeps it when it beats `best`.
fn consider(
    map: &ProvinceMap<'_>,
    area: &HashSet<u32>,
    bridged: &HashSet<u32>,
    segment: ([f32; 2], [f32; 2]),
    best: &mut Option<Fitted>,
) {
    let Some((score, start, end)) = best_run(map, area, bridged, segment.0, segment.1, None) else {
        return;
    };
    if score > 1 && best.is_none_or(|previous| score > previous.score) {
        *best = Some(Fitted { score, start, end });
    }
}

/// One contiguous in-area run of a rasterized line.
///
/// Trim to matching land when present, then retain the longest contiguous run
/// through area members and permitted water. Foreign land ends a run.
/// The returned point count scores the candidate; the endpoint distance fits the text.
///
/// Without near, longest run wins. With near, RUN_NEAR_BIAS weights a closer run; truncate compared
/// lengths to integers before choosing. Straight fits omit near, while bends supply the glyph centre.
///
/// Endpoint trimming requires an area member. The run scan also accepts lakes and, when bridging is
/// enabled, sea provinces. The bridged set supplies water provinces.
///
/// Controller and state names bridge sea while requiring land at trims. With no land match, trim
/// limits remain unchanged; a wholly water candidate can win.
fn best_run(
    map: &ProvinceMap<'_>,
    area: &HashSet<u32>,
    bridged: &HashSet<u32>,
    a: [f32; 2],
    b: [f32; 2],
    near: Option<[f32; 2]>,
) -> Option<(usize, [f32; 2], [f32; 2])> {
    let a = to_pixel(a)?;
    let b = to_pixel(b)?;
    let near = match near {
        Some(point) => Some(to_pixel(point)?),
        None => None,
    };
    let all = bresenham(a, b)?;
    let inside = |point: &(i32, i32)| {
        map.id_at(point.0, map.height as i32 - 1 - point.1)
            .is_some_and(|id| area.contains(&id) && !bridged.contains(&id))
    };
    // The trims: the run may only start and end on the area itself.
    // Native trims keep their original limits when no land point matches.
    // A wholly sea/lake line can consequently win when bridging is enabled.
    let head = all.iter().position(inside).unwrap_or(0);
    let tail = all.iter().rposition(inside).unwrap_or(all.len() - 1);
    let points = &all[head..=tail];

    let mut best_start = 0usize;
    let mut best_len = 0usize;
    let mut best_distance = i64::MAX;
    let mut run_start = 0usize;
    let mut run_len = 0usize;
    // One index past the end closes the last run through the same arm.
    for index in 0..=points.len() {
        let carries = points.get(index).is_some_and(|point| {
            map.id_at(point.0, map.height as i32 - 1 - point.1)
                .is_some_and(|id| area.contains(&id) || bridged.contains(&id))
        });
        if carries {
            if run_len == 0 {
                run_start = index;
            }
            run_len += 1;
            continue;
        }
        if run_len > 0 {
            let wins = match near {
                None => run_len > best_len,
                Some(point) => {
                    let distance = points[run_start..run_start + run_len]
                        .iter()
                        .map(|p| {
                            let dx = i64::from(p.0 - point.0);
                            let dy = i64::from(p.1 - point.1);
                            dx * dx + dy * dy
                        })
                        .min()
                        .unwrap_or(i64::MAX);
                    let mut candidate = run_len as f32;
                    if distance < best_distance {
                        candidate *= RUN_NEAR_BIAS;
                    }
                    let mut incumbent = best_len as f32;
                    if best_distance < distance {
                        incumbent *= RUN_NEAR_BIAS;
                    }
                    let wins = (incumbent as i32) < (candidate as i32);
                    if wins {
                        best_distance = distance;
                    }
                    wins
                }
            };
            if wins {
                best_start = run_start;
                best_len = run_len;
            }
        }
        run_len = 0;
    }
    if best_len == 0 {
        return None;
    }
    let first = points[best_start];
    let last = points[best_start + best_len - 1];
    Some((
        best_len,
        [first.0 as f32, first.1 as f32],
        [last.0 as f32, last.1 as f32],
    ))
}

/// Truncates a map pixel position to integers, rejecting anything a line rasteriser cannot take.
///
/// Truncate all four line coordinates toward zero before Bresenham clipping. Rounding can shift
/// an endpoint and the entire clipped run by one pixel.
fn to_pixel(p: [f32; 2]) -> Option<(i32, i32)> {
    if !p[0].is_finite() || !p[1].is_finite() {
        return None;
    }
    if p[0].abs() > MAX_COORD || p[1].abs() > MAX_COORD {
        return None;
    }
    Some((p[0] as i32, p[1] as i32))
}

/// Integer Bresenham traversal for every octant.
fn bresenham(a: (i32, i32), b: (i32, i32)) -> Option<Vec<(i32, i32)>> {
    // Walk the major coordinate upward. At a half-pixel tie, advance the minor coordinate.
    let steep = (b.0 - a.0).abs() < (b.1 - a.1).abs();
    let (mut a, mut b) = if steep {
        ((a.1, a.0), (b.1, b.0))
    } else {
        (a, b)
    };
    if a.0 > b.0 {
        std::mem::swap(&mut a, &mut b);
    }
    let span = b.0 - a.0;
    if span as usize >= MAX_LINE_POINTS {
        return None;
    }
    let minor_span = (b.1 - a.1).abs();
    let step = if a.1 < b.1 { 1 } else { -1 };
    let mut minor = a.1;
    let mut error = 0;
    let mut out = Vec::with_capacity(span as usize + 1);
    for major in a.0..=b.0 {
        out.push(if steep {
            (minor, major)
        } else {
            (major, minor)
        });
        error += minor_span;
        if 2 * error >= span {
            minor += step;
            error -= span;
        }
    }
    Some(out)
}

/// Probe the perpendicular line clipped to the region's exact integer box.
/// Clipping before Bresenham matters: changing either endpoint changes its phase.
fn medial_sample(
    map: &ProvinceMap<'_>,
    area: &HashSet<u32>,
    bridged: &HashSet<u32>,
    centre: [f32; 2],
    right: [f32; 2],
    scan: &MaskScan,
) -> Option<[f32; 2]> {
    let ymin = scan.min_y as f32;
    let ymax = scan.max_y as f32;
    let (a, b) = if right[1].abs() > TANGENT_STEP {
        let slope = -right[0] / right[1];
        let intercept = centre[1] - centre[0] * slope;
        let endpoint = |x: f32| {
            let y = x * slope + intercept;
            let clipped = y.clamp(ymin, ymax);
            [
                if clipped != y {
                    (clipped - intercept) / slope
                } else {
                    x
                },
                clipped,
            ]
        };
        (endpoint(scan.min_x as f32), endpoint(scan.max_x as f32))
    } else {
        ([centre[0], ymin], [centre[0], ymax])
    };
    let (_, first, last) = best_run(map, area, bridged, a, b, Some(centre))?;
    Some(midpoint(first, last))
}

fn midpoint(a: [f32; 2], b: [f32; 2]) -> [f32; 2] {
    [(a[0] + b[0]) * HALF, (a[1] + b[1]) * HALF]
}

fn distance(a: [f32; 2], b: [f32; 2]) -> f32 {
    ((a[0] - b[0]).powi(2) + (a[1] - b[1]).powi(2)).sqrt()
}

/// The native two uniform Catmull-Rom segments, with reflected outer controls.
/// Glyph progress is a normalized sum of widths and space-run gaps, not arc length.
#[derive(Clone, Debug)]
struct Curve {
    controls: [[f32; 2]; 5],
}

impl Curve {
    fn through(samples: &[[f32; 2]], centres: &[[f32; 2]]) -> Option<Self> {
        let first = midpoint(*samples.first()?, *centres.first()?);
        let last = midpoint(*samples.last()?, *centres.last()?);
        let n = samples.len() as f64;
        let mean = [
            samples.iter().map(|p| p[0] as f64).sum::<f64>() / n,
            samples.iter().map(|p| p[1] as f64).sum::<f64>() / n,
        ];
        // Normalize these two reflected handles in double precision.
        let reflect = |end: [f32; 2], other: [f32; 2]| {
            let u = [end[0] as f64 - mean[0], end[1] as f64 - mean[1]];
            let v = [mean[0] - other[0] as f64, mean[1] - other[1] as f64];
            let ul = (u[0] * u[0] + u[1] * u[1]).sqrt();
            let vl = (v[0] * v[0] + v[1] * v[1]).sqrt();
            if ul == 0.0 || vl == 0.0 {
                return end;
            }
            let v = [v[0] / vl, v[1] / vl];
            let dot = u[0] * v[0] + u[1] * v[1];
            [
                (end[0] as f64 + 2.0 * dot * u[0] / ul - ul * v[0]) as f32,
                (end[1] as f64 + 2.0 * dot * u[1] / ul - ul * v[1]) as f32,
            ]
        };
        Some(Self {
            controls: [
                reflect(first, last),
                first,
                [mean[0] as f32, mean[1] as f32],
                last,
                reflect(last, first),
            ],
        })
    }

    fn at(&self, progress: f32) -> ([f32; 2], [f32; 2]) {
        let (offset, t) = if progress > HALF {
            (1, (progress - HALF) * 2.0)
        } else {
            (0, progress * 2.0)
        };
        let p = &self.controls[offset..offset + 4];
        let evaluate = |t: f32| {
            let t2 = t * t;
            let t3 = t2 * t;
            let b = (4.0 * t2 - 3.0 * t3) + t;
            let a = (3.0 * t3 - 5.0 * t2) + 2.0;
            let outer = ((t2 + t2) - t3) - t;
            std::array::from_fn(|i| {
                ((t3 - t2) * p[3][i] + b * p[2][i] + a * p[1][i] + outer * p[0][i]) * HALF
            })
        };
        let point: [f32; 2] = evaluate(t);
        let (a, b) = if t <= HALF {
            (point, evaluate(t + TANGENT_STEP))
        } else {
            (evaluate(t - TANGENT_STEP), point)
        };
        let length = distance(a, b);
        let tangent = if length > 0.0 {
            [(b[0] - a[0]) / length, (b[1] - a[1]) / length]
        } else {
            [1.0, 0.0]
        };
        (point, tangent)
    }
}

#[cfg(test)]
mod tests;
