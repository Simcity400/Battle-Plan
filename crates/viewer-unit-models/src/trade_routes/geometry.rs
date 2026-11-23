
use super::{LAND_ROUTE, NAVAL_ROUTE, NO_ROUTE, RouteFields};
use dashboard_core::protocol::PlanKind;
use content_reader_map::{adjacencies::SEA_CROSSING, province_geometry::ProvinceGeometry, province_graph::{ProvinceGraph, heap_pop, heap_push}};
use std::collections::{BTreeMap, BTreeSet};
use viewer_gpu::{
    province_map::HeightField,
    ribbon_content::{ArrowType, ExecutionState, OrderDraw, OrderField, OrderVertex},
    ribbon_fixed::{self as fixed, Point, Smoothing},
    ribbon_mesh::{ribbon_height, span},
    ribbon_native as native,
    unit_pose::{fixed as to_fixed, unwrap},
};
const ARROW_SMOOTHNESS: i64 = 75_000;

pub struct RouteContent<'a> {
    pub graph: &'a ProvinceGraph,
    pub geography: &'a ProvinceGeometry,
    pub region_of: &'a BTreeMap<u32, u32>,
    pub region_provinces: &'a BTreeMap<u32, Vec<u32>>,
    pub state_provinces: &'a BTreeMap<u32, Vec<u32>>,
    pub state_of: &'a BTreeMap<u32, u32>,
    pub ports: &'a BTreeMap<u32, u32>,
    pub pixels: &'a dyn Fn(u32) -> Option<u32>,
    pub position: &'a dyn Fn(u32) -> Option<[f32; 3]>,
    pub crossing: &'a dyn Fn(u32, u32) -> Option<[i32; 2]>,
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Stops {
    pub provinces: Vec<u32>,
    pub regions: Vec<u32>,
}

pub fn border_province(from: u32, to: u32, content: &RouteContent<'_>) -> Result<u32, String> {
    let members = content
        .region_provinces
        .get(&from)
        .ok_or_else(|| format!("sea region {from} has no provinces"))?;
    for &province in members {
        for &(neighbour, _) in content.graph.edges.get(&province).into_iter().flatten() {
            if content.region_of.get(&neighbour) != Some(&to) {
                continue;
            }
            if content
                .graph
                .crossing(province, neighbour)
                .is_some_and(|c| c.kind == SEA_CROSSING || c.rule.is_some())
            {
                return Err(format!(
                    "restricted trade crossing {province}/{neighbour}: endpoint controllers' trade permission and fleet blockade are not recorded"
                ));
            }
            return Ok(province);
        }
    }
    Err(format!("sea regions {from} and {to} share no border"))
}

pub fn stops(route: &impl RouteFields, c: &RouteContent<'_>) -> Result<Option<Stops>, String> {
    directed_stops(route, c, false)
}

