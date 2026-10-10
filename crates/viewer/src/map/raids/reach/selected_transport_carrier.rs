//! Current-hour numerical joins for the original carrier and night graph.
//! The receiver's ordered SHIPS packet is produced by the combined original
//! visitor, including fading receivers and repeated handles. Air tables lend
//! their existing original authenticated dense/indexed/pool/wing observations.
use super::*;
use replay::rows::{
    AirBaseRegistrationRow, AirBaseRow, CarrierAirPool, Recorded, TableSchema,
    TransportMissionReceiver, TransportReceiverShips, TransportRiskShips,
};
use replay::{transport_carrier_graph as carrier, transport_damage_graph as damage};

fn hourly<'a, T: TableSchema>(moves: &Moves<'_, 'a>) -> damage::Fact<&'a [T::Row]> {
    if replay::cadence::cadence(T::ID) != replay::cadence::Cadence::Hourly {
        return Err("transport carrier/night table no longer has a current-hour source contract");
    }
    moves
        .table::<T>()
        .map_err(|_| "transport original current-hour carrier/night table refused")
}

fn original_pool_planes(
    registration: &AirBaseRegistrationRow,
    pool_count: usize,
    ordinal: u32,
) -> carrier::Fact<i32> {
    let planes = match &registration.transport_planes {
        Recorded::Value(Some(planes)) => planes,
        Recorded::Value(None) => {
            return Err("transport original carrier complete plane prefix refused");
        }
        Recorded::NotRecorded => {
            return Err("transport original carrier plane sums were not recorded");
        }
    };
    if planes.len() != pool_count {
        return Err("transport original carrier plane sums do not cover ordered pools");
    }
    planes
        .get(ordinal as usize)
        .ok_or("transport original carrier plane sum ordinal outside prefix")?
        .ok_or("transport original carrier pool plane sum refused")
}

pub(super) fn fill(
    moves: &Moves<'_, '_>,
    receiver: &TransportMissionReceiver,
    packet: &TransportRiskShips,
) -> damage::Fact<i64> {
    let count = receiver
        .risk
        .as_ref()
        .and_then(|risk| risk.ship_count)
        .ok_or("transport original carrier ship header refused")?;
    if count == 0 {
        return Ok(0);
    }
    if count < 0 {
        return Err("transport original carrier ship count is signed-negative");
    }
    if packet.sample_hour != moves.world.frame().hour {
        return Err("transport original SHIPS numerical packet is stale");
    }
    let receivers = packet
        .receivers
        .as_ref()
        .ok_or("transport original SHIPS receiver dictionary refused")?;
    let index = receivers
        .binary_search_by_key(&receiver.task_force, |row| row.task_force)
        .map_err(|_| "transport original SHIPS receiver missing")?;
    let source = &receivers[index];
    if source.count != Some(count) {
        return Err("transport original SHIPS signed count differs from receiver graph");
    }
    let ships = source
        .ships
        .as_ref()
        .ok_or("transport original SHIPS complete prefix refused")?;
    if usize::try_from(count).ok() != Some(ships.len()) {
        return Err("transport original SHIPS numerical prefix does not cover its count");
    }
    let mut input = Carrier { source, moves };
    carrier::fill_sum(&mut input)
}

