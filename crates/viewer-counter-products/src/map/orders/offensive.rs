
use std::collections::BTreeSet;

use super::fixed::{self, Point, SCALE};

const SIDE_TOLERANCE: i64 = 20_000;

pub struct World<'a> {
    pub centroid: &'a dyn Fn(u32) -> Option<[i32; 2]>,
    pub neighbours: &'a dyn Fn(u32) -> Option<&'a [u32]>,
    pub adjacent: &'a dyn Fn(u32, u32) -> bool,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Snap {
    pub pairs: Vec<[u32; 2]>,
    pub side: i32,
}

fn centre(world: &World<'_>, province: u32) -> Option<Point> {
    (world.centroid)(province).map(fixed::from_pixel)
}

fn mean_direction(
    world: &World<'_>,
    path: &[u32],
    range: std::ops::RangeInclusive<i64>,
) -> Option<Point> {
    let mut sum = [0i64, 0];
    let mut count = 0;
    for j in range {
        if j < 0 {
            continue;
        }
        if j >= path.len() as i64 - 1 {
            break;
        }
        let a = centre(world, path[j as usize])?;
        let b = centre(world, path[j as usize + 1])?;
        sum = [sum[0] + b[0] - a[0], sum[1] + b[1] - a[1]];
        count += 1;
    }
    if count > 0 {
        sum = fixed::divide(sum, count);
    }
    Some(fixed::normalize(sum))
}

fn move_direction(world: &World<'_>, approach: &[u32]) -> Option<Point> {
    let n = approach.len() as i64;
    let mut sum = [0i64, 0];
    let mut count = 0;
    let mut i = n - 1;
    while i >= n - 2 && i >= 1 {
        let a = centre(world, approach[i as usize - 1])?;
        let b = centre(world, approach[i as usize])?;
        sum = [sum[0] + b[0] - a[0], sum[1] + b[1] - a[1]];
        count += 1;
        i -= 1;
    }
    if count > 0 {
        sum = fixed::divide(sum, count);
    }
    (sum != [0, 0]).then(|| fixed::normalize(sum))
}

pub fn snap(line: &[u32], approach: &[u32], world: &World<'_>) -> Option<Snap> {
    if approach.len() < 2 || line.is_empty() {
        return None;
    }
    let incoming = move_direction(world, approach)?;
    let end = *approach.last()?;
    let at = line.iter().position(|&p| p == end)? as i64;
    let single = line.len() == 1;
    let tangent_at = |range: std::ops::RangeInclusive<i64>| -> Option<Point> {
        if single {
            Some(fixed::rotate_quarter(incoming))
        } else {
            mean_direction(world, line, range)
        }
    };
    let tangent = tangent_at(at - 2..=at + 1)?;
    let side = fixed::cross_y(tangent, incoming);
    let inside: BTreeSet<u32> = line.iter().copied().collect();
    let mut candidates = Vec::new();
    for (k, &province) in line.iter().enumerate() {
        let k = k as i64;
        let Some(t) = tangent_at(k - 1..=k) else {
            continue;
        };
        if t == [0, 0] {
            continue;
        }
        let Some(c) = centre(world, province) else {
            continue;
        };
        for &neighbour in (world.neighbours)(province).unwrap_or_default() {
            if inside.contains(&neighbour) {
                continue;
            }
            let Some(n) = centre(world, neighbour) else {
                continue;
            };
            let v = fixed::normalize([n[0] - c[0], n[1] - c[1]]);
            let value = v[0] * t[1] / SCALE - v[1] * t[0] / SCALE;
            let accepted = if value < 0 {
                side < 0 || value >= -SIDE_TOLERANCE
            } else {
                side >= 0 || value <= SIDE_TOLERANCE
            };
            if accepted {
                candidates.push(neighbour);
            }
        }
    }
    if candidates.is_empty() {
        return None;
    }
    let mut best: Vec<[u32; 2]> = Vec::new();
    for seed in 0..candidates.len() {
        let mut remaining = candidates.clone();
        let mut outside = remaining.swap_remove(seed);
        let mut pairs: Vec<[u32; 2]> = Vec::new();
        for &province in line {
            if !(world.adjacent)(province, outside) {
                continue;
            }
            pairs.push([province, outside]);
            loop {
                let next = remaining.iter().position(|&candidate| {
                    (world.adjacent)(candidate, province)
                        && (world.adjacent)(candidate, outside)
                        && !pairs.contains(&[province, candidate])
                });
                let Some(next) = next else { break };
                outside = remaining.swap_remove(next);
                pairs.push([province, outside]);
            }
        }
        if pairs.len() > best.len() {
            best = pairs;
        }
    }
    let total = candidates.len() as i64;
    let allowed = (total / 5).max(1);
    (total - best.len() as i64 <= allowed).then_some(Snap {
        pairs: best,
        side: if side > 0 { -1 } else { 1 },
    })
}