pub fn directed_stops(
    route: &impl RouteFields,
    c: &RouteContent<'_>,
    reverse: bool,
) -> Result<Option<Stops>, String> {
    if route.kind() == NO_ROUTE || route.path().is_empty() {
        return Ok(None);
    }
    let naval = route.kind() == NAVAL_ROUTE;
    if !naval && route.kind() != LAND_ROUTE {
        return Err(format!("trade route kind {}", route.kind()));
    }
    let first = |state: u32| {
        c.state_provinces
            .get(&state)
            .and_then(|p| p.first())
            .copied()
            .ok_or_else(|| format!("state {state} has no provinces"))
    };
    let (source, target) = if naval {
        (route.from(), route.to())
    } else {
        (first(route.from())?, first(route.to())?)
    };
    let (source, target) = if reverse {
        (target, source)
    } else {
        (source, target)
    };
    let mut path = route.path().to_vec();
    if reverse {
        path.reverse();
    }
    let mut allowed: BTreeSet<u32> = path.iter().copied().collect();
    if naval {
        for pair in path.windows(2) {
            border_province(pair[0], pair[1], c)?;
        }
        for p in [source, target] {
            allowed.insert(
                *c.region_of
                    .get(&p)
                    .ok_or_else(|| format!("province {p} has no strategic region"))?,
            );
        }
    }
    let admitted = |p| {
        if naval {
            c.region_of.get(&p)
        } else {
            c.state_of.get(&p)
        }
    };
    let centroid = |p| {
        c.geography
            .centroids
            .get(&p)
            .copied()
            .ok_or_else(|| format!("province {p} has no centroid"))
    };
    let heuristic = |p| -> Result<i64, String> {
        let ([x, z], [tx, tz]) = (centroid(p)?, centroid(target)?);
        let dx = (x - tx).abs();
        let dx = dx.min((c.geography.width - dx).abs());
        Ok(
            (((i64::from(dx) * i64::from(dx) + i64::from(z - tz).pow(2)) as f64)
                .sqrt()
                .floor() as i64)
                * 100_000,
        )
    };
    let mut heap = Vec::new();
    let mut closed = BTreeSet::new();
    let mut costs = BTreeMap::from([(source, 0_i64)]);
    let mut previous = BTreeMap::new();
    heap_push(&mut heap, (0, source));
    while !heap.is_empty() {
        let from = heap_pop(&mut heap);
        if !closed.insert(from) {
            continue;
        }
        if from == target {
            let mut provinces = vec![target];
            while *provinces.last().unwrap() != source {
                provinces.push(previous[provinces.last().unwrap()]);
            }
            provinces.reverse();
            let regions = if naval {
                provinces
                    .iter()
                    .map(|province| c.region_of[province])
                    .collect()
            } else {
                Vec::new()
            };
            return Ok(Some(Stops { provinces, regions }));
        }
        for &(to, distance) in c.graph.edges.get(&from).into_iter().flatten() {
            if closed.contains(&to)
                || !admitted(from).is_some_and(|r| allowed.contains(r))
                || !admitted(to).is_some_and(|r| allowed.contains(r))
            {
                continue;
            }
            if naval {
                let land = |p| c.geography.land.contains(&p);
                if (land(from) && land(to))
                    || (land(from) && c.ports.get(&from) != Some(&to))
                    || (land(to) && c.ports.get(&to) != Some(&from))
                {
                    continue;
                }
            }
            if let Some(rule) = c
                .graph
                .crossing(from, to)
                .and_then(|crossing| crossing.rule.as_ref())
            {
                return Err(format!(
                    "trade adjacency rule {rule}: both endpoint controllers' trade permission is unavailable"
                ));
            }
            let distance = distance.ok_or_else(|| {
                format!(
                    "trade edge {from}/{to}: {}",
                    c.graph.unavailable_reason(from, to)
                )
            })?;
            let pixels =
                (c.pixels)(to).ok_or_else(|| format!("province {to} has no pixel count"))?;
            let next = costs[&from]
                .wrapping_add((i64::from(pixels) + 1).wrapping_mul(100_000))
                .wrapping_add(distance);
            if costs.get(&to).is_none_or(|&old| next < old) {
                costs.insert(to, next);
                previous.insert(to, from);
                heap_push(&mut heap, (next.wrapping_add(heuristic(to)?), to));
            }
        }
    }
    Ok(None)
}

pub fn province_uses(stops: &Stops, uses: &mut BTreeMap<u32, u8>) -> Vec<u8> {
    stops
        .provinces
        .iter()
        .map(|&p| {
            let count = uses.entry(p).or_default();
            let old = *count;
            *count = count.wrapping_add(1);
            old
        })
        .collect()
}
pub fn overlap_offset(index: u32, max_lines: u32, line_offset: f32) -> f32 {
    let slot = index % max_lines.max(1);
    (if slot.is_multiple_of(2) {
        slot as f32
    } else {
        -(slot as f32)
    }) * line_offset
}

