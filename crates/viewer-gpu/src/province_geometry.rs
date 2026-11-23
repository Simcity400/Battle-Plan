use content_reader_map::map::MapData;
use std::collections::{BTreeMap, BTreeSet};

#[derive(Clone, Debug, Default, PartialEq)]
pub struct ProvinceGeometry {
    pub width: u32,
    centroids: Vec<Option<[i32; 2]>>,
    pixels: Vec<u32>,
    pub borders: BTreeMap<(u32, u32), Vec<[i32; 2]>>,
    pub centers: Centers,
}

impl ProvinceGeometry {
    pub fn build(map: &MapData) -> Self {
        let width = map.width as usize;
        let height = map.height as usize;
        let largest = map.province_ids.iter().copied().max().unwrap_or(0) as usize;
        let mut sums = vec![(0u64, 0u64, 0u64); largest + 1];
        let mut edge_pixels = Vec::new();
        for row in 0..height {
            for column in 0..width {
                let id = map.province_ids[row * width + column];
                let entry = &mut sums[id as usize];
                entry.0 += column as u64;
                entry.1 += (height - 1 - row) as u64;
                entry.2 += 1;
                let right = map.province_ids[row * width + (column + 1) % width];
                if right != id {
                    edge_pixels.push((id, right, (height - 1 - row) as u32, column as u32));
                    edge_pixels.push((
                        right,
                        id,
                        (height - 1 - row) as u32,
                        ((column + 1) % width) as u32,
                    ));
                }
                if row + 1 < height {
                    let below = map.province_ids[(row + 1) * width + column];
                    if below != id {
                        edge_pixels.push((id, below, (height - 1 - row) as u32, column as u32));
                        edge_pixels.push((below, id, (height - 2 - row) as u32, column as u32));
                    }
                }
            }
        }
        edge_pixels.sort_unstable();
        edge_pixels.dedup();
        let mut borders = BTreeMap::new();
        let mut edges = edge_pixels.as_slice();
        while let Some(&(a, b, _, _)) = edges.first() {
            let count = edges.partition_point(|p| (p.0, p.1) == (a, b));
            let points: Vec<_> = edges[..count]
                .iter()
                .map(|p| [p.3 as f32, p.2 as f32])
                .collect();
            let sorted = sort_edge_pixels(&points)
                .into_iter()
                .map(|p| [p[0] as i32, p[1] as i32])
                .collect();
            borders.insert((a, b), sorted);
            edges = &edges[count..];
        }
        let pixels = sums.iter().map(|entry| entry.2 as u32).collect();
        let centroids: Vec<Option<[i32; 2]>> = sums
            .into_iter()
            .map(|(sx, sy, count)| {
                (count > 0).then(|| {
                    let count = count as u32 as i32;
                    [
                        (sx as u32 as i32).wrapping_div(count),
                        (sy as u32 as i32).wrapping_div(count),
                    ]
                })
            })
            .collect();
        Self {
            width: map.width,
            centers: Centers::build(map, &centroids),
            centroids,
            pixels,
            borders,
        }
    }

    pub fn pixel_count(&self, province: u32) -> Option<u32> {
        self.pixels.get(province as usize).copied()
    }

    pub fn centroid(&self, province: u32) -> Option<[f32; 2]> {
        self.pixel_centroid(province)
            .map(|[x, z]| [x as f32, z as f32])
    }

    pub fn map_position(&self, province: u32) -> Option<[i32; 2]> {
        self.pixel_centroid(province)
    }

    fn pixel_centroid(&self, province: u32) -> Option<[i32; 2]> {
        self.centroids.get(province as usize).copied().flatten()
    }

    pub fn provinces(&self) -> usize {
        self.centroids.iter().flatten().count()
    }
}

pub fn sort_edge_pixels(points: &[[f32; 2]]) -> Vec<[f32; 2]> {
    if points.len() < 2 {
        return points.to_vec();
    }
    let pixels: BTreeSet<_> = points.iter().map(|p| (p[0] as i32, p[1] as i32)).collect();
    let seed = points
        .iter()
        .enumerate()
        .min_by_key(|(_, p)| {
            let mut count = 0;
            for dx in -4i32..=4 {
                for dz in -4i32..=4 {
                    if dx.abs() + dz.abs() < 5
                        && pixels.contains(&(p[0] as i32 + dx, p[1] as i32 + dz))
                    {
                        count += 1;
                    }
                }
            }
            count
        })
        .unwrap()
        .0;
    let mut remaining = points.to_vec();
    let mut out = vec![remaining.swap_remove(seed)];
    let squared = |a: [f32; 2], b: [f32; 2]| (a[0] - b[0]).powi(2) + (a[1] - b[1]).powi(2);
    while !remaining.is_empty() {
        let mut tail = (0, f32::MAX);
        let mut head = (0, f32::MAX);
        for (i, &p) in remaining.iter().enumerate() {
            let dt = squared(p, *out.last().unwrap());
            let dh = squared(p, out[0]);
            if dt < tail.1 {
                tail = (i, dt);
            }
            if dh < dt - 0.001 && dh < head.1 {
                head = (i, dh);
            }
        }
        let index = if head.1 < tail.1 {
            out.reverse();
            head.0
        } else {
            tail.0
        };
        out.push(remaining.swap_remove(index));
    }
    out
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Centers {
    pub centers: BTreeMap<u32, [i64; 2]>,
}

impl Centers {
    pub fn build(map: &MapData, centroids: &[Option<[i32; 2]>]) -> Self {
        let mut centers = BTreeMap::new();
        let largest = map.province_ids.iter().copied().max().unwrap_or(0) as usize;
        let mut candidates = vec![None; largest + 1];
        for (province, centroid) in centroids.iter().enumerate() {
            let Some(centroid) = centroid else { continue };
            if province == 0 || province > largest {
                continue;
            }
            let centroid = centroid.map(i64::from);
            if province_at(map, centroid) == province as u32 {
                centers.insert(province as u32, centroid);
            } else {
                candidates[province] = Some((centroid, i64::MAX, None));
            }
        }
        if candidates.iter().any(Option::is_some) {
            for z in 0..map.height {
                let row = (map.height - 1 - z) as usize * map.width as usize;
                for x in 0..map.width {
                    let province = map.province_ids[row + x as usize];
                    let Some((centroid, distance, closest)) = &mut candidates[province as usize]
                    else {
                        continue;
                    };
                    let point = [i64::from(x), i64::from(z)];
                    let next = (point[0] - centroid[0]).abs() + (point[1] - centroid[1]).abs();
                    if next < *distance {
                        *distance = next;
                        *closest = Some(point);
                    }
                }
            }
        }
        centers.extend(
            candidates
                .into_iter()
                .enumerate()
                .filter_map(|(province, candidate)| {
                    candidate?.2.map(|point| (province as u32, point))
                }),
        );
        Self { centers }
    }
}

pub fn province_at(map: &MapData, [x, z]: [i64; 2]) -> u32 {
    if x < 0 || z < 0 || x >= i64::from(map.width) || z >= i64::from(map.height) {
        return 0;
    }
    let row = map.height as usize - 1 - z as usize;
    map.province_ids[row * map.width as usize + x as usize]
}

#[cfg(test)]
mod tests;
