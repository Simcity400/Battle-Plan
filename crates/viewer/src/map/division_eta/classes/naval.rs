//! Class-1 counter admission: 140D5BD20/140D5BAA0 range and target access,
//! 140D5F950 submarine rules, and 140E152B0 hostile strength. Stored physical
//! navy slots and nearest-base IDs retain their native identity and order.

use super::{
    MovementContent,
    fresh_search::{available, index},
};
use crate::map::raids::reach::Moves;
use crate::table_access::TokenNames;
use installed_content::{
    naval_path::Distances,
    unit_path::{Fact, MoveWorld},
};
use replay::{
    frame::DecodedFrame,
    rows::{
        CountryCounterInputRow, CountryLinkRow, CountryMovementInputs, CountryNavalMovementInputs,
        CountryRow, FleetRow, FreshTaskForceCounterEta, NavalBaseRow, NavalPathInputs,
        PhysicalNavyNearestCache, ProvinceRow, ProvinceUnitOrderRow, ReservedCountryNamingRow,
        ShipModelRow, ShipRow, TableSchema, TaskForceMapMotionRow, TaskForceRow,
        TheatreControllerAreas, tables,
    },
};
use std::{
    collections::BTreeMap,
    sync::{Mutex, OnceLock},
};

const P: i64 = 100_000;
type Index<'a, K, R> = OnceLock<Fact<BTreeMap<K, &'a R>>>;

#[derive(Default)]
struct ShipGroup<'a> {
    members: Vec<&'a ShipRow>,
    strength: i64,
}

fn rows<'s, 'a, T: TableSchema, K: Ord>(
    cell: &'s Index<'a, K, T::Row>,
    frame: &'a DecodedFrame,
    key: impl Fn(&T::Row) -> K,
) -> Fact<&'s BTreeMap<K, &'a T::Row>> {
    available(cell.get_or_init(|| index::<T, _>(frame, key)))
}

/// Borrowed immutable frame domains, indexed at first demand once per batch.
/// A refused nearest vector cannot poison theatre, speed or submarine facts.
pub(super) struct NavalInputs<'a> {
    frame: &'a DecodedFrame,
    content: &'a MovementContent,
    forces: Index<'a, u64, TaskForceRow>,
    motions: Index<'a, u64, TaskForceMapMotionRow>,
    ships: OnceLock<Fact<BTreeMap<u64, ShipGroup<'a>>>>,
    models: Index<'a, u64, ShipModelRow>,
    tokens: OnceLock<Fact<TokenNames<'a>>>,
    fleets: Index<'a, u64, FleetRow>,
    bases: Index<'a, u32, NavalBaseRow>,
    provinces: Index<'a, u32, ProvinceRow>,
    occupants: Index<'a, u32, ProvinceUnitOrderRow>,
    links: Index<'a, u32, CountryLinkRow>,
    countries: OnceLock<Fact<BTreeMap<u32, &'a CountryRow>>>,
    owners: Index<'a, u32, CountryCounterInputRow>,
    reserved: Index<'a, (), ReservedCountryNamingRow>,
    nearest: OnceLock<Fact<BTreeMap<u32, &'a PhysicalNavyNearestCache>>>,
    theatres: Mutex<BTreeMap<u32, Fact<&'a TheatreControllerAreas>>>,
    distances: Mutex<Option<Distances>>,
}

impl<'a> NavalInputs<'a> {
    pub(super) fn new(frame: &'a DecodedFrame, content: &'a MovementContent) -> Self {
        Self {
            frame,
            content,
            forces: OnceLock::new(),
            motions: OnceLock::new(),
            ships: OnceLock::new(),
            models: OnceLock::new(),
            tokens: OnceLock::new(),
            fleets: OnceLock::new(),
            bases: OnceLock::new(),
            provinces: OnceLock::new(),
            occupants: OnceLock::new(),
            links: OnceLock::new(),
            countries: OnceLock::new(),
            owners: OnceLock::new(),
            reserved: OnceLock::new(),
            nearest: OnceLock::new(),
            theatres: Mutex::new(BTreeMap::new()),
            distances: Mutex::new(None),
        }
    }

