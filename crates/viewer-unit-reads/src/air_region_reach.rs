use content_reader_map::province_geometry::{Bounds, ProvinceGeometry};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct RegionGeometry {
    centre: [i32; 2],
    rectangles: [Bounds; 2],
    count: usize,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Origin {
    State([i32; 2]),
    Host([i32; 2]),
}

impl Origin {
    fn point(self) -> [i32; 2] {
        match self {
            Self::State(point) | Self::Host(point) => point,
        }
    }
}

impl RegionGeometry {
    pub fn new(map: &ProvinceGeometry, provinces: &[u32]) -> Self {
        let half = map.width / 2;
        let mut primary = [0; 4];
        let mut secondary = [0; 4];
        let mut first_x = 0_i32;
        for province in provinces {
            let Some(&bounds) = map.bounds.get(province) else {
                continue;
            };
            if primary[2] == 0 {
                primary = bounds;
                first_x = bounds[0];
            } else if secondary[2] == 0 && bounds[0].wrapping_sub(first_x).wrapping_abs() > half {
                secondary = bounds;
            } else if secondary[2] > 0 && (bounds[0] > half) != (first_x > half) {
                grow(&mut secondary, bounds);
            } else {
                grow(&mut primary, bounds);
            }
        }
        let mut centre = [
            primary[0].wrapping_add(primary[2] / 2),
            primary[1].wrapping_add(primary[3] / 2),
        ];
        let count = if secondary[2] > 0 {
            let right_x = if first_x > half {
                primary[0]
            } else {
                secondary[0]
            };
            centre[0] = (primary[2].wrapping_add(secondary[2]) / 2)
                .wrapping_sub(map.width)
                .wrapping_add(right_x);
            2
        } else {
            1
        };
        Self {
            centre,
            rectangles: [primary, secondary],
            count,
        }
    }

    pub fn reaches(
        &self,
        map: &ProvinceGeometry,
        provinces: &[u32],
        origin: Origin,
        range: i64,
    ) -> bool {
        let pixels = (range / 100_000) as i32;
        let squared_range = pixels.wrapping_mul(pixels);
        let mut widths = 0_i32;
        let mut bottom = 0_i32;
        let mut top = map.height;
        for &[_, y, width, height] in &self.rectangles[..self.count] {
            widths = widths.wrapping_add(width);
            bottom = bottom.max(y);
            top = top.min(y.wrapping_add(height));
        }
        let height = top.wrapping_sub(bottom);
        if height <= 0 {
            return false;
        }
        let half_height = height / 2;
        let half_width = widths / 2;
        let bound = squared_range
            .wrapping_add(half_width.wrapping_mul(half_width))
            .wrapping_add(half_height.wrapping_mul(half_height));
        if wrapped_squared(origin.point(), self.centre, map.width) > bound {
            return false;
        }
        provinces.iter().any(|id| {
            let point = map.centroids.get(id).copied().unwrap_or([0; 2]);
            let distance = match origin {
                Origin::State(origin) => wrapped_squared(point, origin, map.width),
                Origin::Host(origin) => province_squared(point, origin, map.width),
            };
            distance <= squared_range
        })
    }
}

fn grow(rectangle: &mut Bounds, [x, y, width, height]: Bounds) {
    if x < rectangle[0] {
        rectangle[2] = rectangle[2].wrapping_add(rectangle[0].wrapping_sub(x));
        rectangle[0] = x;
    }
    if rectangle[0].wrapping_add(rectangle[2]) < x.wrapping_add(width) {
        rectangle[2] = width.wrapping_sub(rectangle[0]).wrapping_add(x);
    }
    if y < rectangle[1] {
        rectangle[3] = rectangle[3].wrapping_add(rectangle[1].wrapping_sub(y));
        rectangle[1] = y;
    }
    if rectangle[1].wrapping_add(rectangle[3]) < y.wrapping_add(height) {
        rectangle[3] = height.wrapping_add(y).wrapping_sub(rectangle[1]);
    }
}

fn wrapped_squared(from: [i32; 2], to: [i32; 2], width: i32) -> i32 {
    let mut dx = from[0].wrapping_sub(to[0]);
    let dy = from[1].wrapping_sub(to[1]);
    if dx > width / 2 {
        dx = dx.wrapping_sub(width);
    } else if dx < -(width / 2) {
        dx = dx.wrapping_add(width);
    }
    dx.wrapping_mul(dx).wrapping_add(dy.wrapping_mul(dy))
}

pub fn province_squared(from: [i32; 2], to: [i32; 2], width: i32) -> i32 {
    let delta = from[0].wrapping_sub(to[0]);
    let absolute = delta.wrapping_abs();
    let dx = if absolute > width / 2 {
        width.wrapping_sub(absolute)
    } else {
        delta
    };
    let dy = from[1].wrapping_sub(to[1]);
    dx.wrapping_mul(dx).wrapping_add(dy.wrapping_mul(dy))
}

#[cfg(test)]
mod tests {
    use super::*;
    fn map(bounds: &[(u32, Bounds)], centroids: &[(u32, [i32; 2])]) -> ProvinceGeometry {
        ProvinceGeometry {
            width: 100,
            height: 100,
            bounds: bounds.iter().copied().collect(),
            centroids: centroids.iter().copied().collect(),
            ..Default::default()
        }
    }

    #[test]
    fn first_painted_province_and_strict_half_width_choose_split_boxes() {
        let geometry = map(
            &[(1, [10, 0, 2, 4]), (2, [80, 10, 2, 4]), (3, [49, 30, 2, 4])],
            &[],
        );
        let forward = RegionGeometry::new(&geometry, &[99, 1, 2, 3]);
        assert_eq!(forward.rectangles, [[10, 0, 41, 34], [80, 10, 2, 4]]);
        assert_eq!(forward.centre, [1, 17]);
        let reverse = RegionGeometry::new(&geometry, &[2, 1, 3]);
        assert_eq!(reverse.rectangles, [[80, 10, 2, 4], [10, 0, 41, 34]]);
        assert_eq!(reverse.centre, [1, 12]);
        let tied = map(&[(1, [10, 0, 2, 2]), (2, [60, 0, 2, 2])], &[]);
        assert_eq!(RegionGeometry::new(&tied, &[1, 2]).count, 1);
    }

    #[test]
    fn native_prefilter_can_discard_a_province_at_the_range_boundary() {
        let geometry = map(&[(1, [40, 0, 2, 2])], &[(1, [40, 1])]);
        let region = RegionGeometry::new(&geometry, &[1]);
        assert_eq!(wrapped_squared([40, 1], [0, 1], 100), 1600);
        assert!(!region.reaches(&geometry, &[1], Origin::State([0, 1]), 4_000_000));
        assert!(region.reaches(&geometry, &[1], Origin::State([1, 1]), 4_000_000));
        assert!(region.reaches(&geometry, &[1], Origin::Host([1, 1]), 4_000_000));
    }

    #[test]
    fn disjoint_split_y_spans_refuse_reach_before_the_province_scan() {
        let geometry = map(
            &[(1, [10, 0, 2, 2]), (2, [80, 20, 2, 2])],
            &[(1, [10, 0]), (2, [80, 20])],
        );
        let region = RegionGeometry::new(&geometry, &[1, 2]);
        assert!(!region.reaches(&geometry, &[1, 2], Origin::State([10, 0]), 10_000_000));
        assert!(!RegionGeometry::new(&geometry, &[]).reaches(
            &geometry,
            &[],
            Origin::State([0, 0]),
            10_000_000
        ));
    }

    #[test]
    fn distance_wraps_once_and_preserves_i32_squared_overflow() {
        assert_eq!(wrapped_squared([0, 0], [50, 0], 100), 2500);
        assert_eq!(wrapped_squared([0, 0], [51, 0], 100), 2401);
        assert_eq!(wrapped_squared([0, 0], [251, 0], 100), 22801);
        assert_eq!(wrapped_squared([0, 50_000], [0, 0], 100), -1_794_967_296);
        assert_eq!(wrapped_squared([i32::MIN, 0], [0, 0], 100), 10_000);
        assert_eq!(province_squared([i32::MIN, 0], [0, 0], 100), 0);
    }
}
