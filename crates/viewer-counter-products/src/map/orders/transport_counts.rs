//! Idle transport-wing count and region reach. The count belongs to the
//! viewer. The source supplies recorded simulation operands and content
//! definition categories only when a branch reaches them; registration order
//! and repeated slots are retained.

use installed_content_core::province_geometry::ProvinceGeometry;
use replay_data::rows::{AirWingRow, ArmyOrderPlanRow};

/// Allocated convoys are already stored on the invasion order.
pub(super) fn invasion(plan: &ArmyOrderPlanRow) -> Result<i32, String> {
    plan.invasion_transports
        .require("ArmyOrderPlans.invasion_transports")?
        .ok_or_else(|| "the invasion order's allocated convoy count is unavailable".to_owned())
}

pub(super) trait Sources {
    fn wing(&self, handle: u64) -> Result<&AirWingRow, String>;
    /// The containing pool's content definition. The capture authenticates
    /// the registration's wing backlink before joining this wing's pool.
    /// Only the authored `air_transport` bit (bit 20) is needed.
    fn is_transport_pool(&self, wing: &AirWingRow) -> Result<bool, String>;
    fn range(&self, wing: &AirWingRow) -> Result<i64, String>;
    fn transfer_resolves(&self, handle: u64) -> Result<bool, String>;
    /// Base/mission restrictions followed by region reach.
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

/// Capacity consumes the same admitted occurrences. Negative plane counts
/// still affect the signed count but contribute no capacity. No wing vector
/// or derived total is recorded.
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

/// Invalid signed province IDs use the reserved slot-zero centroid.
/// Preserve the absolute-delta law and fixed5 square root.
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
    installed_content_core::province_geometry::native_fixed_sqrt(
        viewer_foundation::map::air_region_reach::province_squared(
            point(source),
            point(destination),
            map.width,
        ) as u32,
    )
}

pub(super) use viewer_foundation::map::air_region_reach::{Origin, RegionGeometry};

#[cfg(test)]
mod tests;
