
use std::collections::{BTreeMap, BTreeSet};

use crate::{map_label_capture::MapnameVertex, Result};

pub const LABEL_SAMPLE_STEP: usize = 1;

pub const LABEL_OFFSET_PX: f64 = 3.0;

pub const LABEL_AB_HYSTERESIS: f64 = 1.2;


pub const LABEL_HEIGHT_BREADTH_K: f32 = 0.22;

pub const ENGINE_TITLE_FIT_COEF: f32 = 0.72;

pub const ENGINE_TITLE_CAP_COEF: f32 = 0.40;

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct GlyphScale {
    pub scale: f32,
    pub cap_bound: bool,
}

const FLAG_PASSABLE_LAND: u8 = 1;
const FLAG_WATER: u8 = 2;

#[derive(Clone, Debug, PartialEq)]
pub struct CountryLabelPlacement {
    pub tag: String,
    pub is_home: bool,
    pub component_id: u32,
    pub anchor_x: f32,
    pub anchor_y: f32,
    pub angle: f32,
    pub half_length: f32,
    pub bow: f32,
    pub world_height: f32,
    pub pixel_count: u64,
    pub major_extent: f32,
    pub minor_extent: f32,
    pub minor_extent_incl: f32,
    pub breadth_span: f32,
    pub breadth_p2_98: f32,
    pub curve: Option<EngineCurve>,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct EngineCurve {
    pub pts: [(f64, f64); 5],
}

impl EngineCurve {
    fn eval_quad(
        p0: (f64, f64),
        p1: (f64, f64),
        p2: (f64, f64),
        p3: (f64, f64),
        u: f32,
    ) -> (f32, f32) {
        let u2 = u * u;
        let u3 = u2 * u;
        let w0 = 2.0 * u2 - u3 - u;
        let w1 = 3.0 * u3 - 5.0 * u2 + 2.0;
        let w2 = 4.0 * u2 - 3.0 * u3 + u;
        let w3 = u3 - u2;
        let x = 0.5 * (w0 * p0.0 as f32 + w1 * p1.0 as f32 + w2 * p2.0 as f32 + w3 * p3.0 as f32);
        let y = 0.5 * (w0 * p0.1 as f32 + w1 * p1.1 as f32 + w2 * p2.1 as f32 + w3 * p3.1 as f32);
        (x, y)
    }

    pub fn point_at(&self, t: f32) -> (f32, f32) {
        let t = t.clamp(0.0, 1.0);
        let [c0, first, mean, last, c4] = self.pts;
        if t <= 0.5 {
            Self::eval_quad(c0, first, mean, last, 2.0 * t)
        } else {
            Self::eval_quad(first, mean, last, c4, 2.0 * (t - 0.5))
        }
    }