    pub(super) fn tf(&self, handle: u64) -> Fact<&'a TaskForceRow> {
        rows::<tables::TaskForces, _>(&self.forces, self.frame, |row| row.id)?
            .get(&handle)
            .copied()
            .ok_or_else(|| format!("fresh naval ETA task force {handle:#x} is not recorded"))
    }

    pub(super) fn motion(&self, handle: u64) -> Fact<&'a TaskForceMapMotionRow> {
        rows::<tables::TaskForceMapMotion, _>(&self.motions, self.frame, |row| row.task_force)?
            .get(&handle)
            .copied()
            .ok_or_else(|| format!("fresh naval ETA task force {handle:#x} has no motion row"))
    }

    pub(super) fn fresh(&self, handle: u64) -> Fact<&'a FreshTaskForceCounterEta> {
        let fresh = self
            .motion(handle)?
            .fresh
            .require("TaskForceMapMotion.fresh")?
            .as_ref()
            .ok_or("fresh naval ETA task-force packet was refused")?;
        if fresh.sample_hour != self.frame.hour {
            return Err("fresh naval ETA task-force packet belongs to another hour".into());
        }
        Ok(fresh)
    }

    pub(super) fn tokens(&self) -> Fact<TokenNames<'a>> {
        available(self.tokens.get_or_init(|| {
            self.frame
                .complete::<tables::LexicalTokens>()
                .map_err(|error| error.to_string())?;
            TokenNames::new(self.frame).map_err(|error| error.to_string())
        }))
        .copied()
    }

    fn countries(&self) -> Fact<&BTreeMap<u32, &'a CountryRow>> {
        available(self.countries.get_or_init(|| {
            let mut out = BTreeMap::new();
            for row in index::<tables::Countries, _>(self.frame, |row| row.id)?.into_values() {
                if out.insert(row.canonical_slot, row).is_some() {
                    return Err("fresh ETA Countries contains duplicate keys".into());
                }
            }
            Ok(out)
        }))
    }

    pub(super) fn slot(&self, raw: u32) -> Fact<u32> {
        if raw == 0 {
            return Ok(0);
        }
        rows::<tables::CountryLinks, _>(&self.links, self.frame, |row| row.id)?
            .get(&raw)
            .map(|row| row.canonical_slot)
            .ok_or_else(|| format!("fresh naval ETA country reference {raw} has no link"))
    }

    /// Validate identities before the shared movement world can consult them.
    pub(super) fn route_identities(&self) -> Fact<()> {
        rows::<tables::Provinces, _>(&self.provinces, self.frame, |row| row.id)?;
        rows::<tables::CountryLinks, _>(&self.links, self.frame, |row| row.id)?;
        self.countries()?;
        Ok(())
    }

    fn owner(&self, raw: u32) -> Fact<&'a CountryMovementInputs> {
        if raw as i32 <= 0 {
            return Err("fresh naval ETA owner is not a supported positive reference".into());
        }
        let slot = self.slot(raw)?;
        if slot == 0 {
            return Err("fresh naval ETA owner links to no country".into());
        }
        let id = self
            .countries()?
            .get(&slot)
            .ok_or_else(|| format!("fresh naval ETA country slot {slot} is not recorded"))?
            .id;
        let owner =
            rows::<tables::CountryCounterInputs, _>(&self.owners, self.frame, |row| row.country)?
                .get(&id)
                .ok_or_else(|| format!("fresh naval ETA owner {raw} has no counter inputs"))?
                .movement
                .require("CountryCounterInputs.movement")?
                .as_ref()
                .ok_or("fresh naval ETA owner packet was refused")?;
        if owner.sample_hour != self.frame.hour {
            return Err("fresh naval ETA owner packet belongs to another hour".into());
        }
        Ok(owner)
    }

    fn shared(&self) -> Fact<&'a CountryNavalMovementInputs> {
        let packet = rows::<tables::ReservedCountryNaming, _>(&self.reserved, self.frame, |_| ())?
            .get(&())
            .ok_or("fresh naval ETA shared packet is not recorded")?
            .movement
            .require("ReservedCountryNaming.movement")?
            .as_ref()
            .ok_or("fresh naval ETA shared packet was refused")?;
        if packet.sample_hour != self.frame.hour {
            return Err("fresh naval ETA shared packet belongs to another hour".into());
        }
        Ok(packet)
    }

    fn navy(&self, raw: u32) -> Fact<&'a NavalPathInputs> {
        let navy = self
            .owner(raw)?
            .naval
            .as_ref()
            .ok_or("fresh naval ETA owner navy packet was refused")?;
        if navy.navy_slot != self.slot(raw)? {
            return Err("fresh naval ETA owner navy slot differs from its current country".into());
        }
        if !navy.present {
            return Err("fresh naval ETA reached an absent owner navy".into());
        }
        Ok(navy)
    }

    fn theatre(&self, roster_owner: u32, to: u32) -> Fact<bool> {
        let mut domains = self
            .theatres
            .lock()
            .map_err(|_| "fresh naval ETA theatre index lock was poisoned")?;
        let theatre = *available(domains.entry(roster_owner).or_insert_with(|| {
            let theatre = if roster_owner == 0 {
                self.shared()?.reserved_theatre.as_ref()
            } else {
                self.owner(roster_owner)?.theatre.as_ref()
            }
            .ok_or("fresh naval ETA owned-theatre membership was refused")?;
            validate_theatre(theatre)?;
            Ok(theatre)
        }))?;
        // The projection includes every positive province whose actual area
        // is null when any owned theatre contains a null area entry.
        Ok(theatre.provinces.binary_search(&to).is_ok())
    }

    fn ship_groups(&self) -> Fact<&BTreeMap<u64, ShipGroup<'a>>> {
        available(self.ships.get_or_init(|| {
            let mut out: BTreeMap<u64, ShipGroup<'a>> = BTreeMap::new();
            for row in index::<tables::Ships, _>(self.frame, |row| row.id)?.into_values() {
                let group = out.entry(row.task_force).or_default();
                group.members.push(row);
                group.strength = group.strength.wrapping_add(row.strength);
            }
            Ok(out)
        }))
    }

    pub(super) fn ships(&self, handle: u64) -> Fact<&[&'a ShipRow]> {
        Ok(self
            .ship_groups()?
            .get(&handle)
            .map_or(&[], |group| group.members.as_slice()))
    }

    fn strength(&self, handle: u64) -> Fact<i64> {
        Ok(self
            .ship_groups()?
            .get(&handle)
            .map_or(0, |group| group.strength))
    }

    pub(super) fn all_submarine(&self, handle: u64) -> Fact<bool> {
        self.tf(handle)?;
        for ship in self.ships(handle)? {
            let model = rows::<tables::ShipModels, _>(&self.models, self.frame, |row| row.ship)?
                .get(&ship.id)
                .ok_or_else(|| {
                    format!("fresh naval ETA ship {:#x} has no formation flags", ship.id)
                })?;
            // 140C2BAE0 reads bit7 of the raw +628 low byte, not class/category.
            if model.formation_flags & 0x80 == 0 {
                return Ok(false);
            }
        }
        Ok(true)
    }

    pub(super) fn hostile_allows(&self, moves: &Moves<'_, '_>, handle: u64, to: u32) -> Fact<bool> {
        let owner = self.slot(self.tf(handle)?.logical_owner)?;
        let hostile =
            self.hostile_strength(to, owner, |enemy, query| moves.at_war(enemy, query))?;
        Ok(hostile <= self.strength(handle)?)
    }

    fn hostile_strength(
        &self,
        to: u32,
        query: u32,
        mut at_war: impl FnMut(u32, u32) -> Fact<bool>,
    ) -> Fact<i64> {
        let occupants =
            rows::<tables::ProvinceUnitOrder, _>(&self.occupants, self.frame, |row| row.province)?
                .get(&to)
                .ok_or_else(|| format!("fresh naval ETA province {to} has no occupant order"))?;
        let forces = rows::<tables::TaskForces, _>(&self.forces, self.frame, |row| row.id)?;
        let mut sum = 0_i64;
        for handle in &occupants.mixed {
            // The complete class1 census filters mixed class occurrences; repeated
            // task-force occurrences contribute repeatedly, as 140E152B0 does.
            if let Some(force) = forces.get(handle)
                && at_war(self.slot(force.logical_owner)?, query)?
            {
                sum = sum.wrapping_add(self.strength(*handle)?);
            }
        }
        Ok(i64::from((sum / P) as i32) * P)
    }

    pub(super) fn access(&self, moves: &Moves<'_, '_>, handle: u64, to: u32) -> Fact<bool> {
        replay::fixed_layout::validate_recording(
            self.frame,
            replay::fixed_layout::Family {
                keys: &[
                    engine::modifier_keys::NAVAL_RANGE,
                    engine::modifier_keys::NAVAL_RANGE_FACTOR,
                    engine::modifier_keys::NAVY_WEATHER_PENALTY,
                    engine::modifier_keys::UNDERWAY_REPLENISHMENT_RANGE_FACTOR,
                ],
                ..replay::fixed_layout::TASK_FORCE_COUNTER_ETA
            },
        )?;
        let force = self.tf(handle)?;
        let defines = available(&self.content.naval)?;
        let range = counter_range(
            self.fresh(handle)?
                .modified_range
                .ok_or("fresh naval ETA current modified range was refused")?,
            defines.range_scale,
        );
        let flags = self
            .content
            .map
            .terrain(to)
            .ok_or_else(|| format!("fresh naval ETA province {to} has no content terrain"))?
            .flags;
        if flags & 1 != 0 {
            let bases = rows::<tables::NavalBases, _>(&self.bases, self.frame, |row| row.province)?;
            if !bases.contains_key(&to) {
                return Ok(false);
            }
            let owner = self.slot(force.logical_owner)?;
            let controller = moves.controller(to)?;
            // This war direction is target controller -> task-force logical owner.
            if moves.at_war(controller, owner)? {
                return Ok(false);
            }
            let admits = match self.theatre(force.roster_owner, to) {
                Ok(true) => true,
                Ok(false) => moves.counter_discloses(owner, controller)?,
                Err(reason) => match moves.counter_discloses(owner, controller) {
                    Ok(true) => true,
                    _ => return Err(reason),
                },
            };
            if !admits {
                return Ok(false);
            }
            if moves.owner_area_missing(to)? {
                return Ok(false);
            }
        }
        match range_policy(flags, range, &defines.use_home_base)? {
            None => Ok(true),
            Some(true) => self.home_reaches(moves, force, to, range),
            Some(false) => self.nearest_reaches(moves, force.roster_owner, to, range),
        }
    }

    fn home_reaches(
        &self,
        moves: &Moves<'_, '_>,
        force: &TaskForceRow,
        to: u32,
        range: i64,
    ) -> Fact<bool> {
        let fleet = rows::<tables::Fleets, _>(&self.fleets, self.frame, |row| row.id)?
            .get(&force.fleet)
            .ok_or_else(|| format!("fresh naval ETA fleet {:#x} is not recorded", force.fleet))?;
        // 140E93DA0 returns before consulting hubs when the stored home is null.
        if fleet.home_base == 0 {
            return Ok(false);
        }
        let bases = rows::<tables::NavalBases, _>(&self.bases, self.frame, |row| row.province)?;
        if bases.contains_key(&fleet.home_base)
            && self.distance(moves, fleet.home_base, to, false)? <= range
        {
            return Ok(true);
        }
        let hubs = self
            .navy(force.roster_owner)?
            .supply_hubs
            .as_ref()
            .ok_or("fresh naval ETA stored supply hubs were refused")?;
        for &hub in hubs {
            if bases.contains_key(&hub) && self.distance(moves, hub, to, false)? <= range {
                return Ok(true);
            }
        }
        Ok(false)
    }

    fn nearest_base(&self, slot: u32, to: u32) -> Fact<Option<u32>> {
        let caches = available(self.nearest.get_or_init(|| {
            let caches = self
                .shared()?
                .nearest_caches
                .as_ref()
                .ok_or("fresh naval ETA physical nearest-cache domain was refused")?;
            // Validate once; stored foreign slots remain ordered occurrences.
            if let Some(pair) = caches.windows(2).find(|pair| pair[0].slot >= pair[1].slot) {
                return Err(if pair[0].slot == pair[1].slot {
                    "fresh naval ETA physical nearest caches contain duplicate slots"
                } else {
                    "fresh naval ETA physical nearest-cache slots are not increasing"
                }
                .into());
            }
            Ok(caches.iter().map(|cache| (cache.slot, cache)).collect())
        }))?;
        let cache = caches.get(&slot).ok_or_else(|| {
            format!("fresh naval ETA physical navy slot {slot} is outside its domain")
        })?;
        if !cache.present {
            return Err(format!(
                "fresh naval ETA reached absent physical navy slot {slot}"
            ));
        }
        cache
            .nearest_bases
            .as_ref()
            .ok_or_else(|| {
                format!("fresh naval ETA physical navy slot {slot} nearest vector was refused")
            })?
            .get(to as usize)
            .copied()
            .ok_or_else(|| {
                format!("fresh naval ETA nearest province {to} is outside slot {slot}'s vector")
            })
    }

    fn nearest_reaches(
        &self,
        moves: &Moves<'_, '_>,
        roster: u32,
        to: u32,
        range: i64,
    ) -> Fact<bool> {
        let navy = self.navy(roster)?;
        if let Some(base) = self.nearest_base(navy.navy_slot, to)?
            && self.distance(moves, base, to, true)? <= range
        {
            return Ok(true);
        }
        let foreign = navy
            .foreign_navy_slots
            .as_ref()
            .ok_or("fresh naval ETA stored foreign navy slots were refused")?;
        for &slot in foreign {
            // No CountryLinks/current-diplomacy or current-base-map filtering.
            if let Some(base) = self.nearest_base(slot, to)?
                && self.distance(moves, base, to, true)? <= range
            {
                return Ok(true);
            }
        }
        Ok(false)
    }

    fn distance(&self, moves: &Moves<'_, '_>, from: u32, to: u32, mapped: bool) -> Fact<i64> {
        let (graph, geometry) = moves.counter_topology()?;
        let mut distances = self
            .distances
            .lock()
            .map_err(|_| "fresh naval ETA distance lock was poisoned")?;
        if distances.is_none() {
            let ports = rows::<tables::Provinces, _>(&self.provinces, self.frame, |row| row.id)?
                .iter()
                .map(|(&id, row)| (id, row.port_sea as u32))
                .collect();
            *distances = Some(Distances::new(geometry, &ports));
        }
        let distances = distances
            .as_mut()
            .expect("naval distances were just initialized");
        if mapped {
            distances.mapped_distance(graph, geometry, from, to)
        } else {
            distances.distance(graph, geometry, from, to)
        }
    }
}

