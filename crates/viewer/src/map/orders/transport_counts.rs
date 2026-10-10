//! `140C3D550`'s idle transport-wing count and `140FFDCD0`'s region reach.
//! The count belongs to the viewer. The source supplies recorded simulation
//! operands and recorded-content definition categories only when a native
//! branch reaches them; registration order and repeated slots are retained.

use installed_content::province_geometry::{Bounds, ProvinceGeometry};
use replay::rows::{AirWingRow, ArmyOrderPlanRow};

/// `140721250`: allocated convoys are already stored on the invasion order.
pub(super) fn invasion(plan: &ArmyOrderPlanRow) -> Result<i32, String> {
    plan.invasion_transports
        .require("ArmyOrderPlans.invasion_transports")?
        .ok_or_else(|| "the invasion order's allocated convoy count is unavailable".to_owned())
}

pub(super) trait Sources {
    fn wing(&self, handle: u64) -> Result<&AirWingRow, String>;
    /// The containing pool's content definition. The capture authenticates
    /// the registration's wing backlink before joining this wing's pool.
    /// Only the authored `air_transport` bit (140F731B0 bit 20) is needed.
    fn is_transport_pool(&self, wing: &AirWingRow) -> Result<bool, String>;
    fn range(&self, wing: &AirWingRow) -> Result<i64, String>;
    fn transfer_resolves(&self, handle: u64) -> Result<bool, String>;
    /// `140F6B990`: base/mission restrictions followed by region reach.
    fn region_admits(&self, wing: &AirWingRow) -> Result<bool, String>;
}

/// The base's selected country-container wings, flattened in pool/wing
/// order after containing-pool authentication. A null container supplies an
/// empty list. The native sum uses signed planes and wraps at 32 bits.
// Count-only convenience for synthetic fixtures.
#[cfg(test)]
pub(super) fn count(
    registered: &[u64],
    distance: i64,
    has_target_region: bool,
    sources: &impl Sources,
) -> Result<i32, String> {
    Ok(totals(registered, distance, has_target_region, sources)?.plane_count)
}

/// `14101DC30` consumes the same admitted occurrences for its capacity.
/// Negative plane counts still affect the signed count, but contribute no
/// capacity (`140F4B5C0`). No native wing vector or derived total is recorded.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(super) struct AcceptedTotals {
    pub plane_count: i32,
    pub capacity_raw: i64,
}

pub(super) fn totals(
    registered: &[u64],
    distance: i64,
    has_target_region: bool,
    sources: &impl Sources,
) -> Result<AcceptedTotals, String> {
    let mut total = AcceptedTotals::default();
    for &handle in registered {
        let wing = sources.wing(handle)?;
        if !sources.is_transport_pool(wing)?
            || sources.range(wing)? < distance
            || wing.mission_mask != 0
            || wing.eligibility_veto != 0
            || wing.eligibility_counter > wing.eligibility_limit
            || (wing.base_override != 0 && sources.transfer_resolves(wing.base_override)?)
            || (has_target_region && !sources.region_admits(wing)?)
        {
            continue;
        }
        total.plane_count = total.plane_count.wrapping_add(wing.planes);
        total.capacity_raw = total
            .capacity_raw
            .wrapping_add(i64::from(wing.planes.max(0)).wrapping_mul(100_000));
    }
    Ok(total)
}

/// `140E67820 -> 140A51B90`: invalid signed province IDs use the reserved
/// slot-zero centroid. The abs-delta law and fixed5 square root are native.
pub(super) fn distance(map: &ProvinceGeometry, source: u32, destination: u32) -> i64 {
    if source == destination {
        return 0;
    }
    let point = |id: u32| {
        if (id as i32) > 0 {
            map.centroids.get(&id).copied().unwrap_or([0; 2])
        } else {
            [0; 2]
        }
    };
    installed_content::province_geometry::native_fixed_sqrt(province_squared(
        point(source),
        point(destination),
        map.width,
    ) as u32)
}

/// The template's `+0xE0/+0xE4` centre and `+0xF0` rectangles, built by
/// `141591FB0`. The first painted province chooses the primary cluster.
/// This is distinct from a state's minimum-width wrapped bounding box.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) struct RegionGeometry {
    centre: [i32; 2],
    rectangles: [Bounds; 2],
    count: usize,
}

/// FFDCD0 uses different wrapped-distance leaves for the two base anchors.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Origin {
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
    pub(super) fn new(map: &ProvinceGeometry, provinces: &[u32]) -> Self {
        let half = map.width / 2;
        let mut primary = [0; 4];
        let mut secondary = [0; 4];
        let mut first_x = 0_i32;
        for province in provinces {
            // Missing/unpainted CMap province records have no valid raster
            // box. Invalid native IDs resolve to the unpainted slot zero.
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

    /// `140FFDCD0(..., stop_after=1)`: preserve its nonconservative centre
    /// prefilter and split-box y intersection before testing provinces.
    /// `origin` is the native state bounding centre or the host ship's task
    /// force province position (CShip+728 -> TaskForce+1F0), selected by
    /// the source-base law.
    pub(super) fn reaches(
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
            // The CProvince lookup uses slot zero for invalid IDs; its
            // definition centroid is the origin. Unpainted valid records
            // likewise keep zero coordinates in the content geometry.
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

/// `14117A900` wraps once with strict half-width ties. The native distance
/// expression is signed i32, including overflow in the squared sum.
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

/// Hosted per-province reach uses 140E68830 -> 140A52400, whose absolute
/// delta differs from 14117A900 at i32::MIN. Preserve that overflow case.
fn province_squared(from: [i32; 2], to: [i32; 2], width: i32) -> i32 {
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
mod tests;
