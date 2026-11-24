
use std::{collections::HashMap, io};

use content_reader_parser::fields::invalid;

pub(crate) struct Raster<'a> {
    pub width: usize,
    pub height: usize,
    pub ids: &'a [u32],
}

impl Raster<'_> {
    fn at(&self, x: i32, y: i32) -> u32 {
        if x < 0 || y < 0 || x as usize >= self.width || y as usize >= self.height {
            return 0;
        }
        self.ids[y as usize * self.width + x as usize]
    }

    fn wrap(&self, x: i32) -> i32 {
        let width = self.width as i32;
        if x < 0 {
            x + width
        } else if x >= width {
            x - width
        } else {
            x
        }
    }
}

pub(crate) fn anchors(raster: &Raster, count: usize) -> Vec<[i32; 2]> {
    let mut sums = vec![(0_i32, 0_i32, 0_u32); count];
    for y in 0..raster.height {
        for x in 0..raster.width {
            let entry = &mut sums[raster.ids[y * raster.width + x] as usize];
            entry.0 = entry.0.wrapping_add(x as i32);
            entry.1 = entry.1.wrapping_add(y as i32);
            entry.2 += 1;
        }
    }
    sums.into_iter()
        .map(|(x, y, pixels)| match pixels {
            0 => [0, 0],
            pixels => [x.wrapping_div(pixels as i32), y.wrapping_div(pixels as i32)],
        })
        .collect()
}

fn borders(raster: &Raster) -> HashMap<(u32, u32), Vec<[i32; 2]>> {
    let mut borders: HashMap<_, Vec<_>> = HashMap::new();
    for y in 0..raster.height as i32 {
        for x in 0..raster.width as i32 {
            let source = raster.at(x, y);
            let mut seen = [0_u32; 4];
            for (slot, [dx, dy]) in [[0, -1], [-1, 0], [1, 0], [0, 1]].into_iter().enumerate() {
                let target = raster.at(raster.wrap(x + dx), y + dy);
                if target > source && !seen[..slot].contains(&target) {
                    borders.entry((source, target)).or_default().push([x, y]);
                }
                seen[slot] = target;
            }
        }
    }
    borders
}

