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

#[derive(Debug, Clone, Default, PartialEq)]
pub struct OverlayStatic {
    pub province_anchors: BTreeMap<u32, MapPoint>,
    pub state_anchors: BTreeMap<u32, MapPoint>,
    pub region_anchors: BTreeMap<u32, MapPoint>,
    pub states: Vec<StateLabelSeed>,
    pub regions: Vec<RegionLabelSeed>,
}

impl OverlayStatic {
    pub fn load(map_dir: &Path, raster: &ProvinceRaster) -> Result<OverlayStatic, String> {
        let province_anchors = province_centroids(raster);
        let mut state_anchors = BTreeMap::new();
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
    TradeRoute,
    Movement,
    BattlePlan,
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
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct OverlayDiagnostics {
    pub country_labels: u32,
    pub state_labels: u32,
    pub strategic_region_labels: u32,
    pub movement_arrows: u32,
    pub battle_plan_arrows: u32,
    pub trade_route_lines: u32,
    pub unavailable_arrows: u32,
    pub unavailable_routes: u32,
    pub culled_labels: u32,
    pub culled_lines: u32,
}

#[derive(Debug, Clone, Default, PartialEq)]
pub struct OverlayPlan {
    pub labels: Vec<OverlayLabel>,
    pub lines: Vec<OverlayLine>,
    pub diagnostics: OverlayDiagnostics,
}

impl OverlayPlan {
    pub fn digest(&self) -> String {
        let mut out = format!(
            "lbl={}/{}/{} line={}/{}/{} unavail={}/{} cull={}/{}",
            self.diagnostics.country_labels,
            self.diagnostics.state_labels,
            self.diagnostics.strategic_region_labels,
            self.diagnostics.movement_arrows,
            self.diagnostics.battle_plan_arrows,
            self.diagnostics.trade_route_lines,
            self.diagnostics.unavailable_arrows,
            self.diagnostics.unavailable_routes,
            self.diagnostics.culled_labels,
            self.diagnostics.culled_lines
        );
        for line in &self.lines {
            out.push_str("|L:");
            out.push_str(&format!("{:?}:{}:", line.kind, line.id));
            for p in &line.points {
                out.push_str(&format!("{},{};", q(p.x), q(p.y)));
            }
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
            }
            continue;
        };
        let Some(source_pid) = u32_field(div, &["movement_source_province"]) else {
            plan.diagnostics.unavailable_arrows += 1;
            continue;
        };
        let Some(source) = static_data.province_anchors.get(&source_pid).copied() else {
            plan.diagnostics.unavailable_arrows += 1;
            continue;
        };
        let Some(target) = static_data.province_anchors.get(&target_pid).copied() else {
            plan.diagnostics.unavailable_arrows += 1;
            continue;
        };
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
        let members = army.get("members").and_then(Value::as_array);
        let Some(anchor) = members.and_then(|m| average_member_anchor(m, division_anchors)) else {
            continue;
        };
        for key in army_keys(army, idx) {
            out.insert(key, anchor);
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
                }
                continue;
            }
            let Some(army_anchor) = u32_or_string_key(order.get("army_id"))
                .into_iter()
                .find_map(|k| army_anchors.get(&k).copied())
            else {
                plan.diagnostics.unavailable_arrows += 1;
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
            }
        }
    }
}

fn add_explicit_order_path(
    static_data: &OverlayStatic,
    order: &Value,
    id: &str,
    projection: &OverlayProjection,
    options: &OverlayOptions,
    plan: &mut OverlayPlan,
) -> bool {
    let provinces = u32_array(order.get("path"));
    let Some(points) = path_anchors(&provinces, &static_data.province_anchors) else {
        return false;
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
        },
        &points,
        projection,
        options,
    )
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
        let provinces = ["route_provinces", "route", "path", "provinces"]
            .iter()
            .find_map(|field| {
                let p = u32_array(contract.get(*field));
                (p.len() >= 2).then_some(p)
            });
        let Some(provinces) = provinces else {
            plan.diagnostics.unavailable_routes += 1;
            continue;
        };
        let Some(route) = path_anchors(&provinces, &static_data.province_anchors) else {
            plan.diagnostics.unavailable_routes += 1;
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
        OverlayLineKind::Movement => plan.diagnostics.movement_arrows += 1,
        OverlayLineKind::BattlePlan => plan.diagnostics.battle_plan_arrows += 1,
        OverlayLineKind::TradeRoute => plan.diagnostics.trade_route_lines += 1,
    }
    plan.lines.push(line);
    true
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

fn label_sort_key(label: &OverlayLabel) -> (OverlayLabelKind, String) {
    (label.kind.clone(), label.id.clone())
}

fn distance(a: ScreenPoint, b: ScreenPoint) -> f32 {
    let dx = a.x - b.x;
    let dy = a.y - b.y;
    (dx * dx + dy * dy).sqrt()
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
}