pub fn unsnapped_side(controls: &[Point], approach: &[u32], world: &World<'_>) -> Option<i32> {
    if approach.len() < 2 || controls.len() < 2 {
        return None;
    }
    let destination = *approach.last()?;
    let previous = *approach[..approach.len() - 1]
        .iter()
        .rev()
        .find(|&&id| id != destination)?;
    if (previous as i32) < 0 {
        return None;
    }
    let target = centre(world, destination)?;
    let mut best = None;
    let mut distance = 0x17_4875_6160_i64;
    for (index, point) in controls.iter().enumerate() {
        let dx = point[0] - target[0];
        let dz = point[1] - target[1];
        let candidate = dx.wrapping_mul(dx) / SCALE + dz.wrapping_mul(dz) / SCALE;
        if candidate >= 0 && candidate < distance {
            distance = candidate;
            best = Some(index as i64);
        }
    }
    let at = best?;
    let mut sum = [0i64, 0];
    let mut count = 0;
    for j in at - 4..=at + 4 {
        if j < 0 {
            continue;
        }
        if j >= controls.len() as i64 - 1 {
            break;
        }
        let (a, b) = (controls[j as usize], controls[j as usize + 1]);
        sum = [sum[0] + b[0] - a[0], sum[1] + b[1] - a[1]];
        count += 1;
    }
    if count == 0 {
        return None;
    }
    let tangent = fixed::normalize(fixed::divide(sum, count));
    let from = centre(world, previous)?;
    let v = fixed::normalize([target[0] - from[0], target[1] - from[1]]);
    let value = v[0] * tangent[1] / SCALE - v[1] * tangent[0] / SCALE;
    Some(if value > 0 { -1 } else { 1 })
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::BTreeMap;

    struct Grid {
        centroids: BTreeMap<u32, [i32; 2]>,
        neighbours: BTreeMap<u32, Vec<u32>>,
    }

    fn grid() -> Grid {
        let mut centroids = BTreeMap::new();
        let mut neighbours: BTreeMap<u32, Vec<u32>> = BTreeMap::new();
        for z in 0..3i32 {
            for x in 0..3i32 {
                let id = (z * 3 + x + 1) as u32;
                centroids.insert(id, [x * 10, z * 10]);
                let mut list = Vec::new();
                for (dx, dz) in [
                    (0, -1),
                    (-1, 0),
                    (1, 0),
                    (0, 1),
                    (-1, -1),
                    (1, -1),
                    (-1, 1),
                    (1, 1),
                ] {
                    let (nx, nz) = (x + dx, z + dz);
                    if (0..3).contains(&nx) && (0..3).contains(&nz) {
                        list.push((nz * 3 + nx + 1) as u32);
                    }
                }
                neighbours.insert(id, list);
            }
        }
        Grid {
            centroids,
            neighbours,
        }
    }

    fn with_world<R>(g: &Grid, f: impl FnOnce(&World<'_>) -> R) -> R {
        let centroid = |id: u32| g.centroids.get(&id).copied();
        let neighbours = |id: u32| g.neighbours.get(&id).map(Vec::as_slice);
        let adjacent = |a: u32, b: u32| g.neighbours.get(&a).is_some_and(|l| l.contains(&b));
        f(&World {
            centroid: &centroid,
            neighbours: &neighbours,
            adjacent: &adjacent,
        })
    }

    #[test]
    fn a_line_across_the_grid_snaps_to_its_forward_neighbours() {
        let g = grid();
        let north = with_world(&g, |w| snap(&[4, 5, 6], &[2, 5], w)).unwrap();
        assert!(north.pairs.iter().all(|p| [7, 8, 9].contains(&p[1])));
        assert_eq!(north.pairs.len(), 7);
        assert_eq!(north.pairs[0], [4, 8]);
        assert_eq!(north.side, 1);
        let south = with_world(&g, |w| snap(&[4, 5, 6], &[8, 5], w)).unwrap();
        assert!(south.pairs.iter().all(|p| [1, 2, 3].contains(&p[1])));
        assert_eq!(south.side, -1);
    }

    #[test]
    fn a_single_province_line_uses_the_quarter_turned_move_direction() {
        let g = grid();
        let snap = with_world(&g, |w| snap(&[5], &[2, 5], w)).unwrap();
        assert!(snap.pairs.iter().any(|p| p == &[5, 8]));
    }

    #[test]
    fn no_move_or_a_move_ending_off_the_line_does_not_snap() {
        let g = grid();
        assert!(with_world(&g, |w| snap(&[4, 5, 6], &[5], w)).is_none());
        assert!(with_world(&g, |w| snap(&[4, 5, 6], &[1, 2], w)).is_none());
    }

    #[test]
    fn the_unsnapped_side_follows_the_arriving_move() {
        let g = grid();
        let controls = [
            [0, 10 * SCALE],
            [10 * SCALE, 10 * SCALE],
            [20 * SCALE, 10 * SCALE],
        ];
        let north = with_world(&g, |w| unsnapped_side(&controls, &[2, 5], w));
        let south = with_world(&g, |w| unsnapped_side(&controls, &[8, 5], w));
        assert_eq!(north, Some(1));
        assert_eq!(south, Some(-1));
    }
}