struct Carrier<'m, 'w, 'a, 'r> {
    moves: &'m Moves<'w, 'a>,
    source: &'r TransportReceiverShips,
}
impl Carrier<'_, '_, '_, '_> {
    fn base(&self, key: u64) -> damage::Fact<&AirBaseRow> {
        let rows = hourly::<tables::AirBases>(self.moves)?;
        let index = rows
            .binary_search_by_key(&key, |row| row.id)
            .map_err(|_| "transport original carrier base full handle missing")?;
        Ok(&rows[index])
    }
    fn registration(&self, base: u64, occurrence: u32) -> damage::Fact<&AirBaseRegistrationRow> {
        let countries = match &self.base(base)?.carrier {
            Recorded::Value(Some(packet)) => packet
                .registered_countries
                .as_ref()
                .ok_or("transport original carrier dense country prefix refused")?,
            _ => return Err("transport original carrier base suffix unavailable"),
        };
        let country = countries
            .get(occurrence as usize)
            .ok_or("transport original carrier country ordinal outside prefix")?;
        let rows = hourly::<tables::AirBaseRegistrations>(self.moves)?;
        let key = (base, country.slot);
        let index = rows
            .binary_search_by_key(&key, |row| (row.base, row.slot))
            .map_err(|_| "transport original carrier country registration missing")?;
        let row = &rows[index];
        let recorded_country = match &row.carrier {
            Recorded::Value(Some(packet)) => packet.country,
            _ => return Err("transport original carrier registration suffix unavailable"),
        };
        let slot = self
            .moves
            .slot(recorded_country)
            .map_err(|_| "transport carrier registration country link refused")?;
        let dense_slot = self
            .moves
            .slot(country.country)
            .map_err(|_| "transport carrier dense country link refused")?;
        if slot != country.slot || dense_slot != country.slot {
            return Err("transport carrier dense/indexed country association differs");
        }
        Ok(row)
    }
    fn pools(&self, base: u64, occurrence: u32) -> damage::Fact<&[CarrierAirPool]> {
        let row = self.registration(base, occurrence)?;
        let pools = match &row.carrier {
            Recorded::Value(Some(packet)) => packet
                .pools
                .as_deref()
                .ok_or("transport original carrier complete pool prefix refused")?,
            _ => return Err("transport original carrier registration suffix unavailable"),
        };
        let mut consumed = 0_usize;
        for pool in pools {
            if usize::try_from(pool.wing_start).ok() != Some(consumed) {
                return Err("transport original carrier pool wing ranges do not partition prefix");
            }
            consumed = consumed
                .checked_add(pool.wing_count as usize)
                .ok_or("transport original carrier wing range overflow")?;
            if consumed > row.wings.len() {
                return Err("transport original carrier pool wing range outside prefix");
            }
        }
        if consumed != row.wings.len() {
            return Err("transport original carrier wing prefix has uncovered occurrences");
        }
        Ok(pools)
    }
    fn pool(&self, base: u64, container: u32, pool: u32) -> damage::Fact<&CarrierAirPool> {
        self.pools(base, container)?
            .get(pool as usize)
            .ok_or("transport original carrier pool ordinal outside prefix")
    }
}

impl carrier::Inputs for Carrier<'_, '_, '_, '_> {
    fn original_ship_count(&mut self) -> carrier::Fact<i32> {
        self.source
            .count
            .ok_or("transport original carrier ship count refused")
    }
    fn original_ship(&mut self, ordinal: u32) -> carrier::Fact<carrier::Ship> {
        let row = self
            .source
            .ships
            .as_ref()
            .and_then(|ships| ships.get(ordinal as usize))
            .ok_or("transport original carrier ship occurrence missing")?;
        // Authentication belongs to the original receiver/ship loan. This
        // late numerical join checks the same full identity and base backlink.
        let ship = row
            .ship
            .ok_or("transport original carrier ship identity refused")?;
        let mut observation = carrier::Ship {
            formation_flags: row
                .formation_flags
                .ok_or("transport original carrier formation flags refused"),
            positive_equipment_flags: row
                .positive_equipment_flags
                .ok_or("transport original carrier equipment flag union refused"),
            carrier_air_base: Err("transport noncarrier base query unreached"),
        };
        // Check carrier eligibility before demanding any carrier base or table.
        // A later stored bit cannot bypass the first-pool failure.
        if carrier::is_carrier(observation) == Ok(true) {
            observation.carrier_air_base = row
                .carrier_air_base
                .ok_or("transport original carrier base selector refused")
                .and_then(|base| {
                    if let Some(base) = base
                        && self.base(base)?.host != ship
                    {
                        return Err("transport original carrier base host differs from ship loan");
                    }
                    Ok(base)
                });
        }
        Ok(observation)
    }
    fn carrier_capacity(&mut self, base: u64) -> carrier::Fact<i32> {
        Ok(self.base(base)?.capacity)
    }
    fn registered_container_count(&mut self, base: u64) -> carrier::Fact<i32> {
        match &self.base(base)?.carrier {
            Recorded::Value(Some(packet)) => i32::try_from(
                packet
                    .registered_countries
                    .as_ref()
                    .ok_or("transport original carrier dense country prefix refused")?
                    .len(),
            )
            .map_err(|_| "transport carrier container count exceeds signed native count"),
            _ => Err("transport original carrier base suffix unavailable"),
        }
    }
    fn pool_count(&mut self, base: u64, container: u32) -> carrier::Fact<i32> {
        i32::try_from(self.pools(base, container)?.len())
            .map_err(|_| "transport carrier pool count exceeds signed native count")
    }
    fn wing_count(&mut self, base: u64, container: u32, pool: u32) -> carrier::Fact<i32> {
        i32::try_from(self.pool(base, container, pool)?.wing_count)
            .map_err(|_| "transport carrier wing count exceeds signed native count")
    }
    fn pool_planes(&mut self, base: u64, container: u32, pool: u32) -> carrier::Fact<i32> {
        let registration = self.registration(base, container)?;
        let pools = self.pools(base, container)?;
        original_pool_planes(registration, pools.len(), pool)
    }
    fn wing_planes(
        &mut self,
        base: u64,
        container: u32,
        pool: u32,
        wing: u32,
    ) -> carrier::Fact<i32> {
        let row = self.registration(base, container)?;
        let pool = self.pool(base, container, pool)?;
        if wing >= pool.wing_count {
            return Err("transport carrier wing ordinal outside pool");
        }
        let index = pool
            .wing_start
            .checked_add(wing)
            .ok_or("transport original carrier wing ordinal overflow")?;
        let key = *row
            .wings
            .get(index as usize)
            .ok_or("transport original carrier wing occurrence missing")?;
        let wings = hourly::<tables::AirWings>(self.moves)?;
        let index = wings
            .binary_search_by_key(&key, |row| row.id)
            .map_err(|_| "transport original carrier wing full handle missing")?;
        Ok(wings[index].planes)
    }
}

