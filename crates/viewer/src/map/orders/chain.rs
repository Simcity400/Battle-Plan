//! A front or snapped offensive line drawn along its province pairs' integer
//! gradient-border pixels.
//!
//! Each pair `(source, target)` contributes the source province's own border
//! pixels towards the target, or towards a sea crossing's Through neighbour.
//! The original target still drives the teeth probe. The first join orients
//! the chain once; reverse a chain when its nearer raw end is its last pixel.
//! Drop a pixel that jumps four or more pixels on either axis from the last
//! accepted one.
//! Overlapping fronts are offset by `ROOT_FRONT_OFFSET` per stored overlap unit
//! to the side a controller-majority probe picks. The side the teeth face
//! comes from a raster probe across the first chain.

use std::collections::BTreeMap;

use installed_content::map::MapData;

use super::fixed::{self, Point, SCALE, Smoothing};

/// One province pair of a chain and its stored byte-wide overlap units.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Pair {
    pub source: u32,
    pub target: u32,
    pub units: u8,
}

/// What the builder reads besides the pairs.
pub struct ChainWorld<'a> {
    pub borders: &'a BTreeMap<(u32, u32), Vec<[i32; 2]>>,
    /// The source-owned border target after a one-hop sea crossing redirect
    /// through its neighbour. `None` is a neighbour miss.
    pub border_target: &'a dyn Fn(u32, u32) -> Option<u32>,
    pub raster: Option<&'a Raster<'a>>,
}

/// Province raster and recorded controllers compared by the offset probe.
pub struct Raster<'a> {
    pub map: &'a MapData,
    pub controllers: &'a BTreeMap<u32, u32>,
    pub links: &'a BTreeMap<u32, u32>,
}

impl Raster<'_> {
    /// The raster id at world `(x, z)`, zero outside the map; no wrap.
    pub fn at(&self, x: i64, z: i64) -> u32 {
        if x < 0 || z < 0 || x >= i64::from(self.map.width) || z >= i64::from(self.map.height) {
            return 0;
        }
        let row = self.map.height as usize - 1 - z as usize;
        self.map.province_ids[row * self.map.width as usize + x as usize]
    }

    /// Positive controllers that are equal or the same canonical country.
    fn same_controller(&self, source: u32, point: Point) -> bool {
        let [x, z] = point.map(|value| value / SCALE);
        let target = self.at(x, z);
        if target == 0 {
            return false;
        }
        let (Some(&a), Some(&b)) = (self.controllers.get(&source), self.controllers.get(&target))
        else {
            return false;
        };
        (a as i32) > 0
            && (b as i32) > 0
            && (a == b
                || self.links.get(&a).copied().unwrap_or(a)
                    == self.links.get(&b).copied().unwrap_or(b))
    }
}

/// The builder's arguments after the pairs.
#[derive(Clone, Copy, Debug)]
pub struct ChainParams {
    /// The fourth argument: subdivide before each smoothing pass.
    pub subdivided: bool,
    pub smoothing: Smoothing,
    /// The seventh argument, added to every pair's stored units.
    pub extra_units: u8,
    /// `NGraphics.ROOT_FRONT_OFFSET`, fixed5.
    pub offset: i64,
    /// `ROOT_FRONT_MAX_INTERSECTION_TESTS_ORDER`, or `_FRONT` when the
    /// graphics have no bound order.
    pub max_intersection_tests: i32,
    /// Run the teeth probe for a bound order whose type declares culling_direction.
    pub probe_teeth: bool,
}

/// The chain's stored controls and what the builder decided on the way.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Chain {
    pub points: Vec<Point>,
    /// Teeth side stored by the raster probe, when decided.
    pub side: Option<i32>,
    /// True when some pair was offset: then submit each point through the
    /// stored-control builder with intersection tests.
    pub offset: bool,
}

fn manhattan(a: [i32; 2], b: [i32; 2]) -> i32 {
    (a[0] - b[0]).abs() + (a[1] - b[1]).abs()
}