pub fn points(
    stops: &Stops,
    c: &RouteContent<'_>,
    width: f32,
    uses: &[u8],
    max: u32,
    offset: f32,
) -> Result<Vec<[f32; 2]>, String> {
    let mut out: Vec<[f32; 3]> = Vec::new();
    let mut place = |mut p: [f32; 3], count: u8, keep: bool| {
        if let Some(last) = out.last() {
            p[0] = unwrap(p[0], last[0], width);
            if !keep {
                let d = [p[0] - last[0], p[1] - last[1], p[2] - last[2]];
                let length = d.iter().map(|v| v * v).sum::<f32>().sqrt();
                if length > 0.0 {
                    let amount = overlap_offset(u32::from(count), max, offset);
                    p[0] -= d[2] / length * amount;
                    p[2] += d[0] / length * amount;
                }
            }
        }
        out.push(p);
    };
    for (index, &province) in stops.provinces.iter().enumerate() {
        let point = (c.position)(province)
            .ok_or_else(|| format!("province {province} has no trade anchor"))?;
        let count = uses.get(index).copied().unwrap_or(0);
        place(
            point,
            count,
            index == 0 || (c.geography.land.contains(&province) && c.ports.contains_key(&province)),
        );
        if let Some(&next) = stops.provinces.get(index + 1) {
            let [x, z] = (c.crossing)(province, next)
                .ok_or_else(|| format!("trade edge {province}/{next} has no raw crossing"))?;
            place([x as f32, 0.0, z as f32], count, false);
        }
    }
    Ok(out.into_iter().map(|p| [p[0], p[2]]).collect())
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Shaped {
    pub controls: Vec<Point>,
    pub variants: Vec<u8>,
}

fn subdivide(controls: &mut Vec<Point>, variants: &mut Vec<u8>) {
    if controls.len() < 2 {
        return;
    }
    fixed::subdivide(controls);
    let mut out = Vec::with_capacity(variants.len() * 2);
    for (index, &variant) in variants.iter().enumerate() {
        if index > 0 {
            out.push(variants[index - 1].max(variant));
        }
        out.push(variant);
    }
    *variants = out;
}

pub fn shape(points: &[[f32; 2]], variants: &[u8], reverse: bool, smoothing: Smoothing) -> Shaped {
    assert_eq!(points.len(), variants.len());
    let mut controls: Vec<Point> = points.iter().map(|&[x, z]| to_fixed([x, 0.0, z])).collect();
    let mut variants = variants.to_vec();
    if reverse {
        controls.reverse();
        variants.reverse();
    }
    fixed::smooth(&mut controls, smoothing.smoothness);
    for _ in 0..smoothing.passes {
        subdivide(&mut controls, &mut variants);
        fixed::smooth(&mut controls, smoothing.smoothness);
    }
    Shaped { controls, variants }
}

pub fn normal(points: &[[f32; 2]], index: usize) -> (f32, f32) {
    let p = points[index];
    let previous = points[index.saturating_sub(1)];
    let next = points[(index + 1).min(points.len() - 1)];
    let direction = |a: [f32; 2], b: [f32; 2]| {
        let l = span(a, b);
        if l > 1e-6 {
            [(b[0] - a[0]) / l, (b[1] - a[1]) / l]
        } else {
            [0.0, 0.0]
        }
    };
    let a = direction(previous, p);
    let b = direction(p, next);
    let t = [a[0] + b[0], a[1] + b[1]];
    let l = span([0.0, 0.0], t);
    if l > 1e-6 {
        (-t[1] / l, t[0] / l)
    } else {
        (0.0, 1.0)
    }
}

pub fn emit(
    field: &mut OrderField,
    arrow: &ArrowType,
    shaped: &Shaped,
    color: [f32; 4],
    hotspot: [f32; 4],
    heights: &HeightField<'_>,
) -> Option<Vec<[f32; 2]>> {
    let scale = arrow.global_scale(shaped.controls.len());
    let mut smoothed = shaped.controls.clone();
    fixed::smooth(&mut smoothed, ARROW_SMOOTHNESS);
    let points: Vec<[f32; 2]> = smoothed.into_iter().map(fixed::to_world).collect();
    if points.len() < 2 {
        return None;
    }
    let mut lengths = Vec::with_capacity(points.len());
    let mut total = 0.0f32;
    lengths.push(0.0);
    for index in 1..points.len() {
        total += span(points[index - 1], points[index]);
        lengths.push(total);
    }
    if total <= 0.0 {
        return None;
    }
    let uv_to_length = if arrow.texture_loop != 0.0 {
        arrow.texture_loop
    } else {
        total
    };
    let base = field.vertices.len();
    for (index, &point) in points.iter().enumerate() {
        let along = lengths[index];
        let half = (arrow.body_width[0]
            + (arrow.body_width[1] - arrow.body_width[0]) * (along / total))
            * scale;
        let variant = shaped.variants.get(index).copied().unwrap_or(0);
        let uv_scale = if variant == 0 {
            arrow.body_uv_scale
        } else {
            arrow.secondary_body_uv_scale
        };
        let reach = half * uv_scale;
        let (nx, nz) = normal(&points, index);
        for (side, v) in [(1.0f32, 0.0f32), (-1.0, 1.0)] {
            let x = point[0] + nx * reach * side;
            let z = point[1] + nz * reach * side;
            field.vertices.push(OrderVertex {
                position: [x, ribbon_height(heights, x, z), z],
                uv_head_variant: [along / uv_to_length, v, 0.0, f32::from(variant)],
            });
        }
    }
    let bounds = native::bounds(&field.vertices[base..]);
    let (vertices, mut native_strips) =
        native::vertices(&field.vertices[base..], points.len(), false);
    let native_base = field.native_vertices.len() as u32;
    for strip in &mut native_strips {
        strip.start += native_base;
        strip.end += native_base;
    }
    field.native_vertices.extend(vertices);
    field.draws.push(OrderDraw {
        air_mode: false,
        arrow_type: arrow.name.clone(),
        texture_choice: 0,
        color,
        total_length: total,
        uv_to_length,
        culling_offset: f32::from(u8::from(arrow.culling_direction != 0)),
        declaration_index: arrow.declaration_index,
        width: arrow.body_width[0].max(arrow.body_width[1]),
        label: None,
        native_strips,
        bounds,
        secondary_color: hotspot,
        outer_color: [1.0, 1.0, 1.0, color[3]],
        execution: ExecutionState::default(),
        kind: PlanKind::TradeRoute,
    });
    Some(points)
}
