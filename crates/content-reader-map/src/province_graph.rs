
#[path = "province_graph/terrain_cost.rs"]
mod terrain_cost;

use std::{
    collections::{BTreeMap, HashMap},
    io,
};

pub use crate::adjacencies::{IMPASSABLE, SEA_CROSSING};
use content_reader_layers::ResolvedLayers;
use crate::{adjacencies::{self, Adjacency}, adjacency_rules::AdjacencyRule, province_geometry::{EngineMap, ProvinceGeometry, engine_map}};
use content_reader_parser::fields::invalid;
use terrain_cost::TerrainCosts;

#[derive(Clone, Debug, Default)]
pub struct ProvinceGraph {
    pub edges: BTreeMap<u32, Vec<(u32, Option<i64>)>>,
    pub unavailable: BTreeMap<(u32, u32), String>,
    pub crossings: BTreeMap<(u32, u32), Crossing>,
    pub rules: Vec<AdjacencyRule>,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Crossing {
    pub kind: u8,
    pub through: i32,
    pub rule: Option<String>,
}

impl ProvinceGraph {
    pub fn read(layers: &ResolvedLayers) -> io::Result<(Self, ProvinceGeometry)> {
        let files = content_reader_key_names::terrain::read_default_map(layers)?;
        let map = engine_map(
            &layers.read(&files.file("definitions")?)?,
            &layers.read(&files.file("provinces")?)?,
        )?;
        let path = crate::adjacency_rules::map_file(&files, "adjacencies")?;
        let rows = adjacencies::parse(
            &layers
                .read(&path)
                .map_err(|error| io::Error::new(error.kind(), format!("{path}: {error}")))?,
        )
        .map_err(|error| invalid(format!("{path}: {error}")))?;
        let geometry = ProvinceGeometry::from_map(&map);
        let terrain = TerrainCosts::read(layers, &files)?;
        let rules = crate::adjacency_rules::read_map(layers, &files)?;
        let mut graph = derive(&map, &geometry, &rows, &terrain)?;
        graph.crossings = authored_crossings(&rows, &rules)
            .map_err(|error| invalid(format!("{path}: {error}")))?;
        graph.rules = rules;
        Ok((graph, geometry))
    }

    pub fn crossing(&self, from: u32, to: u32) -> Option<&Crossing> {
        self.crossings.get(&(from, to))
    }

    pub fn unavailable_reason(&self, a: u32, b: u32) -> &str {
        self.unavailable
            .get(&(a.min(b), a.max(b)))
            .map_or("its terrain cost is unavailable", String::as_str)
    }

    pub fn neighbor_states(
        &self,
        members: &BTreeMap<u32, Vec<u32>>,
    ) -> io::Result<BTreeMap<u32, Vec<u32>>> {
        let mut state_of: HashMap<u32, u32> = HashMap::new();
        for (&state, provinces) in members {
            if state == 0 || state > i16::MAX as u32 {
                return Err(invalid(format!(
                    "state id {state} does not fit the engine's 16-bit province state table"
                )));
            }
            for &province in provinces {
                if !self.edges.contains_key(&province) {
                    return Err(invalid(format!(
                        "state {state} lists province {province}, which the map does not define"
                    )));
                }
                state_of.insert(province, state);
            }
        }
        Ok(members
            .iter()
            .map(|(&state, provinces)| {
                let mut neighbours = Vec::new();
                for province in provinces {
                    for (next, _) in &self.edges[province] {
                        if let Some(&other) = state_of.get(next)
                            && other != state
                            && !neighbours.contains(&other)
                        {
                            neighbours.push(other);
                        }
                    }
                }
                (state, neighbours)
            })
            .collect())
    }