fn self_border(raster: &Raster, id: u32) -> Vec<[i32; 2]> {
    let mut pixels = Vec::new();
    for y in 0..raster.height as i32 {
        for x in 0..raster.width as i32 {
            if raster.at(x, y) == id
                && [[0, -1], [-1, 0], [1, 0], [0, 1]]
                    .iter()
                    .any(|[dx, dy]| raster.at(raster.wrap(x + dx), y + dy) == id)
            {
                pixels.push([x, y]);
            }
        }
    }
    pixels
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
                    if let Some(indices) = by_coordinate.get(&[p[0] + dx, p[1] + dy]) {
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
    let divisor = component.len() as i64 * 100_000;
    let mut sums = [0_i64; 2];
    for point in component {
        for axis in 0..2 {
            sums[axis] = sums[axis]
                .checked_add(i64::from(point[axis]) * 10_000_000_000 / divisor)
                .ok_or_else(|| invalid("adjacency border sum overflows native i64"))?;
        }
    }
    Ok(sums.map(|value| ((value + if value < 0 { -50_000 } else { 50_000 }) / 100_000) as i32))
}

pub(crate) fn native_line(a: [i32; 2], b: [i32; 2], capacity: usize) -> Vec<[i32; 2]> {
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

pub(crate) fn unwrap_towards(a: [i32; 2], mut b: [i32; 2], width: i32) -> [i32; 2] {
    if b[0].wrapping_sub(a[0]).wrapping_abs() > width / 2 {
        b[0] = b[0].wrapping_add(if a[0] < width / 2 { -width } else { width });
    }
    b
}

pub(crate) fn route_points(
    a: [i32; 2],
    b: [i32; 2],
    crossing: [i32; 2],
    width: usize,
    height: usize,
) -> io::Result<Vec<[i32; 2]>> {
    let capacity = width + height;
    let b = unwrap_towards(a, b, width as i32);
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

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct Crossing {
    pub point: [i32; 2],
    pub no_midpoint: bool,
}

pub(crate) fn crossings(
    raster: &Raster,
    lists: &[Vec<u32>],
    anchors: &[[i32; 2]],
    land: impl Fn(u32) -> bool,
) -> io::Result<HashMap<(u32, u32), Crossing>> {
    let width = raster.width as i32;
    let capacity = raster.width + raster.height;
    let mut borders = borders(raster);
    let mut crossings = HashMap::new();
    for (source, targets) in lists.iter().enumerate() {
        let source = source as u32;
        for &target in targets.iter().filter(|&&target| source <= target) {
            let a = anchors[source as usize];
            let b = anchors[target as usize];
            let border = if source == target {
                self_border(raster, source)
            } else {
                borders.remove(&(source, target)).unwrap_or_default()
            };
            let point = if border.is_empty() {
                let mut bx = b[0];
                if bx.wrapping_sub(a[0]).wrapping_abs() > width / 2 && bx > width / 2 {
                    bx = bx.wrapping_sub(width);
                }
                [bx.wrapping_add(a[0]) / 2, b[1].wrapping_add(a[1]) / 2]
            } else {
                component_crossing(&first_largest_component(&border))?
            };
            let inside = native_line(a, unwrap_towards(a, b, width), capacity)
                .iter()
                .all(|p| {
                    let id = raster.at(p[0].wrapping_add(width) % width, p[1]);
                    id == source || id == target
                });
            let (land_a, land_b) = (land(source), land(target));
            crossings.insert(
                (source, target),
                Crossing {
                    point,
                    no_midpoint: land_a != land_b || (land_a && inside),
                },
            );
        }
    }
    Ok(crossings)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn raster_of<'a>(rows: &[&[u32]], ids: &'a mut Vec<u32>) -> Raster<'a> {
        *ids = rows.iter().flat_map(|row| row.iter().copied()).collect();
        Raster {
            width: rows[0].len(),
            height: rows.len(),
            ids,
        }
    }

    fn crossing(rows: &[&[u32]], lists: &[Vec<u32>], land: &[u32]) -> Crossing {
        let mut ids = Vec::new();
        let raster = raster_of(rows, &mut ids);
        let anchors = anchors(&raster, lists.len());
        crossings(&raster, lists, &anchors, |id| land.contains(&id)).unwrap()[&(1, 2)]
    }

    #[test]
    fn direct_land_line_and_coast_suppress_the_midpoint_but_a_detour_keeps_it() {
        let lists = [vec![], vec![2], vec![1]];
        assert!(crossing(&[&[1, 1, 2, 2]], &lists, &[1, 2]).no_midpoint);
        assert!(crossing(&[&[1, 1, 2, 2]], &lists, &[1]).no_midpoint);
        assert!(!crossing(&[&[1, 1, 2, 2]], &lists, &[]).no_midpoint);
        let lists = [vec![], vec![2, 3], vec![1, 3], vec![1, 2]];
        let detour = crossing(
            &[
                &[1, 1, 1, 2, 2, 2, 2, 3],
                &[1, 1, 3, 3, 3, 2, 2, 3],
                &[1, 1, 3, 3, 3, 2, 2, 3],
            ],
            &lists,
            &[1, 2, 3],
        );
        assert!(!detour.no_midpoint);
    }

    #[test]
    fn an_authored_nonraster_neighbour_can_have_native_invalid_crossing_x() {
        let lists = [vec![], vec![3, 2], vec![3, 1], vec![1, 2]];
        let edge = crossing(&[&[1, 3, 3, 3, 3, 3, 2, 3]], &lists, &[]);
        assert_eq!(edge.point, [-1, 0]);
        assert!(!edge.no_midpoint);
    }

    #[test]
    fn unpainted_records_keep_the_origin_anchor() {
        let mut ids = Vec::new();
        let raster = raster_of(&[&[1, 2], &[1, 1]], &mut ids);
        assert_eq!(anchors(&raster, 4), [[0, 0], [0, 0], [1, 0], [0, 0]]);
        let raster = raster_of(&[&[2, 1], &[1, 1], &[1, 2]], &mut ids);
        assert_eq!(anchors(&raster, 3)[1], [0, 1]);
    }

    #[test]
    fn first_largest_component_and_fractional_rounding_keep_native_ties() {
        assert_eq!(
            first_largest_component(&[[0, 0], [9, 0], [0, 1], [9, 1]]),
            [[0, 0], [0, 1]]
        );
        assert_eq!(
            first_largest_component(&[[9, 0], [0, 0], [9, 1], [0, 1]]),
            [[9, 0], [9, 1]]
        );
        assert_eq!(
            component_crossing(&[[0, 0], [1, 1], [1, 1]]).unwrap(),
            [1, 1]
        );
        assert_eq!(
            component_crossing(&[[0, 0], [0, 0], [0, 0], [1, 1], [1, 1], [1, 1]]).unwrap(),
            [0, 0]
        );
    }

    #[test]
    fn native_line_ties_capacity_and_joined_route() {
        assert_eq!(
            native_line([3, 1], [0, 0], 20),
            [[0, 0], [1, 0], [2, 1], [3, 1]]
        );
        assert_eq!(native_line([0, 0], [1, 2], 20), [[0, 0], [1, 1], [1, 2]]);
        assert_eq!(native_line([0, 0], [2, 2], 2), [[0, 0], [1, 1]]);
        assert!(native_line([0, 0], [2, 2], 0).is_empty());
        assert_eq!(
            route_points([0, 0], [4, 0], [2, 0], 8, 4).unwrap(),
            [[0, 0], [1, 0], [2, 0], [3, 0], [4, 0]]
        );
    }
}