fn counter_range(modified: i64, scale: i64) -> i64 {
    let range = modified.wrapping_mul(scale) / P;
    if (range as u64).wrapping_add(0x7fff_ffff_ffff_fffe) < 0xffff_ffff_ffff_fffd {
        range.wrapping_mul(P) / 200_000
    } else {
        0xffff_ffff
    }
}

/// No range lookup below one. Positive land range always uses home/hubs;
/// only a positive non-land range reads the content's sea-range policy.
fn range_policy(flags: u8, range: i64, use_home_base: &Fact<bool>) -> Fact<Option<bool>> {
    if range < 1 {
        return Ok(None);
    }
    if flags & 1 != 0 {
        return Ok(Some(true));
    }
    Ok(Some(*available(use_home_base)?))
}

fn validate_theatre(theatre: &TheatreControllerAreas) -> Fact<()> {
    if theatre.provinces.first() == Some(&0) {
        return Err("fresh naval ETA theatre membership contains a nonpositive province".into());
    }
    if let Some(pair) = theatre.provinces.windows(2).find(|pair| pair[0] >= pair[1]) {
        return Err(if pair[0] == pair[1] {
            "fresh naval ETA theatre membership contains duplicate provinces"
        } else {
            "fresh naval ETA theatre membership is not increasing"
        }
        .into());
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use installed_content::movement::{MovementMap, NavalMovementDefines};
    use replay::{
        frame::{Metrics, TableView},
        rows::{Recorded, TableRows},
    };

    fn frame() -> DecodedFrame {
        DecodedFrame {
            branch: 1,
            hour: 20,
            tables: std::array::from_fn(|_| {
                TableView::Unavailable {
                    reason: "fixture domain unavailable".into(),
                }
                .into()
            }),
            metrics: Metrics::default(),
        }
    }
    fn put(frame: &mut DecodedFrame, rows: TableRows) {
        let table = rows.table();
        *frame.tables[table.index()] = TableView::Complete(rows);
    }
    fn content() -> MovementContent {
        MovementContent {
            map: MovementMap::default(),
            defines: Default::default(),
            rail: Err("fixture railway content unavailable".into()),
            task_force_terrain: Err("unrelated task-force terrain is unavailable".into()),
            naval: Ok(NavalMovementDefines {
                range_scale: P,
                use_home_base: Ok(false),
            }),
        }
    }
    fn force(id: u64, logical: u32) -> TaskForceRow {
        TaskForceRow {
            id,
            logical_owner: logical,
            roster_owner: 99,
            ..Default::default()
        }
    }
    fn owners(frame: &mut DecodedFrame, hour: i32) {
        put(
            frame,
            TableRows::CountryLinks(vec![
                CountryLinkRow {
                    id: 99,
                    canonical_slot: 1,
                },
                CountryLinkRow {
                    id: 199,
                    canonical_slot: 2,
                },
            ]),
        );
        put(
            frame,
            TableRows::Countries(vec![
                CountryRow {
                    id: 101,
                    canonical_slot: 1,
                    ..Default::default()
                },
                CountryRow {
                    id: 102,
                    canonical_slot: 2,
                    ..Default::default()
                },
            ]),
        );
        put(
            frame,
            TableRows::CountryCounterInputs(vec![CountryCounterInputRow {
                forward_visibility: replay::rows::Recorded::NotRecorded,
                country: 101,
                movement: Recorded::Value(Some(CountryMovementInputs {
                    sample_hour: hour,
                    naval: Some(NavalPathInputs {
                        navy_slot: 1,
                        present: true,
                        foreign_navy_slots: Some(vec![0, 3, 3]),
                        supply_hubs: None,
                    }),
                    theatre: Some(TheatreControllerAreas {
                        provinces: vec![2, 8],
                        null_controller_area: true,
                    }),
                    ..Default::default()
                })),
                ..Default::default()
            }]),
        );
    }
    fn shared(frame: &mut DecodedFrame, caches: Vec<PhysicalNavyNearestCache>) {
        put(
            frame,
            TableRows::ReservedCountryNaming(vec![ReservedCountryNamingRow {
                movement: Recorded::Value(Some(CountryNavalMovementInputs {
                    sample_hour: 20,
                    nearest_caches: Some(caches),
                    reserved_theatre: Some(TheatreControllerAreas {
                        provinces: vec![8],
                        null_controller_area: true,
                    }),
                })),
                ..Default::default()
            }]),
        );
    }

    #[test]
    fn current_packets_require_the_exact_hour_without_poisoning_other_domains() {
        let content = content();
        let mut frame = frame();
        owners(&mut frame, 19);
        put(
            &mut frame,
            TableRows::TaskForceMapMotion(vec![TaskForceMapMotionRow {
                task_force: 7,
                fresh: Recorded::Value(Some(FreshTaskForceCounterEta {
                    sample_hour: 19,
                    base_speed: Some(P),
                    ..Default::default()
                })),
                ..Default::default()
            }]),
        );
        let inputs = NavalInputs::new(&frame, &content);
        assert!(inputs.owner(99).unwrap_err().contains("another hour"));
        assert!(inputs.fresh(7).unwrap_err().contains("another hour"));
        // This independent reserved theatre packet needs no country or nearest cache.
        shared(&mut frame, Vec::new());
        let inputs = NavalInputs::new(&frame, &content);
        assert_eq!(inputs.theatre(0, 8), Ok(true));
        assert_eq!(inputs.theatre(0, 9), Ok(false));
    }

    #[test]
    fn submarine_rule_reads_formation_bit_and_accepts_an_empty_roster() {
        let content = content();
        let mut frame = frame();
        put(
            &mut frame,
            TableRows::TaskForces(vec![force(7, 99), force(8, 99)]),
        );
        put(
            &mut frame,
            TableRows::Ships(vec![ShipRow {
                id: 17,
                task_force: 7,
                category: 0,
                class_id: 700,
                ..Default::default()
            }]),
        );
        let inputs = NavalInputs::new(&frame, &content);
        assert_eq!(inputs.all_submarine(8), Ok(true));
        assert!(inputs.all_submarine(7).is_err());
        put(
            &mut frame,
            TableRows::ShipModels(vec![ShipModelRow {
                ship: 17,
                formation_flags: 0x80,
                ..Default::default()
            }]),
        );
        assert_eq!(
            NavalInputs::new(&frame, &content).all_submarine(7),
            Ok(true)
        );
        put(
            &mut frame,
            TableRows::ShipModels(vec![ShipModelRow {
                ship: 17,
                formation_flags: 0x8000,
                ..Default::default()
            }]),
        );
        assert_eq!(
            NavalInputs::new(&frame, &content).all_submarine(7),
            Ok(false)
        );
    }

    #[test]
    fn hostile_strength_preserves_occurrences_direction_signed_truncation_and_own_fraction() {
        let content = content();
        let mut frame = frame();
        owners(&mut frame, 20);
        put(
            &mut frame,
            TableRows::TaskForces(vec![force(7, 99), force(8, 199)]),
        );
        put(
            &mut frame,
            TableRows::ProvinceUnitOrder(vec![ProvinceUnitOrderRow {
                province: 2,
                mixed: vec![7, 999, 8, 8],
                naval: vec![],
            }]),
        );
        for (enemy, expected) in [
            (175_000, 300_000),
            (-175_000, -300_000),
            (i64::from(i32::MAX) * P + P, 0),
            (i64::MAX, 0),
        ] {
            put(
                &mut frame,
                TableRows::Ships(vec![
                    ShipRow {
                        id: 17,
                        task_force: 7,
                        strength: 375_001,
                        ..Default::default()
                    },
                    ShipRow {
                        id: 18,
                        task_force: 8,
                        strength: enemy,
                        ..Default::default()
                    },
                ]),
            );
            let inputs = NavalInputs::new(&frame, &content);
            let mut directions = Vec::new();
            assert_eq!(
                inputs.hostile_strength(2, 1, |from, to| {
                    directions.push((from, to));
                    Ok(from == 2 && to == 1)
                }),
                Ok(expected)
            );
            assert_eq!(directions, vec![(1, 1), (2, 1), (2, 1)]);
            assert_eq!(inputs.strength(7), Ok(375_001));
        }
    }

    #[test]
    fn physical_nearest_slots_and_stored_zero_are_independent_of_country_links_and_current_bases() {
        let content = content();
        let mut frame = frame();
        shared(
            &mut frame,
            vec![
                PhysicalNavyNearestCache {
                    slot: 0,
                    present: false,
                    nearest_bases: None,
                },
                PhysicalNavyNearestCache {
                    slot: 3,
                    present: true,
                    nearest_bases: Some(vec![None, None, Some(0)]),
                },
                PhysicalNavyNearestCache {
                    slot: 9,
                    present: true,
                    nearest_bases: Some(vec![None, None, Some(42)]),
                },
            ],
        );
        let inputs = NavalInputs::new(&frame, &content);
        assert_eq!(inputs.nearest_base(3, 2), Ok(Some(0)));
        assert_eq!(inputs.nearest_base(9, 2), Ok(Some(42)));
        assert!(
            inputs
                .nearest_base(0, 2)
                .unwrap_err()
                .contains("absent physical navy")
        );
        assert!(
            inputs
                .nearest_base(3, 3)
                .unwrap_err()
                .contains("outside slot")
        );
        assert!(
            inputs
                .nearest_base(8, 2)
                .unwrap_err()
                .contains("outside its domain")
        );
        shared(
            &mut frame,
            vec![
                PhysicalNavyNearestCache {
                    slot: 3,
                    present: true,
                    nearest_bases: Some(vec![None]),
                },
                PhysicalNavyNearestCache {
                    slot: 3,
                    present: false,
                    nearest_bases: None,
                },
            ],
        );
        assert!(
            NavalInputs::new(&frame, &content)
                .nearest_base(3, 0)
                .unwrap_err()
                .contains("duplicate slots")
        );
    }

    #[test]
    fn owner_alias_join_theatre_membership_and_foreign_order_keep_their_domains() {
        let content = content();
        let mut frame = frame();
        owners(&mut frame, 20);
        let inputs = NavalInputs::new(&frame, &content);
        assert_eq!(
            inputs.navy(99).unwrap().foreign_navy_slots,
            Some(vec![0, 3, 3])
        );
        assert_eq!(inputs.theatre(99, 8), Ok(true));
        assert_eq!(inputs.theatre(99, 9), Ok(false));
        assert!(inputs.owner(0).is_err());
        for provinces in [vec![0], vec![2, 2], vec![8, 2]] {
            assert!(
                validate_theatre(&TheatreControllerAreas {
                    provinces,
                    ..Default::default()
                })
                .is_err()
            );
        }
        put(
            &mut frame,
            TableRows::Countries(vec![
                CountryRow {
                    id: 101,
                    canonical_slot: 1,
                    ..Default::default()
                },
                CountryRow {
                    id: 102,
                    canonical_slot: 1,
                    ..Default::default()
                },
            ]),
        );
        assert!(
            NavalInputs::new(&frame, &content)
                .owner(99)
                .unwrap_err()
                .contains("duplicate keys")
        );
    }

    #[test]
    fn range_uses_wrapping_product_then_the_native_guarded_half() {
        assert_eq!(counter_range(2_000_001, P), 1_000_000);
        assert_eq!(counter_range(-2_000_001, P), -1_000_000);
        assert_eq!(counter_range(i64::MIN, P), 0);
        assert_eq!(counter_range(i64::MAX, P), 0);
        assert_eq!(counter_range(i64::MIN, 1), -46_116_860_184_273);
        assert_eq!(counter_range(i64::MAX, 1), 46_116_860_184_273);
    }

    #[test]
    fn only_positive_non_land_range_demands_the_home_policy() {
        let refused = Err("fixture home policy unavailable".into());
        assert_eq!(range_policy(0, 0, &refused), Ok(None));
        assert_eq!(range_policy(0, -1, &refused), Ok(None));
        assert_eq!(range_policy(1, P, &refused), Ok(Some(true)));
        assert_eq!(
            range_policy(0, P, &refused),
            Err("fixture home policy unavailable".into())
        );
        assert_eq!(range_policy(2, P, &Ok(false)), Ok(Some(false)));
        assert_eq!(range_policy(0, P, &Ok(true)), Ok(Some(true)));
    }

    #[test]
    fn class_tokens_require_complete_strict_key_order() {
        use replay::rows::LexicalTokenRow;
        let content = content();
        let mut frame = frame();
        assert!(NavalInputs::new(&frame, &content).tokens().is_err());
        put(
            &mut frame,
            TableRows::LexicalTokens(vec![LexicalTokenRow {
                token: 17,
                literal: b"authored_naval_class".to_vec(),
            }]),
        );
        let inputs = NavalInputs::new(&frame, &content);
        assert_eq!(
            inputs.tokens().unwrap().name(17),
            Ok("authored_naval_class")
        );
        assert_eq!(
            inputs.tokens().unwrap().name(18),
            Err(crate::table_access::TokenError::Absent(18))
        );
        put(
            &mut frame,
            TableRows::LexicalTokens(vec![
                LexicalTokenRow {
                    token: 17,
                    literal: b"first".to_vec(),
                },
                LexicalTokenRow {
                    token: 17,
                    literal: b"second".to_vec(),
                },
            ]),
        );
        assert!(
            NavalInputs::new(&frame, &content)
                .tokens()
                .unwrap_err()
                .contains("not in token order")
        );
    }
}
