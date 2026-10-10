//! Integrated compact attack adapters; source-only, with no pointer reads during projection.
use engine::{Memory, carmy, cprovince, selected_attack as native, walk::Outcome};
use replay::rows::{self, Recorded};

#[derive(Clone, Copy, PartialEq, Eq)]
pub(super) struct UnitCapacities(pub native::UnitCapacity);
impl Default for UnitCapacities {
    fn default() -> Self {
        Self(native::UnitCapacity {
            rows: 0,
            terrains: 0,
            values: 0,
        })
    }
}

pub(super) struct Units {
    pub(super) packet: native::UnitsPacket,
    hour: i32,
}
impl Units {
    pub(super) fn vacant() -> Self {
        Self {
            packet: native::UnitsPacket::vacant(),
            hour: 0,
        }
    }
    pub(super) fn new(capacity: UnitCapacities) -> Self {
        Self {
            packet: native::UnitsPacket::try_new(capacity.0)
                .map_err(crate::allocation::io)
                .unwrap_or_else(|_| native::UnitsPacket::vacant()),
            hour: 0,
        }
    }
    pub(super) fn reset(&mut self) {
        self.packet.clear();
        self.hour = 0;
    }
    pub(super) fn fail(&mut self, reason: &'static str) {
        self.packet.fail(reason);
    }
    pub(super) fn stamp_hour(&mut self, hour: i32) {
        self.hour = hour;
    }
    pub(super) fn allocated_bytes(&self) -> usize {
        self.packet.allocated_bytes()
    }
    pub(super) fn reserve_sources(&mut self, rows: usize, capacity: &mut UnitCapacities) {
        crate::table_capture::grow_capacity(&mut capacity.0.rows, rows);
        let _ = self
            .packet
            .reserve(capacity.0)
            .map_err(crate::allocation::io);
    }
    pub(super) fn grow(&mut self, capacity: &mut UnitCapacities) {
        let required = self.packet.required();
        crate::table_capture::grow_capacity(&mut capacity.0.rows, required.rows);
        crate::table_capture::grow_capacity(&mut capacity.0.terrains, required.terrains);
        crate::table_capture::grow_capacity(&mut capacity.0.values, required.values);
        let _ = self
            .packet
            .reserve(capacity.0)
            .map_err(crate::allocation::io);
    }
    pub(super) fn capture(
        &mut self,
        memory: &impl Memory,
        base: usize,
        hour: i32,
        seeds: &[carmy::DivisionSeed],
        outcome: Outcome,
        geometry: &mut cprovince::GeometryPacket,
    ) {
        self.hour = hour;
        self.packet.enable_growth();
        geometry.enable_growth();
        self.packet
            .capture(memory, base, hour, seeds, outcome, geometry);
    }
    fn division_row(&self, division: u64) -> Option<&native::DivisionRow> {
        if !matches!(self.packet.outcome(), Outcome::Rows { .. }) {
            return None;
        }
        self.packet
            .rows()
            .binary_search_by_key(&division, |row| row.division)
            .ok()
            .and_then(|index| self.packet.rows().get(index))
    }
    fn terrain(&self, span: native::Field<native::Span>) -> Option<Vec<rows::AttackTerrain>> {
        let values = self.packet.values(span.ok()?)?;
        Some(engine::allocation::owned_vec(
            engine::allocation::collect_vec(values.iter().map(|value| rows::AttackTerrain {
                terrain: value.terrain,
                attack: value.attack.ok(),
            })),
        ))
    }
    fn terrain_matches(
        &self,
        span: native::Field<native::Span>,
        prior: &Option<Vec<rows::AttackTerrain>>,
    ) -> bool {
        match (span.ok().and_then(|span| self.packet.values(span)), prior) {
            (None, None) => true,
            (Some(values), Some(prior)) => {
                values.len() == prior.len()
                    && values
                        .iter()
                        .zip(prior)
                        .all(|(a, b)| a.terrain == b.terrain && a.attack.ok() == b.attack)
            }
            _ => false,
        }
    }
    pub(super) fn division(&self, division: u64) -> rows::DivisionAttackInputs {
        let Some(row) = self.division_row(division) else {
            return rows::DivisionAttackInputs {
                sample_hour: self.hour,
                ..Default::default()
            };
        };
        rows::DivisionAttackInputs {
            sample_hour: row.sample_hour,
            template: row.template.ok(),
            stats: rows::AttackStats {
                river: [row.stats[0].ok(), row.stats[1].ok()],
                fort: [row.stats[2].ok(), row.stats[3].ok()],
                snow: row.stats[4].ok(),
                night: row.stats[5].ok(),
            },
            terrain: self.terrain(row.terrain),
            acclimatization: row.acclimatization.ok(),
        }
    }
    pub(super) fn matches(
        &self,
        division: u64,
        prior: &Recorded<rows::DivisionAttackInputs>,
    ) -> bool {
        let Recorded::Value(prior) = prior else {
            return false;
        };
        let Some(row) = self.division_row(division) else {
            return prior
                == &rows::DivisionAttackInputs {
                    sample_hour: self.hour,
                    ..Default::default()
                };
        };
        row.sample_hour == prior.sample_hour
            && row.template.ok() == prior.template
            && [row.stats[0].ok(), row.stats[1].ok()] == prior.stats.river
            && [row.stats[2].ok(), row.stats[3].ok()] == prior.stats.fort
            && row.stats[4].ok() == prior.stats.snow
            && row.stats[5].ok() == prior.stats.night
            && row.acclimatization.ok() == prior.acclimatization
            && self.terrain_matches(row.terrain, &prior.terrain)
    }
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub(super) struct GroupCapacities(pub native::GroupCapacity);
impl Default for GroupCapacities {
    fn default() -> Self {
        Self(native::GroupCapacity { rows: 0, values: 0 })
    }
}
pub(super) struct Groups {
    pub(super) packet: native::GroupsPacket,
    hour: i32,
}
impl Groups {
    pub(super) fn new(capacity: GroupCapacities) -> Self {
        Self {
            packet: native::GroupsPacket::try_new(capacity.0)
                .map_err(crate::allocation::io)
                .unwrap_or_else(|_| native::GroupsPacket::vacant()),
            hour: 0,
        }
    }
    pub(super) fn reset(&mut self) {
        self.packet.clear();
        self.hour = 0;
    }
    pub(super) fn allocated_bytes(&self) -> usize {
        self.packet.allocated_bytes()
    }
    pub(super) fn grow(&mut self, capacity: &mut GroupCapacities, rows: usize) {
        let required = self.packet.required();
        crate::table_capture::grow_capacity(&mut capacity.0.rows, rows.max(required.rows));
        crate::table_capture::grow_capacity(&mut capacity.0.values, required.values);
        let _ = self
            .packet
            .reserve(capacity.0)
            .map_err(crate::allocation::io);
    }
    #[allow(clippy::too_many_arguments)]
    pub(super) fn capture(
        &mut self,
        memory: &impl Memory,
        base: usize,
        hour: i32,
        controls: &[engine::corders_group::DirectCommander],
        source: Outcome,
        units: &mut Units,
        geometry: &mut cprovince::GeometryPacket,
        current_geometry: native::Field<()>,
    ) {
        self.hour = hour;
        self.packet.enable_growth();
        units.packet.enable_growth();
        geometry.enable_growth();
        self.packet.capture(
            memory,
            base,
            hour,
            controls,
            source,
            &mut units.packet,
            geometry,
            current_geometry,
        );
        self.packet.finish_native();
    }
    fn terrain(&self, span: native::Field<native::Span>) -> Option<Vec<rows::AttackTerrain>> {
        let values = self.packet.values(span.ok()?)?;
        Some(engine::allocation::owned_vec(
            engine::allocation::collect_vec(values.iter().map(|value| rows::AttackTerrain {
                terrain: value.terrain,
                attack: value.attack.ok(),
            })),
        ))
    }
    pub(super) fn group(&self, group: u64) -> rows::OrdersGroupAttackInputs {
        let row = matches!(self.packet.outcome(), Outcome::Rows { .. })
            .then(|| {
                self.packet
                    .rows()
                    .binary_search_by_key(&group, |row| row.group)
                    .ok()
                    .and_then(|index| self.packet.rows().get(index))
            })
            .flatten();
        let Some(row) = row else {
            return rows::OrdersGroupAttackInputs {
                sample_hour: self.hour,
                commander: None,
            };
        };
        rows::OrdersGroupAttackInputs {
            sample_hour: row.sample_hour,
            commander: row.commander.ok().map(|value| {
                value.map(|leader| rows::CommanderAttackInputs {
                    leader: leader.leader,
                    fort: leader.scalars[0].ok(),
                    river: leader.scalars[1].ok(),
                    snow: leader.scalars[2].ok(),
                    night: leader.scalars[3].ok(),
                    terrain: self.terrain(leader.terrain),
                })
            }),
        }
    }
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub(crate) struct MapCapacities(pub native::MapCapacity);
impl Default for MapCapacities {
    fn default() -> Self {
        Self(native::MapCapacity {
            countries: 0,
            links: 0,
            provinces: 0,
        })
    }
}
pub(crate) struct Map {
    packet: native::MapPacket,
    hour: i32,
}
impl Map {
    pub(super) fn fail(&mut self, reason: &'static str) {
        self.packet.fail(reason);
    }
    pub(super) fn new(capacity: MapCapacities) -> Self {
        Self {
            packet: native::MapPacket::try_new(capacity.0)
                .map_err(crate::allocation::io)
                .unwrap_or_else(|_| native::MapPacket::vacant()),
            hour: 0,
        }
    }
    pub(super) fn reset(&mut self) {
        self.packet.clear();
        self.hour = 0;
    }
    pub(super) fn allocated_bytes(&self) -> usize {
        self.packet.allocated_bytes()
    }
    pub(super) fn grow(&mut self, capacity: &mut MapCapacities) {
        let required = self.packet.required();
        crate::table_capture::grow_capacity(&mut capacity.0.countries, required.countries);
        crate::table_capture::grow_capacity(&mut capacity.0.links, required.links);
        crate::table_capture::grow_capacity(&mut capacity.0.provinces, required.provinces);
        let _ = self
            .packet
            .reserve(capacity.0)
            .map_err(crate::allocation::io);
    }
    #[allow(clippy::too_many_arguments)]
    pub(super) fn capture_with_observations(
        &mut self,
        memory: &impl Memory,
        base: usize,
        state: usize,
        hour: i32,
        seeds: &[carmy::DivisionSeed],
        divisions: Outcome,
        provinces: &[cprovince::Row],
        province_source: Outcome,
        guard: native::Field<()>,
        journal: &mut engine::raw_country_observations::Journal,
    ) {
        self.hour = hour;
        self.packet.enable_growth();
        self.packet.capture_with_observations(
            memory,
            base,
            state,
            hour,
            seeds,
            divisions,
            provinces,
            province_source,
            guard,
            journal,
        );
    }
    pub(super) fn country(&self, country: u32) -> Option<rows::CountryAttackInputs> {
        if !matches!(self.packet.outcomes()[0], Outcome::Rows { .. }) {
            return None;
        }
        let row = self
            .packet
            .countries()
            .binary_search_by_key(&country, |row| row.country)
            .ok()
            .and_then(|index| self.packet.countries().get(index))?;
        Some(rows::CountryAttackInputs {
            sample_hour: row.sample_hour,
            land_night_attack: row.land_night_attack.ok(),
            land_bunker_effectiveness_factor: row.land_bunker_effectiveness_factor.ok(),
        })
    }
    pub(super) fn country_or_refused(&self, country: u32) -> rows::CountryAttackInputs {
        self.country(country).unwrap_or(rows::CountryAttackInputs {
            sample_hour: self.hour,
            ..Default::default()
        })
    }
    pub(super) fn province(&self, province: u32) -> rows::ProvinceAttackInputs {
        let fallback = rows::ProvinceAttackInputs {
            sample_hour: self.hour,
            ..Default::default()
        };
        if !matches!(self.packet.outcomes()[1], Outcome::Rows { .. }) {
            return fallback;
        }
        let Some(row) = self
            .packet
            .provinces()
            .binary_search_by_key(&province, |row| row.province)
            .ok()
            .and_then(|index| self.packet.provinces().get(index))
        else {
            return fallback;
        };
        rows::ProvinceAttackInputs {
            sample_hour: row.sample_hour,
            fort_sum: row.fort_sum.ok(),
            land_bunker_effectiveness_factor: row.land_bunker_effectiveness_factor.ok(),
        }
    }
}

pub(super) fn weather(value: native::WeatherAttack) -> rows::WeatherRegionAttackInputs {
    rows::WeatherRegionAttackInputs {
        sample_hour: value.sample_hour,
        attack: value.attack.ok(),
        breakthrough: value.breakthrough.ok(),
    }
}

/// Staged rows can contain native controls even when the final census is
/// unavailable (len0). Retire the entire admitted buffer, including old rows.
pub(super) fn retire_province_states(provinces: &mut [cprovince::Row]) {
    for province in provinces {
        province.attack_state = None;
    }
}

#[derive(Clone, Copy, serde::Serialize)]
struct Snapshot {
    sample_hour: i32,
    unit_source_refusal: Option<&'static str>,
    unit_required: [usize; 3],
    group_source_refusal: Option<&'static str>,
    group_required: [usize; 2],
    unit_refused: [u64; 9],
    terrain_leaves_refused: u64,
    commander_refused: u64,
    commander_scalars_refused: [u64; 4],
    commander_terrain_refused: u64,
    known_null_terrain_selectors: usize,
    map_source_refusals: [Option<&'static str>; 2],
    map_required: [usize; 3],
    country_refused: [u64; 2],
    province_refused: [u64; 2],
    independent_demand_failures: [Option<&'static str>; 3],
    weather_refused: [u64; 2],
}

fn source_refusal(outcome: Outcome) -> Option<&'static str> {
    match outcome {
        Outcome::Rows { .. } => None,
        Outcome::Truncated { .. } => Some("selected attack source census incomplete"),
        Outcome::Unavailable { reason } => Some(reason),
    }
}

/// Spool-thread scalar diagnostics. No native pointers or row vectors survive.
#[derive(Default)]
pub(crate) struct Work {
    passes: u64,
    latest: Option<Snapshot>,
}
impl Work {
    pub(super) fn observe(
        &mut self,
        hour: i32,
        units: &Units,
        groups: &Groups,
        map: &Map,
        weather_refused: [u64; 2],
    ) {
        let unit_required = units.packet.required();
        let map_required = map.packet.required();
        let group_required = groups.packet.required();
        let mut latest = Snapshot {
            sample_hour: hour,
            unit_source_refusal: source_refusal(units.packet.outcome()),
            unit_required: [
                unit_required.rows,
                unit_required.terrains,
                unit_required.values,
            ],
            group_source_refusal: source_refusal(groups.packet.outcome()),
            group_required: [group_required.rows, group_required.values],
            unit_refused: [0; 9],
            terrain_leaves_refused: 0,
            commander_refused: 0,
            commander_scalars_refused: [0; 4],
            commander_terrain_refused: 0,
            known_null_terrain_selectors: units.packet.null_terrain_selectors(),
            map_source_refusals: map.packet.outcomes().map(source_refusal),
            map_required: [
                map_required.countries,
                map_required.links,
                map_required.provinces,
            ],
            country_refused: [0; 2],
            province_refused: [0; 2],
            independent_demand_failures: *map.packet.failures(),
            weather_refused,
        };
        for row in units.packet.rows() {
            latest.unit_refused[0] += u64::from(row.template.is_err());
            for (counter, field) in latest.unit_refused[1..7].iter_mut().zip(row.stats) {
                *counter += u64::from(field.is_err());
            }
            latest.unit_refused[7] += u64::from(row.terrain.is_err());
            latest.unit_refused[8] += u64::from(row.acclimatization.is_err());
            latest.terrain_leaves_refused += row
                .terrain
                .ok()
                .and_then(|span| units.packet.values(span))
                .map_or(0, |values| {
                    values.iter().filter(|value| value.attack.is_err()).count() as u64
                });
        }
        for row in groups.packet.rows() {
            match row.commander {
                Err(_) => latest.commander_refused += 1,
                Ok(None) => {}
                Ok(Some(commander)) => {
                    for (counter, field) in latest
                        .commander_scalars_refused
                        .iter_mut()
                        .zip(commander.scalars)
                    {
                        *counter += u64::from(field.is_err());
                    }
                    latest.commander_terrain_refused += u64::from(commander.terrain.is_err());
                    latest.terrain_leaves_refused += commander
                        .terrain
                        .ok()
                        .and_then(|span| groups.packet.values(span))
                        .map_or(0, |values| {
                            values.iter().filter(|value| value.attack.is_err()).count() as u64
                        });
                }
            }
        }
        for row in map.packet.countries() {
            latest.country_refused[0] += u64::from(row.land_night_attack.is_err());
            latest.country_refused[1] += u64::from(row.land_bunker_effectiveness_factor.is_err());
        }
        for row in map.packet.provinces() {
            latest.province_refused[0] += u64::from(row.fort_sum.is_err());
            latest.province_refused[1] += u64::from(row.land_bunker_effectiveness_factor.is_err());
        }
        self.passes = self.passes.saturating_add(1);
        self.latest = Some(latest);
    }
    pub(crate) fn status(&self) -> impl serde::Serialize {
        crate::status::status!({ "passes": self.passes, "latest": self.latest,
            "scope": "latest consumed sample including refused/unwritten frames; independent attack suffixes preserve prefixes; undemanded leaves included in absent counts; not replay acceptance",
            "group_required_order": ["rows", "terrain_values"],
            "unit_required_order": ["rows", "terrain_domain", "terrain_values"],
            "unit_refused_order": ["template", "river0", "river1", "fort0", "fort1", "snow", "night", "terrain_domain", "acclimatization"],
            "map_required_order": ["countries", "raw_links", "provinces"],
            "country_refused_order": ["land_night_attack", "land_bunker_effectiveness_factor"],
            "province_refused_order": ["fort_sum", "land_bunker_effectiveness_factor"],
            "weather_refused_order": ["attack", "breakthrough"] })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn unavailable_province_source_retires_staged_and_previous_controls() {
        // A later native failure can refuse publication after staging row0;
        // reset leaves prior row bytes in the remaining admitted storage.
        let source = Outcome::Unavailable {
            reason: "movement failed after row0 staging",
        };
        assert_eq!(source.len(), 0);
        let mut provinces = [
            cprovince::Row {
                id: 1,
                attack_state: Some(0x1000),
                attack_fort_sum: Some(-23),
                ..Default::default()
            },
            cprovince::Row {
                id: 2,
                attack_state: Some(0x2000),
                attack_fort_sum: Some(7),
                ..Default::default()
            },
            cprovince::Row {
                id: 3,
                attack_state: Some(0),
                attack_fort_sum: None,
                ..Default::default()
            },
        ];
        retire_province_states(&mut provinces);
        assert!(
            provinces
                .iter()
                .all(|province| province.attack_state.is_none())
        );
        assert_eq!(
            provinces.map(|province| (province.id, province.attack_fort_sum)),
            [(1, Some(-23)), (2, Some(7)), (3, None)]
        );
    }

    // Integrated source fixtures; UNRUN.

    #[test]
    fn unit_refusal_projection_uses_the_stamped_request_hour_and_compares_that_hour() {
        let division = (23_u64 << 32) | 51;
        let mut units = Units::vacant();
        units.stamp_hour(41);
        units.fail("accepted presentation request stopped");
        let refused = rows::DivisionAttackInputs {
            sample_hour: 41,
            ..Default::default()
        };
        assert_eq!(
            units.packet.outcome(),
            Outcome::Unavailable {
                reason: "accepted presentation request stopped"
            }
        );
        assert_eq!(units.division(division), refused);
        assert!(units.matches(division, &Recorded::Value(refused.clone())));
        assert!(!units.matches(division, &Recorded::NotRecorded));
        units.stamp_hour(42);
        assert_eq!(
            units.division(division),
            rows::DivisionAttackInputs {
                sample_hour: 42,
                ..Default::default()
            }
        );
        assert!(!units.matches(division, &Recorded::Value(refused)));
        // A refusal after a later restore must retain that new request epoch.
        units.fail("accepted presentation request panicked");
        assert_eq!(units.division(division).sample_hour, 42);
        assert!(units.matches(division, &Recorded::Value(units.division(division))));
    }
}
