
use content_reader_map::province_geometry::ProvinceGeometry;
use replay_data::rows::{AirWingRow, ArmyOrderPlanRow};

pub(super) fn invasion(plan: &ArmyOrderPlanRow) -> Result<i32, String> {
    plan.invasion_transports
        .require("ArmyOrderPlans.invasion_transports")?
        .ok_or_else(|| "the invasion order's allocated convoy count is unavailable".to_owned())
}

pub(super) trait Sources {
    fn wing(&self, handle: u64) -> Result<&AirWingRow, String>;
    fn is_transport_pool(&self, wing: &AirWingRow) -> Result<bool, String>;
    fn range(&self, wing: &AirWingRow) -> Result<i64, String>;
    fn transfer_resolves(&self, handle: u64) -> Result<bool, String>;
    fn region_admits(&self, wing: &AirWingRow) -> Result<bool, String>;
}

#[cfg(test)]
pub(super) fn count(
    registered: &[u64],
    distance: i64,
    has_target_region: bool,
    sources: &impl Sources,
) -> Result<i32, String> {
    Ok(totals(registered, distance, has_target_region, sources)?.plane_count)
}

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
    content_reader_map::province_geometry::native_fixed_sqrt(
        viewer_unit_reads::air_region_reach::province_squared(
            point(source),
            point(destination),
            map.width,
        ) as u32,
    )
}

pub(super) use viewer_unit_reads::air_region_reach::{Origin, RegionGeometry};

#[cfg(test)]
mod tests;