    pub fn tangent_at(&self, t: f32) -> (f32, f32) {
        let t = t.clamp(0.0, 1.0);
        let (seg_u, quad) = if t <= 0.5 {
            (2.0 * t, 0usize)
        } else {
            (2.0 * (t - 0.5), 1usize)
        };
        let [c0, first, mean, last, c4] = self.pts;
        let eval = |u: f32| -> (f32, f32) {
            if quad == 0 {
                Self::eval_quad(c0, first, mean, last, u)
            } else {
                Self::eval_quad(first, mean, last, c4, u)
            }
        };
        const EPS: f32 = 0.01;
        let (dx, dy) = if seg_u <= 0.5 {
            let a = eval(seg_u);
            let b = eval(seg_u + EPS);
            (b.0 - a.0, b.1 - a.1)
        } else {
            let a = eval(seg_u - EPS);
            let b = eval(seg_u);
            (b.0 - a.0, b.1 - a.1)
        };
        let l = (dx * dx + dy * dy).sqrt();
        if l > 0.0 {
            (dx / l, dy / l)
        } else {
            (1.0, 0.0)
        }
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct LabelTextMetrics {
    pub advances: Vec<(f32, bool)>,
    pub line_height_px: f32,
    pub space_advance_px: f32,
    pub straight_layout_stations: Vec<MapLabelStraightLayoutStation>,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct MapLabelStraightLayoutStation {
    pub slot_index: usize,
    pub x: f32,
    pub y: f32,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct MapLabelGlyphLayoutQuad {
    pub x0: f32,
    pub y0: f32,
    pub x1: f32,
    pub y1: f32,
    pub u0: f32,
    pub v0: f32,
    pub u1: f32,
    pub v1: f32,
    pub measured_t: f32,
}

fn measured_station_parameters(stations: &[(f32, f32)]) -> Option<Vec<f32>> {
    if stations.is_empty() {
        return Some(Vec::new());
    }
    if stations
        .iter()
        .any(|(x, y)| !x.is_finite() || !y.is_finite())
    {
        return None;
    }
    let mut cumulative = Vec::with_capacity(stations.len());
    cumulative.push(0.0f32);
    for pair in stations.windows(2) {
        let dx = pair[1].0 - pair[0].0;
        let dy = pair[1].1 - pair[0].1;
        let distance = (dx * dx + dy * dy).sqrt();
        if !distance.is_finite() {
            return None;
        }
        cumulative.push(cumulative.last().copied().unwrap_or(0.0) + distance);
    }
    let total = cumulative.last().copied().unwrap_or(0.0);
    if stations.len() > 1 && !(total > 0.0) {
        return None;
    }
    if total > 0.0 {
        for value in &mut cumulative {
            *value = (*value / total).clamp(0.0, 1.0);
        }
    }
    Some(cumulative)
}

pub fn mapname_measured_glyph_parameters(glyphs: &[MapLabelGlyphLayoutQuad]) -> Option<Vec<f32>> {
    let stations: Vec<(f32, f32)> = glyphs
        .iter()
        .map(|glyph| ((glyph.x0 + glyph.x1) * 0.5, glyph.y0))
        .collect();
    measured_station_parameters(&stations)
}

pub fn mapname_tracking_px(placement: &CountryLabelPlacement, metrics: &LabelTextMetrics) -> f32 {
    let text_w: f32 = metrics.advances.iter().map(|(advance, _)| *advance).sum();
    let len = (2.0 * placement.half_length).max(f32::MIN_POSITIVE);
    let text_h = metrics.line_height_px.max(f32::MIN_POSITIVE);
    if !(text_w > 0.0) {
        return 0.0;
    }
    let s_w = 0.9 * len / text_w;
    let s_t = 0.5 * (placement.pixel_count as f32 / len) / text_h;
    if s_t >= s_w {
        return 0.0;
    }
    let gaps = metrics.advances.len().saturating_sub(1) as f32;
    if gaps <= 0.0 || metrics.space_advance_px <= 0.0 {
        return 0.0;
    }
    let w_target = 0.9 * len / s_t.max(f32::MIN_POSITIVE);
    let mut k_spaces = 0u32;
    for k in 1..=19u32 {
        k_spaces = k;
        if text_w + k as f32 * metrics.space_advance_px * gaps >= w_target {
            break;
        }
    }
    k_spaces as f32 * metrics.space_advance_px
}

pub fn mapname_tracked_advance_px(metrics: &LabelTextMetrics, tracking_px: f32) -> f32 {
    let text_w: f32 = metrics.advances.iter().map(|(advance, _)| *advance).sum();
    let gaps = metrics.advances.len().saturating_sub(1) as f32;
    text_w + tracking_px.max(0.0) * gaps
}

pub fn build_mapname_vertices(
    placement: &CountryLabelPlacement,
    glyphs: &[MapLabelGlyphLayoutQuad],
    natural_text_width_px: f32,
    tracked_advance_px: f32,
    line_height_px: f32,
    map_height: f32,
    world_y: f32,
) -> Vec<MapnameVertex> {
    if glyphs.is_empty() {
        return Vec::new();
    }
    let glyph_scale = placement.engine_glyph_scale(natural_text_width_px, line_height_px);
    let scale = glyph_scale.scale;
    if !tracked_advance_px.is_finite() || tracked_advance_px <= 0.0 {
        return Vec::new();
    }
    let mut vertices = Vec::with_capacity(glyphs.len() * 6);

    let push = |out: &mut Vec<MapnameVertex>, map_x: f32, map_y: f32, u: f32, v: f32| {
        out.push(MapnameVertex {
            x: map_x,
            y: world_y,
            z: map_height - map_y,
            u,
            v,
        });
    };

    for glyph in glyphs {
        let x0 = glyph.x0;
        let x1 = glyph.x1;
        let y0 = glyph.y0;
        let y1 = glyph.y1;
        let station_x = (x0 + x1) * 0.5;
        let station_y = y0;
        if !glyph.measured_t.is_finite() {
            return Vec::new();
        }
        let t = glyph.measured_t.clamp(0.0, 1.0) * 2.0 - 1.0;
        let (bx, by) = placement.baseline_point(t);
        let theta = placement.tangent_angle(t);
        let (sin, cos) = theta.sin_cos();
        let rigid_corner = |x: f32, y: f32| -> (f32, f32) {
            let along = (x - station_x) * scale;
            let across = (y - station_y) * scale;
            (
                bx + along * cos - across * sin,
                by + along * sin + across * cos,
            )
        };
        let (tl_x, tl_y) = rigid_corner(x0, y0);
        let (tr_x, tr_y) = rigid_corner(x1, y0);
        let (bl_x, bl_y) = rigid_corner(x0, y1);
        let (br_x, br_y) = rigid_corner(x1, y1);
        push(&mut vertices, tl_x, tl_y, glyph.u0, glyph.v0);
        push(&mut vertices, tr_x, tr_y, glyph.u1, glyph.v0);
        push(&mut vertices, bl_x, bl_y, glyph.u0, glyph.v1);
        push(&mut vertices, tr_x, tr_y, glyph.u1, glyph.v0);
        push(&mut vertices, br_x, br_y, glyph.u1, glyph.v1);
        push(&mut vertices, bl_x, bl_y, glyph.u0, glyph.v1);
    }
    vertices
}

pub fn build_mapname_vertices_with_height<F>(
    placement: &CountryLabelPlacement,
    glyphs: &[MapLabelGlyphLayoutQuad],
    natural_text_width_px: f32,
    tracked_advance_px: f32,
    line_height_px: f32,
    map_height: f32,
    mut sample_height: F,
) -> Result<Vec<MapnameVertex>>
where
    F: FnMut(f32, f32) -> Result<f32>,
{
    const MAPNAME_MIN_WORLD_HEIGHT: f32 = 9.5;
    if glyphs.is_empty() {
        return Ok(Vec::new());
    }
    let glyph_scale = placement.engine_glyph_scale(natural_text_width_px, line_height_px);
    let scale = glyph_scale.scale;
    if !tracked_advance_px.is_finite() || tracked_advance_px <= 0.0 {
        return Ok(Vec::new());
    }
    let mut vertices = Vec::with_capacity(glyphs.len() * 6);
    let mut corner = |map_x: f32, map_y: f32, u: f32, v: f32| -> Result<MapnameVertex> {
        let z = map_height - map_y;
        let sampled = sample_height(map_x, z)?;
        if !sampled.is_finite() {
            return Err("mapname terrain-height sampler returned a non-finite value".into());
        }
        Ok(MapnameVertex {
            x: map_x,
            y: sampled.max(MAPNAME_MIN_WORLD_HEIGHT),
            z,
            u,
            v,
        })
    };

    for glyph in glyphs {
        if !glyph.measured_t.is_finite() {
            return Err("mapname glyph has a non-finite measured spline station".into());
        }
        let station_x = (glyph.x0 + glyph.x1) * 0.5;
        let station_y = glyph.y0;
        let t = glyph.measured_t.clamp(0.0, 1.0) * 2.0 - 1.0;
        let (bx, by) = placement.baseline_point(t);
        let theta = placement.tangent_angle(t);
        let (sin, cos) = theta.sin_cos();
        let rigid_corner = |x: f32, y: f32| -> (f32, f32) {
            let along = (x - station_x) * scale;
            let across = (y - station_y) * scale;
            (
                bx + along * cos - across * sin,
                by + along * sin + across * cos,
            )
        };
        let (tl_x, tl_y) = rigid_corner(glyph.x0, glyph.y0);
        let (tr_x, tr_y) = rigid_corner(glyph.x1, glyph.y0);
        let (bl_x, bl_y) = rigid_corner(glyph.x0, glyph.y1);
        let (br_x, br_y) = rigid_corner(glyph.x1, glyph.y1);
        let tl = corner(tl_x, tl_y, glyph.u0, glyph.v0)?;
        let tr = corner(tr_x, tr_y, glyph.u1, glyph.v0)?;
        let bl = corner(bl_x, bl_y, glyph.u0, glyph.v1)?;
        let br = corner(br_x, br_y, glyph.u1, glyph.v1)?;
        vertices.extend_from_slice(&[tl, tr, bl, tr, br, bl]);
    }
    Ok(vertices)
}

impl CountryLabelPlacement {
    pub fn baseline_point(&self, t: f32) -> (f32, f32) {
        let t = t.clamp(-1.0, 1.0);
        if let Some(curve) = &self.curve {
            return curve.point_at((t + 1.0) * 0.5);
        }
        let (sin, cos) = self.angle.sin_cos();
        let along = t * self.half_length;
        let across = self.bow * (1.0 - t * t);
        (
            self.anchor_x + along * cos - across * sin,
            self.anchor_y + along * sin + across * cos,
        )
    }

    pub fn arc_length(&self) -> f32 {
        const SEGMENTS: usize = 64;
        let mut length = 0.0f32;
        let (mut px, mut py) = self.baseline_point(-1.0);
        for i in 1..=SEGMENTS {
            let t = (i as f32 / SEGMENTS as f32) * 2.0 - 1.0;
            let (x, y) = self.baseline_point(t);
            length += ((x - px).powi(2) + (y - py).powi(2)).sqrt();
            px = x;
            py = y;
        }
        length
    }

    pub fn engine_glyph_scale(&self, text_w: f32, line_height: f32) -> GlyphScale {
        let len = (2.0 * self.half_length).max(f32::MIN_POSITIVE);
        let text_w = text_w.max(f32::MIN_POSITIVE);
        let line_height = line_height.max(f32::MIN_POSITIVE);
        let pixels = self.pixel_count as f32;
        let s_fit = ENGINE_TITLE_FIT_COEF * len / text_w;
        let s_cap = ENGINE_TITLE_CAP_COEF * (pixels / len) / line_height;
        if s_cap < s_fit {
            GlyphScale {
                scale: s_cap.max(f32::MIN_POSITIVE),
                cap_bound: true,
            }
        } else {
            GlyphScale {
                scale: s_fit.max(f32::MIN_POSITIVE),
                cap_bound: false,
            }
        }
    }

    pub fn t_at_arc_fraction(&self, f: f32) -> f32 {
        const SEGMENTS: usize = 64;
        let f = f.clamp(0.0, 1.0);
        let mut cum = [0.0f32; SEGMENTS + 1];
        let (mut px, mut py) = self.baseline_point(-1.0);
        for i in 1..=SEGMENTS {
            let t = (i as f32 / SEGMENTS as f32) * 2.0 - 1.0;
            let (x, y) = self.baseline_point(t);
            cum[i] = cum[i - 1] + ((x - px).powi(2) + (y - py).powi(2)).sqrt();
            px = x;
            py = y;
        }
        let total = cum[SEGMENTS];
        if total <= 0.0 {
            return f * 2.0 - 1.0;
        }
        let target = f * total;
        for i in 1..=SEGMENTS {
            if cum[i] >= target {
                let seg = (cum[i] - cum[i - 1]).max(1e-6);
                let local = (target - cum[i - 1]) / seg;
                let t0 = ((i - 1) as f32 / SEGMENTS as f32) * 2.0 - 1.0;
                let t1 = (i as f32 / SEGMENTS as f32) * 2.0 - 1.0;
                return t0 + (t1 - t0) * local;
            }
        }
        1.0
    }

    pub fn tangent_angle(&self, t: f32) -> f32 {
        let t = t.clamp(-1.0, 1.0);
        if let Some(curve) = &self.curve {
            let (dx, dy) = curve.tangent_at((t + 1.0) * 0.5);
            return dy.atan2(dx);
        }
        let (sin, cos) = self.angle.sin_cos();
        let d_along = self.half_length;
        let d_across = -2.0 * self.bow * t;
        let dx = d_along * cos - d_across * sin;
        let dy = d_along * sin + d_across * cos;
        dy.atan2(dx)
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ProvinceKind {
    Land,
    Water,
}

impl ProvinceKind {
    pub fn from_type_str(s: &str) -> ProvinceKind {
        match s {
            "sea" | "lake" => ProvinceKind::Water,
            _ => ProvinceKind::Land,
        }
    }
}

/// current ownership and the per-province type.
pub struct LabelLawInputs<'a> {
    pub province_ids: &'a [u32],
    pub width: usize,
    pub height: usize,
    pub province_to_state: &'a BTreeMap<u32, u32>,
    pub owners: &'a BTreeMap<u32, String>,
    pub impassable_states: &'a BTreeSet<u32>,
    pub province_importance: &'a BTreeMap<u32, u64>,
    pub province_type: &'a BTreeMap<u32, ProvinceKind>,
}

pub struct ControllerLabelLawInputs<'a> {
    pub province_ids: &'a [u32],
    pub width: usize,
    pub height: usize,
    pub province_to_state: &'a BTreeMap<u32, u32>,
    pub controllers: &'a BTreeMap<u32, String>,
    pub impassable_states: &'a BTreeSet<u32>,
    pub province_importance: &'a BTreeMap<u32, u64>,
    pub province_type: &'a BTreeMap<u32, ProvinceKind>,
    pub province_adjacencies: &'a BTreeSet<(u32, u32)>,
}

pub struct ExplicitAreaLabelLawInputs<'a> {
    pub province_ids: &'a [u32],
    pub width: usize,
    pub height: usize,
    pub area_by_province: &'a BTreeMap<u32, String>,
    pub province_to_state: &'a BTreeMap<u32, u32>,
    pub impassable_states: &'a BTreeSet<u32>,
    pub province_importance: &'a BTreeMap<u32, u64>,
    pub province_type: &'a BTreeMap<u32, ProvinceKind>,
}

#[derive(Default, Clone)]
struct MomentAccum {
    count: u64,
    sum_x: f64,
    sum_y: f64,
    sum_xx: f64,
    sum_yy: f64,
    sum_xy: f64,
    min_x: f64,
    min_y: f64,
    max_x: f64,
    max_y: f64,
}

impl MomentAccum {
    fn add(&mut self, x: f64, y: f64) {
        if self.count == 0 {
            self.min_x = x;
            self.min_y = y;
            self.max_x = x;
            self.max_y = y;
        } else {
            self.min_x = self.min_x.min(x);
            self.min_y = self.min_y.min(y);
            self.max_x = self.max_x.max(x);
            self.max_y = self.max_y.max(y);
        }
        self.count += 1;
        self.sum_x += x;
        self.sum_y += y;
        self.sum_xx += x * x;
        self.sum_yy += y * y;
        self.sum_xy += x * y;
    }
}

#[derive(Clone, Copy)]
struct CloudAxes {
    mean_x: f64,
    mean_y: f64,
    angle: f64,
    major_extent: f64,
    minor_extent: f64,
}

#[derive(Clone, Copy)]
struct EngineBaseline {
    anchor_x: f64,
    anchor_y: f64,
    angle: f64,
    half_len: f64,
}

pub fn compute_country_label_placements(inputs: &LabelLawInputs) -> Vec<CountryLabelPlacement> {
    compute_country_label_placements_with_text(inputs, &BTreeMap::new())
}

pub fn compute_controller_label_placements(
    inputs: &ControllerLabelLawInputs,
) -> Vec<CountryLabelPlacement> {
    compute_controller_label_placements_with_text(inputs, &BTreeMap::new())
}

pub fn compute_controller_label_placements_with_text(
    inputs: &ControllerLabelLawInputs,
    text_metrics: &BTreeMap<String, LabelTextMetrics>,
) -> Vec<CountryLabelPlacement> {
    let expected = inputs.width.saturating_mul(inputs.height);
    if inputs.province_ids.len() != expected || expected == 0 {
        return Vec::new();
    }
    let grid = OwnerGrid::sample_controllers(inputs);
    compute_label_placements_from_grid(
        inputs.province_ids,
        grid,
        text_metrics,
        false,
        Some(inputs.province_adjacencies),
    )
}

pub fn compute_explicit_area_label_placements_with_text(
    inputs: &ExplicitAreaLabelLawInputs,
    text_metrics: &BTreeMap<String, LabelTextMetrics>,
) -> Vec<CountryLabelPlacement> {
    let expected = inputs.width.saturating_mul(inputs.height);
    if inputs.province_ids.len() != expected || expected == 0 {
        return Vec::new();
    }
    let grid = OwnerGrid::sample_explicit_areas(inputs);
    compute_label_placements_from_grid(inputs.province_ids, grid, text_metrics, true, None)
}

pub fn compute_country_label_placements_with_text(
    inputs: &LabelLawInputs,
    text_metrics: &BTreeMap<String, LabelTextMetrics>,
) -> Vec<CountryLabelPlacement> {
    let expected = inputs.width.saturating_mul(inputs.height);
    if inputs.province_ids.len() != expected || expected == 0 {
        return Vec::new();
    }

    let grid = OwnerGrid::sample(inputs);
    compute_label_placements_from_grid(inputs.province_ids, grid, text_metrics, false, None)
}

fn compute_label_placements_from_grid(
    province_ids: &[u32],
    mut grid: OwnerGrid<'_>,
    text_metrics: &BTreeMap<String, LabelTextMetrics>,
    explicit_areas: bool,
    province_adjacencies: Option<&BTreeSet<(u32, u32)>>,
) -> Vec<CountryLabelPlacement> {
    let home = if explicit_areas {
        grid.assign_one_component_per_owner()
    } else {
        grid.analyze_components(province_ids, province_adjacencies)
    };

    let mut moments: BTreeMap<u32, MomentAccum> = BTreeMap::new();
    let mut comp_owner: BTreeMap<u32, u32> = BTreeMap::new();
    grid.for_each_cell(|owner, gx, gy, comp| {
        comp_owner.entry(comp).or_insert(owner);
        let (x, y) = grid.cell_to_map(gx, gy);
        moments.entry(comp).or_default().add(x as f64, y as f64);
    });

    let mut axes: BTreeMap<u32, CloudAxes> = BTreeMap::new();
    for (comp, m) in &moments {
        if let Some(a) = axes_from_moments(m) {
            axes.insert(*comp, a);
        }
    }

    let mut engine: BTreeMap<u32, EngineBaseline> = BTreeMap::new();
    for (comp, m) in &moments {
        if !axes.contains_key(comp) {
            continue;
        }
        let owner = comp_owner[comp];
        let target = grid.tag(owner);
        if let Some(e) = grid.engine_baseline(*comp, m, target) {
            engine.insert(*comp, e);
        }
    }

    let incl = grid.inclusive_components();
    let mut incl_moments: BTreeMap<u32, MomentAccum> = BTreeMap::new();
    let mut comp_to_incl: BTreeMap<u32, u32> = BTreeMap::new();
    for cell in 0..incl.len() {
        let ic = incl[cell];
        if ic == 0 {
            continue;
        }
        let (x, y) = grid.cell_to_map(cell % grid.gw, cell / grid.gw);
        incl_moments.entry(ic).or_default().add(x as f64, y as f64);
        let pc = grid.comp[cell];
        if pc != 0 {
            comp_to_incl.entry(pc).or_insert(ic);
        }
    }
    let incl_axes: BTreeMap<u32, CloudAxes> = incl_moments
        .iter()
        .filter_map(|(ic, m)| axes_from_moments(m).map(|a| (*ic, a)))
        .collect();
    let mut incl_to_comps: BTreeMap<u32, Vec<u32>> = BTreeMap::new();
    for (pc, ic) in &comp_to_incl {
        incl_to_comps.entry(*ic).or_default().push(*pc);
    }
    let mut perp: BTreeMap<u32, Vec<f32>> = BTreeMap::new();
    for cell in 0..incl.len() {
        let ic = incl[cell];
        if ic == 0 {
            continue;
        }
        let Some(pcs) = incl_to_comps.get(&ic) else {
            continue;
        };
        let (x, y) = grid.cell_to_map(cell % grid.gw, cell / grid.gw);
        for pc in pcs {
            let Some(a) = axes.get(pc) else { continue };
            let (sin, cos) = a.angle.sin_cos();
            let v = -(x as f64 - a.mean_x) * sin + (y as f64 - a.mean_y) * cos;
            perp.entry(*pc).or_default().push(v as f32);
        }
    }

    let mut placements = Vec::new();
    for (comp, a) in &axes {
        let m = &moments[comp];
        let Some(eng) = engine.get(comp) else {
            continue;
        };
        let (anchor_x, anchor_y, angle) = (eng.anchor_x, eng.anchor_y, eng.angle);
        let half_length = eng.half_len as f32;
        if !half_length.is_finite() || half_length <= 0.0 {
            continue;
        }
        let owner = comp_owner[comp];
        let tag = grid.tag(owner);
        let is_home = home[owner as usize] == *comp;
        let incl_minor = comp_to_incl
            .get(comp)
            .and_then(|ic| incl_axes.get(ic))
            .map(|ia| ia.minor_extent as f32)
            .unwrap_or(a.minor_extent as f32);
        let world_height = (LABEL_HEIGHT_BREADTH_K * incl_minor).max(0.5);
        let (breadth_span, breadth_p2_98) = match perp.get_mut(comp) {
            Some(vs) if !vs.is_empty() => {
                vs.sort_by(f32::total_cmp);
                let n = vs.len();
                let lo = ((n as f64) * 0.02).floor() as usize;
                let hi = (((n as f64) * 0.98).ceil() as usize).clamp(lo + 1, n) - 1;
                (vs[n - 1] - vs[0], vs[hi] - vs[lo])
            }
            _ => (a.minor_extent as f32, a.minor_extent as f32),
        };
        let curve_and_anchor = text_metrics
            .get(tag)
            .and_then(|metrics| grid.medial_curve(m, tag, eng, metrics));
        let (curve, anchor_x, anchor_y) = match curve_and_anchor {
            Some((curve, (ax, ay))) => (Some(curve), ax, ay),
            None => (None, anchor_x, anchor_y),
        };
        placements.push(CountryLabelPlacement {
            tag: tag.to_string(),
            is_home,
            component_id: *comp,
            anchor_x: anchor_x as f32,
            anchor_y: anchor_y as f32,
            angle: angle as f32,
            half_length,
            bow: 0.0,
            world_height,
            pixel_count: m.count,
            major_extent: a.major_extent as f32,
            minor_extent: a.minor_extent as f32,
            minor_extent_incl: incl_minor,
            breadth_span,
            breadth_p2_98,
            curve,
        });
    }
    placements.sort_by(|l, r| {
        l.tag
            .cmp(&r.tag)
            .then((!l.is_home).cmp(&(!r.is_home)))
            .then(l.component_id.cmp(&r.component_id))
    });
    placements
}

struct OwnerGrid<'a> {
    gw: usize,
    gh: usize,
    owner: Vec<u32>,
    passable: Vec<bool>,
    flag: Vec<u8>,
    weight: Vec<u64>,
    comp: Vec<u32>,
    tags: Vec<&'a str>,
}

const NO_OWNER: u32 = u32::MAX;

impl<'a> OwnerGrid<'a> {
    fn sample(inputs: &LabelLawInputs<'a>) -> OwnerGrid<'a> {
        let gw = inputs.width.div_ceil(LABEL_SAMPLE_STEP);
        let gh = inputs.height.div_ceil(LABEL_SAMPLE_STEP);
        let mut owner = vec![NO_OWNER; gw * gh];
        let mut passable = vec![true; gw * gh];
        let mut flag = vec![0u8; gw * gh];
        let mut weight = vec![0u64; gw * gh];
        let mut tags: Vec<&str> = Vec::new();
        let mut tag_index: BTreeMap<&str, u32> = BTreeMap::new();
        let mut gy = 0usize;
        let mut y = 0usize;
        while y < inputs.height {
            let row = y * inputs.width;
            let mut gx = 0usize;
            let mut x = 0usize;
            while x < inputs.width {
                let province = inputs.province_ids[row + x];
                let cell = gy * gw + gx;
                if province != 0 {
                    let kind = inputs
                        .province_type
                        .get(&province)
                        .copied()
                        .unwrap_or(ProvinceKind::Land);
                    match kind {
                        ProvinceKind::Water => {
                            flag[cell] = FLAG_WATER;
                        }
                        ProvinceKind::Land => {
                            let state = inputs.province_to_state.get(&province);
                            let is_passable_land = match state {
                                Some(state) => !inputs.impassable_states.contains(state),
                                None => false,
                            };
                            if is_passable_land {
                                flag[cell] = FLAG_PASSABLE_LAND;
                            }
                        }
                    }
                    if let Some(state) = inputs.province_to_state.get(&province) {
                        if let Some(tag) = inputs.owners.get(state) {
                            let idx = *tag_index.entry(tag.as_str()).or_insert_with(|| {
                                let i = tags.len() as u32;
                                tags.push(tag.as_str());
                                i
                            });
                            owner[cell] = idx;
                            passable[cell] = !inputs.impassable_states.contains(state);
                            weight[cell] = inputs
                                .province_importance
                                .get(&province)
                                .copied()
                                .unwrap_or(0);
                        }
                    }
                }
                gx += 1;
                x += LABEL_SAMPLE_STEP;
            }
            gy += 1;
            y += LABEL_SAMPLE_STEP;
        }
        let comp = vec![0u32; gw * gh];
        OwnerGrid {
            gw,
            gh,
            owner,
            passable,
            flag,
            weight,
            comp,
            tags,
        }
    }

    fn sample_controllers(inputs: &ControllerLabelLawInputs<'a>) -> OwnerGrid<'a> {
        let gw = inputs.width.div_ceil(LABEL_SAMPLE_STEP);
        let gh = inputs.height.div_ceil(LABEL_SAMPLE_STEP);
        let mut owner = vec![NO_OWNER; gw * gh];
        let mut passable = vec![true; gw * gh];
        let mut flag = vec![0u8; gw * gh];
        let mut weight = vec![0u64; gw * gh];
        let mut tags: Vec<&str> = Vec::new();
        let mut tag_index: BTreeMap<&str, u32> = BTreeMap::new();
        let mut gy = 0usize;
        let mut y = 0usize;
        while y < inputs.height {
            let row = y * inputs.width;
            let mut gx = 0usize;
            let mut x = 0usize;
            while x < inputs.width {
                let province = inputs.province_ids[row + x];
                let cell = gy * gw + gx;
                if province != 0 {
                    let kind = inputs
                        .province_type
                        .get(&province)
                        .copied()
                        .unwrap_or(ProvinceKind::Land);
                    let state = inputs.province_to_state.get(&province);
                    match kind {
                        ProvinceKind::Water => flag[cell] = FLAG_WATER,
                        ProvinceKind::Land => {
                            if state.is_some_and(|state| !inputs.impassable_states.contains(state))
                            {
                                flag[cell] = FLAG_PASSABLE_LAND;
                            }
                        }
                    }
                    if let Some(tag) = inputs.controllers.get(&province) {
                        let idx = *tag_index.entry(tag.as_str()).or_insert_with(|| {
                            let i = tags.len() as u32;
                            tags.push(tag.as_str());
                            i
                        });
                        owner[cell] = idx;
                        passable[cell] =
                            state.is_some_and(|state| !inputs.impassable_states.contains(state));
                        weight[cell] = inputs
                            .province_importance
                            .get(&province)
                            .copied()
                            .unwrap_or(0);
                    }
                }
                gx += 1;
                x += LABEL_SAMPLE_STEP;
            }
            gy += 1;
            y += LABEL_SAMPLE_STEP;
        }
        OwnerGrid {
            gw,
            gh,
            owner,
            passable,
            flag,
            weight,
            comp: vec![0u32; gw * gh],
            tags,
        }
    }

    fn sample_explicit_areas(inputs: &ExplicitAreaLabelLawInputs<'a>) -> OwnerGrid<'a> {
        let gw = inputs.width.div_ceil(LABEL_SAMPLE_STEP);
        let gh = inputs.height.div_ceil(LABEL_SAMPLE_STEP);
        let mut owner = vec![NO_OWNER; gw * gh];
        let mut passable = vec![true; gw * gh];
        let mut flag = vec![0u8; gw * gh];
        let mut weight = vec![0u64; gw * gh];
        let mut tags: Vec<&str> = Vec::new();
        let mut tag_index: BTreeMap<&str, u32> = BTreeMap::new();
        let mut gy = 0usize;
        let mut y = 0usize;
        while y < inputs.height {
            let row = y * inputs.width;
            let mut gx = 0usize;
            let mut x = 0usize;
            while x < inputs.width {
                let province = inputs.province_ids[row + x];
                let cell = gy * gw + gx;
                if province != 0 {
                    let kind = inputs
                        .province_type
                        .get(&province)
                        .copied()
                        .unwrap_or(ProvinceKind::Land);
                    let state = inputs.province_to_state.get(&province);
                    match kind {
                        ProvinceKind::Water => flag[cell] = FLAG_WATER,
                        ProvinceKind::Land => {
                            if state.is_some_and(|state| !inputs.impassable_states.contains(state))
                            {
                                flag[cell] = FLAG_PASSABLE_LAND;
                            }
                        }
                    }
                    if let Some(area) = inputs.area_by_province.get(&province) {
                        let idx = *tag_index.entry(area.as_str()).or_insert_with(|| {
                            let i = tags.len() as u32;
                            tags.push(area.as_str());
                            i
                        });
                        owner[cell] = idx;
                        passable[cell] =
                            state.is_some_and(|state| !inputs.impassable_states.contains(state));
                        weight[cell] = inputs
                            .province_importance
                            .get(&province)
                            .copied()
                            .unwrap_or(0);
                    }
                }
                gx += 1;
                x += LABEL_SAMPLE_STEP;
            }
            gy += 1;
            y += LABEL_SAMPLE_STEP;
        }
        OwnerGrid {
            gw,
            gh,
            owner,
            passable,
            flag,
            weight,
            comp: vec![0u32; gw * gh],
            tags,
        }
    }

    fn assign_one_component_per_owner(&mut self) -> Vec<u32> {
        let mut home = vec![0u32; self.tags.len()];
        for cell in 0..self.comp.len() {
            if self.is_placement_cell(cell) {
                let component = self.owner[cell] + 1;
                self.comp[cell] = component;
                home[self.owner[cell] as usize] = component;
            }
        }
        home
    }

    fn tag(&self, owner: u32) -> &'a str {
        self.tags[owner as usize]
    }

    fn cell_to_map(&self, gx: usize, gy: usize) -> (usize, usize) {
        (gx * LABEL_SAMPLE_STEP, gy * LABEL_SAMPLE_STEP)
    }

    fn owner_tag_at(&self, x: f64, y: f64) -> &'a str {
        match self.cell_index(x, y) {
            Some(cell) => {
                let o = self.owner[cell];
                if o == NO_OWNER {
                    ""
                } else {
                    self.tags[o as usize]
                }
            }
            None => "",
        }
    }

    fn flag_at(&self, x: f64, y: f64) -> u8 {
        self.cell_index(x, y).map(|c| self.flag[c]).unwrap_or(0)
    }

    fn is_placement_cell(&self, cell: usize) -> bool {
        self.owner[cell] != NO_OWNER
            && self.passable[cell]
            && (self.flag[cell] & FLAG_PASSABLE_LAND) != 0
    }

    fn cell_index(&self, x: f64, y: f64) -> Option<usize> {
        let step = LABEL_SAMPLE_STEP as f64;
        let gxf = (x / step).round();
        let gyf = (y / step).round();
        if gxf < 0.0 || gxf as usize >= self.gw || gyf < 0.0 || gyf as usize >= self.gh {
            return None;
        }
        Some(gyf as usize * self.gw + gxf as usize)
    }

    fn engine_baseline(&self, comp: u32, m: &MomentAccum, target: &str) -> Option<EngineBaseline> {
        let n = m.count as f64;
        if n < 2.0 {
            return None;
        }
        let (sx, sy, sxx, syy, sxy) = (m.sum_x, m.sum_y, m.sum_xx, m.sum_yy, m.sum_xy);
        let mean_x = sx / n;
        let mean_y = sy / n;
        let (min_x, min_y, max_x, max_y) = (m.min_x, m.min_y, m.max_x, m.max_y);
        let num = n * sxy - sy * sx;
        let la = {
            let dxx = n * sxx - sx * sx;
            if dxx.abs() > 1e-6 {
                let slope = num / dxx;
                let b = mean_y - slope * mean_x;
                self.best_seg_for_line(
                    comp,
                    target,
                    (min_x, slope * min_x + b),
                    (max_x, slope * max_x + b),
                )
            } else {
                None
            }
        };
        let lb = {
            let dyy = n * syy - sy * sy;
            if dyy.abs() > 1e-6 {
                let slope = num / dyy;
                let b = mean_x - slope * mean_y;
                self.best_seg_for_line(
                    comp,
                    target,
                    (slope * min_y + b, min_y),
                    (slope * max_y + b, max_y),
                )
            } else {
                None
            }
        };
        let chosen = match (la, lb) {
            (Some(a), Some(b)) => {
                if b.2 * b.2 >= LABEL_AB_HYSTERESIS * (a.2 * a.2) {
                    b
                } else {
                    a
                }
            }
            (Some(a), None) => a,
            (None, Some(b)) => b,
            (None, None) => return None,
        };
        let ((sx0, sy0), (sx1, sy1), len) = chosen;
        let ax = (sx0 + sx1) * 0.5;
        let ay = (sy0 + sy1) * 0.5;
        let mut angle = (sy1 - sy0).atan2(sx1 - sx0);
        if angle > std::f64::consts::FRAC_PI_2 {
            angle -= std::f64::consts::PI;
        } else if angle <= -std::f64::consts::FRAC_PI_2 {
            angle += std::f64::consts::PI;
        }
        Some(EngineBaseline {
            anchor_x: ax,
            anchor_y: ay,
            angle,
            half_len: len * 0.5,
        })
    }

    fn best_seg_for_line(
        &self,
        comp: u32,
        target: &str,
        p0: (f64, f64),
        p1: (f64, f64),
    ) -> Option<((f64, f64), (f64, f64), f64)> {
        let (x0, y0) = p0;
        let (x1, y1) = p1;
        let (dx, dy) = (x1 - x0, y1 - y0);
        let l = (dx * dx + dy * dy).sqrt();
        if l < 1.0 {
            return None;
        }
        let (px, py) = (-dy / l, dx / l);
        let mut best: Option<((f64, f64), (f64, f64), f64)> = None;
        let mut best_len2 = -1.0f64;
        for iv in [-1.0f64, 0.0, 1.0] {
            let off = iv * LABEL_OFFSET_PX;
            let ax0 = trunc_toward_zero((x0 + px * off) as f32);
            let ay0 = trunc_toward_zero((y0 + py * off) as f32);
            let ax1 = trunc_toward_zero((x1 + px * off) as f32);
            let ay1 = trunc_toward_zero((y1 + py * off) as f32);
            let pts = dda(ax0, ay0, ax1, ay1);
            let Some((sa, sb)) = self.owned_trim(&pts, comp, target) else {
                continue;
            };
            let (sx0, sy0) = (sa.0 as f64, sa.1 as f64);
            let (sx1, sy1) = (sb.0 as f64, sb.1 as f64);
            let l2 = (sx1 - sx0).powi(2) + (sy1 - sy0).powi(2);
            if l2 > best_len2 {
                best_len2 = l2;
                best = Some(((sx0, sy0), (sx1, sy1), l2.sqrt()));
            }
        }
        best
    }

    fn owned_trim(
        &self,
        pts: &[(i64, i64)],
        _comp: u32,
        target: &str,
    ) -> Option<((i64, i64), (i64, i64))> {
        let n = pts.len();
        if n == 0 {
            return None;
        }
        let strictly_owned = |i: usize| -> bool {
            let (x, y) = pts[i];
            let owner = self.owner_tag_at(x as f64, y as f64);
            let fl = self.flag_at(x as f64, y as f64);
            owner == target && (fl & FLAG_PASSABLE_LAND) != 0
        };
        let end = (0..n)
            .rev()
            .find(|&i| strictly_owned(i))
            .map(|i| i + 1)
            .unwrap_or(n);
        let head = (0..end).find(|&j| strictly_owned(j)).unwrap_or(0);
        let span = &pts[head..end];
        let m = span.len();
        if m < 1 {
            return None;
        }
        let extends = |i: usize| -> bool {
            let (x, y) = span[i];
            let owner = self.owner_tag_at(x as f64, y as f64);
            let fl = self.flag_at(x as f64, y as f64);
            owner == target || (fl & FLAG_WATER) != 0
        };
        let mut best_len = 0usize;
        let mut best: Option<(usize, usize)> = None;
        let mut cur: Option<usize> = None;
        for i in 0..m {
            if extends(i) {
                if cur.is_none() {
                    cur = Some(i);
                }
            } else if let Some(s) = cur.take() {
                if i - s > best_len {
                    best_len = i - s;
                    best = Some((s, i - 1));
                }
            }
        }
        if let Some(s) = cur {
            if m - s > best_len {
                best = Some((s, m - 1));
            }
        }
        let (a, b) = best?;
        Some((span[a], span[b]))
    }

    fn owned_trim_station(
        &self,
        pts: &[(i64, i64)],
        target: &str,
        station: (i64, i64),
    ) -> Option<((i64, i64), (i64, i64))> {
        let n = pts.len();
        if n == 0 {
            return None;
        }
        let strictly_owned = |i: usize| -> bool {
            let (x, y) = pts[i];
            let owner = self.owner_tag_at(x as f64, y as f64);
            let fl = self.flag_at(x as f64, y as f64);
            owner == target && (fl & FLAG_PASSABLE_LAND) != 0
        };
        let end = (0..n)
            .rev()
            .find(|&i| strictly_owned(i))
            .map(|i| i + 1)
            .unwrap_or(n);
        let head = (0..end).find(|&j| strictly_owned(j)).unwrap_or(0);
        let span = &pts[head..end];
        let m = span.len();
        if m < 1 {
            return None;
        }
        let extends = |i: usize| -> bool {
            let (x, y) = span[i];
            let owner = self.owner_tag_at(x as f64, y as f64);
            let fl = self.flag_at(x as f64, y as f64);
            owner == target || (fl & FLAG_WATER) != 0
        };
        const NEAR_BOOST: f32 = 1.4;
        let mut best: Option<(usize, usize)> = None;
        let mut best_len = 0usize;
        let mut best_d2 = 0x7fff_ffffi64;
        let consider = |s: usize,
                        e: usize,
                        best: &mut Option<(usize, usize)>,
                        best_len: &mut usize,
                        best_d2: &mut i64| {
            let len = e - s + 1;
            let d2 = (s..=e)
                .map(|i| {
                    let dx = span[i].0 - station.0;
                    let dy = span[i].1 - station.1;
                    dx * dx + dy * dy
                })
                .min()
                .unwrap_or(0x7fff_ffff);
            let mut cand_v = len as f32;
            if d2 < *best_d2 {
                cand_v *= NEAR_BOOST;
            }
            let mut best_v = *best_len as f32;
            if *best_d2 < d2 {
                best_v *= NEAR_BOOST;
            }
            if (best_v as i64) < (cand_v as i64) {
                *best = Some((s, e));
                *best_len = len;
                *best_d2 = d2;
            }
        };
        let mut cur: Option<usize> = None;
        for i in 0..m {
            if extends(i) {
                if cur.is_none() {
                    cur = Some(i);
                }
            } else if let Some(s) = cur.take() {
                consider(s, i - 1, &mut best, &mut best_len, &mut best_d2);
            }
        }
        if let Some(s) = cur {
            consider(s, m - 1, &mut best, &mut best_len, &mut best_d2);
        }
        let (a, b) = best?;
        Some((span[a], span[b]))
    }

    fn medial_curve(
        &self,
        m: &MomentAccum,
        target: &str,
        eng: &EngineBaseline,
        metrics: &LabelTextMetrics,
    ) -> Option<(EngineCurve, (f64, f64))> {
        let len = (2.0 * eng.half_len) as f32;
        if !(len > 1.0) {
            return None;
        }
        let text_w: f32 = metrics.advances.iter().map(|(a, _)| *a).sum();
        let text_h = metrics.line_height_px;
        if !(text_w > 0.0) || !(text_h > 0.0) {
            return None;
        }
        let pixels = m.count as f32;
        let s_w = 0.9 * len / text_w;
        let s_t = 0.5 * (pixels / len) / text_h;
        let cap_bound = s_t < s_w;
        let scale = 0.8 * s_w.min(s_t);
        if !(scale > 0.0) {
            return None;
        }
        let gaps = metrics.advances.len().saturating_sub(1) as f32;
        let mut k_spaces = 0u32;
        if cap_bound && gaps > 0.0 && metrics.space_advance_px > 0.0 {
            let w_target = 0.9 * len / s_t;
            for k in 1..=19u32 {
                k_spaces = k;
                if text_w + k as f32 * metrics.space_advance_px * gaps >= w_target {
                    break;
                }
            }
        }
        let extra = k_spaces as f32 * metrics.space_advance_px;
        let expected_glyphs = metrics
            .advances
            .iter()
            .filter(|(_, is_space)| !*is_space)
            .count();
        if metrics.straight_layout_stations.len() != expected_glyphs
            || expected_glyphs < 2
            || metrics.straight_layout_stations.iter().any(|station| {
                station.slot_index >= metrics.advances.len()
                    || !station.x.is_finite()
                    || !station.y.is_finite()
            })
        {
            return None;
        }
        let total_px = mapname_tracked_advance_px(metrics, extra);
        if !(total_px > 0.0) || !total_px.is_finite() {
            return None;
        }
        let station_font_points: Vec<(f32, f32)> = metrics
            .straight_layout_stations
            .iter()
            .map(|station| {
                let x = station.x + station.slot_index as f32 * extra;
                (x, station.y)
            })
            .collect();
        let measured_t = measured_station_parameters(&station_font_points)?;
        let layout_stations: Vec<(f32, f32, f32)> = station_font_points
            .into_iter()
            .zip(measured_t)
            .map(|((x, y), t)| (x, y, t))
            .collect();
        let (sin, cos) = (eng.angle.sin() as f32, eng.angle.cos() as f32);
        let (ax, ay) = (eng.anchor_x as f32, eng.anchor_y as f32);
        let stations: Vec<(f32, f32)> = layout_stations
            .iter()
            .map(|(x, y, _)| {
                let along = (x - 0.5 * total_px) * scale;
                let across = (y - 0.5 * text_h) * scale;
                (
                    ax + along * cos - across * sin,
                    ay + along * sin + across * cos,
                )
            })
            .collect();
        let (bx0, by0, bx1, by1) = (m.min_x, m.min_y, m.max_x, m.max_y);
        let seg_dx = (len * cos) as f64;
        let seg_dy = (len * sin) as f64;
        let steep = seg_dy.abs() > 0.01;
        let mut sum_x = 0.0f64;
        let mut sum_y = 0.0f64;
        let mut first_med = (0.0f64, 0.0f64);
        let mut last_med = (0.0f64, 0.0f64);
        let n_st = stations.len();
        for (i, &(sx, sy)) in stations.iter().enumerate() {
            let (p0, p1) = if steep {
                let slope = -(seg_dx / seg_dy);
                let intercept = sy as f64 - sx as f64 * slope;
                let endpoint = |x_edge: f64| -> (f64, f64) {
                    let y = x_edge * slope + intercept;
                    let yc = y.clamp(by0, by1);
                    if y != yc {
                        (((yc - intercept) / slope), yc)
                    } else {
                        (x_edge, y)
                    }
                };
                (endpoint(bx0), endpoint(bx1))
            } else {
                ((sx as f64, by0), (sx as f64, by1))
            };
            let pts = dda(
                trunc_toward_zero(p0.0 as f32),
                trunc_toward_zero(p0.1 as f32),
                trunc_toward_zero(p1.0 as f32),
                trunc_toward_zero(p1.1 as f32),
            );
            let station_i = (trunc_toward_zero(sx), trunc_toward_zero(sy));
            let run = self.owned_trim_station(&pts, target, station_i);
            let (mx, my) = match run {
                Some(((fx, fy), (lx, ly))) => {
                    ((fx as f32 + lx as f32) * 0.5, (fy as f32 + ly as f32) * 0.5)
                }
                None => (sx, sy),
            };
            sum_x += mx as f64;
            sum_y += my as f64;
            if i == 0 {
                first_med = (((mx + sx) * 0.5) as f64, ((my + sy) * 0.5) as f64);
            }
            if i + 1 == n_st {
                last_med = (((mx + sx) * 0.5) as f64, ((my + sy) * 0.5) as f64);
            }
        }
        let mean = (sum_x / n_st as f64, sum_y / n_st as f64);
        let c0 = phantom_control(first_med, mean, last_med)?;
        let c4 = phantom_control_end(first_med, mean, last_med)?;
        let (c0, first, mean_p, last, c4) = if last_med.0 < first_med.0 {
            (c4, last_med, mean, first_med, c0)
        } else {
            (c0, first_med, mean, last_med, c4)
        };
        let curve = EngineCurve {
            pts: [c0, first, mean_p, last, c4],
        };
        let mut ax_sum = 0.0f64;
        let mut ay_sum = 0.0f64;
        for (_, _, t) in &layout_stations {
            let (px, py) = curve.point_at(*t);
            ax_sum += px as f64;
            ay_sum += py as f64;
        }
        let anchor = (ax_sum / n_st as f64, ay_sum / n_st as f64);
        Some((curve, anchor))
    }

    fn analyze_components(
        &mut self,
        province_ids: &[u32],
        province_adjacencies: Option<&BTreeSet<(u32, u32)>>,
    ) -> Vec<u32> {
        let mut next_comp = 1u32;
        let mut queue: Vec<usize> = Vec::new();
        for start in 0..self.owner.len() {
            let owner = self.owner[start];
            if !self.is_placement_cell(start) || self.comp[start] != 0 {
                continue;
            }
            let comp_id = next_comp;
            next_comp += 1;
            queue.clear();
            queue.push(start);
            self.comp[start] = comp_id;
            while let Some(cell) = queue.pop() {
                let gx = cell % self.gw;
                let gy = cell / self.gw;
                let west = if gx == 0 { self.gw - 1 } else { gx - 1 };
                let east = if gx + 1 == self.gw { 0 } else { gx + 1 };
                let mut neighbours = [usize::MAX; 4];
                neighbours[0] = gy * self.gw + west;
                neighbours[1] = gy * self.gw + east;
                if gy > 0 {
                    neighbours[2] = (gy - 1) * self.gw + gx;
                }
                if gy + 1 < self.gh {
                    neighbours[3] = (gy + 1) * self.gw + gx;
                }
                for &nb in &neighbours {
                    if nb != usize::MAX
                        && self.owner[nb] == owner
                        && self.is_placement_cell(nb)
                        && self.comp[nb] == 0
                    {
                        self.comp[nb] = comp_id;
                        queue.push(nb);
                    }
                }
            }
        }
        self.merge_provinces_atomic(province_ids, next_comp, province_adjacencies);
        let mut best: BTreeMap<u32, (u32, u64, u64, u64)> = BTreeMap::new();
        let mut agg: BTreeMap<u32, (u32, u64, u64, u64)> = BTreeMap::new();
        for cell in 0..self.owner.len() {
            if !self.is_placement_cell(cell) {
                continue;
            }
            let comp = self.comp[cell];
            if comp == 0 {
                continue;
            }
            let w = self.weight[cell];
            let e = agg.entry(comp).or_insert((self.owner[cell], 0, 0, 0));
            e.1 = e.1.max(w);
            e.2 = e.2.saturating_add(w);
            e.3 += 1;
        }
        for (comp, (owner, peak, sum, size)) in agg {
            let entry = best.entry(owner).or_insert((comp, 0, 0, 0));
            if (peak, sum, size) > (entry.1, entry.2, entry.3) {
                *entry = (comp, peak, sum, size);
            }
        }
        let mut home = vec![0u32; self.tags.len()];
        for (owner, (comp_id, _, _, _)) in best {
            home[owner as usize] = comp_id;
        }
        home
    }

    fn merge_provinces_atomic(
        &mut self,
        province_ids: &[u32],
        next_comp: u32,
        province_adjacencies: Option<&BTreeSet<(u32, u32)>>,
    ) {
        let n = next_comp as usize;
        if n <= 1 {
            return;
        }
        let mut parent: Vec<u32> = (0..next_comp).collect();
        fn find(parent: &mut [u32], mut a: u32) -> u32 {
            while parent[a as usize] != a {
                parent[a as usize] = parent[parent[a as usize] as usize];
                a = parent[a as usize];
            }
            a
        }
        debug_assert_eq!(LABEL_SAMPLE_STEP, 1);
        debug_assert_eq!(province_ids.len(), self.comp.len());
        let mut first: BTreeMap<u32, u32> = BTreeMap::new();
        let mut component_owner = BTreeMap::<u32, u32>::new();
        for cell in 0..self.comp.len() {
            let c = self.comp[cell];
            if c == 0 {
                continue;
            }
            component_owner.entry(c).or_insert(self.owner[cell]);
            let pid = province_ids[cell];
            match first.get(&pid) {
                None => {
                    first.insert(pid, c);
                }
                Some(&c0) => {
                    let (ra, rb) = (find(&mut parent, c0), find(&mut parent, c));
                    if ra != rb {
                        parent[ra as usize] = rb;
                    }
                }
            }
        }
        if let Some(adjacencies) = province_adjacencies {
            for &(a, b) in adjacencies {
                let (Some(&ca), Some(&cb)) = (first.get(&a), first.get(&b)) else {
                    continue;
                };
                if component_owner.get(&ca) != component_owner.get(&cb) {
                    continue;
                }
                let (ra, rb) = (find(&mut parent, ca), find(&mut parent, cb));
                if ra != rb {
                    parent[ra as usize] = rb;
                }
            }
        }
        for cell in 0..self.comp.len() {
            let c = self.comp[cell];
            if c != 0 {
                self.comp[cell] = find(&mut parent, c);
            }
        }
    }

    fn for_each_cell(&self, mut visit: impl FnMut(u32, usize, usize, u32)) {
        for cell in 0..self.owner.len() {
            if !self.is_placement_cell(cell) {
                continue;
            }
            let owner = self.owner[cell];
            visit(owner, cell % self.gw, cell / self.gw, self.comp[cell]);
        }
    }

    fn inclusive_components(&self) -> Vec<u32> {
        let mut comp = vec![0u32; self.owner.len()];
        let mut next = 1u32;
        let mut queue: Vec<usize> = Vec::new();
        for start in 0..self.owner.len() {
            let owner = self.owner[start];
            if owner == NO_OWNER || comp[start] != 0 {
                continue;
            }
            let id = next;
            next += 1;
            comp[start] = id;
            queue.clear();
            queue.push(start);
            while let Some(cell) = queue.pop() {
                let gx = cell % self.gw;
                let gy = cell / self.gw;
                let west = if gx == 0 { self.gw - 1 } else { gx - 1 };
                let east = if gx + 1 == self.gw { 0 } else { gx + 1 };
                let mut neighbours = [usize::MAX; 4];
                neighbours[0] = gy * self.gw + west;
                neighbours[1] = gy * self.gw + east;
                if gy > 0 {
                    neighbours[2] = (gy - 1) * self.gw + gx;
                }
                if gy + 1 < self.gh {
                    neighbours[3] = (gy + 1) * self.gw + gx;
                }
                for &nb in &neighbours {
                    if nb != usize::MAX && self.owner[nb] == owner && comp[nb] == 0 {
                        comp[nb] = id;
                        queue.push(nb);
                    }
                }
            }
        }
        comp
    }
}

fn trunc_toward_zero(x: f32) -> i64 {
    x as i64
}

pub fn label_draw_visible(
    world_pts: &[[f32; 3]],
    vp_col_major: &[f32; 16],
    cam_height: f32,
    cutoff: f32,
) -> bool {
    if cam_height < cutoff || world_pts.is_empty() {
        return false;
    }
    let m = |r: usize, c: usize| vp_col_major[r * 4 + c] as f64;
    let mut any_front = false;
    let (mut min_x, mut max_x) = (f64::INFINITY, f64::NEG_INFINITY);
    let (mut min_y, mut max_y) = (f64::INFINITY, f64::NEG_INFINITY);
    for p in world_pts {
        let v = [p[0] as f64, p[1] as f64, p[2] as f64, 1.0];
        let clip: [f64; 4] = std::array::from_fn(|r| (0..4).map(|c| m(c, r) * v[c]).sum());
        if clip[3] <= 0.0 {
            continue;
        }
        any_front = true;
        let nx = clip[0] / clip[3];
        let ny = clip[1] / clip[3];
        min_x = min_x.min(nx);
        max_x = max_x.max(nx);
        min_y = min_y.min(ny);
        max_y = max_y.max(ny);
    }
    any_front && max_x >= -1.0 && min_x <= 1.0 && max_y >= -1.0 && min_y <= 1.0
}

fn phantom_control(first: (f64, f64), mean: (f64, f64), last: (f64, f64)) -> Option<(f64, f64)> {
    let v = (first.0 - mean.0, first.1 - mean.1);
    let w = (mean.0 - last.0, mean.1 - last.1);
    let lw = (w.0 * w.0 + w.1 * w.1).sqrt();
    let lv = (v.0 * v.0 + v.1 * v.1).sqrt();
    if !(lw > 0.0) || !(lv > 0.0) {
        return None;
    }
    let n2 = (w.0 / lw, w.1 / lw);
    let dot = v.0 * n2.0 + v.1 * n2.1;
    let cross = n2.0 * v.1 - n2.1 * v.0;
    let n1 = (v.0 / lv, v.1 / lv);
    Some((
        first.0 + dot * n1.0 - cross * n1.1,
        first.1 + dot * n1.1 + cross * n1.0,
    ))
}

fn phantom_control_end(
    first: (f64, f64),
    mean: (f64, f64),
    last: (f64, f64),
) -> Option<(f64, f64)> {
    let u = (last.0 - mean.0, last.1 - mean.1);
    let p = (mean.0 - first.0, mean.1 - first.1);
    let lp = (p.0 * p.0 + p.1 * p.1).sqrt();
    let lu = (u.0 * u.0 + u.1 * u.1).sqrt();
    if !(lp > 0.0) || !(lu > 0.0) {
        return None;
    }
    let n1p = (p.0 / lp, p.1 / lp);
    let dotp = u.0 * n1p.0 + u.1 * n1p.1;
    let crossp = u.0 * n1p.1 - u.1 * n1p.0;
    let n2p = (u.0 / lu, u.1 / lu);
    Some((
        last.0 + dotp * n2p.0 + crossp * n2p.1,
        last.1 + dotp * n2p.1 - crossp * n2p.0,
    ))
}

fn dda(x0: i64, y0: i64, x1: i64, y1: i64) -> Vec<(i64, i64)> {
    let dxabs = (x1 - x0).abs();
    let dyabs = (y1 - y0).abs();
    let major_is_x = dxabs >= dyabs;
    let (mut a_lo, mut a_hi, mut b_lo, mut b_hi) = if major_is_x {
        (x0, x1, y0, y1)
    } else {
        (y0, y1, x0, x1)
    };
    if a_hi < a_lo {
        std::mem::swap(&mut a_lo, &mut a_hi);
        std::mem::swap(&mut b_lo, &mut b_hi);
    }
    let da = a_hi - a_lo;
    let db_signed = b_hi - b_lo;
    let step = if db_signed >= 0 { 1 } else { -1 };
    let db = db_signed.abs();
    let mut err = 0i64;
    let mut b = b_lo;
    let mut a = a_lo;
    let cap = (da + 1).max(0) as usize;
    let mut pts = Vec::with_capacity(cap);
    while a <= a_hi {
        if major_is_x {
            pts.push((a, b));
        } else {
            pts.push((b, a));
        }
        err += db;
        if da <= 2 * err {
            b += step;
            err -= da;
        }
        a += 1;
    }
    pts
}

fn axes_from_moments(m: &MomentAccum) -> Option<CloudAxes> {
    if m.count == 0 {
        return None;
    }
    let n = m.count as f64;
    let mean_x = m.sum_x / n;
    let mean_y = m.sum_y / n;
    let cxx = m.sum_xx / n - mean_x * mean_x;
    let cyy = m.sum_yy / n - mean_y * mean_y;
    let cxy = m.sum_xy / n - mean_x * mean_y;
    let (angle, major_var, minor_var) = principal_axis(cxx, cyy, cxy);
    let major_extent = (12.0 * major_var).max(0.0).sqrt();
    let minor_extent = (12.0 * minor_var).max(0.0).sqrt();
    if !major_extent.is_finite() || major_extent <= 0.0 {
        return None;
    }
    Some(CloudAxes {
        mean_x,
        mean_y,
        angle,
        major_extent,
        minor_extent,
    })
}

pub fn principal_axis(cxx: f64, cyy: f64, cxy: f64) -> (f64, f64, f64) {
    let trace = cxx + cyy;
    let diff = cxx - cyy;
    let disc = (diff * diff + 4.0 * cxy * cxy).max(0.0).sqrt();
    let major_var = 0.5 * (trace + disc);
    let minor_var = 0.5 * (trace - disc);
    let mut angle = 0.5 * (2.0 * cxy).atan2(diff);
    if angle > std::f64::consts::FRAC_PI_2 {
        angle -= std::f64::consts::PI;
    } else if angle <= -std::f64::consts::FRAC_PI_2 {
        angle += std::f64::consts::PI;
    }
    (angle, major_var, minor_var)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn raster_rect(ids: &mut [u32], width: usize, id: u32, rect: (usize, usize, usize, usize)) {
        let (x0, y0, x1, y1) = rect;
        for y in y0..y1 {
            for x in x0..x1 {
                ids[y * width + x] = id;
            }
        }
    }

    fn land_types(ids: &[u32]) -> BTreeMap<u32, ProvinceKind> {
        ids.iter().map(|&id| (id, ProvinceKind::Land)).collect()
    }

    #[test]
    fn horizontal_country_labels_horizontally() {
        let width = 200;
        let height = 100;
        let mut ids = vec![0u32; width * height];
        raster_rect(&mut ids, width, 1, (20, 40, 180, 60));
        let ptype = land_types(&[1]);
        let inputs = LabelLawInputs {
            province_ids: &ids,
            width,
            height,
            province_to_state: &BTreeMap::from([(1u32, 10u32)]),
            owners: &BTreeMap::from([(10u32, "GER".to_string())]),
            impassable_states: &BTreeSet::new(),
            province_importance: &BTreeMap::new(),
            province_type: &ptype,
        };
        let placements = compute_country_label_placements(&inputs);
        assert_eq!(placements.len(), 1);
        let p = &placements[0];
        assert_eq!(p.tag, "GER");
        assert!((p.anchor_x - 100.0).abs() < 3.0, "anchor_x {}", p.anchor_x);
        assert!(
            (p.anchor_y - 50.0).abs() <= LABEL_OFFSET_PX as f32 + 1.0,
            "anchor_y {}",
            p.anchor_y
        );
        assert!(p.angle.abs() < 0.08, "angle {}", p.angle);
        assert!(p.half_length > 30.0, "half_length {}", p.half_length);
        assert_eq!(p.bow, 0.0, "engine baseline is straight");
    }

    #[test]
    fn b10_controller_key_splits_same_owner_state_at_occupation_boundary() {
        let width = 400;
        let height = 100;
        let mut ids = vec![0u32; width * height];
        raster_rect(&mut ids, width, 1, (20, 40, 200, 60));
        raster_rect(&mut ids, width, 2, (200, 40, 380, 60));
        let ptype = land_types(&[1, 2]);
        let placements = compute_controller_label_placements(&ControllerLabelLawInputs {
            province_ids: &ids,
            width,
            height,
            province_to_state: &BTreeMap::from([(1, 10), (2, 10)]),
            controllers: &BTreeMap::from([(1, "GER".to_string()), (2, "POL".to_string())]),
            impassable_states: &BTreeSet::new(),
            province_importance: &BTreeMap::new(),
            province_type: &ptype,
            province_adjacencies: &BTreeSet::new(),
        });
        assert_eq!(
            placements
                .iter()
                .map(|placement| placement.tag.as_str())
                .collect::<Vec<_>>(),
            vec!["GER", "POL"]
        );
    }

    #[test]
    fn case2_explicit_state_keeps_disconnected_provinces_in_one_area() {
        let width = 240;
        let height = 100;
        let mut ids = vec![0u32; width * height];
        raster_rect(&mut ids, width, 1, (20, 40, 90, 60));
        raster_rect(&mut ids, width, 2, (150, 40, 220, 60));
        let ptype = land_types(&[1, 2]);
        let placements = compute_explicit_area_label_placements_with_text(
            &ExplicitAreaLabelLawInputs {
                province_ids: &ids,
                width,
                height,
                area_by_province: &BTreeMap::from([
                    (1, "STATE:10".to_string()),
                    (2, "STATE:10".to_string()),
                ]),
                province_to_state: &BTreeMap::from([(1, 10), (2, 10)]),
                impassable_states: &BTreeSet::new(),
                province_importance: &BTreeMap::new(),
                province_type: &ptype,
            },
            &BTreeMap::new(),
        );
        assert_eq!(placements.len(), 1);
        assert_eq!(placements[0].tag, "STATE:10");
        assert_eq!(placements[0].component_id, 1);
    }

    #[test]
    fn authored_adjacency_joins_same_controller_provinces() {
        let width = 240;
        let height = 100;
        let mut ids = vec![0u32; width * height];
        raster_rect(&mut ids, width, 1, (20, 40, 90, 60));
        raster_rect(&mut ids, width, 2, (150, 40, 220, 60));
        let ptype = land_types(&[1, 2]);
        let placements = compute_controller_label_placements(&ControllerLabelLawInputs {
            province_ids: &ids,
            width,
            height,
            province_to_state: &BTreeMap::from([(1, 10), (2, 11)]),
            controllers: &BTreeMap::from([(1, "GER".to_string()), (2, "GER".to_string())]),
            impassable_states: &BTreeSet::new(),
            province_importance: &BTreeMap::new(),
            province_type: &ptype,
            province_adjacencies: &BTreeSet::from([(1, 2)]),
        });
        assert_eq!(placements.len(), 1);
        assert_eq!(placements[0].tag, "GER");
    }

    #[test]
    fn pixel_split_province_emits_one_area() {
        let width = 200;
        let height = 100;
        let mut ids = vec![0u32; width * height];
        raster_rect(&mut ids, width, 1, (20, 40, 80, 60));
        raster_rect(&mut ids, width, 1, (120, 40, 180, 60));
        let ptype = land_types(&[1]);
        let inputs = LabelLawInputs {
            province_ids: &ids,
            width,
            height,
            province_to_state: &BTreeMap::from([(1u32, 10u32)]),
            owners: &BTreeMap::from([(10u32, "GER".to_string())]),
            impassable_states: &BTreeSet::new(),
            province_importance: &BTreeMap::new(),
            province_type: &ptype,
        };
        let placements = compute_country_label_placements(&inputs);
        assert_eq!(
            placements.len(),
            1,
            "one province split across two raster blobs is ONE atomic area"
        );
    }

    #[test]
    fn two_disconnected_provinces_same_owner_emit_two_areas() {
        let width = 200;
        let height = 100;
        let mut ids = vec![0u32; width * height];
        raster_rect(&mut ids, width, 1, (20, 40, 80, 60));
        raster_rect(&mut ids, width, 2, (120, 40, 180, 60));
        let ptype = land_types(&[1, 2]);
        let inputs = LabelLawInputs {
            province_ids: &ids,
            width,
            height,
            province_to_state: &BTreeMap::from([(1u32, 10u32), (2u32, 11u32)]),
            owners: &BTreeMap::from([(10u32, "GBR".to_string()), (11u32, "GBR".to_string())]),
            impassable_states: &BTreeSet::new(),
            province_importance: &BTreeMap::new(),
            province_type: &ptype,
        };
        let placements = compute_country_label_placements(&inputs);
        assert_eq!(
            placements.len(),
            2,
            "two disconnected provinces of one owner are two areas (per-island labels)"
        );
        assert!(placements.iter().all(|p| p.tag == "GBR"));
    }

    #[test]
    fn vertical_country_labels_vertically() {
        let width = 100;
        let height = 200;
        let mut ids = vec![0u32; width * height];
        raster_rect(&mut ids, width, 1, (40, 20, 60, 180));
        let ptype = land_types(&[1]);
        let inputs = LabelLawInputs {
            province_ids: &ids,
            width,
            height,
            province_to_state: &BTreeMap::from([(1u32, 10u32)]),
            owners: &BTreeMap::from([(10u32, "CHL".to_string())]),
            impassable_states: &BTreeSet::new(),
            province_importance: &BTreeMap::new(),
            province_type: &ptype,
        };
        let placements = compute_country_label_placements(&inputs);
        assert_eq!(placements.len(), 1);
        let p = &placements[0];
        assert!(
            (p.angle.abs() - std::f32::consts::FRAC_PI_2).abs() < 0.08,
            "angle {}",
            p.angle
        );
    }

    #[test]
    fn straight_country_has_no_bow_and_spans_its_axis() {
        let width = 240;
        let height = 160;
        let mut ids = vec![0u32; width * height];
        raster_rect(&mut ids, width, 1, (20, 60, 220, 100));
        let ptype = land_types(&[1]);
        let inputs = LabelLawInputs {
            province_ids: &ids,
            width,
            height,
            province_to_state: &BTreeMap::from([(1u32, 10u32)]),
            owners: &BTreeMap::from([(10u32, "STR".to_string())]),
            impassable_states: &BTreeSet::new(),
            province_importance: &BTreeMap::new(),
            province_type: &ptype,
        };
        let p = &compute_country_label_placements(&inputs)[0];
        assert_eq!(p.bow, 0.0, "engine baseline is straight, never bowed");
        assert!(
            (p.half_length - 100.0).abs() < 15.0,
            "half_length should track the engine owned segment (~100), got {}",
            p.half_length
        );
        assert!((p.anchor_x - 120.0).abs() < 4.0, "anchor_x {}", p.anchor_x);
        assert!(
            (p.anchor_y - 80.0).abs() <= LABEL_OFFSET_PX as f32 + 1.0,
            "anchor_y {}",
            p.anchor_y
        );
        assert!(
            p.angle.abs() < 0.05,
            "horizontal rectangle, angle {}",
            p.angle
        );
    }

    #[test]
    fn offset_trial_anchors_on_the_long_owned_run() {
        let width = 300;
        let height = 200;
        let mut ids = vec![0u32; width * height];
        raster_rect(&mut ids, width, 1, (20, 90, 280, 120));
        raster_rect(&mut ids, width, 1, (140, 30, 170, 90));
        let ptype = land_types(&[1]);
        let inputs = LabelLawInputs {
            province_ids: &ids,
            width,
            height,
            province_to_state: &BTreeMap::from([(1u32, 10u32)]),
            owners: &BTreeMap::from([(10u32, "OFF".to_string())]),
            impassable_states: &BTreeSet::new(),
            province_importance: &BTreeMap::new(),
            province_type: &ptype,
        };
        let p = &compute_country_label_placements(&inputs)[0];
        assert!(
            (p.anchor_x - 150.0).abs() < 30.0,
            "anchor_x should be on the horizontal bar centre, got {}",
            p.anchor_x
        );
        assert!(
            (p.anchor_y - 105.0).abs() < 15.0,
            "anchor_y should be on the horizontal bar, got {}",
            p.anchor_y
        );
        assert!(
            p.angle.abs() < 0.35,
            "angle should follow the horizontal bar, got {}",
            p.angle
        );
        assert!(p.bow == 0.0);
    }

    #[test]
    fn water_bridges_an_owned_run_within_a_component() {
        let width = 300;
        let height = 100;
        let mut ids = vec![0u32; width * height];
        raster_rect(&mut ids, width, 1, (20, 20, 280, 80));
        raster_rect(&mut ids, width, 2, (145, 20, 155, 45));
        let mut ptype = BTreeMap::new();
        ptype.insert(1u32, ProvinceKind::Land);
        ptype.insert(2u32, ProvinceKind::Water);
        let inputs = LabelLawInputs {
            province_ids: &ids,
            width,
            height,
            province_to_state: &BTreeMap::from([(1u32, 10u32)]),
            owners: &BTreeMap::from([(10u32, "BRI".to_string())]),
            impassable_states: &BTreeSet::new(),
            province_importance: &BTreeMap::new(),
            province_type: &ptype,
        };
        let ps = compute_country_label_placements(&inputs);
        assert_eq!(ps.len(), 1, "one owned block -> one label");
        let p = &ps[0];
        assert!(
            (p.anchor_x - 150.0).abs() < 20.0,
            "water inlet should bridge the owned run so the anchor centres on \
             the whole block, got anchor_x {}",
            p.anchor_x
        );
        assert!(
            p.angle.abs() < 0.1,
            "block is horizontal, angle {}",
            p.angle
        );
    }

    #[test]
    fn home_component_leads_and_colony_also_labelled_by_victory_points() {
        let width = 400;
        let height = 100;
        let mut ids = vec![0u32; width * height];
        raster_rect(&mut ids, width, 1, (10, 20, 90, 80));
        raster_rect(&mut ids, width, 2, (300, 10, 395, 95));
        let ptype = land_types(&[1, 2]);
        let inputs = LabelLawInputs {
            province_ids: &ids,
            width,
            height,
            province_to_state: &BTreeMap::from([(1u32, 10u32), (2u32, 20u32)]),
            owners: &BTreeMap::from([(10u32, "FRA".to_string()), (20u32, "FRA".to_string())]),
            impassable_states: &BTreeSet::new(),
            province_importance: &BTreeMap::from([(1u32, 50u64), (2u32, 1u64)]),
            province_type: &ptype,
        };
        let ps = compute_country_label_placements(&inputs);
        assert_eq!(ps.len(), 2, "home + colony both labelled");
        assert!(ps.iter().all(|p| p.tag == "FRA"));
        let home = &ps[0];
        assert!(home.is_home, "home component must sort first");
        assert!(
            home.anchor_x < 120.0,
            "home label captured by the larger colony: anchor_x {}",
            home.anchor_x
        );
        let colony = &ps[1];
        assert!(!colony.is_home);
        assert!(
            colony.anchor_x > 280.0,
            "colony label not on the colony block: anchor_x {}",
            colony.anchor_x
        );
        assert_ne!(home.component_id, colony.component_id);
    }

    #[test]
    fn peak_vp_home_leads_over_a_higher_summed_vp_colonial_sprawl() {
        let width = 400;
        let height = 100;
        let mut ids = vec![0u32; width * height];
        raster_rect(&mut ids, width, 1, (10, 30, 70, 70));
        raster_rect(&mut ids, width, 2, (250, 10, 395, 95));
        let ptype = land_types(&[1, 2]);
        let inputs = LabelLawInputs {
            province_ids: &ids,
            width,
            height,
            province_to_state: &BTreeMap::from([(1u32, 10u32), (2u32, 20u32)]),
            owners: &BTreeMap::from([(10u32, "ENG".to_string()), (20u32, "ENG".to_string())]),
            impassable_states: &BTreeSet::new(),
            province_importance: &BTreeMap::from([(1u32, 50u64), (2u32, 40u64)]),
            province_type: &ptype,
        };
        let ps = compute_country_label_placements(&inputs);
        assert_eq!(ps.len(), 2);
        let home = &ps[0];
        assert!(home.is_home);
        assert!(
            home.anchor_x < 120.0,
            "peak-VP capital lost the home slot to colonial sprawl: anchor_x {}",
            home.anchor_x
        );
    }

    #[test]
    fn distant_colony_gets_its_own_label_and_home_leads() {
        let width = 400;
        let height = 100;
        let mut ids = vec![0u32; width * height];
        raster_rect(&mut ids, width, 1, (10, 10, 110, 90));
        raster_rect(&mut ids, width, 2, (360, 40, 390, 70));
        let ptype = land_types(&[1, 2]);
        let inputs = LabelLawInputs {
            province_ids: &ids,
            width,
            height,
            province_to_state: &BTreeMap::from([(1u32, 10u32), (2u32, 20u32)]),
            owners: &BTreeMap::from([(10u32, "NLD".to_string()), (20u32, "NLD".to_string())]),
            impassable_states: &BTreeSet::new(),
            province_importance: &BTreeMap::from([(1u32, 40u64)]),
            province_type: &ptype,
        };
        let ps = compute_country_label_placements(&inputs);
        assert_eq!(ps.len(), 2, "home + colony both labelled");
        let home = &ps[0];
        assert!(home.is_home);
        assert!(
            home.anchor_x < 120.0,
            "anchor pulled off home mass: {}",
            home.anchor_x
        );
        assert!(!ps[1].is_home);
        assert!(ps[1].anchor_x > 340.0, "colony anchor {}", ps[1].anchor_x);
    }

    #[test]
    fn empty_cloud_fails_closed() {
        let width = 40;
        let height = 40;
        let ids = vec![0u32; width * height];
        let ptype = BTreeMap::new();
        let inputs = LabelLawInputs {
            province_ids: &ids,
            width,
            height,
            province_to_state: &BTreeMap::new(),
            owners: &BTreeMap::new(),
            impassable_states: &BTreeSet::new(),
            province_importance: &BTreeMap::new(),
            province_type: &ptype,
        };
        assert!(compute_country_label_placements(&inputs).is_empty());
    }

    #[test]
    fn single_cell_degenerate_cloud_fails_closed() {
        let width = 40;
        let height = 40;
        let mut ids = vec![0u32; width * height];
        ids[0] = 1;
        let ptype = land_types(&[1]);
        let inputs = LabelLawInputs {
            province_ids: &ids,
            width,
            height,
            province_to_state: &BTreeMap::from([(1u32, 10u32)]),
            owners: &BTreeMap::from([(10u32, "TNY".to_string())]),
            impassable_states: &BTreeSet::new(),
            province_importance: &BTreeMap::new(),
            province_type: &ptype,
        };
        assert!(compute_country_label_placements(&inputs).is_empty());
    }

    #[test]
    fn impassable_state_is_never_labelled() {
        let width = 100;
        let height = 100;
        let mut ids = vec![0u32; width * height];
        raster_rect(&mut ids, width, 1, (10, 10, 90, 90));
        let ptype = land_types(&[1]);
        let inputs = LabelLawInputs {
            province_ids: &ids,
            width,
            height,
            province_to_state: &BTreeMap::from([(1u32, 10u32)]),
            owners: &BTreeMap::from([(10u32, "GER".to_string())]),
            impassable_states: &BTreeSet::from([10u32]),
            province_importance: &BTreeMap::new(),
            province_type: &ptype,
        };
        assert!(
            compute_country_label_placements(&inputs).is_empty(),
            "an owner with ONLY impassable territory has no passable placement \
             cloud and must fail closed"
        );
    }

    #[test]
    fn impassable_interior_widens_breadth_but_not_placement() {
        let width = 400;
        let height = 200;
        let mut ids = vec![0u32; width * height];
        raster_rect(&mut ids, width, 1, (20, 8, 380, 32));
        raster_rect(&mut ids, width, 2, (20, 32, 380, 182));
        let ptype = land_types(&[1, 2]);
        let inputs = LabelLawInputs {
            province_ids: &ids,
            width,
            height,
            province_to_state: &BTreeMap::from([(1u32, 10u32), (2u32, 20u32)]),
            owners: &BTreeMap::from([(10u32, "ITA".to_string()), (20u32, "ITA".to_string())]),
            impassable_states: &BTreeSet::from([20u32]),
            province_importance: &BTreeMap::new(),
            province_type: &ptype,
        };
        let ps = compute_country_label_placements(&inputs);
        assert_eq!(ps.len(), 1, "one label on the passable strip");
        let p = &ps[0];
        assert!((p.anchor_y - 20.0).abs() < 6.0, "anchor_y {}", p.anchor_y);
        assert!(p.angle.abs() < 0.05, "angle {}", p.angle);
        assert!(
            (p.minor_extent - 24.0).abs() < 8.0,
            "placement minor is the strip's ({})",
            p.minor_extent
        );
        assert!(
            p.minor_extent_incl > 120.0,
            "inclusive breadth must include the impassable interior ({})",
            p.minor_extent_incl
        );
        assert!(
            p.breadth_span > 150.0,
            "perpendicular span must cross the interior ({})",
            p.breadth_span
        );
        assert!(
            (p.world_height - LABEL_HEIGHT_BREADTH_K * p.minor_extent_incl).abs() < 1e-3,
            "world_height {} vs K*incl {}",
            p.world_height,
            LABEL_HEIGHT_BREADTH_K * p.minor_extent_incl
        );
    }

    #[test]
    fn placements_are_deterministic_and_sorted() {
        let width = 200;
        let height = 100;
        let mut ids = vec![0u32; width * height];
        raster_rect(&mut ids, width, 1, (10, 10, 90, 90));
        raster_rect(&mut ids, width, 2, (110, 10, 190, 90));
        let ptype = land_types(&[1, 2]);
        let inputs = LabelLawInputs {
            province_ids: &ids,
            width,
            height,
            province_to_state: &BTreeMap::from([(1u32, 10u32), (2u32, 20u32)]),
            owners: &BTreeMap::from([(10u32, "SOV".to_string()), (20u32, "GER".to_string())]),
            impassable_states: &BTreeSet::new(),
            province_importance: &BTreeMap::new(),
            province_type: &ptype,
        };
        let a = compute_country_label_placements(&inputs);
        let b = compute_country_label_placements(&inputs);
        assert_eq!(a, b, "law must be deterministic");
        assert_eq!(a[0].tag, "GER");
        assert_eq!(a[1].tag, "SOV");
    }

    #[test]
    fn multi_region_power_gets_one_label_per_detached_component() {
        let width = 600;
        let height = 100;
        let mut ids = vec![0u32; width * height];
        raster_rect(&mut ids, width, 1, (10, 10, 140, 90));
        raster_rect(&mut ids, width, 2, (250, 20, 330, 80));
        raster_rect(&mut ids, width, 3, (450, 25, 520, 75));
        let ptype = land_types(&[1, 2, 3]);
        let inputs = LabelLawInputs {
            province_ids: &ids,
            width,
            height,
            province_to_state: &BTreeMap::from([(1u32, 10u32), (2u32, 20u32), (3u32, 30u32)]),
            owners: &BTreeMap::from([
                (10u32, "GBR".to_string()),
                (20u32, "GBR".to_string()),
                (30u32, "GBR".to_string()),
            ]),
            impassable_states: &BTreeSet::new(),
            province_importance: &BTreeMap::from([(1u32, 50u64)]),
            province_type: &ptype,
        };
        let ps = compute_country_label_placements(&inputs);
        assert_eq!(ps.len(), 3, "one label per detached component");
        assert!(ps.iter().all(|p| p.tag == "GBR"));
        assert_eq!(ps.iter().filter(|p| p.is_home).count(), 1);
        assert!(ps[0].is_home, "home component sorts first");
        assert!(ps[0].anchor_x < 150.0, "home on the home isle");
        let others: Vec<f32> = ps[1..].iter().map(|p| p.anchor_x).collect();
        assert!(others.iter().any(|&x| (240.0..340.0).contains(&x)));
        assert!(others.iter().any(|&x| (440.0..530.0).contains(&x)));
        let mut cids: Vec<u32> = ps.iter().map(|p| p.component_id).collect();
        cids.sort_unstable();
        cids.dedup();
        assert_eq!(cids.len(), 3, "distinct component ids");
    }

    #[test]
    fn nonhome_components_emit_without_app_declutter_fraction() {
        let width = 800;
        let height = 200;
        let mut ids = vec![0u32; width * height];
        raster_rect(&mut ids, width, 1, (10, 10, 200, 190));
        raster_rect(&mut ids, width, 2, (400, 80, 424, 104));
        raster_rect(&mut ids, width, 3, (600, 80, 624, 104));
        let ptype = land_types(&[1, 2, 3]);
        let inputs = LabelLawInputs {
            province_ids: &ids,
            width,
            height,
            province_to_state: &BTreeMap::from([(1u32, 10u32), (2u32, 20u32), (3u32, 30u32)]),
            owners: &BTreeMap::from([
                (10u32, "GBR".to_string()),
                (20u32, "GBR".to_string()),
                (30u32, "GBR".to_string()),
            ]),
            impassable_states: &BTreeSet::new(),
            province_importance: &BTreeMap::from([(1u32, 50u64), (2u32, 5u64)]),
            province_type: &ptype,
        };
        let ps = compute_country_label_placements(&inputs);
        assert_eq!(
            ps.len(),
            3,
            "all engine-resolved components should emit; no app-owned de-clutter fraction"
        );
        assert!(ps[0].is_home);
        let mut nonhome_xs: Vec<f32> = ps
            .iter()
            .filter(|p| !p.is_home)
            .map(|p| p.anchor_x)
            .collect();
        nonhome_xs.sort_by(f32::total_cmp);
        assert_eq!(nonhome_xs.len(), 2);
        assert!(
            (390.0..440.0).contains(&nonhome_xs[0]),
            "the VP enclave (x~412) should survive, got {:?}",
            nonhome_xs
        );
        assert!(
            (590.0..640.0).contains(&nonhome_xs[1]),
            "the no-VP enclave (x~612) should no longer be threshold-filtered, got {:?}",
            nonhome_xs
        );
    }

    #[test]
    fn nonhome_large_colony_labels_even_without_a_sampled_vp() {
        let width = 800;
        let height = 200;
        let mut ids = vec![0u32; width * height];
        raster_rect(&mut ids, width, 1, (10, 10, 210, 190));
        raster_rect(&mut ids, width, 2, (400, 30, 560, 170));
        let ptype = land_types(&[1, 2]);
        let inputs = LabelLawInputs {
            province_ids: &ids,
            width,
            height,
            province_to_state: &BTreeMap::from([(1u32, 10u32), (2u32, 20u32)]),
            owners: &BTreeMap::from([(10u32, "FRA".to_string()), (20u32, "FRA".to_string())]),
            impassable_states: &BTreeSet::new(),
            province_importance: &BTreeMap::from([(1u32, 50u64)]),
            province_type: &ptype,
        };
        let ps = compute_country_label_placements(&inputs);
        assert_eq!(ps.len(), 2, "a large no-VP colony still labels");
        assert!(!ps[1].is_home);
        assert!(ps[1].anchor_x > 380.0);
    }

    #[test]
    fn contiguous_controlled_territory_merges_into_one_label() {
        let width = 200;
        let height = 100;
        let mut ids = vec![0u32; width * height];
        raster_rect(&mut ids, width, 1, (10, 20, 100, 80));
        raster_rect(&mut ids, width, 2, (100, 20, 190, 80));
        let ptype = land_types(&[1, 2]);
        let inputs = LabelLawInputs {
            province_ids: &ids,
            width,
            height,
            province_to_state: &BTreeMap::from([(1u32, 10u32), (2u32, 20u32)]),
            owners: &BTreeMap::from([(10u32, "GER".to_string()), (20u32, "GER".to_string())]),
            impassable_states: &BTreeSet::new(),
            province_importance: &BTreeMap::new(),
            province_type: &ptype,
        };
        let ps = compute_country_label_placements(&inputs);
        assert_eq!(ps.len(), 1, "contiguous territory is one merged label");
        assert!(
            (ps[0].anchor_x - 100.0).abs() < 25.0,
            "merged anchor {}",
            ps[0].anchor_x
        );
    }

    fn geom_placement(
        anchor_x: f32,
        anchor_y: f32,
        angle: f32,
        half_length: f32,
        bow: f32,
        world_height: f32,
    ) -> CountryLabelPlacement {
        CountryLabelPlacement {
            tag: "T".to_string(),
            is_home: true,
            component_id: 1,
            anchor_x,
            anchor_y,
            angle,
            half_length,
            bow,
            world_height,
            pixel_count: 100,
            major_extent: 2.0 * half_length,
            minor_extent: 2.0 * world_height,
            minor_extent_incl: 2.0 * world_height,
            breadth_span: 2.0 * world_height,
            breadth_p2_98: 2.0 * world_height,
            curve: None,
        }
    }

    #[test]
    fn baseline_and_tangent_are_consistent() {
        let p = geom_placement(100.0, 50.0, 0.0, 40.0, 10.0, 20.0);
        let (cx, cy) = p.baseline_point(0.0);
        assert!((cx - 100.0).abs() < 1e-4);
        assert!((cy - 60.0).abs() < 1e-4, "cy {cy}");
        let (ex, ey) = p.baseline_point(1.0);
        assert!((ex - 140.0).abs() < 1e-4, "end x {ex}");
        assert!((ey - 50.0).abs() < 1e-4, "end y {ey}");
        assert!(p.tangent_angle(0.0).abs() < 1e-4);
    }

    #[test]
    fn principal_axis_of_horizontal_spread_is_zero() {
        let (angle, major, minor) = principal_axis(100.0, 1.0, 0.0);
        assert!(angle.abs() < 1e-9, "angle {angle}");
        assert!(major > minor);
    }

    #[test]
    fn arc_length_of_straight_baseline_is_the_chord() {
        let p = geom_placement(0.0, 0.0, 0.3, 50.0, 0.0, 20.0);
        assert!((p.arc_length() - 100.0).abs() < 1e-2, "{}", p.arc_length());
    }

    #[test]
    fn phantom_controls_extend_a_straight_medial_collinearly() {
        let first = (2.0, 0.0);
        let mean = (1.0, 0.0);
        let last = (0.0, 0.0);
        let c0 = phantom_control(first, mean, last).unwrap();
        let c4 = phantom_control_end(first, mean, last).unwrap();
        assert!((c0.0 - 3.0).abs() < 1e-12 && c0.1.abs() < 1e-12, "{c0:?}");
        assert!((c4.0 + 1.0).abs() < 1e-12 && c4.1.abs() < 1e-12, "{c4:?}");
    }

    #[test]
    fn phantom_control_continues_the_turn() {
        let first = (0.0, 1.0);
        let mean = (0.0, 0.0);
        let last = (1.0, 0.0);
        let c0 = phantom_control(first, mean, last).unwrap();
        assert!(
            (c0.0 - 1.0).abs() < 1e-12 && (c0.1 - 1.0).abs() < 1e-12,
            "{c0:?}"
        );
        assert!(phantom_control((1.0, 1.0), (1.0, 1.0), (0.0, 0.0)).is_none());
        assert!(phantom_control_end((1.0, 1.0), (0.0, 0.0), (0.0, 0.0)).is_none());
    }

    #[test]
    fn engine_curve_interpolates_first_mean_last() {
        let curve = EngineCurve {
            pts: [
                (-10.0, 2.0),
                (0.0, 0.0),
                (10.0, 4.0),
                (20.0, 0.0),
                (30.0, 2.0),
            ],
        };
        let p0 = curve.point_at(0.0);
        let pm = curve.point_at(0.5);
        let p1 = curve.point_at(1.0);
        assert!(p0.0.abs() < 1e-4 && p0.1.abs() < 1e-4, "{p0:?}");
        assert!(
            (pm.0 - 10.0).abs() < 1e-4 && (pm.1 - 4.0).abs() < 1e-4,
            "{pm:?}"
        );
        assert!((p1.0 - 20.0).abs() < 1e-4 && p1.1.abs() < 1e-4, "{p1:?}");
        let line = EngineCurve {
            pts: [
                (-10.0, 0.0),
                (0.0, 0.0),
                (10.0, 0.0),
                (20.0, 0.0),
                (30.0, 0.0),
            ],
        };
        for i in 0..=10 {
            let t = i as f32 / 10.0;
            let (x, y) = line.point_at(t);
            assert!(y.abs() < 1e-4, "t {t} y {y}");
            assert!((x - 20.0 * t).abs() < 1e-3, "t {t} x {x}");
        }
        let (tx, ty) = line.tangent_at(0.3);
        assert!((tx - 1.0).abs() < 1e-4 && ty.abs() < 1e-4);
    }

    #[test]
    fn curved_placement_baseline_follows_the_engine_curve() {
        let mut p = geom_placement(10.0, 4.0, 0.1, 10.0, 0.0, 5.0);
        p.curve = Some(EngineCurve {
            pts: [
                (-10.0, 2.0),
                (0.0, 0.0),
                (10.0, 4.0),
                (20.0, 0.0),
                (30.0, 2.0),
            ],
        });
        let (x0, y0) = p.baseline_point(-1.0);
        let (xm, ym) = p.baseline_point(0.0);
        let (x1, y1) = p.baseline_point(1.0);
        assert!(x0.abs() < 1e-4 && y0.abs() < 1e-4);
        assert!((xm - 10.0).abs() < 1e-4 && (ym - 4.0).abs() < 1e-4);
        assert!((x1 - 20.0).abs() < 1e-4 && y1.abs() < 1e-4);
        assert!(p.arc_length() > 20.0);
    }

    #[test]
    fn n02_measured_parameters_use_full_euclidean_station_spacing() {
        let glyph = |x: f32, y: f32| MapLabelGlyphLayoutQuad {
            x0: x - 1.0,
            y0: y,
            x1: x + 1.0,
            y1: y + 2.0,
            u0: 0.0,
            v0: 0.0,
            u1: 1.0,
            v1: 1.0,
            measured_t: 0.0,
        };
        let measured =
            mapname_measured_glyph_parameters(&[glyph(0.0, 0.0), glyph(3.0, 4.0), glyph(9.0, 4.0)])
                .expect("valid stations");
        assert_eq!(measured[0].to_bits(), 0.0f32.to_bits());
        assert!((measured[1] - 5.0 / 11.0).abs() < 1e-6);
        assert_eq!(measured[2].to_bits(), 1.0f32.to_bits());
        assert!(mapname_measured_glyph_parameters(&[glyph(2.0, 3.0), glyph(2.0, 3.0),]).is_none());
    }

    #[test]
    fn n02_mapname_emission_uses_direct_t_and_one_rigid_frame() {
        let mut p = geom_placement(10.0, 4.0, 0.0, 10.0, 0.0, 20.0);
        p.pixel_count = 4000;
        p.curve = Some(EngineCurve {
            pts: [
                (-10.0, 2.0),
                (0.0, 0.0),
                (10.0, 8.0),
                (20.0, 0.0),
                (30.0, 2.0),
            ],
        });
        let glyph = MapLabelGlyphLayoutQuad {
            x0: 20.0,
            y0: 10.0,
            x1: 40.0,
            y1: 30.0,
            u0: 0.1,
            v0: 0.2,
            u1: 0.3,
            v1: 0.4,
            measured_t: 0.3,
        };
        let vertices = build_mapname_vertices(&p, &[glyph], 100.0, 100.0, 40.0, 200.0, 9.5);
        assert_eq!(vertices.len(), 6);

        let expected = p.baseline_point(-0.4);
        let station = (
            (vertices[0].x + vertices[1].x) * 0.5,
            200.0 - (vertices[0].z + vertices[1].z) * 0.5,
        );
        assert!(
            (station.0 - expected.0).abs() < 1e-5,
            "{station:?} != {expected:?}"
        );
        assert!(
            (station.1 - expected.1).abs() < 1e-5,
            "{station:?} != {expected:?}"
        );

        let map = |index: usize| (vertices[index].x, 200.0 - vertices[index].z);
        let (tl, tr, bl, br) = (map(0), map(1), map(2), map(4));
        assert!((tl.0 + br.0 - tr.0 - bl.0).abs() < 1e-5);
        assert!((tl.1 + br.1 - tr.1 - bl.1).abs() < 1e-5);

        let retired = p.baseline_point(p.t_at_arc_fraction(0.3));
        assert!((retired.0 - expected.0).hypot(retired.1 - expected.1) > 0.05);
    }

    #[test]
    fn mapname_height_samples_each_transformed_glyph_corner() {
        let placement = geom_placement(40.0, 30.0, 0.0, 30.0, 0.0, 60.0);
        let glyph = MapLabelGlyphLayoutQuad {
            x0: 0.0,
            y0: 0.0,
            x1: 20.0,
            y1: 30.0,
            u0: 0.0,
            v0: 0.0,
            u1: 1.0,
            v1: 1.0,
            measured_t: 0.5,
        };
        let mut samples = 0usize;
        let vertices = build_mapname_vertices_with_height(
            &placement,
            &[glyph],
            20.0,
            20.0,
            30.0,
            100.0,
            |x, z| {
                samples += 1;
                Ok(100.0 + x + z)
            },
        )
        .unwrap();
        assert_eq!(samples, 4);
        assert_eq!(vertices.len(), 6);
        for vertex in vertices {
            assert!((vertex.y - (100.0 + vertex.x + vertex.z)).abs() < 1e-5);
        }
    }

    #[test]
    fn label_draw_visible_applies_cutoff_and_frustum() {
        let mut vp = [0.0f32; 16];
        vp[0] = 1.0;
        vp[5] = 1.0;
        vp[10] = 1.0;
        vp[15] = 1.0;
        let inside = [[0.0f32, 0.0, 0.0]];
        let outside = [[5.0f32, 0.0, 0.0]];
        assert!(!label_draw_visible(&inside, &vp, 200.0, 260.0));
        assert!(label_draw_visible(&inside, &vp, 300.0, 260.0));
        assert!(!label_draw_visible(&outside, &vp, 300.0, 260.0));
        let straddle = [[5.0f32, 0.0, 0.0], [0.5, 0.0, 0.0]];
        assert!(label_draw_visible(&straddle, &vp, 300.0, 260.0));
        let mut flip = vp;
        flip[15] = -1.0;
        assert!(!label_draw_visible(&inside, &flip, 300.0, 260.0));
    }

    #[test]
    fn owned_trim_station_prefers_the_near_run_with_the_engine_boost() {
        let width = 40;
        let height = 3;
        let mut ids = vec![0u32; width * height];
        raster_rect(&mut ids, width, 1, (2, 1, 12, 2));
        raster_rect(&mut ids, width, 2, (20, 1, 28, 2));
        let ptype = land_types(&[1, 2]);
        let inputs = LabelLawInputs {
            province_ids: &ids,
            width,
            height,
            province_to_state: &BTreeMap::from([(1u32, 10u32), (2u32, 11u32)]),
            owners: &BTreeMap::from([(10u32, "AAA".to_string()), (11u32, "AAA".to_string())]),
            impassable_states: &BTreeSet::new(),
            province_importance: &BTreeMap::new(),
            province_type: &ptype,
        };
        let grid = OwnerGrid::sample(&inputs);
        let pts: Vec<(i64, i64)> = (0..width as i64).map(|x| (x, 1)).collect();
        let (a, b) = grid
            .owned_trim_station(&pts, "AAA", (24, 1))
            .expect("run resolves");
        assert_eq!((a, b), ((20, 1), (27, 1)), "near run B wins via 1.4 boost");
        let (a, b) = grid
            .owned_trim_station(&pts, "AAA", (3, 1))
            .expect("run resolves");
        assert_eq!((a, b), ((2, 1), (11, 1)), "near+long run A wins");
    }

    #[test]
    fn medial_curve_on_a_rectangle_stays_on_the_straight_baseline() {
        let width = 200;
        let height = 100;
        let mut ids = vec![0u32; width * height];
        raster_rect(&mut ids, width, 1, (20, 40, 180, 60));
        let ptype = land_types(&[1]);
        let inputs = LabelLawInputs {
            province_ids: &ids,
            width,
            height,
            province_to_state: &BTreeMap::from([(1u32, 10u32)]),
            owners: &BTreeMap::from([(10u32, "GER".to_string())]),
            impassable_states: &BTreeSet::new(),
            province_importance: &BTreeMap::new(),
            province_type: &ptype,
        };
        let metrics = LabelTextMetrics {
            advances: vec![
                (52.0, false),
                (43.0, false),
                (48.0, false),
                (75.0, false),
                (46.0, false),
                (48.0, false),
                (46.0, false),
            ],
            line_height_px: 84.0,
            space_advance_px: 20.0,
            straight_layout_stations: vec![
                MapLabelStraightLayoutStation {
                    slot_index: 0,
                    x: 26.0,
                    y: 42.0,
                },
                MapLabelStraightLayoutStation {
                    slot_index: 1,
                    x: 73.5,
                    y: 42.0,
                },
                MapLabelStraightLayoutStation {
                    slot_index: 2,
                    x: 119.0,
                    y: 42.0,
                },
                MapLabelStraightLayoutStation {
                    slot_index: 3,
                    x: 180.5,
                    y: 42.0,
                },
                MapLabelStraightLayoutStation {
                    slot_index: 4,
                    x: 241.0,
                    y: 42.0,
                },
                MapLabelStraightLayoutStation {
                    slot_index: 5,
                    x: 288.0,
                    y: 42.0,
                },
                MapLabelStraightLayoutStation {
                    slot_index: 6,
                    x: 335.0,
                    y: 42.0,
                },
            ],
        };
        let texts = BTreeMap::from([("GER".to_string(), metrics.clone())]);
        let straight = compute_country_label_placements(&inputs);
        let curved = compute_country_label_placements_with_text(&inputs, &texts);
        assert_eq!(straight.len(), 1);
        assert_eq!(curved.len(), 1);
        let s = &straight[0];
        let c = &curved[0];
        assert!(s.curve.is_none());
        let curve = c.curve.as_ref().expect("curve builds with metrics");
        for (i, (px, py)) in curve.pts.iter().enumerate() {
            assert!(
                (py - s.anchor_y as f64).abs() < 4.0,
                "control {i} at ({px:.1},{py:.1}) far from straight baseline y {}",
                s.anchor_y
            );
        }
        let d = (c.anchor_x - s.anchor_x).hypot(c.anchor_y - s.anchor_y);
        assert!(d < 4.0, "anchor moved {d} px on a uniform rectangle");

        let mut shifted_metrics = metrics.clone();
        for station in &mut shifted_metrics.straight_layout_stations {
            station.y = 20.0;
        }
        let shifted = compute_country_label_placements_with_text(
            &inputs,
            &BTreeMap::from([("GER".to_string(), shifted_metrics)]),
        );
        let shifted_curve = shifted[0].curve.as_ref().expect("shifted exact stations");
        assert!((shifted_curve.pts[1].1 - curve.pts[1].1).abs() > 1.0);
        assert!((shifted_curve.pts[3].1 - curve.pts[3].1).abs() > 1.0);

        let mut missing_stations = metrics;
        missing_stations.straight_layout_stations.clear();
        let fail_closed = compute_country_label_placements_with_text(
            &inputs,
            &BTreeMap::from([("GER".to_string(), missing_stations)]),
        );
        assert!(fail_closed[0].curve.is_none());
    }

    #[test]
    fn trunc_toward_zero_matches_engine_cvttss2si() {
        assert_eq!(trunc_toward_zero(0.5), 0);
        assert_eq!(trunc_toward_zero(1.5), 1);
        assert_eq!(trunc_toward_zero(2.9), 2);
        assert_eq!(trunc_toward_zero(-0.5), 0);
        assert_eq!(trunc_toward_zero(-1.5), -1);
        assert_eq!(trunc_toward_zero(-2.9), -2);
        assert_eq!(trunc_toward_zero(1723.8), 1723);
        assert_eq!(trunc_toward_zero(-1723.8), -1723);
    }

    #[test]
    fn dda_walks_a_diagonal_line_endpoints_inclusive() {
        let pts = dda(0, 0, 10, 5);
        assert_eq!(pts.first().copied(), Some((0, 0)));
        assert_eq!(pts.last().copied(), Some((10, 5)));
        assert_eq!(pts.len(), 11, "x major: 11 points from x=0..=10");
        let pts = dda(0, 0, 5, 10);
        assert_eq!(pts.first().copied(), Some((0, 0)));
        assert_eq!(pts.last().copied(), Some((5, 10)));
        assert_eq!(pts.len(), 11);
    }

    #[test]
    fn t_at_arc_fraction_advances_uniformly_on_a_straight_baseline() {
        let p = geom_placement(0.0, 0.0, 0.4, 60.0, 0.0, 20.0);
        let total = p.arc_length();
        let seg = |f0: f32, f1: f32| -> f32 {
            let (x0, y0) = p.baseline_point(p.t_at_arc_fraction(f0));
            let (x1, y1) = p.baseline_point(p.t_at_arc_fraction(f1));
            ((x1 - x0).powi(2) + (y1 - y0).powi(2)).sqrt()
        };
        for (a, b) in [(0.0, 0.25), (0.25, 0.5), (0.5, 0.75), (0.75, 1.0)] {
            let len = seg(a, b);
            assert!(
                (len - total / 4.0).abs() < total * 0.03,
                "quarter [{a},{b}] len {len} vs {}",
                total / 4.0
            );
        }
        assert!((p.t_at_arc_fraction(0.0) + 1.0).abs() < 1e-3);
        assert!((p.t_at_arc_fraction(1.0) - 1.0).abs() < 1e-3);
    }
}