    pub fn distance(&self, from: u32, to: u32) -> Option<i64> {
        match self.edges.get(&from)?.iter().find(|(id, _)| *id == to) {
            Some((_, distance)) => *distance,
            None => Some(100_000),
        }
    }
}

fn append_target(
    targets: &mut BTreeMap<u32, Vec<u32>>,
    source: u32,
    target: u32,
) -> io::Result<()> {
    let list = targets
        .get_mut(&source)
        .ok_or_else(|| invalid("native adjacency template identity unavailable"))?;
    if !list.contains(&target) {
        list.push(target);
    }
    Ok(())
}

fn append_pair(targets: &mut BTreeMap<u32, Vec<u32>>, source: u32, target: u32) -> io::Result<()> {
    if target == 0 || source == target {
        return Ok(());
    }
    if !targets
        .get(&source)
        .ok_or_else(|| invalid("native adjacency template identity unavailable"))?
        .contains(&target)
    {
        append_target(targets, target, source)?;
        append_target(targets, source, target)?;
    }
    Ok(())
}

fn at(map: &EngineMap, x: i32, y: i32) -> u32 {
    if x < 0 || y < 0 || x as usize >= map.width || y as usize >= map.height {
        return 0;
    }
    map.ids[(map.height - 1 - y as usize) * map.width + x as usize] as u16 as u32
}

fn ordered_bitmap_targets(map: &EngineMap) -> io::Result<BTreeMap<u32, Vec<u32>>> {
    let (width, height) = (map.width as i32, map.height as i32);
    let mut targets: BTreeMap<_, _> = map.defined.iter().map(|&id| (id, Vec::new())).collect();
    for y in 0..height {
        for x in 0..width {
            let source = at(map, x, y);
            if source == 0 {
                continue;
            }
            let east = if x + 1 == width { 0 } else { x + 1 };
            let right = at(map, east, y);
            let down = at(map, x, y + 1);
            let left = at(map, if x == 0 { width - 1 } else { x - 1 }, y);
            let up = at(map, x, y - 1);
            for target in [right, down, left, up] {
                append_pair(&mut targets, source, target)?;
            }
            let diagonal = at(map, east, y + 1);
            let four = [source, right, down, diagonal];
            if four.iter().all(|&id| id != 0)
                && (0..4).all(|a| (a + 1..4).all(|b| four[a] != four[b]))
            {
                append_pair(&mut targets, source, diagonal)?;
            }
        }
    }
    Ok(targets)
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum RaidPath {
    Naval,
    Land,
}

#[derive(Clone, Copy)]
struct Node {
    f: i64,
    g: i64,
}

pub fn raid_distance(
    graph: &ProvinceGraph,
    geometry: &ProvinceGeometry,
    from: u32,
    to: u32,
    path: RaidPath,
) -> Result<Option<i64>, String> {
    raid_search(graph, geometry, from, to, path, None)
}

pub fn raid_route(
    graph: &ProvinceGraph,
    geometry: &ProvinceGeometry,
    from: u32,
    to: u32,
    path: RaidPath,
) -> Result<Option<(i64, Vec<u32>)>, String> {
    let mut parents = HashMap::new();
    let Some(distance) = raid_search(graph, geometry, from, to, path, Some(&mut parents))? else {
        return Ok(None);
    };
    let mut route = vec![to];
    let mut at = to;
    while at != from {
        at = *parents
            .get(&at)
            .ok_or_else(|| format!("raid path province {at} has no predecessor"))?;
        route.push(at);
    }
    route.reverse();
    Ok(Some((distance, route)))
}

fn raid_search(
    graph: &ProvinceGraph,
    geometry: &ProvinceGeometry,
    from: u32,
    to: u32,
    path: RaidPath,
    mut parents: Option<&mut HashMap<u32, u32>>,
) -> Result<Option<i64>, String> {
    const UNSEEN: i64 = 0x53e2_d620_fb40;
    let width = geometry.width;
    let target = geometry
        .centroids
        .get(&to)
        .copied()
        .ok_or_else(|| format!("raid path target {to} has no centroid"))?;
    let flag = |id: u32| {
        geometry
            .kind_flags(id)
            .ok_or_else(|| format!("province {id} is not defined"))
    };
    let mut nodes: HashMap<u32, Node> = HashMap::new();
    let mut closed = std::collections::HashSet::new();
    let mut heap: Vec<(i64, u32)> = Vec::new();
    closed.insert(from);
    nodes.insert(from, Node { f: 0, g: 0 });
    heap_push(&mut heap, (0, from));
    while !heap.is_empty() {
        let id = heap_pop(&mut heap);
        let current = nodes[&id];
        if id == to {
            return Ok(Some(current.g));
        }
        closed.insert(id);
        let neighbours = graph
            .edges
            .get(&id)
            .ok_or_else(|| format!("raid path province {id} has no neighbour table"))?;
        for &(next, _) in neighbours {
            if closed.contains(&next) {
                continue;
            }
            let admitted = match path {
                RaidPath::Naval => next == to || flag(next)? & 3 == 0,
                RaidPath::Land => flag(next)? & 1 != 0,
            };
            if !admitted {
                continue;
            }
            let cost = graph.distance(id, next).ok_or_else(|| {
                format!(
                    "raid path edge {id}->{next} distance is unavailable: {}",
                    graph.unavailable_reason(id, next)
                )
            })?;
            if cost < 0 {
                continue;
            }
            let first = !nodes.contains_key(&next);
            let node = nodes.entry(next).or_insert(Node {
                f: UNSEEN,
                g: UNSEEN,
            });
            let g = current.g.wrapping_add(cost);
            if g < node.g {
                let h = if first {
                    let [x, y] = geometry.centroids.get(&next).copied().unwrap_or_default();
                    let mut dx = target[0].wrapping_sub(x);
                    if dx < -(width / 2) {
                        dx += width;
                    }
                    if width / 2 < dx {
                        dx -= width;
                    }
                    let dy = target[1].wrapping_sub(y);
                    i64::from(isqrt(dy.wrapping_mul(dy).wrapping_add(dx.wrapping_mul(dx))))
                        .wrapping_mul(100_000)
                } else {
                    node.f.wrapping_sub(node.g)
                };
                *node = Node {
                    f: h.wrapping_add(g),
                    g,
                };
                if let Some(parents) = parents.as_deref_mut() {
                    parents.insert(next, id);
                }
                heap_push(&mut heap, (node.f, next));
            }
        }
    }
    Ok(None)
}

fn isqrt(squared: i32) -> i32 {
    let mut root = squared;
    let mut next = squared / 2;
    if squared > 1 {
        loop {
            root = next;
            next = (squared / root + root) / 2;
            if next >= root {
                break;
            }
        }
    }
    root
}

pub fn heap_pop(heap: &mut Vec<(i64, u32)>) -> u32 {
    let id = heap[0].1;
    if heap.len() < 2 {
        heap.clear();
    } else {
        let last = heap.pop().expect("a heap of two or more");
        heap[0] = last;
        sift_down(heap);
    }
    id
}

pub fn heap_push(heap: &mut Vec<(i64, u32)>, entry: (i64, u32)) {
    heap.push(entry);
    let mut index = heap.len() - 1;
    while index > 0 {
        let parent = (index - 1) / 2;
        if heap[parent].0 < heap[index].0 {
            break;
        }
        heap.swap(parent, index);
        index = parent;
    }
}

fn sift_down(heap: &mut [(i64, u32)]) {
    let mut index = 0;
    loop {
        let (left, right) = (2 * index + 1, 2 * index + 2);
        if left >= heap.len() {
            return;
        }
        if right >= heap.len() {
            if heap[left].0 <= heap[index].0 {
                heap.swap(left, index);
            }
            return;
        }
        if heap[index].0 < heap[left].0 && heap[index].0 < heap[right].0 {
            return;
        }
        let child = if heap[left].0 < heap[right].0 {
            left
        } else {
            right
        };
        heap.swap(child, index);
        index = child;
    }
}

fn add_authored_targets(
    targets: &mut BTreeMap<u32, Vec<u32>>,
    rows: &[Adjacency],
) -> io::Result<()> {
    for row in rows {
        let (source, target) = (row.from, row.to);
        if target == 0 || !targets.contains_key(&source) || !targets.contains_key(&target) {
            return Err(invalid(format!(
                "authored adjacency {source};{target} names an unavailable province"
            )));
        }
        append_target(targets, source, target)?;
        append_target(targets, target, source)?;
    }
    Ok(())
}

fn border_pixels(
    map: &EngineMap,
    targets: &BTreeMap<u32, Vec<u32>>,
) -> HashMap<(u32, u32), Vec<[i32; 2]>> {
    let mut borders: HashMap<(u32, u32), Vec<[i32; 2]>> = targets
        .iter()
        .flat_map(|(&source, neighbours)| {
            neighbours
                .iter()
                .filter(move |&&target| source < target)
                .map(move |&target| ((source, target), Vec::new()))
        })
        .collect();
    let width = map.width as i32;
    for y in 0..map.height {
        let row = &map.ids[(map.height - 1 - y) * map.width..(map.height - y) * map.width];
        let mut x = 0;
        while x < map.width {
            let start = x;
            let id = row[x];
            while x < map.width && row[x] == id {
                x += 1;
            }
            let source = id as u16 as u32;
            for dx in 0..(x - start) as u16 {
                let p = [i32::from(start as u16) + i32::from(dx), i32::from(y as u16)];
                let mut seen = [0_u32; 4];
                for (slot, d) in [[0, -1], [-1, 0], [1, 0], [0, 1]].iter().enumerate() {
                    let target = at(map, (p[0] + d[0]).rem_euclid(width), p[1] + d[1]);
                    seen[slot] = target;
                    if target <= source || seen[..slot].contains(&target) {
                        continue;
                    }
                    if let Some(border) = borders.get_mut(&(source, target)) {
                        border.push(p);
                    }
                }
            }
        }
    }
    borders
}

fn first_largest_component(pixels: &[[i32; 2]]) -> Vec<[i32; 2]> {
    let mut by_coordinate: HashMap<[i32; 2], Vec<usize>> = HashMap::new();
    for (index, &point) in pixels.iter().enumerate() {
        by_coordinate.entry(point).or_default().push(index);
    }
    let mut seen = vec![false; pixels.len()];
    let mut best = Vec::new();
    let mut candidates = Vec::new();
    for seed in 0..pixels.len() {
        if seen[seed] {
            continue;
        }
        seen[seed] = true;
        let mut pending = vec![seed];
        let mut component = Vec::new();
        while let Some(index) = pending.pop() {
            let p = pixels[index];
            component.push(p);
            candidates.clear();
            for dy in -1..=1 {
                for dx in -1..=1 {
                    if let Some(indices) =
                        by_coordinate.get(&[p[0].wrapping_add(dx), p[1].wrapping_add(dy)])
                    {
                        candidates.extend(
                            indices
                                .iter()
                                .copied()
                                .filter(|&next| next > seed && !seen[next]),
                        );
                    }
                }
            }
            candidates.sort_unstable();
            for &next in &candidates {
                if !seen[next] {
                    seen[next] = true;
                    pending.push(next);
                }
            }
        }
        if component.len() > best.len() {
            best = component;
        }
    }
    best
}

fn component_crossing(component: &[[i32; 2]]) -> io::Result<[i32; 2]> {
    let divisor = i64::try_from(component.len())
        .ok()
        .and_then(|count| count.checked_mul(100_000))
        .ok_or_else(|| invalid("adjacency border divisor overflows"))?;
    let mut sums = [0_i64; 2];
    for point in component {
        for axis in 0..2 {
            let contribution = i64::from(point[axis])
                .checked_mul(10_000_000_000)
                .ok_or_else(|| invalid("adjacency border coordinate overflows"))?
                / divisor;
            sums[axis] = sums[axis]
                .checked_add(contribution)
                .ok_or_else(|| invalid("adjacency border sum overflows"))?;
        }
    }
    Ok(sums.map(|value| ((value + if value < 0 { -50_000 } else { 50_000 }) / 100_000) as i32))
}

fn native_line(a: [i32; 2], b: [i32; 2], capacity: usize) -> Vec<[i32; 2]> {
    let steep = b[0].wrapping_sub(a[0]).wrapping_abs() < b[1].wrapping_sub(a[1]).wrapping_abs();
    let (mut a, mut b) = if steep {
        ([a[1], a[0]], [b[1], b[0]])
    } else {
        (a, b)
    };
    if a[0] > b[0] {
        std::mem::swap(&mut a, &mut b);
    }
    let dx = b[0].wrapping_sub(a[0]);
    let dy = b[1].wrapping_sub(a[1]).wrapping_abs();
    let step = if a[1] < b[1] { 1 } else { -1 };
    let mut y = a[1];
    let mut error = 0_i32;
    let mut out = Vec::new();
    for x in (a[0]..=b[0]).take(capacity) {
        out.push(if steep { [y, x] } else { [x, y] });
        error = error.wrapping_add(dy);
        if error.wrapping_mul(2) >= dx {
            y = y.wrapping_add(step);
            error = error.wrapping_sub(dx);
        }
    }
    out
}

fn crossings(
    map: &EngineMap,
    geometry: &ProvinceGeometry,
    targets: &BTreeMap<u32, Vec<u32>>,
) -> io::Result<BTreeMap<(u32, u32), [i32; 2]>> {
    let width = map.width as i32;
    let borders = border_pixels(map, targets);
    let mut out = BTreeMap::new();
    for (&source, neighbours) in targets {
        for &target in neighbours {
            if source > target {
                continue;
            }
            let anchor = |id: u32| {
                geometry
                    .centroids
                    .get(&id)
                    .copied()
                    .ok_or_else(|| invalid("adjacency anchor unavailable"))
            };
            let (a, b) = (anchor(source)?, anchor(target)?);
            let border = borders
                .get(&(source, target))
                .map_or(&[][..], Vec::as_slice);
            let component = first_largest_component(border);
            let crossing = if component.is_empty() {
                let mut bx = b[0];
                if bx.wrapping_sub(a[0]).wrapping_abs() > width / 2 && bx > width / 2 {
                    bx = bx.wrapping_sub(width);
                }
                [bx.wrapping_add(a[0]) / 2, b[1].wrapping_add(a[1]) / 2]
            } else {
                component_crossing(&component)?
            };
            out.insert((source, target), crossing);
        }
    }
    Ok(out)
}

fn route_points(
    a: [i32; 2],
    mut b: [i32; 2],
    crossing: [i32; 2],
    width: usize,
    height: usize,
) -> io::Result<Vec<[i32; 2]>> {
    let width_i = width as i32;
    let capacity = width + height;
    if b[0].wrapping_sub(a[0]).wrapping_abs() > width_i / 2 {
        b[0] = b[0].wrapping_add(if a[0] < width_i / 2 {
            -width_i
        } else {
            width_i
        });
    }
    let mut points = native_line(a, crossing, capacity);
    if points.is_empty() {
        return Err(invalid("movement line has no source point"));
    }
    if points[0] == crossing {
        let last = points.len() - 1;
        points.swap(0, last);
    }
    points.pop();
    points.extend(native_line(crossing, b, capacity));
    if points.len() > capacity {
        return Err(invalid("movement line exceeds native capacity"));
    }
    Ok(points)
}

fn scale(sum: i64) -> i64 {
    if (sum as u64).wrapping_add(0x7fff_ffff_ffff_fffe) <= 0xffff_ffff_ffff_fffc {
        sum.wrapping_mul(100_000) / 300_000
    } else {
        0xffff_ffff
    }
}

fn edge_distance(
    a: [i32; 2],
    b: [i32; 2],
    crossing: [i32; 2],
    map: &EngineMap,
    terrain: &TerrainCosts,
) -> io::Result<i64> {
    let width = map.width as i32;
    let mut sum = 0_i64;
    for [x, y] in route_points(a, b, crossing, map.width, map.height)? {
        let index = terrain.index(x.wrapping_add(width) % width, y)?;
        sum = sum.wrapping_add(terrain.cost(index));
    }
    Ok(scale(sum))
}

fn authored_crossings(
    rows: &[Adjacency],
    rules: &[AdjacencyRule],
) -> io::Result<BTreeMap<(u32, u32), Crossing>> {
    let mut out = BTreeMap::new();
    for row in rows.iter().filter(|row| row.to > 0) {
        let rule = match &row.rule {
            None => None,
            Some(bytes) => match rules.iter().position(|rule| rule.identity == *bytes) {
                None => None,
                Some(at) => {
                    let name = &rules[at].name;
                    if rules.iter().position(|rule| rule.name == *name) != Some(at) {
                        return Err(invalid(format!(
                            "adjacency {};{} names rule {name}, which an earlier rule's name text cannot be told apart from",
                            row.from, row.to
                        )));
                    }
                    Some(name.clone())
                }
            },
        };
        let crossing = Crossing {
            kind: row.kind,
            through: row.through,
            rule,
        };
        let (source, target) = (row.from, row.to);
        out.insert((source, target), crossing.clone());
        out.insert((target, source), crossing);
    }
    Ok(out)
}

fn derive(
    map: &EngineMap,
    geometry: &ProvinceGeometry,
    rows: &[Adjacency],
    terrain: &TerrainCosts,
) -> io::Result<ProvinceGraph> {
    let mut targets = ordered_bitmap_targets(map)?;
    add_authored_targets(&mut targets, rows)?;
    let crossings = crossings(map, geometry, &targets)?;
    let mut distances: BTreeMap<(u32, u32), Result<i64, String>> = BTreeMap::new();
    for (&(a, b), &crossing) in &crossings {
        let distance = edge_distance(
            geometry.centroids[&a],
            geometry.centroids[&b],
            crossing,
            map,
            terrain,
        )
        .map_err(|error| error.to_string());
        distances.insert((a, b), distance);
    }
    let edges = targets
        .into_iter()
        .map(|(source, neighbours)| {
            let entries = neighbours
                .into_iter()
                .map(|to| {
                    let distance = &distances[&(source.min(to), source.max(to))];
                    (to, distance.as_ref().ok().copied())
                })
                .collect();
            (source, entries)
        })
        .collect();
    let unavailable = distances
        .into_iter()
        .filter_map(|(pair, result)| result.err().map(|reason| (pair, reason)))
        .collect();
    Ok(ProvinceGraph {
        edges,
        unavailable,
        ..ProvinceGraph::default()
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::BTreeSet;

    fn map(engine_rows: &[&[u32]], count: u32) -> EngineMap {
        EngineMap {
            ids: engine_rows
                .iter()
                .rev()
                .flat_map(|row| row.iter().copied())
                .collect(),
            width: engine_rows[0].len(),
            height: engine_rows.len(),
            defined: (1..count).collect(),
            land: (1..count).collect::<BTreeSet<_>>(),
            lake: BTreeSet::new(),
            nonsea_indices: (0..count).map(|id| (id, id as i32)).collect(),
        }
    }

    #[test]
    fn border_pixels_match_a_per_pair_scan_of_every_pixel() {
        let (width, height, count) = (9_usize, 7_usize, 6_u32);
        let mut seed = 0x2545_f491_u32;
        let rows: Vec<Vec<u32>> = (0..height)
            .map(|_| {
                (0..width)
                    .map(|_| {
                        seed ^= seed << 13;
                        seed ^= seed >> 17;
                        seed ^= seed << 5;
                        1 + seed % (count - 1)
                    })
                    .collect()
            })
            .collect();
        let rows: Vec<&[u32]> = rows.iter().map(Vec::as_slice).collect();
        let input = map(&rows, count);
        let targets = ordered_bitmap_targets(&input).unwrap();
        let borders = border_pixels(&input, &targets);
        for (&source, neighbours) in &targets {
            for &target in neighbours.iter().filter(|&&target| source < target) {
                let mut expected = Vec::new();
                for y in 0..height as i32 {
                    for x in 0..width as i32 {
                        let touches = [[0, -1], [-1, 0], [1, 0], [0, 1]].iter().any(|d| {
                            at(&input, (x + d[0]).rem_euclid(width as i32), y + d[1]) == target
                        });
                        if at(&input, x, y) == source && touches {
                            expected.push([x, y]);
                        }
                    }
                }
                assert_eq!(borders[&(source, target)], expected, "{source}->{target}");
            }
        }
    }

    #[test]
    fn scaling_is_signed_and_keeps_the_sentinel() {
        assert_eq!(scale(700_000), 233_333);
        assert_eq!(scale(-700_000), -233_333);
        assert_eq!(scale(i64::MAX), 0xffff_ffff);
    }

    fn line(ids: &[u32], cost: i64) -> (ProvinceGraph, ProvinceGeometry) {
        let mut graph = ProvinceGraph::default();
        let mut geometry = ProvinceGeometry {
            width: 100,
            height: 1,
            ..Default::default()
        };
        for (index, &id) in ids.iter().enumerate() {
            let mut edges = Vec::new();
            if index > 0 {
                edges.push((ids[index - 1], Some(cost)));
            }
            if index + 1 < ids.len() {
                edges.push((ids[index + 1], Some(cost)));
            }
            graph.edges.insert(id, edges);
            geometry.centroids.insert(id, [index as i32 * 10, 0]);
        }
        (graph, geometry)
    }

    #[test]
    fn the_raid_search_sums_edges_along_admitted_provinces() {
        let (graph, mut geometry) = line(&[1, 2, 3, 4], 50_000);
        let search =
            |geometry: &ProvinceGeometry, path| raid_distance(&graph, geometry, 1, 4, path);
        assert_eq!(search(&geometry, RaidPath::Naval), Ok(Some(150_000)));
        geometry.land.insert(4);
        assert_eq!(search(&geometry, RaidPath::Naval), Ok(Some(150_000)));
        assert_eq!(search(&geometry, RaidPath::Land), Ok(None));
        geometry.lake.insert(3);
        assert_eq!(search(&geometry, RaidPath::Naval), Ok(None));
        geometry.centroids.remove(&2);
        assert!(search(&geometry, RaidPath::Land).is_err());
    }

    #[test]
    fn the_raid_route_is_the_predecessor_chain_source_first() {
        let mut graph = ProvinceGraph::default();
        graph
            .edges
            .insert(1, vec![(2, Some(10_000)), (3, Some(40_000))]);
        graph
            .edges
            .insert(2, vec![(1, Some(10_000)), (4, Some(10_000)), (5, Some(1))]);
        graph.edges.insert(3, vec![(1, Some(40_000)), (4, Some(1))]);
        graph.edges.insert(4, vec![(2, Some(10_000)), (3, Some(1))]);
        graph.edges.insert(5, vec![(2, Some(1))]);
        let mut geometry = ProvinceGeometry {
            width: 100,
            height: 10,
            ..Default::default()
        };
        for (id, at) in [
            (1, [0, 0]),
            (2, [0, 1]),
            (3, [1, 0]),
            (4, [1, 1]),
            (5, [0, 2]),
        ] {
            geometry.centroids.insert(id, at);
        }
        let route = raid_route(&graph, &geometry, 1, 4, RaidPath::Naval).unwrap();
        assert_eq!(route, Some((20_000, vec![1, 2, 4])));
        assert_eq!(
            raid_distance(&graph, &geometry, 1, 4, RaidPath::Naval),
            Ok(Some(20_000))
        );
        assert_eq!(
            raid_route(&graph, &geometry, 1, 1, RaidPath::Naval),
            Ok(Some((0, vec![1])))
        );
        assert_eq!(
            raid_route(&graph, &geometry, 1, 4, RaidPath::Land),
            Ok(None)
        );
    }

    #[test]
    fn a_known_province_without_the_edge_takes_the_native_fallback() {
        let graph = ProvinceGraph {
            edges: BTreeMap::from([
                (1, vec![(2, Some(321_000)), (2, Some(654_000))]),
                (2, vec![]),
            ]),
            ..ProvinceGraph::default()
        };
        assert_eq!(graph.distance(1, 2), Some(321_000));
        assert_eq!(graph.distance(1, 3), Some(100_000));
        assert_eq!(graph.distance(9, 1), None);
    }

    #[test]
    fn neighbour_states_keep_member_and_neighbour_order() {
        let input = map(&[&[1, 2, 3, 4, 5, 6]], 8);
        let mut targets = ordered_bitmap_targets(&input).unwrap();
        add_authored_targets(
            &mut targets,
            &adjacencies::parse(b"header\n1;7;sea;-1;0;0;0;0;\n-1\n").unwrap(),
        )
        .unwrap();
        let graph = ProvinceGraph {
            edges: targets
                .into_iter()
                .map(|(id, next)| (id, next.into_iter().map(|to| (to, None)).collect()))
                .collect(),
            ..ProvinceGraph::default()
        };
        let members = BTreeMap::from([
            (10, vec![2, 1]),
            (20, vec![3, 4]),
            (30, vec![5, 4]),
            (40, vec![7]),
        ]);
        let neighbours = graph.neighbor_states(&members).unwrap();
        assert_eq!(neighbours[&10], [20, 40]);
        assert_eq!(neighbours[&20], [10, 30]);
        assert_eq!(neighbours[&30], [20]);
        assert_eq!(neighbours[&40], [10]);
        let unknown = BTreeMap::from([(10, vec![9])]);
        assert!(graph.neighbor_states(&unknown).is_err());
        let wide = BTreeMap::from([(40_000, vec![1])]);
        assert!(graph.neighbor_states(&wide).is_err());
    }

    #[test]
    fn authored_crossings_keep_kind_through_and_known_rules_last_row_winning() {
        let rules = crate::adjacency_rules::parse(b"adjacency_rule = { name = STRAIT }").unwrap();
        let csv = b"From;To;Type;Through;a;b;c;d;rule;Comment
                    1;5;sea;9;-1;-1;-1;-1;STRAIT;x
                    2;3;impassable;-1;-1;-1;-1;-1;;y
                    2;3;;-1;-1;-1;-1;-1;MISSING;z
                    -1;-1;;-1;-1;-1;-1;-1;;
";
        let crossings = authored_crossings(&adjacencies::parse(csv).unwrap(), &rules).unwrap();
        let strait = Crossing {
            kind: SEA_CROSSING,
            through: 9,
            rule: Some("STRAIT".into()),
        };
        assert_eq!(crossings[&(1, 5)], strait);
        assert_eq!(crossings[&(5, 1)], strait);
        assert_eq!(
            crossings[&(3, 2)],
            Crossing {
                kind: 0,
                through: -1,
                rule: None
            }
        );
    }

    #[test]
    fn authored_crossings_match_rule_names_by_their_bytes() {
        let rules = crate::adjacency_rules::parse(
            b"adjacency_rule = { name = OPEN } adjacency_rule = { name = \"\xc9TROIT\" enemy = { army = no } }",
        )
        .unwrap();
        let csv = b"h\n1;5;sea;9;-1;-1;-1;-1;\xc9TROIT;x\n2;3;sea;9;-1;-1;-1;-1;\xc9troit;y\n";
        let crossings = authored_crossings(&adjacencies::parse(csv).unwrap(), &rules).unwrap();
        let name = crossings[&(5, 1)].rule.as_deref().unwrap();
        let rule = crate::adjacency_rules::find(&rules, name).unwrap();
        let refused = rule.unsupported.as_deref().unwrap();
        assert!(refused.contains("not UTF-8"), "{refused}");
        assert_eq!(crossings[&(2, 3)].rule, None);
        let clash = crate::adjacency_rules::parse(
            b"adjacency_rule = { name = \\xc9X } adjacency_rule = { name = \"\xc9X\" }",
        )
        .unwrap();
        assert_eq!(clash[0].name, clash[1].name);
        let csv = b"h\n1;5;sea;9;-1;-1;-1;-1;\xc9X;x\n";
        let error = authored_crossings(&adjacencies::parse(csv).unwrap(), &clash).unwrap_err();
        assert!(error.to_string().contains("cannot be told apart"), "{error}");
    }
}
