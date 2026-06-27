//! Deterministic native-map overlay planning.
//!
//! This module is deliberately GPU-free. It turns static map definitions plus
//! measured snapshot domains into screen-space labels and vector lines. Missing
//! geometry stays missing and is counted in diagnostics; no caller gets a guessed
//! trade route, army route, or label anchor from this layer.

use std::collections::{BTreeMap, BTreeSet};
use std::path::Path;

use serde_json::Value;

use crate::map::ProvinceRaster;

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct MapPoint {
    pub x: f32,
    pub y: f32,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ScreenPoint {
    pub x: f32,
    pub y: f32,
}

#[derive(Debug, Clone, PartialEq)]
pub struct StateLabelSeed {
    pub state_id: u32,
    pub name_key: String,
    pub anchor: MapPoint,
    pub province_count: usize,
    pub vp_total: f32,
}

#[derive(Debug, Clone, PartialEq)]
pub struct RegionLabelSeed {
    pub region_id: u32,
    pub name_key: String,
    pub anchor: MapPoint,
    pub province_count: usize,
}

#[derive(Debug, Clone, PartialEq)]
pub struct FrontLineSeed {
    pub province_a: u32,
    pub province_b: u32,
    pub start: MapPoint,
    pub end: MapPoint,
}

#[derive(Debug, Clone, Default, PartialEq)]
pub struct OverlayStatic {
    pub province_anchors: BTreeMap<u32, MapPoint>,
    pub state_anchors: BTreeMap<u32, MapPoint>,
    pub region_anchors: BTreeMap<u32, MapPoint>,
    pub province_states: BTreeMap<u32, u32>,
    pub front_edges: Vec<FrontLineSeed>,
    pub states: Vec<StateLabelSeed>,
    pub regions: Vec<RegionLabelSeed>,
}

impl OverlayStatic {
    pub fn load(map_dir: &Path, raster: &ProvinceRaster) -> Result<OverlayStatic, String> {
        let province_anchors = province_centroids(raster);
        let mut state_anchors = BTreeMap::new();
        let mut province_states = BTreeMap::new();
        let mut states = Vec::new();

        let state_meta_path = map_dir.join("state_meta.json");
        let state_txt = std::fs::read_to_string(&state_meta_path)
            .map_err(|e| format!("{}: {e}", state_meta_path.display()))?;
        let state_raw: BTreeMap<String, Value> = serde_json::from_str(&state_txt)
            .map_err(|e| format!("{}: {e}", state_meta_path.display()))?;
        for (sid_s, meta) in state_raw {
            let Some(state_id) = parse_u32_str(&sid_s) else {
                continue;
            };
            let name_key = meta
                .get("name_key")
                .and_then(Value::as_str)
                .unwrap_or("")
                .to_string();
            let provinces = u32_array(meta.get("provinces"));
            for pid in &provinces {
                province_states.insert(*pid, state_id);
            }
            let Some(anchor) = average_anchor(&provinces, &province_anchors) else {
                continue;
            };
            let vp_total = meta.get("vp_total").and_then(Value::as_f64).unwrap_or(0.0) as f32;
            state_anchors.insert(state_id, anchor);
            states.push(StateLabelSeed {
                state_id,
                name_key,
                anchor,
                province_count: provinces.len(),
                vp_total,
            });
        }
        states.sort_by_key(|s| s.state_id);

        let region_meta_path = map_dir.join("strategic_region_meta.json");
        let region_join_path = map_dir.join("strategic_region_to_provinces.json");
        let region_txt = std::fs::read_to_string(&region_meta_path)
            .map_err(|e| format!("{}: {e}", region_meta_path.display()))?;
        let region_join_txt = std::fs::read_to_string(&region_join_path)
            .map_err(|e| format!("{}: {e}", region_join_path.display()))?;
        let region_raw: BTreeMap<String, Value> = serde_json::from_str(&region_txt)
            .map_err(|e| format!("{}: {e}", region_meta_path.display()))?;
        let region_join: BTreeMap<String, Value> = serde_json::from_str(&region_join_txt)
            .map_err(|e| format!("{}: {e}", region_join_path.display()))?;

        let mut region_anchors = BTreeMap::new();
        let mut regions = Vec::new();
        for (rid_s, meta) in region_raw {
            let Some(region_id) = parse_u32_str(&rid_s) else {
                continue;
            };
            let provinces = u32_array(region_join.get(&rid_s));
            let Some(anchor) = average_anchor(&provinces, &province_anchors) else {
                continue;
            };
            let name_key = meta
                .get("name")
                .and_then(Value::as_str)
                .unwrap_or("")
                .to_string();
            region_anchors.insert(region_id, anchor);
            regions.push(RegionLabelSeed {
                region_id,
                name_key,
                anchor,
                province_count: provinces.len(),
            });
        }
        regions.sort_by_key(|r| r.region_id);

        Ok(OverlayStatic {
            province_anchors,
            state_anchors,
            region_anchors,
            province_states,
            front_edges: province_front_edges(raster),
            states,
            regions,
        })
    }

    pub fn for_test(
        province_anchors: BTreeMap<u32, MapPoint>,
        states: Vec<StateLabelSeed>,
        regions: Vec<RegionLabelSeed>,
    ) -> OverlayStatic {
        let state_anchors = states.iter().map(|s| (s.state_id, s.anchor)).collect();
        let region_anchors = regions.iter().map(|r| (r.region_id, r.anchor)).collect();
        OverlayStatic {
            province_anchors,
            state_anchors,
            region_anchors,
            province_states: BTreeMap::new(),
            front_edges: Vec::new(),
            states,
            regions,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct OverlayProjection {
    pub offset: [f32; 2],
    pub scale: f32,
    pub screen_w: f32,
    pub screen_h: f32,
    pub raster_w: f32,
    pub raster_h: f32,
}

impl OverlayProjection {
    pub fn project(&self, p: MapPoint) -> Option<ScreenPoint> {
        if !(p.x.is_finite()
            && p.y.is_finite()
            && self.offset[0].is_finite()
            && self.offset[1].is_finite()
            && self.scale.is_finite()
            && self.screen_w.is_finite()
            && self.screen_h.is_finite()
            && self.raster_w.is_finite()
            && self.raster_h.is_finite())
        {
            return None;
        }
        if self.screen_w <= 0.0
            || self.screen_h <= 0.0
            || self.raster_w <= 0.0
            || self.raster_h <= 0.0
            || self.scale <= 0.0
        {
            return None;
        }
        let map_u = p.x / self.raster_w;
        let map_v = p.y / self.raster_h;
        let out_u = (map_u - self.offset[0]) * self.scale;
        let out_v = (map_v - self.offset[1]) * self.scale;
        let sx = out_u * self.screen_w;
        let sy = out_v * self.screen_h;
        if sx.is_finite() && sy.is_finite() {
            Some(ScreenPoint { x: sx, y: sy })
        } else {
            None
        }
    }

    fn visible_point(&self, p: ScreenPoint, margin: f32) -> bool {
        p.x >= -margin
            && p.y >= -margin
            && p.x <= self.screen_w + margin
            && p.y <= self.screen_h + margin
    }
}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub enum OverlayLabelKind {
    StrategicRegion,
    State,
    Country,
}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub enum OverlayLineKind {
    FrontLine,
    TradeRoute,
    Movement,
    BattlePlan,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum OverlayGeometryFidelity {
    Exact,
    Derived,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum OverlayLinePattern {
    Solid,
    Dashed,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct OverlayLineStyle {
    pub pattern: OverlayLinePattern,
    pub dash_px: f32,
    pub gap_px: f32,
}

impl OverlayLineStyle {
    pub const fn solid() -> OverlayLineStyle {
        OverlayLineStyle {
            pattern: OverlayLinePattern::Solid,
            dash_px: 0.0,
            gap_px: 0.0,
        }
    }

    pub const fn dashed(dash_px: f32, gap_px: f32) -> OverlayLineStyle {
        OverlayLineStyle {
            pattern: OverlayLinePattern::Dashed,
            dash_px,
            gap_px,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub enum OverlayMarkerKind {
    UnavailableMovement,
    UnavailableBattlePlan,
    UnavailableTradeRoute,
    UnavailableFrontLine,
}

#[derive(Debug, Clone, PartialEq)]
pub struct OverlayLabel {
    pub kind: OverlayLabelKind,
    pub id: String,
    pub text: String,
    pub x: f32,
    pub y: f32,
    pub tint: [f32; 4],
    pub priority: i32,
}

#[derive(Debug, Clone, PartialEq)]
pub struct OverlayLine {
    pub kind: OverlayLineKind,
    pub id: String,
    pub points: Vec<ScreenPoint>,
    pub tint: [f32; 4],
    pub width: f32,
    pub arrow: bool,
    pub fidelity: OverlayGeometryFidelity,
    pub style: OverlayLineStyle,
}

#[derive(Debug, Clone, PartialEq)]
pub struct OverlayMarker {
    pub kind: OverlayMarkerKind,
    pub id: String,
    pub text: String,
    pub x: f32,
    pub y: f32,
    pub tint: [f32; 4],
    pub priority: i32,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct OverlayDiagnostics {
    pub country_labels: u32,
    pub state_labels: u32,
    pub strategic_region_labels: u32,
    pub movement_arrows: u32,
    pub battle_plan_arrows: u32,
    pub trade_route_lines: u32,
    pub front_line_lines: u32,
    pub unavailable_arrows: u32,
    pub unavailable_routes: u32,
    pub unavailable_front_lines: u32,
    pub culled_labels: u32,
    pub culled_lines: u32,
}

#[derive(Debug, Clone, Default, PartialEq)]
pub struct OverlayPlan {
    pub labels: Vec<OverlayLabel>,
    pub lines: Vec<OverlayLine>,
    pub markers: Vec<OverlayMarker>,
    pub diagnostics: OverlayDiagnostics,
}

impl OverlayPlan {
    pub fn digest(&self) -> String {
        let mut out = format!(
            "lbl={}/{}/{} line={}/{}/{}/{} mark={} unavail={}/{}/{} cull={}/{}",
            self.diagnostics.country_labels,
            self.diagnostics.state_labels,
            self.diagnostics.strategic_region_labels,
            self.diagnostics.movement_arrows,
            self.diagnostics.battle_plan_arrows,
            self.diagnostics.trade_route_lines,
            self.diagnostics.front_line_lines,
            self.markers.len(),
            self.diagnostics.unavailable_arrows,
            self.diagnostics.unavailable_routes,
            self.diagnostics.unavailable_front_lines,
            self.diagnostics.culled_labels,
            self.diagnostics.culled_lines
        );
        for line in &self.lines {
            out.push_str("|L:");
            out.push_str(&format!(
                "{:?}:{:?}:{:?}:{}/{}:{}:",
                line.kind,
                line.fidelity,
                line.style.pattern,
                q(line.style.dash_px),
                q(line.style.gap_px),
                line.id
            ));
            for p in &line.points {
                out.push_str(&format!("{},{};", q(p.x), q(p.y)));
            }
        }
        for marker in &self.markers {
            out.push_str("|M:");
            out.push_str(&format!(
                "{:?}:{}:{}:{},{}",
                marker.kind,
                marker.id,
                marker.text,
                q(marker.x),
                q(marker.y)
            ));
        }
        for label in &self.labels {
            out.push_str("|T:");
            out.push_str(&format!(
                "{:?}:{}:{}:{},{}",
                label.kind,
                label.id,
                label.text,
                q(label.x),
                q(label.y)
            ));
        }
        out
    }
}

#[derive(Debug, Clone, Default)]
pub struct OverlayNames {
    pub countries: BTreeMap<String, String>,
    pub states: BTreeMap<u32, String>,
    pub strategic_regions: BTreeMap<u32, String>,
}

#[derive(Debug, Clone, Copy)]
pub struct OverlayOptions {
    pub show_country_labels: bool,
    pub show_state_labels: bool,
    pub show_strategic_region_labels: bool,
    pub show_movement_arrows: bool,
    pub show_battle_plan_arrows: bool,
    pub show_trade_routes: bool,
    pub show_front_lines: bool,
    pub max_country_labels: usize,
    pub max_state_labels: usize,
    pub max_strategic_region_labels: usize,
    pub max_lines: usize,
    pub cull_margin_px: f32,
    pub max_segment_px: f32,
}

impl Default for OverlayOptions {
    fn default() -> Self {
        OverlayOptions {
            show_country_labels: true,
            show_state_labels: true,
            show_strategic_region_labels: true,
            show_movement_arrows: true,
            show_battle_plan_arrows: true,
            show_trade_routes: true,
            show_front_lines: false,
            max_country_labels: 80,
            max_state_labels: 160,
            max_strategic_region_labels: 96,
            max_lines: 256,
            cull_margin_px: 72.0,
            max_segment_px: 3200.0,
        }
    }
}

pub struct OverlayInputs<'a> {
    pub states: Option<&'a Value>,
    pub map: Option<&'a Value>,
    pub divisions: Option<&'a Value>,
    pub order_of_battle: Option<&'a Value>,
    pub diplomacy_world: Option<&'a Value>,
}

pub fn build_overlay_plan(
    static_data: &OverlayStatic,
    inputs: OverlayInputs<'_>,
    names: &OverlayNames,
    projection: OverlayProjection,
    options: OverlayOptions,
) -> OverlayPlan {
    let mut plan = OverlayPlan::default();
    let division_anchors = division_anchors_and_moves(
        static_data,
        inputs.divisions,
        &projection,
        &options,
        &mut plan,
    );
    let army_anchors = army_anchors(inputs.order_of_battle, &division_anchors);

    if options.show_front_lines {
        add_front_line_lines(
            static_data,
            inputs.map,
            inputs.states,
            &projection,
            &options,
            &mut plan,
        );
    }
    if options.show_battle_plan_arrows {
        add_oob_order_lines(
            static_data,
            inputs.order_of_battle,
            &army_anchors,
            &projection,
            &options,
            &mut plan,
        );
    }
    if options.show_trade_routes {
        add_trade_route_lines(
            static_data,
            inputs.diplomacy_world,
            inputs.states,
            &projection,
            &options,
            &mut plan,
        );
    }

    add_labels(
        static_data,
        inputs.states,
        names,
        &projection,
        options,
        &mut plan,
    );
    plan.lines.sort_by_key(line_sort_key);
    plan.markers.sort_by_key(marker_sort_key);
    plan.labels.sort_by_key(label_sort_key);
    plan
}

fn province_centroids(raster: &ProvinceRaster) -> BTreeMap<u32, MapPoint> {
    let mut sums: BTreeMap<u32, (f64, f64, u64)> = BTreeMap::new();
    for y in 0..raster.height {
        for x in 0..raster.width {
            let pid = raster.ids[(y as usize) * (raster.width as usize) + x as usize];
            if pid == 0 {
                continue;
            }
            let e = sums.entry(pid).or_insert((0.0, 0.0, 0));
            e.0 += x as f64 + 0.5;
            e.1 += y as f64 + 0.5;
            e.2 += 1;
        }
    }
    sums.into_iter()
        .filter_map(|(pid, (sx, sy, n))| {
            if n == 0 {
                None
            } else {
                Some((
                    pid,
                    MapPoint {
                        x: (sx / n as f64) as f32,
                        y: (sy / n as f64) as f32,
                    },
                ))
            }
        })
        .collect()
}

#[derive(Default)]
struct EdgeAgg {
    n: u32,
    sx: f64,
    sy: f64,
    min_x: f32,
    max_x: f32,
    min_y: f32,
    max_y: f32,
}

impl EdgeAgg {
    fn push(&mut self, p: MapPoint) {
        if self.n == 0 {
            self.min_x = p.x;
            self.max_x = p.x;
            self.min_y = p.y;
            self.max_y = p.y;
        } else {
            self.min_x = self.min_x.min(p.x);
            self.max_x = self.max_x.max(p.x);
            self.min_y = self.min_y.min(p.y);
            self.max_y = self.max_y.max(p.y);
        }
        self.n += 1;
        self.sx += p.x as f64;
        self.sy += p.y as f64;
    }
}

fn province_front_edges(raster: &ProvinceRaster) -> Vec<FrontLineSeed> {
    let w = raster.width as usize;
    let h = raster.height as usize;
    let mut edges: BTreeMap<(u32, u32), EdgeAgg> = BTreeMap::new();
    for y in 0..h {
        for x in 0..w {
            let pid = raster.ids[y * w + x];
            if pid == 0 {
                continue;
            }
            if x + 1 < w {
                add_boundary_point(
                    &mut edges,
                    pid,
                    raster.ids[y * w + x + 1],
                    MapPoint {
                        x: x as f32 + 1.0,
                        y: y as f32 + 0.5,
                    },
                );
            }
            if y + 1 < h {
                add_boundary_point(
                    &mut edges,
                    pid,
                    raster.ids[(y + 1) * w + x],
                    MapPoint {
                        x: x as f32 + 0.5,
                        y: y as f32 + 1.0,
                    },
                );
            }
        }
    }

    edges
        .into_iter()
        .filter_map(|((province_a, province_b), agg)| {
            if agg.n == 0 {
                return None;
            }
            let cx = (agg.sx / agg.n as f64) as f32;
            let cy = (agg.sy / agg.n as f64) as f32;
            let dx = agg.max_x - agg.min_x;
            let dy = agg.max_y - agg.min_y;
            let (start, end) = if dx >= dy {
                (
                    MapPoint {
                        x: agg.min_x,
                        y: cy,
                    },
                    MapPoint {
                        x: agg.max_x,
                        y: cy,
                    },
                )
            } else {
                (
                    MapPoint {
                        x: cx,
                        y: agg.min_y,
                    },
                    MapPoint {
                        x: cx,
                        y: agg.max_y,
                    },
                )
            };
            if same_map_point(start, end) {
                return None;
            }
            Some(FrontLineSeed {
                province_a,
                province_b,
                start,
                end,
            })
        })
        .collect()
}

fn add_boundary_point(edges: &mut BTreeMap<(u32, u32), EdgeAgg>, a: u32, b: u32, p: MapPoint) {
    if a == 0 || b == 0 || a == b {
        return;
    }
    let key = if a < b { (a, b) } else { (b, a) };
    edges.entry(key).or_default().push(p);
}

fn average_anchor(provinces: &[u32], anchors: &BTreeMap<u32, MapPoint>) -> Option<MapPoint> {
    let mut sx = 0.0_f64;
    let mut sy = 0.0_f64;
    let mut n = 0_u32;
    for pid in provinces {
        if let Some(p) = anchors.get(pid) {
            sx += p.x as f64;
            sy += p.y as f64;
            n += 1;
        }
    }
    (n > 0).then_some(MapPoint {
        x: (sx / n as f64) as f32,
        y: (sy / n as f64) as f32,
    })
}

fn add_labels(
    static_data: &OverlayStatic,
    states: Option<&Value>,
    names: &OverlayNames,
    projection: &OverlayProjection,
    options: OverlayOptions,
    plan: &mut OverlayPlan,
) {
    let mut candidates = Vec::new();

    if options.show_country_labels {
        for (tag, anchor, weight) in country_label_anchors(static_data, states) {
            candidates.push(LabelCandidate {
                kind: OverlayLabelKind::Country,
                id: format!("country:{tag}"),
                text: names.countries.get(&tag).cloned().unwrap_or(tag),
                anchor,
                priority: 10_000 + weight as i32,
                tint: [0.93, 0.92, 0.82, 1.0],
                max_for_kind: options.max_country_labels,
            });
        }
    }
    if options.show_strategic_region_labels {
        for r in &static_data.regions {
            let Some(text) = label_text(names.strategic_regions.get(&r.region_id), &r.name_key)
            else {
                continue;
            };
            candidates.push(LabelCandidate {
                kind: OverlayLabelKind::StrategicRegion,
                id: format!("sr:{}", r.region_id),
                text,
                anchor: r.anchor,
                priority: 2_000 + r.province_count.min(i32::MAX as usize) as i32,
                tint: [0.68, 0.82, 0.92, 0.82],
                max_for_kind: options.max_strategic_region_labels,
            });
        }
    }
    if options.show_state_labels {
        for s in &static_data.states {
            let Some(text) = label_text(names.states.get(&s.state_id), &s.name_key) else {
                continue;
            };
            candidates.push(LabelCandidate {
                kind: OverlayLabelKind::State,
                id: format!("state:{}", s.state_id),
                text,
                anchor: s.anchor,
                priority: 4_000 + (s.vp_total as i32 * 100) + s.province_count.min(500) as i32,
                tint: [0.88, 0.90, 0.86, 0.92],
                max_for_kind: options.max_state_labels,
            });
        }
    }

    candidates.sort_by(|a, b| {
        b.priority
            .cmp(&a.priority)
            .then_with(|| a.kind.cmp(&b.kind))
            .then_with(|| a.id.cmp(&b.id))
    });

    let mut occupied = Vec::<Rect>::new();
    let mut used_country = 0_usize;
    let mut used_state = 0_usize;
    let mut used_region = 0_usize;
    for cand in candidates {
        let used = match cand.kind {
            OverlayLabelKind::Country => &mut used_country,
            OverlayLabelKind::State => &mut used_state,
            OverlayLabelKind::StrategicRegion => &mut used_region,
        };
        if *used >= cand.max_for_kind {
            plan.diagnostics.culled_labels += 1;
            continue;
        }
        let Some(screen) = projection.project(cand.anchor) else {
            plan.diagnostics.culled_labels += 1;
            continue;
        };
        if !projection.visible_point(screen, options.cull_margin_px) {
            plan.diagnostics.culled_labels += 1;
            continue;
        }
        let rect = label_rect(screen, &cand.text, &cand.kind);
        if occupied.iter().any(|r| r.intersects(rect)) {
            plan.diagnostics.culled_labels += 1;
            continue;
        }
        occupied.push(rect);
        *used += 1;
        match cand.kind {
            OverlayLabelKind::Country => plan.diagnostics.country_labels += 1,
            OverlayLabelKind::State => plan.diagnostics.state_labels += 1,
            OverlayLabelKind::StrategicRegion => plan.diagnostics.strategic_region_labels += 1,
        }
        plan.labels.push(OverlayLabel {
            kind: cand.kind,
            id: cand.id,
            text: cand.text,
            x: rect.x,
            y: rect.y,
            tint: cand.tint,
            priority: cand.priority,
        });
    }
}

fn country_label_anchors(
    static_data: &OverlayStatic,
    states: Option<&Value>,
) -> Vec<(String, MapPoint, usize)> {
    let Some(obj) = states.and_then(Value::as_object) else {
        return Vec::new();
    };
    let state_weights: BTreeMap<u32, usize> = static_data
        .states
        .iter()
        .map(|s| (s.state_id, s.province_count.max(1)))
        .collect();
    let mut acc: BTreeMap<String, (f64, f64, usize)> = BTreeMap::new();
    for (sid_s, state) in obj {
        let Some(sid) = parse_u32_str(sid_s) else {
            continue;
        };
        let Some(owner) = state.get("owner").and_then(Value::as_str) else {
            continue;
        };
        let Some(anchor) = static_data.state_anchors.get(&sid) else {
            continue;
        };
        let weight = *state_weights.get(&sid).unwrap_or(&1);
        let e = acc.entry(owner.to_string()).or_insert((0.0, 0.0, 0));
        e.0 += anchor.x as f64 * weight as f64;
        e.1 += anchor.y as f64 * weight as f64;
        e.2 += weight;
    }
    acc.into_iter()
        .filter_map(|(tag, (sx, sy, n))| {
            (n > 0).then_some((
                tag,
                MapPoint {
                    x: (sx / n as f64) as f32,
                    y: (sy / n as f64) as f32,
                },
                n,
            ))
        })
        .collect()
}

fn division_anchors_and_moves(
    static_data: &OverlayStatic,
    divisions: Option<&Value>,
    projection: &OverlayProjection,
    options: &OverlayOptions,
    plan: &mut OverlayPlan,
) -> BTreeMap<String, MapPoint> {
    let mut out = BTreeMap::new();
    let Some(arr) = divisions.and_then(Value::as_array) else {
        return out;
    };
    for (idx, div) in arr.iter().enumerate() {
        if let Some(anchor) = u32_field(div, &["location_province", "province"])
            .and_then(|pid| static_data.province_anchors.get(&pid).copied())
        {
            for key in division_keys(div, idx) {
                out.insert(key, anchor);
            }
        }

        if !options.show_movement_arrows {
            continue;
        }
        let Some(target_pid) = u32_field(div, &["move_target_province"]) else {
            if div.get("moving").and_then(Value::as_bool) == Some(true) {
                plan.diagnostics.unavailable_arrows += 1;
                add_unavailable_marker(
                    plan,
                    OverlayMarkerKind::UnavailableMovement,
                    format!("move:idx:{idx}"),
                    "movement unavailable",
                    movement_marker_anchor(static_data, div),
                    projection,
                    options,
                );
            }
            continue;
        };
        let Some(source_pid) = u32_field(div, &["movement_source_province"]) else {
            plan.diagnostics.unavailable_arrows += 1;
            add_unavailable_marker(
                plan,
                OverlayMarkerKind::UnavailableMovement,
                format!("move:idx:{idx}"),
                "movement source unavailable",
                movement_marker_anchor(static_data, div),
                projection,
                options,
            );
            continue;
        };
        if source_pid == target_pid {
            plan.diagnostics.unavailable_arrows += 1;
            add_unavailable_marker(
                plan,
                OverlayMarkerKind::UnavailableMovement,
                format!("move:idx:{idx}"),
                "movement unavailable",
                movement_marker_anchor(static_data, div),
                projection,
                options,
            );
            continue;
        }
        let Some(source) = static_data.province_anchors.get(&source_pid).copied() else {
            plan.diagnostics.unavailable_arrows += 1;
            add_unavailable_marker(
                plan,
                OverlayMarkerKind::UnavailableMovement,
                format!("move:idx:{idx}"),
                "movement source unavailable",
                movement_marker_anchor(static_data, div),
                projection,
                options,
            );
            continue;
        };
        let Some(target) = static_data.province_anchors.get(&target_pid).copied() else {
            plan.diagnostics.unavailable_arrows += 1;
            add_unavailable_marker(
                plan,
                OverlayMarkerKind::UnavailableMovement,
                format!("move:idx:{idx}"),
                "movement target unavailable",
                movement_marker_anchor(static_data, div),
                projection,
                options,
            );
            continue;
        };
        if same_map_point(source, target) {
            plan.diagnostics.unavailable_arrows += 1;
            add_unavailable_marker(
                plan,
                OverlayMarkerKind::UnavailableMovement,
                format!("move:idx:{idx}"),
                "movement unavailable",
                movement_marker_anchor(static_data, div),
                projection,
                options,
            );
            continue;
        }
        let id = division_keys(div, idx)
            .into_iter()
            .next()
            .unwrap_or_else(|| format!("idx:{idx}"));
        add_projected_line(
            plan,
            OverlayLine {
                kind: OverlayLineKind::Movement,
                id: format!("move:{id}"),
                points: Vec::new(),
                tint: [0.32, 0.78, 0.98, 0.92],
                width: 2.0,
                arrow: true,
                fidelity: OverlayGeometryFidelity::Derived,
                style: movement_line_style(),
            },
            &[source, target],
            projection,
            options,
        );
    }
    out
}

fn army_anchors(
    oob: Option<&Value>,
    division_anchors: &BTreeMap<String, MapPoint>,
) -> BTreeMap<String, MapPoint> {
    let mut out = BTreeMap::new();
    let Some(armies) = oob.and_then(|v| v.get("armies")).and_then(Value::as_array) else {
        return out;
    };
    for (idx, army) in armies.iter().enumerate() {
        let members = army
            .get("members")
            .or_else(|| army.get("member_div_ids"))
            .and_then(Value::as_array);
        let Some(anchor) = members.and_then(|m| average_member_anchor(m, division_anchors)) else {
            continue;
        };
        for key in army_keys(army, idx) {
            out.insert(key, anchor);
        }
    }
    out
}

fn add_front_line_lines(
    static_data: &OverlayStatic,
    map: Option<&Value>,
    states: Option<&Value>,
    projection: &OverlayProjection,
    options: &OverlayOptions,
    plan: &mut OverlayPlan,
) {
    if add_explicit_front_line_lines(static_data, map, projection, options, plan) {
        return;
    }

    if static_data.front_edges.is_empty() {
        plan.diagnostics.unavailable_front_lines += 1;
        return;
    }
    let controllers = province_controllers(static_data, map, states);
    if controllers.is_empty() {
        plan.diagnostics.unavailable_front_lines += 1;
        add_unavailable_marker(
            plan,
            OverlayMarkerKind::UnavailableFrontLine,
            "front:control",
            "front control unavailable",
            front_diagnostic_anchor(static_data),
            projection,
            options,
        );
        return;
    }

    let mut considered = 0_u32;
    for edge in &static_data.front_edges {
        let Some(a) = controllers.get(&edge.province_a) else {
            continue;
        };
        let Some(b) = controllers.get(&edge.province_b) else {
            continue;
        };
        considered += 1;
        if a == b {
            continue;
        }
        add_projected_line(
            plan,
            OverlayLine {
                kind: OverlayLineKind::FrontLine,
                id: format!("front:{}:{}", edge.province_a, edge.province_b),
                points: Vec::new(),
                tint: [0.95, 0.18, 0.16, 0.88],
                width: 3.2,
                arrow: false,
                fidelity: OverlayGeometryFidelity::Derived,
                style: front_line_style(),
            },
            &[edge.start, edge.end],
            projection,
            options,
        );
    }
    if considered == 0 {
        plan.diagnostics.unavailable_front_lines += 1;
        add_unavailable_marker(
            plan,
            OverlayMarkerKind::UnavailableFrontLine,
            "front:controller-match",
            "front boundary unavailable",
            front_diagnostic_anchor(static_data),
            projection,
            options,
        );
    }
}

fn add_explicit_front_line_lines(
    static_data: &OverlayStatic,
    map: Option<&Value>,
    projection: &OverlayProjection,
    options: &OverlayOptions,
    plan: &mut OverlayPlan,
) -> bool {
    let Some(front_lines_value) = map.and_then(|v| v.get("front_lines")) else {
        return false;
    };
    if front_lines_value.is_null() {
        return false;
    }
    let Some(front_lines) = front_lines_value.as_array() else {
        plan.diagnostics.unavailable_front_lines += 1;
        add_unavailable_marker(
            plan,
            OverlayMarkerKind::UnavailableFrontLine,
            "front:front_lines",
            "front lines domain malformed",
            front_diagnostic_anchor(static_data),
            projection,
            options,
        );
        return true;
    };
    if front_lines.is_empty() {
        return false;
    }

    let mut unavailable_without_geometry = Vec::new();
    for (idx, front_line) in front_lines.iter().enumerate() {
        match measured_front_line_polyline(front_line) {
            Some(Ok(polyline)) => {
                let id = explicit_front_line_id(front_line, idx);
                add_projected_line(
                    plan,
                    OverlayLine {
                        kind: OverlayLineKind::FrontLine,
                        id,
                        points: Vec::new(),
                        tint: [0.95, 0.18, 0.16, 0.88],
                        width: 3.2,
                        arrow: false,
                        fidelity: OverlayGeometryFidelity::Exact,
                        style: front_line_style(),
                    },
                    &polyline,
                    projection,
                    options,
                );
            }
            Some(Err(())) => {
                let id = explicit_front_line_id(front_line, idx);
                plan.diagnostics.unavailable_front_lines += 1;
                add_unavailable_marker(
                    plan,
                    OverlayMarkerKind::UnavailableFrontLine,
                    id,
                    "front line measured path unavailable",
                    first_measured_front_line_anchor(front_line)
                        .or_else(|| front_line_marker_anchor(static_data, front_line)),
                    projection,
                    options,
                );
            }
            None => unavailable_without_geometry.push((idx, front_line)),
        }
    }
    for (idx, front_line) in unavailable_without_geometry {
        plan.diagnostics.unavailable_front_lines += 1;
        add_unavailable_marker(
            plan,
            OverlayMarkerKind::UnavailableFrontLine,
            explicit_front_line_id(front_line, idx),
            "front line geometry unavailable",
            front_line_marker_anchor(static_data, front_line),
            projection,
            options,
        );
    }
    true
}

fn province_controllers(
    static_data: &OverlayStatic,
    map: Option<&Value>,
    states: Option<&Value>,
) -> BTreeMap<u32, String> {
    let mut out = BTreeMap::new();
    if let Some(obj) = map
        .and_then(|v| v.get("control_by_province"))
        .and_then(Value::as_object)
    {
        for (pid_s, tag) in obj {
            if let (Some(pid), Some(tag)) = (parse_u32_str(pid_s), tag.as_str()) {
                if !tag.trim().is_empty() {
                    out.insert(pid, tag.to_string());
                }
            }
        }
    }
    if !out.is_empty() {
        return out;
    }

    let Some(states_obj) = states.and_then(Value::as_object) else {
        return out;
    };
    for (pid, sid) in &static_data.province_states {
        let Some(state) = states_obj.get(&sid.to_string()) else {
            continue;
        };
        let Some(tag) = state
            .get("controller")
            .or_else(|| state.get("owner"))
            .and_then(Value::as_str)
        else {
            continue;
        };
        if !tag.trim().is_empty() {
            out.insert(*pid, tag.to_string());
        }
    }
    out
}

fn add_oob_order_lines(
    static_data: &OverlayStatic,
    oob: Option<&Value>,
    army_anchors: &BTreeMap<String, MapPoint>,
    projection: &OverlayProjection,
    options: &OverlayOptions,
    plan: &mut OverlayPlan,
) {
    let Some(oob) = oob else {
        return;
    };
    if let Some(armies) = oob.get("armies").and_then(Value::as_array) {
        for (army_idx, army) in armies.iter().enumerate() {
            let army_anchor = army_keys(army, army_idx)
                .into_iter()
                .find_map(|k| army_anchors.get(&k).copied());
            if let Some(orders) = army.get("orders").and_then(Value::as_array) {
                for (order_idx, order) in orders.iter().enumerate() {
                    if add_explicit_order_path(
                        static_data,
                        order,
                        &format!("orderpath:{army_idx}:{order_idx}"),
                        army_anchor,
                        projection,
                        options,
                        plan,
                    ) {
                        continue;
                    }
                    if let Some(anchor) = army_anchor {
                        if add_state_target_lines(
                            static_data,
                            order.get("states"),
                            anchor,
                            &format!("orderstate:{army_idx}:{order_idx}"),
                            projection,
                            options,
                            plan,
                        ) {
                            continue;
                        }
                    }
                    plan.diagnostics.unavailable_arrows += 1;
                    add_unavailable_marker(
                        plan,
                        OverlayMarkerKind::UnavailableBattlePlan,
                        format!("order:{army_idx}:{order_idx}"),
                        "battleplan unavailable",
                        battleplan_marker_anchor(static_data, order, army_anchor),
                        projection,
                        options,
                    );
                }
            }
        }
    }

    if let Some(orders) = oob.get("battle_plan_orders").and_then(Value::as_array) {
        for (idx, order) in orders.iter().enumerate() {
            let order_type = u32_field(order, &["order_type", "type"]);
            if order_type != Some(5) {
                if order.get("target_states").is_some() {
                    plan.diagnostics.unavailable_arrows += 1;
                    add_unavailable_marker(
                        plan,
                        OverlayMarkerKind::UnavailableBattlePlan,
                        format!("battleplan:{idx}"),
                        "battleplan geometry unavailable",
                        first_state_anchor(static_data, order.get("target_states")),
                        projection,
                        options,
                    );
                }
                continue;
            }
            let Some(army_anchor) = u32_or_string_key(order.get("army_id"))
                .into_iter()
                .find_map(|k| army_anchors.get(&k).copied())
            else {
                plan.diagnostics.unavailable_arrows += 1;
                add_unavailable_marker(
                    plan,
                    OverlayMarkerKind::UnavailableBattlePlan,
                    format!("battleplan:{idx}"),
                    "battleplan army unavailable",
                    first_state_anchor(static_data, order.get("target_states")),
                    projection,
                    options,
                );
                continue;
            };
            if !add_state_target_lines(
                static_data,
                order.get("target_states"),
                army_anchor,
                &format!("battleplan:{idx}"),
                projection,
                options,
                plan,
            ) {
                plan.diagnostics.unavailable_arrows += 1;
                add_unavailable_marker(
                    plan,
                    OverlayMarkerKind::UnavailableBattlePlan,
                    format!("battleplan:{idx}"),
                    "battleplan target unavailable",
                    first_state_anchor(static_data, order.get("target_states"))
                        .or(Some(army_anchor)),
                    projection,
                    options,
                );
            }
        }
    }
}

fn add_explicit_order_path(
    static_data: &OverlayStatic,
    order: &Value,
    id: &str,
    army_anchor: Option<MapPoint>,
    projection: &OverlayProjection,
    options: &OverlayOptions,
    plan: &mut OverlayPlan,
) -> bool {
    match measured_order_polyline(order) {
        Some(Ok(route)) => {
            add_projected_line(
                plan,
                OverlayLine {
                    kind: OverlayLineKind::BattlePlan,
                    id: id.to_string(),
                    points: Vec::new(),
                    tint: [0.98, 0.78, 0.22, 0.96],
                    width: 3.0,
                    arrow: true,
                    fidelity: OverlayGeometryFidelity::Exact,
                    style: battle_plan_line_style(),
                },
                &route,
                projection,
                options,
            );
            return true;
        }
        Some(Err(())) => {
            plan.diagnostics.unavailable_arrows += 1;
            add_unavailable_marker(
                plan,
                OverlayMarkerKind::UnavailableBattlePlan,
                id,
                "battleplan measured path unavailable",
                first_measured_order_anchor(order)
                    .or_else(|| battleplan_marker_anchor(static_data, order, army_anchor)),
                projection,
                options,
            );
            return true;
        }
        None => {}
    }

    let Some(path_value) = order.get("path").filter(|v| !v.is_null()) else {
        return false;
    };
    let provinces = u32_array(Some(path_value));
    if provinces.is_empty() {
        return false;
    }
    let Some(points) = path_anchors(&provinces, &static_data.province_anchors) else {
        plan.diagnostics.unavailable_arrows += 1;
        add_unavailable_marker(
            plan,
            OverlayMarkerKind::UnavailableBattlePlan,
            id,
            "battleplan path anchor unavailable",
            order_path_marker_anchor(static_data, order, army_anchor),
            projection,
            options,
        );
        return true;
    };
    add_projected_line(
        plan,
        OverlayLine {
            kind: OverlayLineKind::BattlePlan,
            id: id.to_string(),
            points: Vec::new(),
            tint: [0.98, 0.78, 0.22, 0.94],
            width: 3.0,
            arrow: true,
            fidelity: OverlayGeometryFidelity::Derived,
            style: battle_plan_line_style(),
        },
        &points,
        projection,
        options,
    );
    true
}

fn add_state_target_lines(
    static_data: &OverlayStatic,
    states: Option<&Value>,
    origin: MapPoint,
    id_prefix: &str,
    projection: &OverlayProjection,
    options: &OverlayOptions,
    plan: &mut OverlayPlan,
) -> bool {
    let targets = u32_array(states);
    if targets.is_empty() {
        return false;
    }
    let mut any = false;
    for sid in targets {
        let Some(target) = static_data.state_anchors.get(&sid).copied() else {
            continue;
        };
        any |= add_projected_line(
            plan,
            OverlayLine {
                kind: OverlayLineKind::BattlePlan,
                id: format!("{id_prefix}:{sid}"),
                points: Vec::new(),
                tint: [0.98, 0.67, 0.18, 0.92],
                width: 2.5,
                arrow: true,
                fidelity: OverlayGeometryFidelity::Derived,
                style: battle_plan_line_style(),
            },
            &[origin, target],
            projection,
            options,
        );
    }
    any
}

fn add_trade_route_lines(
    static_data: &OverlayStatic,
    diplomacy_world: Option<&Value>,
    states: Option<&Value>,
    projection: &OverlayProjection,
    options: &OverlayOptions,
    plan: &mut OverlayPlan,
) {
    let Some(contracts) = diplomacy_world
        .and_then(|v| v.get("trade_contracts"))
        .and_then(Value::as_array)
    else {
        return;
    };
    for (idx, contract) in contracts.iter().enumerate() {
        match measured_route_polyline(contract) {
            Some(Ok(route)) => {
                add_projected_line(
                    plan,
                    OverlayLine {
                        kind: OverlayLineKind::TradeRoute,
                        id: format!("trade:{idx}"),
                        points: Vec::new(),
                        tint: [0.24, 0.92, 0.70, 0.76],
                        width: 2.2,
                        arrow: false,
                        fidelity: OverlayGeometryFidelity::Exact,
                        style: trade_route_line_style(),
                    },
                    &route,
                    projection,
                    options,
                );
                continue;
            }
            Some(Err(())) => {
                plan.diagnostics.unavailable_routes += 1;
                add_unavailable_marker(
                    plan,
                    OverlayMarkerKind::UnavailableTradeRoute,
                    format!("trade:{idx}"),
                    "measured route unavailable",
                    first_measured_route_anchor(contract)
                        .or_else(|| trade_marker_anchor(static_data, contract, states)),
                    projection,
                    options,
                );
                continue;
            }
            None => {}
        }

        let provinces = ["route_provinces", "route", "path", "provinces"]
            .iter()
            .find_map(|field| {
                let p = u32_array(contract.get(*field));
                (p.len() >= 2).then_some(p)
            });
        let Some(provinces) = provinces else {
            plan.diagnostics.unavailable_routes += 1;
            add_unavailable_marker(
                plan,
                OverlayMarkerKind::UnavailableTradeRoute,
                format!("trade:{idx}"),
                "route unavailable",
                trade_marker_anchor(static_data, contract, states),
                projection,
                options,
            );
            continue;
        };
        let Some(route) = path_anchors(&provinces, &static_data.province_anchors) else {
            plan.diagnostics.unavailable_routes += 1;
            add_unavailable_marker(
                plan,
                OverlayMarkerKind::UnavailableTradeRoute,
                format!("trade:{idx}"),
                "route anchor unavailable",
                trade_marker_anchor(static_data, contract, states),
                projection,
                options,
            );
            continue;
        };
        add_projected_line(
            plan,
            OverlayLine {
                kind: OverlayLineKind::TradeRoute,
                id: format!("trade:{idx}"),
                points: Vec::new(),
                tint: [0.24, 0.92, 0.70, 0.70],
                width: 2.0,
                arrow: false,
                fidelity: OverlayGeometryFidelity::Derived,
                style: trade_route_line_style(),
            },
            &route,
            projection,
            options,
        );
    }
}

fn add_projected_line(
    plan: &mut OverlayPlan,
    mut line: OverlayLine,
    map_points: &[MapPoint],
    projection: &OverlayProjection,
    options: &OverlayOptions,
) -> bool {
    if plan.lines.len() >= options.max_lines {
        plan.diagnostics.culled_lines += 1;
        return false;
    }
    let mut points = Vec::with_capacity(map_points.len());
    for p in map_points {
        let Some(sp) = projection.project(*p) else {
            plan.diagnostics.culled_lines += 1;
            return false;
        };
        points.push(sp);
    }
    if points.len() < 2 {
        plan.diagnostics.culled_lines += 1;
        return false;
    }
    if !line_intersects_screen(&points, projection, options.cull_margin_px) {
        plan.diagnostics.culled_lines += 1;
        return false;
    }
    if points
        .windows(2)
        .any(|w| distance(w[0], w[1]) > options.max_segment_px)
    {
        plan.diagnostics.culled_lines += 1;
        return false;
    }
    line.points = points;
    match line.kind {
        OverlayLineKind::FrontLine => plan.diagnostics.front_line_lines += 1,
        OverlayLineKind::Movement => plan.diagnostics.movement_arrows += 1,
        OverlayLineKind::BattlePlan => plan.diagnostics.battle_plan_arrows += 1,
        OverlayLineKind::TradeRoute => plan.diagnostics.trade_route_lines += 1,
    }
    plan.lines.push(line);
    true
}

fn add_unavailable_marker(
    plan: &mut OverlayPlan,
    kind: OverlayMarkerKind,
    id: impl Into<String>,
    text: impl Into<String>,
    anchor: Option<MapPoint>,
    projection: &OverlayProjection,
    options: &OverlayOptions,
) -> bool {
    if plan.markers.len() >= options.max_lines {
        return false;
    }
    let Some(anchor) = anchor else {
        return false;
    };
    let Some(screen) = projection.project(anchor) else {
        return false;
    };
    if !projection.visible_point(screen, options.cull_margin_px) {
        return false;
    }
    let tint = marker_tint(&kind);
    let priority = marker_priority(&kind);
    plan.markers.push(OverlayMarker {
        kind,
        id: id.into(),
        text: text.into(),
        x: screen.x,
        y: screen.y,
        tint,
        priority,
    });
    true
}

fn marker_tint(kind: &OverlayMarkerKind) -> [f32; 4] {
    match kind {
        OverlayMarkerKind::UnavailableMovement => [0.70, 0.88, 1.0, 0.95],
        OverlayMarkerKind::UnavailableBattlePlan => [1.0, 0.78, 0.30, 0.95],
        OverlayMarkerKind::UnavailableTradeRoute => [0.42, 1.0, 0.76, 0.95],
        OverlayMarkerKind::UnavailableFrontLine => [1.0, 0.35, 0.30, 0.95],
    }
}

fn marker_priority(kind: &OverlayMarkerKind) -> i32 {
    match kind {
        OverlayMarkerKind::UnavailableMovement => 120,
        OverlayMarkerKind::UnavailableBattlePlan => 130,
        OverlayMarkerKind::UnavailableTradeRoute => 110,
        OverlayMarkerKind::UnavailableFrontLine => 140,
    }
}

fn movement_line_style() -> OverlayLineStyle {
    OverlayLineStyle::solid()
}

fn battle_plan_line_style() -> OverlayLineStyle {
    OverlayLineStyle::dashed(28.0, 10.0)
}

fn trade_route_line_style() -> OverlayLineStyle {
    OverlayLineStyle::dashed(8.0, 12.0)
}

fn front_line_style() -> OverlayLineStyle {
    OverlayLineStyle::dashed(12.0, 6.0)
}

fn measured_route_polyline(contract: &Value) -> Option<Result<Vec<MapPoint>, ()>> {
    measured_polyline_from_fields(
        contract,
        &[
            "measured_route_polyline",
            "measured_route_points",
            "measured_map_points",
        ],
        &["route_geometry", "geometry"],
    )
}

fn measured_order_polyline(order: &Value) -> Option<Result<Vec<MapPoint>, ()>> {
    measured_polyline_from_fields(
        order,
        &[
            "measured_order_polyline",
            "measured_path_polyline",
            "measured_map_points",
        ],
        &["order_geometry", "path_geometry", "geometry"],
    )
}

fn measured_front_line_polyline(front_line: &Value) -> Option<Result<Vec<MapPoint>, ()>> {
    measured_polyline_from_fields(
        front_line,
        &[
            "measured_front_polyline",
            "measured_line_polyline",
            "measured_map_points",
        ],
        &["front_geometry", "line_geometry", "geometry"],
    )
}

fn measured_polyline_from_fields(
    root: &Value,
    direct_fields: &[&str],
    geometry_fields: &[&str],
) -> Option<Result<Vec<MapPoint>, ()>> {
    for field in direct_fields {
        if let Some(value) = root.get(*field) {
            return Some(parse_measured_polyline(value));
        }
    }
    for field in geometry_fields {
        let Some(geometry) = root.get(*field) else {
            continue;
        };
        if geometry_claims_exact_measurement(geometry) {
            return Some(parse_measured_polyline(geometry));
        }
    }
    None
}

fn geometry_claims_exact_measurement(geometry: &Value) -> bool {
    let exact = ["fidelity", "geometry_fidelity", "source_fidelity"]
        .iter()
        .any(|field| value_str_eq(geometry.get(*field), "exact"));
    let measured = ["source", "geometry_source"]
        .iter()
        .any(|field| value_str_contains(geometry.get(*field), "measured"));
    exact || measured
}

fn parse_measured_polyline(value: &Value) -> Result<Vec<MapPoint>, ()> {
    let points = match value {
        Value::Array(arr) => arr,
        Value::Object(obj) => ["map_points", "points", "polyline"]
            .iter()
            .find_map(|field| obj.get(*field).and_then(Value::as_array))
            .ok_or(())?,
        _ => return Err(()),
    };
    if points.len() < 2 {
        return Err(());
    }
    let mut out = Vec::with_capacity(points.len());
    for point in points {
        out.push(parse_measured_point(point).ok_or(())?);
    }
    Ok(out)
}

fn parse_measured_point(value: &Value) -> Option<MapPoint> {
    let (x, y) = match value {
        Value::Array(pair) if pair.len() == 2 => {
            (value_to_f32(pair.first()?)?, value_to_f32(pair.get(1)?)?)
        }
        Value::Object(obj) => (
            value_to_f32(obj.get("x").or_else(|| obj.get("map_x"))?)?,
            value_to_f32(obj.get("y").or_else(|| obj.get("map_y"))?)?,
        ),
        _ => return None,
    };
    (x.is_finite() && y.is_finite()).then_some(MapPoint { x, y })
}

fn first_measured_route_anchor(contract: &Value) -> Option<MapPoint> {
    for field in [
        "measured_route_polyline",
        "measured_route_points",
        "measured_map_points",
    ] {
        if let Some(anchor) = contract.get(field).and_then(first_measured_point_in_value) {
            return Some(anchor);
        }
    }
    contract
        .get("route_geometry")
        .or_else(|| contract.get("geometry"))
        .and_then(first_measured_point_in_value)
}

fn first_measured_order_anchor(order: &Value) -> Option<MapPoint> {
    for field in [
        "measured_order_polyline",
        "measured_path_polyline",
        "measured_map_points",
    ] {
        if let Some(anchor) = order.get(field).and_then(first_measured_point_in_value) {
            return Some(anchor);
        }
    }
    ["order_geometry", "path_geometry", "geometry"]
        .iter()
        .find_map(|field| order.get(*field).and_then(first_measured_point_in_value))
}

fn first_measured_front_line_anchor(front_line: &Value) -> Option<MapPoint> {
    for field in [
        "measured_front_polyline",
        "measured_line_polyline",
        "measured_map_points",
    ] {
        if let Some(anchor) = front_line
            .get(field)
            .and_then(first_measured_point_in_value)
        {
            return Some(anchor);
        }
    }
    ["front_geometry", "line_geometry", "geometry"]
        .iter()
        .find_map(|field| {
            front_line
                .get(*field)
                .and_then(first_measured_point_in_value)
        })
}

fn first_measured_point_in_value(value: &Value) -> Option<MapPoint> {
    match value {
        Value::Array(arr) => arr.iter().find_map(parse_measured_point),
        Value::Object(obj) => ["map_points", "points", "polyline"]
            .iter()
            .find_map(|field| obj.get(*field).and_then(first_measured_point_in_value)),
        _ => None,
    }
}

fn movement_marker_anchor(static_data: &OverlayStatic, div: &Value) -> Option<MapPoint> {
    [
        "location_province",
        "province",
        "movement_source_province",
        "move_target_province",
    ]
    .iter()
    .find_map(|field| {
        u32_field(div, &[*field]).and_then(|pid| static_data.province_anchors.get(&pid).copied())
    })
}

fn front_diagnostic_anchor(static_data: &OverlayStatic) -> Option<MapPoint> {
    static_data.front_edges.first().map(|edge| MapPoint {
        x: (edge.start.x + edge.end.x) * 0.5,
        y: (edge.start.y + edge.end.y) * 0.5,
    })
}

fn explicit_front_line_id(front_line: &Value, idx: usize) -> String {
    if let Some(id) = front_line
        .get("id")
        .and_then(Value::as_str)
        .filter(|id| !id.trim().is_empty())
    {
        return format!("front:{id}");
    }
    if let (Some(a), Some(b)) = (
        u32_field(
            front_line,
            &["a", "province_a", "from_province", "source_province"],
        ),
        u32_field(
            front_line,
            &["b", "province_b", "to_province", "target_province"],
        ),
    ) {
        return format!("front:{a}:{b}");
    }
    format!("front:explicit:{idx}")
}

fn front_line_marker_anchor(static_data: &OverlayStatic, front_line: &Value) -> Option<MapPoint> {
    [
        "a",
        "province_a",
        "from_province",
        "source_province",
        "start_province",
        "left_province",
        "b",
        "province_b",
        "to_province",
        "target_province",
        "end_province",
        "right_province",
        "province",
    ]
    .iter()
    .find_map(|field| {
        u32_field(front_line, &[*field])
            .and_then(|pid| static_data.province_anchors.get(&pid).copied())
    })
    .or_else(|| {
        ["provinces", "province_ids", "path", "route"]
            .iter()
            .find_map(|field| first_path_anchor(static_data, front_line.get(*field)))
    })
    .or_else(|| {
        [
            "state",
            "state_id",
            "state_a",
            "from_state",
            "source_state",
            "start_state",
            "left_state",
            "state_b",
            "to_state",
            "target_state",
            "end_state",
            "right_state",
        ]
        .iter()
        .find_map(|field| {
            u32_field(front_line, &[*field])
                .and_then(|sid| static_data.state_anchors.get(&sid).copied())
        })
    })
    .or_else(|| {
        ["states", "state_ids", "target_states"]
            .iter()
            .find_map(|field| first_state_anchor(static_data, front_line.get(*field)))
    })
}

fn battleplan_marker_anchor(
    static_data: &OverlayStatic,
    order: &Value,
    army_anchor: Option<MapPoint>,
) -> Option<MapPoint> {
    first_state_anchor(static_data, order.get("target_states"))
        .or_else(|| first_state_anchor(static_data, order.get("states")))
        .or_else(|| first_path_anchor(static_data, order.get("path")))
        .or(army_anchor)
}

fn order_path_marker_anchor(
    static_data: &OverlayStatic,
    order: &Value,
    army_anchor: Option<MapPoint>,
) -> Option<MapPoint> {
    first_measured_order_anchor(order)
        .or_else(|| first_path_anchor(static_data, order.get("path")))
        .or_else(|| first_state_anchor(static_data, order.get("target_states")))
        .or_else(|| first_state_anchor(static_data, order.get("states")))
        .or(army_anchor)
}

fn trade_marker_anchor(
    static_data: &OverlayStatic,
    contract: &Value,
    states: Option<&Value>,
) -> Option<MapPoint> {
    ["route_provinces", "route", "path", "provinces"]
        .iter()
        .find_map(|field| first_path_anchor(static_data, contract.get(*field)))
        .or_else(|| {
            ["source_state", "target_state", "state"]
                .iter()
                .find_map(|field| {
                    u32_field(contract, &[*field])
                        .and_then(|sid| static_data.state_anchors.get(&sid).copied())
                })
        })
        .or_else(|| {
            contract
                .get("seller")
                .and_then(Value::as_str)
                .and_then(|tag| country_anchor(static_data, states, tag))
        })
        .or_else(|| {
            contract
                .get("buyer")
                .and_then(Value::as_str)
                .and_then(|tag| country_anchor(static_data, states, tag))
        })
}

fn first_path_anchor(static_data: &OverlayStatic, value: Option<&Value>) -> Option<MapPoint> {
    u32_array(value)
        .into_iter()
        .find_map(|pid| static_data.province_anchors.get(&pid).copied())
}

fn first_state_anchor(static_data: &OverlayStatic, value: Option<&Value>) -> Option<MapPoint> {
    u32_array(value)
        .into_iter()
        .find_map(|sid| static_data.state_anchors.get(&sid).copied())
}

fn country_anchor(
    static_data: &OverlayStatic,
    states: Option<&Value>,
    tag: &str,
) -> Option<MapPoint> {
    country_label_anchors(static_data, states)
        .into_iter()
        .find_map(|(owner, anchor, _)| (owner == tag).then_some(anchor))
}

fn line_intersects_screen(
    points: &[ScreenPoint],
    projection: &OverlayProjection,
    margin: f32,
) -> bool {
    let mut min_x = f32::INFINITY;
    let mut min_y = f32::INFINITY;
    let mut max_x = f32::NEG_INFINITY;
    let mut max_y = f32::NEG_INFINITY;
    for p in points {
        min_x = min_x.min(p.x);
        min_y = min_y.min(p.y);
        max_x = max_x.max(p.x);
        max_y = max_y.max(p.y);
    }
    max_x >= -margin
        && max_y >= -margin
        && min_x <= projection.screen_w + margin
        && min_y <= projection.screen_h + margin
}

fn path_anchors(provinces: &[u32], anchors: &BTreeMap<u32, MapPoint>) -> Option<Vec<MapPoint>> {
    if provinces.len() < 2 {
        return None;
    }
    let mut out = Vec::with_capacity(provinces.len());
    for pid in provinces {
        out.push(*anchors.get(pid)?);
    }
    Some(out)
}

fn average_member_anchor(
    members: &[Value],
    division_anchors: &BTreeMap<String, MapPoint>,
) -> Option<MapPoint> {
    let mut sx = 0.0_f64;
    let mut sy = 0.0_f64;
    let mut n = 0_u32;
    for member in members {
        for key in u32_or_string_key(Some(member)) {
            if let Some(p) = division_anchors.get(&key) {
                sx += p.x as f64;
                sy += p.y as f64;
                n += 1;
                break;
            }
        }
    }
    (n > 0).then_some(MapPoint {
        x: (sx / n as f64) as f32,
        y: (sy / n as f64) as f32,
    })
}

fn division_keys(div: &Value, idx: usize) -> Vec<String> {
    let mut keys = BTreeSet::new();
    if let Some(id) = div.get("id") {
        keys.extend(u32_or_string_key(Some(id)));
    }
    if let Some(id) = div.get("div_id") {
        keys.extend(u32_or_string_key(Some(id)));
        if let Some(t) = div.get("div_type").and_then(Value::as_i64) {
            if let Some(i) = numeric_key(id) {
                keys.insert(format!("{i}:{t}"));
            }
        }
    }
    if keys.is_empty() {
        keys.insert(format!("idx:{idx}"));
    }
    keys.into_iter().collect()
}

fn army_keys(army: &Value, idx: usize) -> Vec<String> {
    let mut keys = BTreeSet::new();
    if let Some(id) = army.get("id") {
        keys.extend(u32_or_string_key(Some(id)));
    }
    if let Some(id) = army.get("army_id") {
        keys.extend(u32_or_string_key(Some(id)));
    }
    if keys.is_empty() {
        keys.insert(format!("idx:{idx}"));
    }
    keys.into_iter().collect()
}

fn u32_or_string_key(v: Option<&Value>) -> Vec<String> {
    let mut out = BTreeSet::new();
    if let Some(v) = v {
        if let Some(s) = v.as_str() {
            out.insert(s.to_string());
            if let Some((head, _)) = s.split_once(':') {
                out.insert(head.to_string());
            }
        }
        if let Some(i) = numeric_key(v) {
            out.insert(i.to_string());
        }
    }
    out.into_iter().collect()
}

fn numeric_key(v: &Value) -> Option<i64> {
    v.as_i64()
        .or_else(|| v.as_u64().and_then(|u| i64::try_from(u).ok()))
        .or_else(|| v.as_str()?.parse::<i64>().ok())
}

fn u32_field(v: &Value, fields: &[&str]) -> Option<u32> {
    fields.iter().find_map(|field| value_to_u32(v.get(*field)?))
}

fn value_to_u32(v: &Value) -> Option<u32> {
    v.as_u64()
        .and_then(|u| u32::try_from(u).ok())
        .or_else(|| v.as_i64().and_then(|i| u32::try_from(i).ok()))
        .or_else(|| v.as_str()?.parse::<u32>().ok())
}

fn value_to_f32(v: &Value) -> Option<f32> {
    v.as_f64()
        .map(|f| f as f32)
        .or_else(|| v.as_str()?.parse::<f32>().ok())
}

fn u32_array(v: Option<&Value>) -> Vec<u32> {
    match v {
        Some(Value::Array(arr)) => arr.iter().filter_map(value_to_u32).collect(),
        Some(Value::Object(obj)) => obj.keys().filter_map(|k| parse_u32_str(k)).collect(),
        _ => Vec::new(),
    }
}

fn parse_u32_str(s: &str) -> Option<u32> {
    s.parse::<u32>().ok()
}

fn value_str_eq(value: Option<&Value>, expected: &str) -> bool {
    value
        .and_then(Value::as_str)
        .map(|s| s.eq_ignore_ascii_case(expected))
        .unwrap_or(false)
}

fn value_str_contains(value: Option<&Value>, needle: &str) -> bool {
    value
        .and_then(Value::as_str)
        .map(|s| s.to_ascii_lowercase().contains(needle))
        .unwrap_or(false)
}

fn label_text(localized: Option<&String>, name_key: &str) -> Option<String> {
    if let Some(text) = localized.filter(|text| !text.trim().is_empty()) {
        return Some(text.clone());
    }
    (!name_key.trim().is_empty()).then(|| name_key.to_string())
}

#[derive(Debug, Clone)]
struct LabelCandidate {
    kind: OverlayLabelKind,
    id: String,
    text: String,
    anchor: MapPoint,
    priority: i32,
    tint: [f32; 4],
    max_for_kind: usize,
}

#[derive(Debug, Clone, Copy)]
struct Rect {
    x: f32,
    y: f32,
    w: f32,
    h: f32,
}

impl Rect {
    fn intersects(self, other: Rect) -> bool {
        self.x < other.x + other.w
            && self.x + self.w > other.x
            && self.y < other.y + other.h
            && self.y + self.h > other.y
    }
}

fn label_rect(center: ScreenPoint, text: &str, kind: &OverlayLabelKind) -> Rect {
    let char_w = match kind {
        OverlayLabelKind::Country => 8.0,
        OverlayLabelKind::State => 6.8,
        OverlayLabelKind::StrategicRegion => 6.4,
    };
    let height = match kind {
        OverlayLabelKind::Country => 18.0,
        OverlayLabelKind::State => 16.0,
        OverlayLabelKind::StrategicRegion => 15.0,
    };
    let width = (text.chars().count() as f32 * char_w + 10.0).clamp(24.0, 260.0);
    Rect {
        x: center.x - width / 2.0,
        y: center.y - height / 2.0,
        w: width,
        h: height,
    }
}

fn line_sort_key(line: &OverlayLine) -> (OverlayLineKind, String) {
    (line.kind.clone(), line.id.clone())
}

fn marker_sort_key(marker: &OverlayMarker) -> (OverlayMarkerKind, i32, String) {
    (marker.kind.clone(), marker.priority, marker.id.clone())
}

fn label_sort_key(label: &OverlayLabel) -> (OverlayLabelKind, String) {
    (label.kind.clone(), label.id.clone())
}

fn distance(a: ScreenPoint, b: ScreenPoint) -> f32 {
    let dx = a.x - b.x;
    let dy = a.y - b.y;
    (dx * dx + dy * dy).sqrt()
}

fn same_map_point(a: MapPoint, b: MapPoint) -> bool {
    a.x == b.x && a.y == b.y
}

fn q(v: f32) -> i32 {
    (v * 100.0).round() as i32
}

#[cfg(test)]
mod tests {
    use super::*;

    fn test_static() -> OverlayStatic {
        let province_anchors = BTreeMap::from([
            (1, MapPoint { x: 100.0, y: 100.0 }),
            (2, MapPoint { x: 300.0, y: 100.0 }),
            (3, MapPoint { x: 500.0, y: 100.0 }),
            (4, MapPoint { x: 700.0, y: 100.0 }),
        ]);
        let states = vec![
            StateLabelSeed {
                state_id: 10,
                name_key: "STATE_10".into(),
                anchor: MapPoint { x: 100.0, y: 100.0 },
                province_count: 4,
                vp_total: 5.0,
            },
            StateLabelSeed {
                state_id: 20,
                name_key: "STATE_20".into(),
                anchor: MapPoint { x: 500.0, y: 100.0 },
                province_count: 4,
                vp_total: 1.0,
            },
        ];
        let regions = vec![RegionLabelSeed {
            region_id: 100,
            name_key: "STRATEGICREGION_100".into(),
            anchor: MapPoint { x: 300.0, y: 250.0 },
            province_count: 8,
        }];
        OverlayStatic::for_test(province_anchors, states, regions)
    }

    fn proj() -> OverlayProjection {
        OverlayProjection {
            offset: [0.0, 0.0],
            scale: 1.0,
            screen_w: 800.0,
            screen_h: 400.0,
            raster_w: 800.0,
            raster_h: 400.0,
        }
    }

    fn names() -> OverlayNames {
        OverlayNames {
            countries: BTreeMap::from([("GER".into(), "Germany".into())]),
            states: BTreeMap::from([(10, "State 10".into()), (20, "State 20".into())]),
            strategic_regions: BTreeMap::from([(100, "Region 100".into())]),
        }
    }

    #[test]
    fn projection_matches_map_camera_inverse_math() {
        let p = OverlayProjection {
            offset: [0.25, 0.10],
            scale: 2.0,
            screen_w: 1000.0,
            screen_h: 500.0,
            raster_w: 4000.0,
            raster_h: 2000.0,
        };
        let s = p
            .project(MapPoint {
                x: 2000.0,
                y: 1000.0,
            })
            .unwrap();
        assert!((s.x - 500.0).abs() < 0.01);
        assert!((s.y - 400.0).abs() < 0.01);
    }

    #[test]
    fn projection_rejects_invalid_camera_transform() {
        let mut p = proj();
        p.scale = 0.0;
        assert!(
            p.project(MapPoint { x: 100.0, y: 100.0 }).is_none(),
            "invalid camera scale must cull, not silently use identity"
        );

        p = proj();
        p.offset[0] = f32::NAN;
        assert!(
            p.project(MapPoint { x: 100.0, y: 100.0 }).is_none(),
            "non-finite camera offset must cull overlay geometry"
        );
    }

    #[test]
    fn absent_routes_and_arrows_do_not_fabricate_geometry() {
        let st = test_static();
        let snap_states = serde_json::json!({ "10": { "owner": "GER" } });
        let divisions = serde_json::json!([
            { "div_id": 1, "div_type": 42, "location_province": 1, "moving": true }
        ]);
        let oob = serde_json::json!({
            "armies": [
                { "id": "9:1", "members": ["1:42"], "orders": [ { "type": 5, "path": [], "states": [] } ] }
            ],
            "battle_plan_orders": [
                { "army_id": "9:1", "order_type": 3, "target_states": null }
            ]
        });
        let diplo = serde_json::json!({
            "trade_contracts": [
                { "seller": "GER", "buyer": "ITA", "delivery_state": 20 }
            ]
        });
        let plan = build_overlay_plan(
            &st,
            OverlayInputs {
                states: Some(&snap_states),
                map: None,
                divisions: Some(&divisions),
                order_of_battle: Some(&oob),
                diplomacy_world: Some(&diplo),
            },
            &names(),
            proj(),
            OverlayOptions::default(),
        );
        assert!(
            plan.lines.is_empty(),
            "missing routes/targets must not fabricate lines"
        );
        assert!(plan.diagnostics.unavailable_arrows >= 2);
        assert_eq!(plan.diagnostics.unavailable_routes, 1);
        assert!(
            plan.markers.len() >= 3,
            "anchored unavailable route/arrow data should be visible as diagnostic markers"
        );
    }

    #[test]
    fn movement_arrow_requires_measured_source_and_target() {
        let st = test_static();
        let divisions = serde_json::json!([
            { "div_id": 1, "div_type": 42, "location_province": 1, "move_target_province": 2, "moving": true }
        ]);
        let plan = build_overlay_plan(
            &st,
            OverlayInputs {
                states: None,
                map: None,
                divisions: Some(&divisions),
                order_of_battle: None,
                diplomacy_world: None,
            },
            &names(),
            proj(),
            OverlayOptions::default(),
        );
        assert_eq!(plan.diagnostics.movement_arrows, 0);
        assert_eq!(plan.diagnostics.unavailable_arrows, 1);
        assert!(
            plan.lines
                .iter()
                .all(|line| line.kind != OverlayLineKind::Movement),
            "location->target must not be used as a guessed movement source"
        );
        assert!(
            plan.markers
                .iter()
                .any(|marker| marker.kind == OverlayMarkerKind::UnavailableMovement),
            "missing measured movement source should draw a marker, not a guessed arrow"
        );
    }

    #[test]
    fn missing_static_label_key_does_not_synthesize_state_or_region_text() {
        let mut st = test_static();
        st.states[0].name_key.clear();
        st.regions[0].name_key.clear();
        let mut options = OverlayOptions::default();
        options.show_country_labels = false;
        let plan = build_overlay_plan(
            &st,
            OverlayInputs {
                states: None,
                map: None,
                divisions: None,
                order_of_battle: None,
                diplomacy_world: None,
            },
            &OverlayNames::default(),
            proj(),
            options,
        );
        assert!(
            plan.labels
                .iter()
                .all(|label| label.text != "STATE_10" && label.text != "STRATEGICREGION_100"),
            "missing static label keys must stay missing, not become id-shaped labels"
        );
    }

    #[test]
    fn route_with_missing_static_anchor_is_unavailable_not_spliced() {
        let st = test_static();
        let diplo = serde_json::json!({
            "trade_contracts": [
                { "route_provinces": [1, 99, 4] }
            ]
        });
        let plan = build_overlay_plan(
            &st,
            OverlayInputs {
                states: None,
                map: None,
                divisions: None,
                order_of_battle: None,
                diplomacy_world: Some(&diplo),
            },
            &names(),
            proj(),
            OverlayOptions::default(),
        );
        assert_eq!(plan.diagnostics.trade_route_lines, 0);
        assert_eq!(plan.diagnostics.unavailable_routes, 1);
        assert!(
            plan.lines
                .iter()
                .all(|line| line.kind != OverlayLineKind::TradeRoute),
            "route [1, missing, 4] must not be spliced into a false 1->4 line"
        );
        let marker = plan
            .markers
            .iter()
            .find(|marker| marker.kind == OverlayMarkerKind::UnavailableTradeRoute)
            .expect("missing route anchor should be visible as an unavailable marker");
        assert_eq!((marker.x, marker.y), (100.0, 100.0));
    }

    #[test]
    fn measured_route_polyline_is_exact_while_province_route_is_derived() {
        let st = test_static();
        let diplo = serde_json::json!({
            "trade_contracts": [
                {
                    "measured_route_polyline": [
                        [110.0, 120.0],
                        { "x": 210.0, "y": 180.0 },
                        { "map_x": "320.0", "map_y": "240.0" }
                    ]
                },
                { "route_provinces": [1, 4] }
            ]
        });
        let plan = build_overlay_plan(
            &st,
            OverlayInputs {
                states: None,
                map: None,
                divisions: None,
                order_of_battle: None,
                diplomacy_world: Some(&diplo),
            },
            &names(),
            proj(),
            OverlayOptions::default(),
        );
        let routes: Vec<_> = plan
            .lines
            .iter()
            .filter(|line| line.kind == OverlayLineKind::TradeRoute)
            .collect();
        assert_eq!(routes.len(), 2);

        let exact = routes
            .iter()
            .find(|line| line.fidelity == OverlayGeometryFidelity::Exact)
            .expect("measured route should be exact");
        assert_screen_points_close(
            &exact.points,
            &[
                ScreenPoint { x: 110.0, y: 120.0 },
                ScreenPoint { x: 210.0, y: 180.0 },
                ScreenPoint { x: 320.0, y: 240.0 },
            ],
        );

        let derived = routes
            .iter()
            .find(|line| line.fidelity == OverlayGeometryFidelity::Derived)
            .expect("province route should remain derived");
        assert_screen_points_close(
            &derived.points,
            &[
                ScreenPoint { x: 100.0, y: 100.0 },
                ScreenPoint { x: 700.0, y: 100.0 },
            ],
        );
        assert_eq!(plan.diagnostics.trade_route_lines, 2);
        assert_eq!(plan.diagnostics.unavailable_routes, 0);
    }

    #[test]
    fn malformed_measured_route_is_unavailable_not_replaced_by_province_path() {
        let st = test_static();
        let diplo = serde_json::json!({
            "trade_contracts": [
                {
                    "measured_route_polyline": [
                        { "x": 125.0, "y": 130.0 },
                        { "x": 200.0 }
                    ],
                    "route_provinces": [1, 4]
                }
            ]
        });
        let plan = build_overlay_plan(
            &st,
            OverlayInputs {
                states: None,
                map: None,
                divisions: None,
                order_of_battle: None,
                diplomacy_world: Some(&diplo),
            },
            &names(),
            proj(),
            OverlayOptions::default(),
        );
        assert_eq!(plan.diagnostics.trade_route_lines, 0);
        assert_eq!(plan.diagnostics.unavailable_routes, 1);
        assert!(
            plan.lines
                .iter()
                .all(|line| line.kind != OverlayLineKind::TradeRoute),
            "an invalid measured route must not silently fall back to route_provinces"
        );
        let marker = plan
            .markers
            .iter()
            .find(|marker| marker.kind == OverlayMarkerKind::UnavailableTradeRoute)
            .expect("malformed measured route should render a visible diagnostic");
        assert_eq!(marker.text, "measured route unavailable");
        assert_eq!((marker.x, marker.y), (125.0, 130.0));
    }

    #[test]
    fn exact_route_geometry_without_polyline_is_unavailable_not_replaced_by_province_path() {
        let st = test_static();
        let diplo = serde_json::json!({
            "trade_contracts": [
                {
                    "route_geometry": {
                        "fidelity": "exact",
                        "source": "measured_save_route"
                    },
                    "route_provinces": [1, 4]
                }
            ]
        });
        let plan = build_overlay_plan(
            &st,
            OverlayInputs {
                states: None,
                map: None,
                divisions: None,
                order_of_battle: None,
                diplomacy_world: Some(&diplo),
            },
            &names(),
            proj(),
            OverlayOptions::default(),
        );
        assert_eq!(plan.diagnostics.trade_route_lines, 0);
        assert_eq!(plan.diagnostics.unavailable_routes, 1);
        assert!(
            plan.lines
                .iter()
                .all(|line| line.kind != OverlayLineKind::TradeRoute),
            "an exact route geometry object without points must not fall back to route_provinces"
        );
    }

    #[test]
    fn measured_front_line_geometry_draws_exact_without_controller_boundary_duplication() {
        let mut st = test_static();
        st.front_edges = vec![FrontLineSeed {
            province_a: 1,
            province_b: 2,
            start: MapPoint { x: 200.0, y: 80.0 },
            end: MapPoint { x: 200.0, y: 140.0 },
        }];
        let map = serde_json::json!({
            "control_by_province": {
                "1": "GER",
                "2": "POL"
            },
            "front_lines": [
                {
                    "id": "western",
                    "a": 1,
                    "b": 2,
                    "front_geometry": {
                        "fidelity": "exact",
                        "source": "measured_save_front",
                        "map_points": [
                            [150.0, 120.0],
                            { "x": 190.0, "y": 145.0 },
                            { "map_x": "260.0", "map_y": "160.0" }
                        ]
                    }
                }
            ]
        });
        let mut options = OverlayOptions::default();
        options.show_front_lines = true;
        let plan = build_overlay_plan(
            &st,
            OverlayInputs {
                states: None,
                map: Some(&map),
                divisions: None,
                order_of_battle: None,
                diplomacy_world: None,
            },
            &names(),
            proj(),
            options,
        );

        let fronts: Vec<_> = plan
            .lines
            .iter()
            .filter(|line| line.kind == OverlayLineKind::FrontLine)
            .collect();
        assert_eq!(
            fronts.len(),
            1,
            "explicit front geometry must not be duplicated by controller-boundary fallback"
        );
        let line = fronts[0];
        assert_eq!(line.fidelity, OverlayGeometryFidelity::Exact);
        assert_eq!(line.style, front_line_style());
        assert!(!line.arrow);
        assert_screen_points_close(
            &line.points,
            &[
                ScreenPoint { x: 150.0, y: 120.0 },
                ScreenPoint { x: 190.0, y: 145.0 },
                ScreenPoint { x: 260.0, y: 160.0 },
            ],
        );
        assert_eq!(plan.diagnostics.front_line_lines, 1);
        assert_eq!(plan.diagnostics.unavailable_front_lines, 0);
    }

    #[test]
    fn mixed_front_lines_account_unmeasured_siblings_without_controller_boundary_fallback() {
        let mut st = test_static();
        st.front_edges = vec![FrontLineSeed {
            province_a: 1,
            province_b: 2,
            start: MapPoint { x: 200.0, y: 80.0 },
            end: MapPoint { x: 200.0, y: 140.0 },
        }];
        let map = serde_json::json!({
            "control_by_province": {
                "1": "GER",
                "2": "POL"
            },
            "front_lines": [
                {
                    "id": "western",
                    "province_a": 1,
                    "province_b": 2,
                    "front_geometry": {
                        "fidelity": "exact",
                        "source": "measured_save_front",
                        "map_points": [
                            [150.0, 120.0],
                            [190.0, 145.0]
                        ]
                    }
                },
                {
                    "province_a": 3,
                    "province_b": 4
                }
            ]
        });
        let mut options = OverlayOptions::default();
        options.show_front_lines = true;
        let plan = build_overlay_plan(
            &st,
            OverlayInputs {
                states: None,
                map: Some(&map),
                divisions: None,
                order_of_battle: None,
                diplomacy_world: None,
            },
            &names(),
            proj(),
            options,
        );

        let fronts: Vec<_> = plan
            .lines
            .iter()
            .filter(|line| line.kind == OverlayLineKind::FrontLine)
            .collect();
        assert_eq!(
            fronts.len(),
            1,
            "mixed explicit front lines must not also draw controller-boundary fallback"
        );
        assert_eq!(fronts[0].fidelity, OverlayGeometryFidelity::Exact);
        assert!(
            fronts
                .iter()
                .all(|line| line.fidelity != OverlayGeometryFidelity::Derived),
            "unmeasured siblings must not be replaced by derived front geometry"
        );
        assert_eq!(plan.diagnostics.front_line_lines, 1);
        assert_eq!(plan.diagnostics.unavailable_front_lines, 1);

        let marker = plan
            .markers
            .iter()
            .find(|marker| marker.id == "front:3:4")
            .expect("unmeasured sibling should render an anchored unavailable marker");
        assert_eq!(marker.kind, OverlayMarkerKind::UnavailableFrontLine);
        assert_eq!(marker.text, "front line geometry unavailable");
        assert!(
            (marker.x - 500.0).abs() < 0.01 && (marker.y - 100.0).abs() < 0.01,
            "{marker:?}"
        );
    }

    #[test]
    fn nonempty_front_lines_without_geometry_fail_closed_not_controller_boundary() {
        let mut st = test_static();
        st.front_edges = vec![FrontLineSeed {
            province_a: 1,
            province_b: 2,
            start: MapPoint { x: 200.0, y: 80.0 },
            end: MapPoint { x: 200.0, y: 140.0 },
        }];
        let map = serde_json::json!({
            "control_by_province": {
                "1": "GER",
                "2": "POL"
            },
            "front_lines": [
                { "province_a": 1, "province_b": 2 },
                { "province_a": 3, "province_b": 4 }
            ]
        });
        let mut options = OverlayOptions::default();
        options.show_front_lines = true;
        let plan = build_overlay_plan(
            &st,
            OverlayInputs {
                states: None,
                map: Some(&map),
                divisions: None,
                order_of_battle: None,
                diplomacy_world: None,
            },
            &names(),
            proj(),
            options,
        );

        assert_eq!(plan.diagnostics.front_line_lines, 0);
        assert_eq!(plan.diagnostics.unavailable_front_lines, 2);
        assert!(
            plan.lines
                .iter()
                .all(|line| line.kind != OverlayLineKind::FrontLine),
            "non-empty explicit front-lines domain must not be replaced by derived controller-boundary geometry"
        );
        assert!(
            plan.markers.iter().any(|marker| marker.id == "front:1:2"
                && marker.kind == OverlayMarkerKind::UnavailableFrontLine),
            "first explicit front-line entry should be accounted as unavailable"
        );
        assert!(
            plan.markers.iter().any(|marker| marker.id == "front:3:4"
                && marker.kind == OverlayMarkerKind::UnavailableFrontLine),
            "second explicit front-line entry should be accounted as unavailable"
        );
    }

    #[test]
    fn malformed_front_lines_domain_fails_closed_not_controller_boundary() {
        let mut st = test_static();
        st.front_edges = vec![FrontLineSeed {
            province_a: 1,
            province_b: 2,
            start: MapPoint { x: 200.0, y: 80.0 },
            end: MapPoint { x: 200.0, y: 140.0 },
        }];
        let map = serde_json::json!({
            "control_by_province": {
                "1": "GER",
                "2": "POL"
            },
            "front_lines": { "malformed": true }
        });
        let mut options = OverlayOptions::default();
        options.show_front_lines = true;
        let plan = build_overlay_plan(
            &st,
            OverlayInputs {
                states: None,
                map: Some(&map),
                divisions: None,
                order_of_battle: None,
                diplomacy_world: None,
            },
            &names(),
            proj(),
            options,
        );

        assert_eq!(plan.diagnostics.front_line_lines, 0);
        assert_eq!(plan.diagnostics.unavailable_front_lines, 1);
        assert!(
            plan.lines
                .iter()
                .all(|line| line.kind != OverlayLineKind::FrontLine),
            "malformed explicit front-lines domain must not be replaced by controller-boundary geometry"
        );
        let marker = plan
            .markers
            .iter()
            .find(|marker| marker.id == "front:front_lines")
            .expect("malformed front_lines domain should render a diagnostic");
        assert_eq!(marker.kind, OverlayMarkerKind::UnavailableFrontLine);
        assert_eq!(marker.text, "front lines domain malformed");
    }

    #[test]
    fn malformed_measured_front_line_is_unavailable_not_replaced_by_controller_boundary() {
        let mut st = test_static();
        st.front_edges = vec![FrontLineSeed {
            province_a: 1,
            province_b: 2,
            start: MapPoint { x: 200.0, y: 80.0 },
            end: MapPoint { x: 200.0, y: 140.0 },
        }];
        let map = serde_json::json!({
            "control_by_province": {
                "1": "GER",
                "2": "POL"
            },
            "front_lines": [
                {
                    "a": 1,
                    "b": 2,
                    "front_geometry": {
                        "fidelity": "exact",
                        "source": "measured_save_front",
                        "map_points": [
                            { "x": 155.0, "y": 125.0 },
                            { "x": 210.0 }
                        ]
                    }
                }
            ]
        });
        let mut options = OverlayOptions::default();
        options.show_front_lines = true;
        let plan = build_overlay_plan(
            &st,
            OverlayInputs {
                states: None,
                map: Some(&map),
                divisions: None,
                order_of_battle: None,
                diplomacy_world: None,
            },
            &names(),
            proj(),
            options,
        );

        assert_eq!(plan.diagnostics.front_line_lines, 0);
        assert_eq!(plan.diagnostics.unavailable_front_lines, 1);
        assert!(
            plan.lines
                .iter()
                .all(|line| line.kind != OverlayLineKind::FrontLine),
            "malformed exact front geometry must not fall back to controller-boundary geometry"
        );
        let marker = plan
            .markers
            .iter()
            .find(|marker| marker.kind == OverlayMarkerKind::UnavailableFrontLine)
            .expect("malformed front geometry should render a diagnostic marker");
        assert_eq!(marker.text, "front line measured path unavailable");
        assert!(
            (marker.x - 155.0).abs() < 0.01 && (marker.y - 125.0).abs() < 0.01,
            "{marker:?}"
        );
    }

    #[test]
    fn absent_and_empty_front_lines_allow_controller_boundary_fallback() {
        let mut st = test_static();
        st.front_edges = vec![FrontLineSeed {
            province_a: 1,
            province_b: 2,
            start: MapPoint { x: 200.0, y: 80.0 },
            end: MapPoint { x: 200.0, y: 140.0 },
        }];
        let absent = serde_json::json!({
            "control_by_province": {
                "1": "GER",
                "2": "POL"
            }
        });
        let empty = serde_json::json!({
            "control_by_province": {
                "1": "GER",
                "2": "POL"
            },
            "front_lines": []
        });
        let mut options = OverlayOptions::default();
        options.show_front_lines = true;

        for map in [&absent, &empty] {
            let plan = build_overlay_plan(
                &st,
                OverlayInputs {
                    states: None,
                    map: Some(map),
                    divisions: None,
                    order_of_battle: None,
                    diplomacy_world: None,
                },
                &names(),
                proj(),
                options,
            );
            let fronts: Vec<_> = plan
                .lines
                .iter()
                .filter(|line| line.kind == OverlayLineKind::FrontLine)
                .collect();
            assert_eq!(fronts.len(), 1);
            assert_eq!(fronts[0].fidelity, OverlayGeometryFidelity::Derived);
            assert_eq!(plan.diagnostics.front_line_lines, 1);
            assert_eq!(plan.diagnostics.unavailable_front_lines, 0);
        }
    }

    #[test]
    fn measured_route_and_front_geometry_digest_is_deterministic() {
        let mut st = test_static();
        st.front_edges = vec![FrontLineSeed {
            province_a: 1,
            province_b: 2,
            start: MapPoint { x: 200.0, y: 80.0 },
            end: MapPoint { x: 200.0, y: 140.0 },
        }];
        let map = serde_json::json!({
            "control_by_province": {
                "1": "GER",
                "2": "POL"
            }
        });
        let diplo = serde_json::json!({
            "trade_contracts": [
                {
                    "route_geometry": {
                        "fidelity": "exact",
                        "source": "measured_save_route",
                        "map_points": [
                            { "x": 110.0, "y": 120.0 },
                            { "x": 220.0, "y": 150.0 }
                        ]
                    }
                }
            ]
        });
        let mut options = OverlayOptions::default();
        options.show_front_lines = true;
        let build = || {
            build_overlay_plan(
                &st,
                OverlayInputs {
                    states: None,
                    map: Some(&map),
                    divisions: None,
                    order_of_battle: None,
                    diplomacy_world: Some(&diplo),
                },
                &names(),
                proj(),
                options,
            )
        };
        let plan = build();
        assert_eq!(plan.diagnostics.trade_route_lines, 1);
        assert_eq!(plan.diagnostics.front_line_lines, 1);
        assert!(plan
            .lines
            .iter()
            .any(|line| line.kind == OverlayLineKind::TradeRoute
                && line.fidelity == OverlayGeometryFidelity::Exact));
        assert!(plan
            .lines
            .iter()
            .any(|line| line.kind == OverlayLineKind::FrontLine
                && line.fidelity == OverlayGeometryFidelity::Derived));
        assert_eq!(plan.digest(), build().digest());
    }

    #[test]
    fn measured_controller_boundary_draws_derived_front_line_overlay() {
        let mut st = test_static();
        st.front_edges = vec![FrontLineSeed {
            province_a: 1,
            province_b: 2,
            start: MapPoint { x: 200.0, y: 80.0 },
            end: MapPoint { x: 200.0, y: 140.0 },
        }];
        let map = serde_json::json!({
            "control_by_province": {
                "1": "GER",
                "2": "POL"
            }
        });
        let mut options = OverlayOptions::default();
        options.show_front_lines = true;
        let plan = build_overlay_plan(
            &st,
            OverlayInputs {
                states: None,
                map: Some(&map),
                divisions: None,
                order_of_battle: None,
                diplomacy_world: None,
            },
            &names(),
            proj(),
            options,
        );
        assert_eq!(plan.diagnostics.front_line_lines, 1);
        assert_eq!(plan.diagnostics.unavailable_front_lines, 0);
        assert!(
            plan.lines
                .iter()
                .any(|line| line.kind == OverlayLineKind::FrontLine && !line.arrow),
            "front overlay must draw an explicit non-arrow boundary line"
        );
        assert!(
            plan.lines
                .iter()
                .all(|line| line.fidelity == OverlayGeometryFidelity::Derived),
            "controller-boundary front geometry is derived from raster adjacency and must be labeled"
        );
    }

    #[test]
    fn front_line_overlay_missing_control_data_fails_closed() {
        let mut st = test_static();
        st.front_edges = vec![FrontLineSeed {
            province_a: 1,
            province_b: 2,
            start: MapPoint { x: 200.0, y: 80.0 },
            end: MapPoint { x: 200.0, y: 140.0 },
        }];
        let mut options = OverlayOptions::default();
        options.show_front_lines = true;
        let plan = build_overlay_plan(
            &st,
            OverlayInputs {
                states: None,
                map: None,
                divisions: None,
                order_of_battle: None,
                diplomacy_world: None,
            },
            &names(),
            proj(),
            options,
        );
        assert_eq!(plan.diagnostics.front_line_lines, 0);
        assert_eq!(plan.diagnostics.unavailable_front_lines, 1);
        assert!(
            plan.lines
                .iter()
                .all(|line| line.kind != OverlayLineKind::FrontLine),
            "missing controller data must not fabricate front-line geometry"
        );
        assert!(
            plan.markers
                .iter()
                .any(|marker| marker.kind == OverlayMarkerKind::UnavailableFrontLine),
            "missing front controller data should still render a diagnostic marker"
        );
    }

    #[test]
    fn semantic_line_styles_are_deterministic_and_fidelity_is_explicit() {
        let mut st = test_static();
        st.front_edges = vec![FrontLineSeed {
            province_a: 1,
            province_b: 2,
            start: MapPoint { x: 200.0, y: 80.0 },
            end: MapPoint { x: 200.0, y: 140.0 },
        }];
        let states = serde_json::json!({ "10": { "owner": "GER" }, "20": { "owner": "GER" } });
        let map = serde_json::json!({
            "control_by_province": {
                "1": "GER",
                "2": "POL"
            }
        });
        let divisions = serde_json::json!([
            { "div_id": 1, "div_type": 42, "location_province": 1, "movement_source_province": 1, "move_target_province": 2, "moving": true }
        ]);
        let oob = serde_json::json!({
            "armies": [
                { "id": "9:1", "members": ["1:42"], "orders": [ { "type": 5, "path": [1, 2, 3], "states": [] } ] }
            ],
            "battle_plan_orders": [
                { "army_id": "9:1", "order_type": 5, "target_states": [20] }
            ]
        });
        let diplo = serde_json::json!({
            "trade_contracts": [
                { "route_provinces": [1, 4] }
            ]
        });
        let mut options = OverlayOptions::default();
        options.show_front_lines = true;
        let build = || {
            build_overlay_plan(
                &st,
                OverlayInputs {
                    states: Some(&states),
                    map: Some(&map),
                    divisions: Some(&divisions),
                    order_of_battle: Some(&oob),
                    diplomacy_world: Some(&diplo),
                },
                &names(),
                proj(),
                options,
            )
        };
        let plan = build();
        assert_eq!(plan.diagnostics.movement_arrows, 1);
        assert_eq!(plan.diagnostics.battle_plan_arrows, 2);
        assert_eq!(plan.diagnostics.trade_route_lines, 1);
        assert_eq!(plan.diagnostics.front_line_lines, 1);
        assert!(
            plan.lines
                .iter()
                .all(|line| line.fidelity == OverlayGeometryFidelity::Derived),
            "centroid and controller-boundary overlays must remain explicitly derived"
        );

        let movement = plan
            .lines
            .iter()
            .find(|line| line.kind == OverlayLineKind::Movement)
            .expect("movement line");
        assert_eq!(movement.style, OverlayLineStyle::solid());
        assert!(movement.arrow);

        assert!(
            plan.lines
                .iter()
                .filter(|line| line.kind == OverlayLineKind::BattlePlan)
                .all(|line| line.style == OverlayLineStyle::dashed(28.0, 10.0) && line.arrow),
            "battleplan orders use the long dashed attack style"
        );
        let trade = plan
            .lines
            .iter()
            .find(|line| line.kind == OverlayLineKind::TradeRoute)
            .expect("trade route line");
        assert_eq!(trade.style, OverlayLineStyle::dashed(8.0, 12.0));
        assert!(!trade.arrow);

        let front = plan
            .lines
            .iter()
            .find(|line| line.kind == OverlayLineKind::FrontLine)
            .expect("front line");
        assert_eq!(front.style, OverlayLineStyle::dashed(12.0, 6.0));
        assert!(!front.arrow);
        assert!(front.width > movement.width);

        let digest = plan.digest();
        assert_eq!(
            digest,
            build().digest(),
            "line styles must digest deterministically"
        );
        assert!(digest.contains("Movement:Derived:Solid:0/0"));
        assert!(digest.contains("BattlePlan:Derived:Dashed:2800/1000"));
        assert!(digest.contains("TradeRoute:Derived:Dashed:800/1200"));
        assert!(digest.contains("FrontLine:Derived:Dashed:1200/600"));
    }

    #[test]
    fn explicit_movement_order_battle_plan_and_trade_route_draw() {
        let st = test_static();
        let snap_states = serde_json::json!({ "10": { "owner": "GER" }, "20": { "owner": "GER" } });
        let divisions = serde_json::json!([
            { "div_id": 1, "div_type": 42, "location_province": 1, "movement_source_province": 1, "move_target_province": 2, "moving": true }
        ]);
        let oob = serde_json::json!({
            "armies": [
                { "id": "9:1", "members": ["1:42"], "orders": [ { "type": 5, "path": [1, 2, 3], "states": [] } ] }
            ],
            "battle_plan_orders": [
                { "army_id": "9:1", "order_type": 5, "target_states": [20] }
            ]
        });
        let diplo = serde_json::json!({
            "trade_contracts": [
                { "route_provinces": [1, 4] }
            ]
        });
        let plan = build_overlay_plan(
            &st,
            OverlayInputs {
                states: Some(&snap_states),
                map: None,
                divisions: Some(&divisions),
                order_of_battle: Some(&oob),
                diplomacy_world: Some(&diplo),
            },
            &names(),
            proj(),
            OverlayOptions::default(),
        );
        assert_eq!(plan.diagnostics.movement_arrows, 1);
        assert_eq!(plan.diagnostics.battle_plan_arrows, 2);
        assert_eq!(plan.diagnostics.trade_route_lines, 1);
        assert_eq!(plan.lines.len(), 4);
        assert!(
            plan.lines
                .iter()
                .all(|line| line.fidelity == OverlayGeometryFidelity::Derived),
            "centroid polylines are derived geometry until real the game route/front paths exist"
        );
    }

    #[test]
    fn save_order_measured_polyline_draws_exact_battle_plan_path() {
        let st = test_static();
        let divisions = serde_json::json!([
            { "div_id": 1, "div_type": 42, "location_province": 1 }
        ]);
        let oob = serde_json::json!({
            "armies": [
                {
                    "id": "9:1",
                    "members": ["1:42"],
                    "orders": [
                        {
                            "type": 5,
                            "order_geometry": {
                                "fidelity": "exact",
                                "source": "measured_save_order",
                                "map_points": [
                                    [130.0, 120.0],
                                    { "x": 240.0, "y": 180.0 },
                                    { "map_x": "360.0", "map_y": "140.0" }
                                ]
                            },
                            "path": [1, 2, 3],
                            "states": [20]
                        }
                    ]
                }
            ]
        });
        let plan = build_overlay_plan(
            &st,
            OverlayInputs {
                states: None,
                map: None,
                divisions: Some(&divisions),
                order_of_battle: Some(&oob),
                diplomacy_world: None,
            },
            &names(),
            proj(),
            OverlayOptions::default(),
        );

        let battle_plan_lines: Vec<_> = plan
            .lines
            .iter()
            .filter(|line| line.kind == OverlayLineKind::BattlePlan)
            .collect();
        assert_eq!(
            battle_plan_lines.len(),
            1,
            "measured order geometry must consume the order instead of also drawing target-state fallback"
        );
        let line = battle_plan_lines[0];
        assert_eq!(line.fidelity, OverlayGeometryFidelity::Exact);
        assert_eq!(line.style, OverlayLineStyle::dashed(28.0, 10.0));
        assert!(line.arrow);
        assert_screen_points_close(
            &line.points,
            &[
                ScreenPoint { x: 130.0, y: 120.0 },
                ScreenPoint { x: 240.0, y: 180.0 },
                ScreenPoint { x: 360.0, y: 140.0 },
            ],
        );
        assert_eq!(plan.diagnostics.battle_plan_arrows, 1);
        assert_eq!(plan.diagnostics.unavailable_arrows, 0);
    }

    #[test]
    fn malformed_measured_order_path_is_unavailable_not_replaced_by_save_path_or_states() {
        let st = test_static();
        let divisions = serde_json::json!([
            { "div_id": 1, "div_type": 42, "location_province": 1 }
        ]);
        let oob = serde_json::json!({
            "armies": [
                {
                    "id": "9:1",
                    "members": ["1:42"],
                    "orders": [
                        {
                            "type": 5,
                            "order_geometry": {
                                "fidelity": "exact",
                                "source": "measured_save_order",
                                "map_points": [
                                    { "x": 130.0, "y": 120.0 },
                                    { "x": 240.0 }
                                ]
                            },
                            "path": [1, 2, 3],
                            "states": [20]
                        }
                    ]
                }
            ]
        });
        let plan = build_overlay_plan(
            &st,
            OverlayInputs {
                states: None,
                map: None,
                divisions: Some(&divisions),
                order_of_battle: Some(&oob),
                diplomacy_world: None,
            },
            &names(),
            proj(),
            OverlayOptions::default(),
        );

        assert_eq!(plan.diagnostics.battle_plan_arrows, 0);
        assert_eq!(plan.diagnostics.unavailable_arrows, 1);
        assert!(
            plan.lines
                .iter()
                .all(|line| line.kind != OverlayLineKind::BattlePlan),
            "malformed measured order geometry must not fall back to save path or target states"
        );
        let marker = plan
            .markers
            .iter()
            .find(|marker| marker.kind == OverlayMarkerKind::UnavailableBattlePlan)
            .expect("malformed measured order should render a diagnostic");
        assert_eq!(marker.text, "battleplan measured path unavailable");
        assert!(
            (marker.x - 130.0).abs() < 0.01 && (marker.y - 120.0).abs() < 0.01,
            "{marker:?}"
        );
    }

    #[test]
    fn save_order_path_with_missing_static_anchor_is_unavailable_not_replaced_by_states() {
        let st = test_static();
        let divisions = serde_json::json!([
            { "div_id": 1, "div_type": 42, "location_province": 1 }
        ]);
        let oob = serde_json::json!({
            "armies": [
                {
                    "id": "9:1",
                    "members": ["1:42"],
                    "orders": [
                        { "type": 5, "path": [1, 99, 3], "states": [20] }
                    ]
                }
            ]
        });
        let plan = build_overlay_plan(
            &st,
            OverlayInputs {
                states: None,
                map: None,
                divisions: Some(&divisions),
                order_of_battle: Some(&oob),
                diplomacy_world: None,
            },
            &names(),
            proj(),
            OverlayOptions::default(),
        );

        assert_eq!(plan.diagnostics.battle_plan_arrows, 0);
        assert_eq!(plan.diagnostics.unavailable_arrows, 1);
        assert!(
            plan.lines
                .iter()
                .all(|line| line.kind != OverlayLineKind::BattlePlan),
            "a malformed save path must not be replaced by a centroid-to-state arrow"
        );
        let marker = plan
            .markers
            .iter()
            .find(|marker| marker.kind == OverlayMarkerKind::UnavailableBattlePlan)
            .expect("missing static path anchor should render a diagnostic");
        assert_eq!(marker.text, "battleplan path anchor unavailable");
        assert_eq!((marker.x, marker.y), (100.0, 100.0));
    }

    #[test]
    fn live_oob_member_div_ids_anchor_battle_plan_orders() {
        let st = test_static();
        let divisions = serde_json::json!([
            { "div_id": 1, "location_province": 1 }
        ]);
        let oob = serde_json::json!({
            "armies": [
                { "id": 9, "member_div_ids": [1] }
            ],
            "battle_plan_orders": [
                { "army_id": 9, "order_type": 5, "target_states": [20] }
            ]
        });
        let plan = build_overlay_plan(
            &st,
            OverlayInputs {
                states: None,
                map: None,
                divisions: Some(&divisions),
                order_of_battle: Some(&oob),
                diplomacy_world: None,
            },
            &names(),
            proj(),
            OverlayOptions::default(),
        );

        assert_eq!(
            plan.diagnostics.battle_plan_arrows, 1,
            "live oob_command member_div_ids must anchor live battle_plan_orders"
        );
        assert_eq!(plan.diagnostics.unavailable_arrows, 0);
        let line = plan
            .lines
            .iter()
            .find(|line| line.kind == OverlayLineKind::BattlePlan)
            .expect("battle plan line");
        assert_eq!(line.fidelity, OverlayGeometryFidelity::Derived);
        assert!(
            line.id.starts_with("battleplan:0:20"),
            "target-state order line keeps the live battle-plan identity"
        );
    }

    #[test]
    fn offscreen_labels_and_long_segments_are_bounded() {
        let st = test_static();
        let states = serde_json::json!({ "10": { "owner": "GER" }, "20": { "owner": "GER" } });
        let divisions = serde_json::json!([
            { "div_id": 1, "location_province": 1, "movement_source_province": 1, "move_target_province": 4 }
        ]);
        let mut options = OverlayOptions::default();
        options.max_segment_px = 100.0;
        let plan = build_overlay_plan(
            &st,
            OverlayInputs {
                states: Some(&states),
                map: None,
                divisions: Some(&divisions),
                order_of_battle: None,
                diplomacy_world: None,
            },
            &names(),
            proj(),
            options,
        );
        assert_eq!(plan.diagnostics.movement_arrows, 0);
        assert_eq!(plan.diagnostics.culled_lines, 1);
    }

    #[test]
    fn deterministic_digest_round_trips_a_b_a() {
        let st = test_static();
        let states_a = serde_json::json!({ "10": { "owner": "GER" }, "20": { "owner": "GER" } });
        let states_b = serde_json::json!({ "10": { "owner": "ITA" }, "20": { "owner": "GER" } });
        let divisions = serde_json::json!([
            { "div_id": 1, "div_type": 42, "location_province": 1, "movement_source_province": 1, "move_target_province": 2 }
        ]);
        let build = |states: &Value| {
            build_overlay_plan(
                &st,
                OverlayInputs {
                    states: Some(states),
                    map: None,
                    divisions: Some(&divisions),
                    order_of_battle: None,
                    diplomacy_world: None,
                },
                &names(),
                proj(),
                OverlayOptions::default(),
            )
            .digest()
        };
        let a1 = build(&states_a);
        let b = build(&states_b);
        let a2 = build(&states_a);
        assert_eq!(a1, a2, "same scrub state must rebuild identically");
        assert_ne!(a1, b, "different ownership should perturb overlay digest");
    }

    fn assert_screen_points_close(actual: &[ScreenPoint], expected: &[ScreenPoint]) {
        assert_eq!(actual.len(), expected.len());
        for (actual, expected) in actual.iter().zip(expected.iter()) {
            assert!(
                (actual.x - expected.x).abs() < 0.01 && (actual.y - expected.y).abs() < 0.01,
                "{actual:?} != {expected:?}"
            );
        }
    }
}