/// Builds the chain; `None` when some offset is needed but no raster was given.
pub fn build(pairs: &[Pair], world: &ChainWorld<'_>, params: ChainParams) -> Option<Chain> {
    let mut chain = Chain::default();
    let mut out: Vec<Point> = Vec::new();
    let mut previous_ends: Option<([i32; 2], [i32; 2])> = None;
    let mut oriented = false;
    for pair in pairs {
        let Some(border_target) = (world.border_target)(pair.source, pair.target) else {
            continue;
        };
        let Some(raw) = world.borders.get(&(pair.source, border_target)) else {
            continue;
        };
        if raw.is_empty() {
            continue;
        }
        let first = raw[0];
        let last = raw[raw.len() - 1];
        let mut reverse = false;
        if !out.is_empty()
            && let Some((start, end)) = previous_ends
        {
            let ds = manhattan(start, first).min(manhattan(start, last));
            let de = manhattan(end, first).min(manhattan(end, last));
            reverse = if ds < de {
                ds == manhattan(start, last)
            } else {
                de == manhattan(end, last)
            };
            if !oriented {
                oriented = true;
                if ds < de {
                    out.reverse();
                }
            }
        }
        let ordered: Vec<[i32; 2]> = if reverse {
            raw.iter().rev().copied().collect()
        } else {
            raw.clone()
        };
        previous_ends = Some((ordered[0], ordered[ordered.len() - 1]));
        let mut kept = Vec::with_capacity(ordered.len());
        let mut previous = ordered[0];
        kept.push(previous);
        for &p in &ordered[1..] {
            if (p[0] - previous[0]).abs() < 4 && (p[1] - previous[1]).abs() < 4 {
                kept.push(p);
                previous = p;
            }
        }
        if params.probe_teeth
            && chain.side.is_none()
            && kept.len() > 1
            && (pairs.len() == 1 || oriented)
            && let Some(raster) = world.raster
        {
            chain.side = teeth_side(raster, &kept, pair.source, pair.target);
        }
        let units = i32::from(pair.units) + i32::from(params.extra_units);
        if units == 0 || kept.len() < 2 {
            out.extend(kept.iter().map(|&p| fixed::from_pixel(p)));
        } else {
            chain.offset = true;
            let raster = world.raster?;
            out.extend(offset_points(
                &kept,
                pair.source,
                units,
                params.offset,
                raster,
            ));
        }
    }
    if !out.is_empty() {
        fixed::curve(&mut out, params.smoothing, params.subdivided);
        if chain.offset {
            out = super::submit::controls(&out, Some(params.max_intersection_tests));
        }
    }
    chain.points = out;
    Some(chain)
}

/// Rotate by the float quarter-turn angle -1.5708.
fn rotate_back(x: f32, z: f32) -> (f32, f32) {
    let angle = f32::from_bits(0xbfc9_0fdb);
    let (sine, cosine) = (angle.sin(), angle.cos());
    (x * cosine - sine * z, x * sine + cosine * z)
}

/// At the first kept pixel inside the source, sample the raster one
/// quarter-turned step to either side. Source on the positive side and target
/// on the negative stores -1; the reverse stores 1.
fn teeth_side(raster: &Raster<'_>, kept: &[[i32; 2]], source: u32, target: u32) -> Option<i32> {
    for window in kept.windows(2) {
        let [x, z] = window[0];
        if raster.at(i64::from(x), i64::from(z)) != source {
            continue;
        }
        let (dx, dz) = rotate_back((window[1][0] - x) as f32, (window[1][1] - z) as f32);
        let (dx, dz) = (dx as i64, dz as i64);
        let plus = raster.at(i64::from(x) + dx, i64::from(z) + dz);
        let minus = raster.at(i64::from(x) - dx, i64::from(z) - dz);
        if plus == source && minus == target {
            return Some(-1);
        }
        if minus == source && plus == target {
            return Some(1);
        }
    }
    None
}