pub(super) fn night(
    moves: &Moves<'_, '_>,
    receiver: &TransportMissionReceiver,
) -> damage::Fact<i64> {
    let location = receiver
        .risk
        .as_ref()
        .and_then(|packet| packet.location)
        .ok_or("transport original simulation-night actual location refused")?;
    let rows = hourly::<tables::WeatherProvinces>(moves)?;
    let index = rows
        .binary_search_by_key(&location, |row| row.id)
        .map_err(|_| "transport original simulation-night province missing")?;
    match rows[index].night_factor {
        Recorded::Value(raw) => Ok(raw),
        Recorded::NotRecorded => Err("transport simulation-night raw factor was not recorded"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn selected_carrier_preserves_original_pool_ordinals_and_failed_leaves() {
        let row = AirBaseRegistrationRow {
            // Original sums are independent of late global wing availability.
            wings: Vec::new(),
            transport_planes: Recorded::Value(Some(vec![
                Some(i32::MIN),
                Some(0),
                None,
                Some(i32::MAX),
            ])),
            ..Default::default()
        };
        assert_eq!(original_pool_planes(&row, 4, 0), Ok(i32::MIN));
        assert_eq!(original_pool_planes(&row, 4, 1), Ok(0));
        assert_eq!(
            original_pool_planes(&row, 4, 2),
            Err("transport original carrier pool plane sum refused")
        );
        assert_eq!(original_pool_planes(&row, 4, 3), Ok(i32::MAX));
        assert!(original_pool_planes(&row, 4, 4).is_err());
        assert!(original_pool_planes(&row, 3, 0).is_err());
    }

    #[test]
    fn selected_carrier_historical_absence_and_complete_prefix_refusal_are_distinct() {
        let mut row = AirBaseRegistrationRow {
            transport_planes: Recorded::NotRecorded,
            ..Default::default()
        };
        assert_eq!(
            original_pool_planes(&row, 1, 0),
            Err("transport original carrier plane sums were not recorded")
        );
        row.transport_planes = Recorded::Value(None);
        assert_eq!(
            original_pool_planes(&row, 1, 0),
            Err("transport original carrier complete plane prefix refused")
        );
        row.transport_planes = Recorded::Value(Some(Vec::new()));
        assert!(original_pool_planes(&row, 0, 0).is_err());
        row.transport_planes = Recorded::Value(Some(vec![Some(0)]));
        assert_eq!(original_pool_planes(&row, 1, 0), Ok(0));
    }
}