/// One side for the whole directed chain from its fixed5 normals: a strict
/// majority of `ROOT_FRONT_OFFSET * 1.5` probes landing on the source's
/// controller chooses the positive side; ties and misses the negative one.
fn offset_points(
    kept: &[[i32; 2]],
    source: u32,
    units: i32,
    offset: i64,
    raster: &Raster<'_>,
) -> Vec<Point> {
    let points: Vec<Point> = kept.iter().map(|&p| fixed::from_pixel(p)).collect();
    let probe = offset * 150_000 / SCALE;
    let distance = i64::from(units) * SCALE * offset / SCALE;
    let mut hits = 0;
    let mut normals = Vec::with_capacity(points.len());
    for (index, &point) in points.iter().enumerate() {
        let incoming = index.checked_sub(1).map(|previous| {
            [
                point[0] - points[previous][0],
                point[1] - points[previous][1],
            ]
        });
        let outgoing = points
            .get(index + 1)
            .map(|next| [next[0] - point[0], next[1] - point[1]]);
        let tangent = match (incoming, outgoing) {
            (Some(a), Some(b)) => [(a[0] + b[0]) / 2, (a[1] + b[1]) / 2],
            (Some(value), None) | (None, Some(value)) => value,
            (None, None) => [0, 0],
        };
        let [dx, dz] = fixed::normalize(tangent);
        let normal = [-dz, dx];
        if raster.same_controller(
            source,
            [
                point[0] + normal[0] * probe / SCALE,
                point[1] + normal[1] * probe / SCALE,
            ],
        ) {
            hits += 1;
        }
        normals.push(normal);
    }
    let signed = if hits > points.len() / 2 {
        distance
    } else {
        -distance
    };
    points
        .into_iter()
        .zip(normals)
        .map(|(point, normal)| {
            [
                point[0] + normal[0] * signed / SCALE,
                point[1] + normal[1] * signed / SCALE,
            ]
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn map(width: u32, height: u32, ids: Vec<u32>) -> MapData {
        MapData {
            width,
            height,
            province_ids: ids,
            definitions: BTreeMap::new(),
            country_colors: Default::default(),
        }
    }

    fn plain() -> ChainParams {
        ChainParams {
            subdivided: false,
            smoothing: Smoothing {
                smoothness: 0,
                passes: 0,
            },
            extra_units: 0,
            offset: 150_000,
            max_intersection_tests: 25,
            probe_teeth: false,
        }
    }

    #[test]
    fn integer_chains_keep_scan_ties_and_reject_disconnected_islands() {
        let borders = BTreeMap::from([((1, 2), vec![[0, 0], [1, 0], [9, 0], [2, 0]])]);
        let border_target = |_: u32, target: u32| Some(target);
        let world = ChainWorld {
            borders: &borders,
            border_target: &border_target,
            raster: None,
        };
        let chain = build(
            &[Pair {
                source: 1,
                target: 2,
                units: 0,
            }],
            &world,
            plain(),
        )
        .unwrap();
        assert_eq!(chain.points, vec![[0, 0], [SCALE, 0], [2 * SCALE, 0]]);
    }

    #[test]
    fn redirected_pairs_use_each_sources_pixels_and_missing_neighbours_contribute_none() {
        let borders = BTreeMap::from([
            ((1, 2), vec![[9, 9]]),
            ((1, 3), vec![[0, 1], [1, 1]]),
            ((2, 3), vec![[2, 3], [3, 3]]),
            ((3, 4), vec![[5, 5], [6, 5]]),
        ]);
        let border_target = |source: u32, target: u32| match (source, target) {
            (1, 2) | (2, 1) => Some(3),
            _ => None,
        };
        let world = ChainWorld {
            borders: &borders,
            border_target: &border_target,
            raster: None,
        };
        let pair = |source, target| {
            [Pair {
                source,
                target,
                units: 0,
            }]
        };
        assert_eq!(
            build(&pair(1, 2), &world, plain()).unwrap().points,
            vec![[0, SCALE], [SCALE, SCALE]],
        );
        assert_eq!(
            build(&pair(2, 1), &world, plain()).unwrap().points,
            vec![[2 * SCALE, 3 * SCALE], [3 * SCALE, 3 * SCALE]],
        );
        assert!(
            build(&pair(3, 4), &world, plain())
                .unwrap()
                .points
                .is_empty()
        );
    }

    #[test]
    fn the_first_join_orients_the_chain_and_later_chains_follow() {
        let borders = BTreeMap::from([
            ((1, 2), vec![[0, 0], [1, 0]]),
            ((2, 3), vec![[2, 0], [1, 0]]),
        ]);
        let border_target = |_: u32, target: u32| Some(target);
        let world = ChainWorld {
            borders: &borders,
            border_target: &border_target,
            raster: None,
        };
        let pairs = [(1, 2), (2, 3)].map(|(source, target)| Pair {
            source,
            target,
            units: 0,
        });
        let chain = build(&pairs, &world, plain()).unwrap();
        assert_eq!(
            chain.points,
            vec![[0, 0], [SCALE, 0], [SCALE, 0], [2 * SCALE, 0]]
        );
    }

    #[test]
    fn border_offset_uses_one_controller_majority_and_rejects_zero_aliases() {
        let mut m = map(5, 5, vec![2; 25]);
        m.province_ids[6] = 3;
        m.province_ids[7] = 3;
        let controllers = BTreeMap::from([(1, 10), (2, 20), (3, 11)]);
        let links = BTreeMap::from([(10, 10), (11, 10)]);
        let raster = Raster {
            map: &m,
            controllers: &controllers,
            links: &links,
        };
        let kept = [[1, 1], [2, 1], [3, 1]];
        assert_eq!(
            offset_points(&kept, 1, 1, 150_000, &raster),
            [[SCALE, 250_000], [2 * SCALE, 250_000], [3 * SCALE, 250_000]]
        );
        assert_eq!(
            offset_points(&kept[1..], 1, 1, 150_000, &raster),
            [[2 * SCALE, -50_000], [3 * SCALE, -50_000]]
        );
        assert!(!raster.same_controller(1, [-100_000, 100_000]));
    }

    #[test]
    fn the_teeth_probe_reads_the_source_side_of_the_first_chain() {
        // Bitmap rows run downward: province 2 is the north row (z = 2),
        // province 1 the two rows below it.
        let m = map(3, 3, vec![2, 2, 2, 1, 1, 1, 1, 1, 1]);
        let controllers = BTreeMap::new();
        let links = BTreeMap::new();
        let raster = Raster {
            map: &m,
            controllers: &controllers,
            links: &links,
        };
        // West (-1, 0) turns by -90 degrees to (0, 1): plus is north (the
        // target), minus south (the source), which stores 1.
        assert_eq!(teeth_side(&raster, &[[2, 1], [1, 1]], 1, 2), Some(1));
        // East (1, 0) turns to (0, -1): plus is the source, which stores -1.
        assert_eq!(teeth_side(&raster, &[[0, 1], [1, 1]], 1, 2), Some(-1));
        // A pixel outside the source is not probed.
        assert_eq!(teeth_side(&raster, &[[0, 2], [1, 2]], 1, 2), None);
    }

    #[test]
    fn redirected_pixels_keep_the_original_target_probe_and_pair_overlap_units() {
        let m = map(3, 3, vec![3, 3, 3, 1, 1, 1, 1, 1, 1]);
        let controllers = BTreeMap::from([(1, 10), (3, 30)]);
        let links = BTreeMap::new();
        let raster = Raster {
            map: &m,
            controllers: &controllers,
            links: &links,
        };
        let borders = BTreeMap::from([((1, 3), vec![[0, 1], [1, 1]])]);
        let border_target = |_: u32, _: u32| Some(3);
        let world = ChainWorld {
            borders: &borders,
            border_target: &border_target,
            raster: Some(&raster),
        };
        let mut params = plain();
        params.probe_teeth = true;
        let pair = |target, units| {
            [Pair {
                source: 1,
                target,
                units,
            }]
        };
        let redirected = build(&pair(2, 0), &world, params).unwrap();
        assert_eq!(redirected.points, vec![[0, SCALE], [SCALE, SCALE]]);
        // Through is province 3, but the original pair's target is province 2.
        assert_eq!(redirected.side, None);
        assert_eq!(build(&pair(3, 0), &world, params).unwrap().side, Some(-1));
        params.extra_units = 1;
        let overlapping = build(&pair(2, 2), &world, params).unwrap();
        assert!(overlapping.offset);
        assert_eq!(overlapping.points[0], [0, -350_000]);
        assert_eq!(overlapping.side, None);
    }

    #[test]
    fn only_offset_chains_resubmit_and_the_extra_unit_does_not_saturate_the_pair_byte() {
        let m = map(3, 5, vec![1; 15]);
        let controllers = BTreeMap::from([(1, 10)]);
        let links = BTreeMap::new();
        let raster = Raster {
            map: &m,
            controllers: &controllers,
            links: &links,
        };
        let borders = BTreeMap::from([((1, 2), vec![[0, 1], [1, 1], [2, 1]])]);
        let border_target = |_: u32, target: u32| Some(target);
        let world = ChainWorld {
            borders: &borders,
            border_target: &border_target,
            raster: Some(&raster),
        };
        let pair = |units| {
            [Pair {
                source: 1,
                target: 2,
                units,
            }]
        };
        let plain_chain = build(&pair(0), &world, plain()).unwrap();
        assert_eq!(
            plain_chain.points,
            vec![[0, SCALE], [SCALE, SCALE], [2 * SCALE, SCALE]]
        );
        let offset = build(&pair(1), &world, plain()).unwrap();
        assert_eq!(
            offset.points,
            vec![
                [0, 250_000],
                [0, 250_000],
                [50_000, 250_000],
                [SCALE, 250_000],
                [2 * SCALE, 250_000],
            ]
        );
        let mut extra = plain();
        extra.extra_units = 1;
        let wide = build(&pair(u8::MAX), &world, extra).unwrap();
        assert_eq!(wide.points[0], [0, 385 * SCALE]);
    }
}
