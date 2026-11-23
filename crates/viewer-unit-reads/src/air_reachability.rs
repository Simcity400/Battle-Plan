
#[cfg(any(test, feature = "test-support"))]
use std::cell::Cell;
use std::{
    collections::{BTreeMap, BTreeSet},
    io,
};

mod scans;
pub use scans::Scans;

use engine_native::cbuilding_template;
use content_reader_map::map::ProvinceKind;
#[cfg(test)]
use replay_data::rows::AirBaseRow;
use replay_data::rows::{
    AIR_MISSION_BASE_EQUIPMENT_FLAGS, AIR_MISSION_FOREIGN_TARGET_FLAG, AIR_MISSION_LAY_MINES,
    AirWingRow, BuildingRow, DivisionMovementRow, DivisionRow, LandCombatRow, LandCombatSideRow,
    NavalBaseRow, NavalCombatDetailsRow, NavalCombatRow, ProvinceRow, ProvinceUnitOrderRow,
    StateRow, TableSchema, TaskForceRow, tables,
};
pub use replay_derive::air_mission_sources::{MissionCountry, MissionCountrySources};
use replay_derive::air_mission_sources::{canonical, effective_raw};
use replay_frame::frame::DecodedFrame;

#[cfg(test)]
use super::air_registry_sources::AirRegistrySources;
use viewer_foundation::{tables::{Available, rows}, visibility::Families};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Category {
    Transport,
    Scout,
    BomberFlight,
    BaseEquipment,
    Fighter,
}

pub fn category(mask: u32, last_target_province: i32, definition_flags: u64) -> Option<Category> {
    let has_target = last_target_province != 0;
    if mask & (1 << 10) != 0 && has_target {
        Some(Category::Transport)
    } else if mask & (1 << 14) != 0 && has_target {
        Some(Category::Scout)
    } else if mask & 0x1f7fa != 0 && mask & (1 << 7) == 0 && has_target {
        if definition_flags & AIR_MISSION_BASE_EQUIPMENT_FLAGS == 0 {
            Some(Category::BomberFlight)
        } else {
            Some(Category::BaseEquipment)
        }
    } else if mask & (1 << 17) != 0 {
        Some(Category::BaseEquipment)
    } else if mask & 0x85 != 0 {
        Some(Category::Fighter)
    } else {
        None
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Cause {
    Tactical,
    Strategic,
    StrategicRetry,
    StrategicRailwayGun,
    Naval,
    NavalRailwayGun,
    Port,
    Mine,
    MineSweep,
    LandScout,
    ScoutFractions,
    Sea,
}

impl Cause {
    const ALL: [Self; 12] = [
        Self::Tactical,
        Self::Strategic,
        Self::StrategicRetry,
        Self::StrategicRailwayGun,
        Self::Naval,
        Self::NavalRailwayGun,
        Self::Port,
        Self::Mine,
        Self::MineSweep,
        Self::LandScout,
        Self::ScoutFractions,
        Self::Sea,
    ];

    pub fn reason(self) -> &'static str {
        match self {
            Self::Tactical => {
                "tactical target needs region land fraction, land combat sides, wars and divisions"
            }
            Self::Strategic => {
                "strategic target needs region land fraction and complete states, provinces, buildings and occupants"
            }
            Self::StrategicRetry => {
                "strategic retry needs region land fraction and complete states, provinces, buildings and occupants"
            }
            Self::StrategicRailwayGun => {
                "strategic target needs an unrecorded railway gun's type and owner"
            }
            Self::Naval => "naval target needs complete combat, task-force and embarked-unit state",
            Self::NavalRailwayGun => {
                "naval target needs the unrecorded embarked railway-gun transfer, combat and owner"
            }
            Self::Port => "port target needs land fraction and complete hourly simulation joins",
            Self::Mine => "mine-laying target needs the owner-region mine level and matching cap",
            Self::MineSweep => "mine-sweeping needs region sea fraction and DLC state",
            Self::LandScout => {
                "land scout target needs complete regional states and country relations"
            }
            Self::ScoutFractions => "land scout target needs region land and sea fractions",
            Self::Sea => "sea target needs region sea fraction",
        }
    }

    fn railway_gun(self) -> bool {
        matches!(self, Self::StrategicRailwayGun | Self::NavalRailwayGun)
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Causes(u16);

impl Causes {
    fn add(&mut self, cause: Cause) {
        self.0 |= 1 << cause as u16;
    }

    pub fn iter(self) -> impl Iterator<Item = Cause> {
        Cause::ALL
            .into_iter()
            .filter(move |&cause| self.0 & (1 << cause as u16) != 0)
    }

    pub fn railway_gun_only(self) -> bool {
        self.0 != 0 && self.iter().all(Cause::railway_gun)
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TargetAdmission {
    Allowed,
    Blocked,
    Unknown(Causes),
}

#[derive(Clone, Copy, Debug, Default)]
pub struct TargetPredicates {
    pub tactical: Option<bool>,
    pub port: Option<bool>,
    pub scout: Option<bool>,
    pub naval_combat: Option<bool>,
    pub naval_occupants: NavalOccupants,
    pub mine: Option<bool>,
    pub strategic: Option<bool>,
    pub strategic_railway_gun: bool,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct RegionFractions {
    pub land_raw: i64,
    pub sea_raw: i64,
}

pub fn region_fractions(total: u64, land: u64, sea: u64) -> RegionFractions {
    if total == 0 {
        return RegionFractions::default();
    }
    let fixed = |count: u64| ((u128::from(count) * 100_000) / u128::from(total)) as i64;
    RegionFractions {
        land_raw: fixed(land),
        sea_raw: fixed(sea),
    }
}

pub fn scanned_bits(mask: u32) -> u32 {
    if mask & (1 << 6) != 0 {
        0
    } else if mask & ((1 << 9) | (1 << 10)) != 0 {
        mask & !0x1_f000
    } else {
        mask
    }
}

pub fn target_admission(
    mask: u32,
    reached_count: i32,
    fractions: Option<RegionFractions>,
    dlc_mask: Option<u32>,
    targets: TargetPredicates,
) -> TargetAdmission {
    if reached_count == 0 {
        return TargetAdmission::Blocked;
    }
    if mask & (1 << 6) != 0 {
        return TargetAdmission::Allowed;
    }

    let land = fractions.map(|region| region.land_raw != 0);
    let sea = fractions.map(|region| region.sea_raw != 0);
    let mut unknown = Causes::default();
    let strategic = |unknown: &mut Causes, otherwise| {
        match (land, targets.strategic) {
            (Some(true), Some(true)) => return true,
            (Some(false), _) | (_, Some(false)) => {}
            (Some(true), None) if targets.strategic_railway_gun => {
                unknown.add(Cause::StrategicRailwayGun)
            }
            _ => unknown.add(otherwise),
        }
        false
    };
    if mask & (1 << 1) != 0 {
        match (land, targets.tactical) {
            (Some(true), Some(true)) => return TargetAdmission::Allowed,
            (Some(false), _) | (_, Some(false)) => {}
            _ => unknown.add(Cause::Tactical),
        }
    }
    if mask & (1 << 3) != 0 && strategic(&mut unknown, Cause::Strategic) {
        return TargetAdmission::Allowed;
    }
    if mask & 0x90 != 0 {
        let occupants = targets.naval_occupants;
        let recorded = [
            targets.naval_combat,
            occupants.task_force,
            occupants.embarked,
        ];
        let found = recorded.contains(&Some(true)) || occupants.railway_gun == Some(true);
        match sea {
            Some(false) => {}
            Some(true) if found => return TargetAdmission::Allowed,
            _ => {
                if sea.is_none() || recorded.contains(&None) {
                    unknown.add(Cause::Naval);
                }
                if !found && occupants.railway_gun.is_none() {
                    unknown.add(Cause::NavalRailwayGun);
                }
            }
        }
    }
    if mask & (1 << 8) != 0 {
        match (land, targets.port) {
            (Some(true), Some(true)) => return TargetAdmission::Allowed,
            (Some(false), _) | (_, Some(false)) => {}
            _ => unknown.add(Cause::Port),
        }
    }

    if mask & ((1 << 9) | (1 << 10)) != 0 {
        return TargetAdmission::Allowed;
    }

    let sea_dlc = dlc_mask.map(|mask| mask & 0x10 != 0);
    if mask & AIR_MISSION_LAY_MINES != 0 {
        match (sea, sea_dlc, targets.mine) {
            (Some(false), _, _) | (_, Some(false), _) | (_, _, Some(false)) => {}
            (Some(true), Some(true), Some(true)) => return TargetAdmission::Allowed,
            _ => unknown.add(Cause::Mine),
        }
    }
    if mask & (1 << 13) != 0 {
        match (sea, sea_dlc) {
            (Some(true), Some(true)) => return TargetAdmission::Allowed,
            (Some(false), _) | (_, Some(false)) => {}
            _ => unknown.add(Cause::MineSweep),
        }
    }
    if mask & (1 << 14) != 0 {
        match fractions {
            Some(region) if region.sea_raw != 0 => return TargetAdmission::Allowed,
            Some(region) if region.land_raw == 0 => {}
            Some(_) => match targets.scout {
                Some(true) => return TargetAdmission::Allowed,
                Some(false) => {}
                None => unknown.add(Cause::LandScout),
            },
            None => unknown.add(Cause::ScoutFractions),
        }
    }
    if mask & (1 << 15) != 0 {
        match sea {
            Some(true) => return TargetAdmission::Allowed,
            Some(false) => {}
            None => unknown.add(Cause::Sea),
        }
    }
    if mask & (1 << 16) != 0 && strategic(&mut unknown, Cause::StrategicRetry) {
        return TargetAdmission::Allowed;
    }

    if mask & 0x1f7fa == 0 {
        TargetAdmission::Allowed
    } else if unknown != Causes::default() {
        TargetAdmission::Unknown(unknown)
    } else {
        TargetAdmission::Blocked
    }
}

fn complete<T: TableSchema>(frame: &DecodedFrame) -> io::Result<Option<&[T::Row]>> {
    Ok(match rows::<T>(frame)? {
        Available::Complete(rows) => Some(rows),
        Available::Truncated { .. }
        | Available::Unavailable { .. }
        | Available::NotRecorded { .. } => None,
    })
}

fn find<R, K: Ord>(rows: &[R], key: K, by: impl FnMut(&R) -> K) -> Option<&R> {
    rows.binary_search_by_key(&key, by)
        .ok()
        .map(|index| &rows[index])
}

fn war_pair(first: u32, second: u32) -> (u32, u32) {
    (first.min(second), first.max(second))
}

pub struct Wars<'a> {
    pairs: Option<BTreeSet<(u32, u32)>>,
    own: Option<BTreeMap<u32, u32>>,
    links: &'a BTreeMap<u32, u32>,
    #[cfg(any(test, feature = "test-support"))]
    pub tests: Cell<usize>,
}

impl<'a> Wars<'a> {
    pub fn read(frame: &DecodedFrame, links: &'a BTreeMap<u32, u32>) -> io::Result<Self> {
        let pairs = complete::<tables::WarRelations>(frame)?.and_then(|relations| {
            let mut pairs = BTreeSet::new();
            for row in relations {
                let first = canonical(row.first_country, links)?;
                let second = canonical(row.second_country, links)?;
                if first != 0 && second != 0 && first != second {
                    pairs.insert(war_pair(first, second));
                }
            }
            Some(pairs)
        });
        let own = complete::<tables::Countries>(frame)?.map(|countries| {
            countries
                .iter()
                .map(|row| (row.canonical_slot, row.id))
                .collect()
        });
        Ok(Self {
            pairs,
            own,
            links,
            #[cfg(any(test, feature = "test-support"))]
            tests: Default::default(),
        })
    }

    fn at_war(&self, first: MissionCountry, second_raw: u32) -> Option<bool> {
        #[cfg(any(test, feature = "test-support"))]
        self.tests.set(self.tests.get() + 1);
        if first.slot == 0 {
            return Some(false);
        }
        let own = *self.own.as_ref()?.get(&first.slot)?;
        if own == second_raw {
            return Some(false);
        }
        let second = canonical(second_raw, self.links)?;
        if second == 0 || first.slot == second || (own as i32) <= 0 {
            return Some(false);
        }
        Some(self.pairs.as_ref()?.contains(&war_pair(first.slot, second)))
    }

    fn share_visibility(&self, families: &Families, a: u32, b: u32) -> Option<bool> {
        if a == b {
            return Some(true);
        }
        if self.pairs.as_ref()?.contains(&war_pair(a, b)) {
            return Some(false);
        }
        Some(families.family(a, b))
    }
}

#[derive(Clone, Copy)]
struct Occupants<'a> {
    province_units: Option<&'a [ProvinceUnitOrderRow]>,
    divisions: Option<&'a [DivisionRow]>,
    task_forces: Option<&'a [TaskForceRow]>,
}

impl<'a> Occupants<'a> {
    fn read(frame: &'a DecodedFrame) -> io::Result<Self> {
        Ok(Self {
            province_units: complete::<tables::ProvinceUnitOrder>(frame)?,
            divisions: complete::<tables::Divisions>(frame)?,
            task_forces: complete::<tables::TaskForces>(frame)?,
        })
    }

    fn order(self, province: u32) -> Option<&'a ProvinceUnitOrderRow> {
        find(self.province_units?, province, |row| row.province)
    }

    fn railway_gun(self, handle: u64) -> Option<bool> {
        if find(self.divisions?, handle, |row| row.id).is_some() {
            return Some(false);
        }
        Some(
            self.task_forces
                .is_none_or(|rows| find(rows, handle, |row| row.id).is_none()),
        )
    }
}

pub struct PortTargetSources<'a> {
    provinces: Option<&'a [ProvinceRow]>,
    naval_bases: Option<&'a [NavalBaseRow]>,
    province_units: Option<&'a [ProvinceUnitOrderRow]>,
    task_forces: Option<&'a [TaskForceRow]>,
    wars: &'a Wars<'a>,
}

impl<'a> PortTargetSources<'a> {
    pub fn read(frame: &'a DecodedFrame, wars: &'a Wars<'a>) -> io::Result<Self> {
        Ok(Self {
            provinces: complete::<tables::Provinces>(frame)?,
            naval_bases: complete::<tables::NavalBases>(frame)?,
            province_units: complete::<tables::ProvinceUnitOrder>(frame)?,
            task_forces: complete::<tables::TaskForces>(frame)?,
            wars,
        })
    }

    pub fn admit(&self, country: MissionCountry, reached: &[u32]) -> Option<bool> {
        let naval_bases = self.naval_bases?;
        let mut unknown = false;
        for &id in reached {
            if find(naval_bases, id, |row| row.province).is_none() {
                continue;
            }
            let Some(province) = self.provinces.and_then(|rows| find(rows, id, |row| row.id))
            else {
                unknown = true;
                continue;
            };
            match self.wars.at_war(country, province.controller) {
                Some(true) => {}
                Some(false) => continue,
                None => {
                    unknown = true;
                    continue;
                }
            }
            let Some(order) = self
                .province_units
                .and_then(|rows| find(rows, id, |row| row.province))
            else {
                unknown = true;
                continue;
            };
            for &handle in &order.naval {
                let Some(task_force) = self
                    .task_forces
                    .and_then(|rows| find(rows, handle, |row| row.id))
                else {
                    unknown = true;
                    continue;
                };
                match self.wars.at_war(country, task_force.logical_owner) {
                    Some(true) => return Some(true),
                    Some(false) => {}
                    None => unknown = true,
                }
            }
        }
        if unknown { None } else { Some(false) }
    }
}

pub struct TacticalTargetSources<'a> {
    wars: &'a Wars<'a>,
    combats: Option<BTreeMap<u32, Vec<&'a LandCombatRow>>>,
    sides: Option<&'a [LandCombatSideRow]>,
    divisions: Option<&'a [DivisionRow]>,
}

impl<'a> TacticalTargetSources<'a> {
    pub fn read(frame: &'a DecodedFrame, wars: &'a Wars<'a>) -> io::Result<Self> {
        let combats = complete::<tables::LandCombats>(frame)?.map(|combats| {
            let mut by_province = BTreeMap::<u32, Vec<&LandCombatRow>>::new();
            for combat in combats {
                by_province.entry(combat.province).or_default().push(combat);
            }
            by_province
        });
        Ok(Self {
            wars,
            combats,
            sides: complete::<tables::LandCombatSides>(frame)?,
            divisions: complete::<tables::Divisions>(frame)?,
        })
    }

    pub fn admit(
        &self,
        wing: &AirWingRow,
        mission_country: MissionCountry,
        reached: &[u32],
    ) -> Option<bool> {
        let combats = self.combats.as_ref()?;
        let group = wing.attached_orders_group;
        let mut unknown = false;
        for combat in reached
            .iter()
            .filter_map(|province| combats.get(province))
            .flatten()
        {
            let Some(side) = self
                .sides
                .and_then(|sides| find(sides, combat.id, |row| row.combat))
            else {
                unknown = true;
                continue;
            };
            let mut war = Some(false);
            for &country in side.attacker_primary.iter().chain(&side.defender_primary) {
                match self.wars.at_war(mission_country, country) {
                    Some(true) => {
                        war = Some(true);
                        break;
                    }
                    Some(false) => {}
                    None => war = None,
                }
            }
            match war {
                Some(true) => {}
                Some(false) => continue,
                None => {
                    unknown = true;
                    continue;
                }
            }
            let group = match group {
                Some(0) => return Some(true),
                Some(group) => group,
                None => {
                    unknown = true;
                    continue;
                }
            };
            let Some(divisions) = self.divisions else {
                unknown = true;
                continue;
            };
            for army in combat.attackers.iter().chain(&combat.defenders) {
                match find(divisions, *army, |row| row.id) {
                    Some(division) if division.orders_group == group => return Some(true),
                    Some(_) => {}
                    None => unknown = true,
                }
            }
        }
        if unknown { None } else { Some(false) }
    }
}

pub struct RegionStates {
    strategic: BTreeMap<u32, Option<BTreeSet<u32>>>,
    scout: Option<BTreeMap<u32, Option<ScoutRegion>>>,
}

impl RegionStates {
    pub fn states_read(frame: &DecodedFrame) -> io::Result<Vec<u64>> {
        Ok(complete::<tables::Provinces>(frame)?.map_or_else(Vec::new, |provinces| provinces.iter().map(|province| u64::from(province.id) << 32 | u64::from(province.state)).collect()))
    }

    pub fn read(frame: &DecodedFrame, region_provinces: &BTreeMap<u32, Vec<u32>>) -> io::Result<Self> {
        let provinces = complete::<tables::Provinces>(frame)?;
        let strategic = provinces.map_or_else(BTreeMap::new, |provinces| {
            region_provinces
                .iter()
                .map(|(&region, ids)| {
                    let states = ids.iter().try_fold(BTreeSet::new(), |mut states, &id| {
                        let province = find(provinces, id, |row| row.id)?;
                        if province.state != 0 {
                            states.insert(province.state);
                        }
                        Some(states)
                    });
                    (region, states)
                })
                .collect()
        });
        let scout = provinces.map(|provinces| {
            let mut state_totals = BTreeMap::<u32, u64>::new();
            for province in provinces.iter().filter(|province| province.state != 0) {
                *state_totals.entry(province.state).or_default() += 1;
            }
            region_provinces
                .iter()
                .map(|(&region, ids)| (region, ScoutRegion::from_provinces(ids, provinces, &state_totals)))
                .collect()
        });
        Ok(Self { strategic, scout })
    }
}

pub struct StrategicTargetSources<'a> {
    wars: &'a Wars<'a>,
    provinces: Option<&'a [ProvinceRow]>,
    states: Option<&'a [StateRow]>,
    regional_states: &'a BTreeMap<u32, Option<BTreeSet<u32>>>,
    state_buildings: Option<BTreeMap<u32, ActiveBuildings>>,
    province_buildings: Option<BTreeMap<u32, ActiveBuildings>>,
    occupants: Occupants<'a>,
}

#[derive(Clone, Copy, Debug, Default)]
struct ActiveBuildings {
    counted: bool,
    railway_below_one: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Strategic {
    pub found: Option<bool>,
    pub railway_gun: bool,
}

impl<'a> StrategicTargetSources<'a> {
    pub fn read(
        frame: &'a DecodedFrame,
        wars: &'a Wars<'a>,
        regions: &'a RegionStates,
    ) -> io::Result<Self> {
        let provinces = complete::<tables::Provinces>(frame)?;
        let regional_states = &regions.strategic;
        let state_buildings = complete::<tables::StateBuildings>(frame)?;
        let province_buildings = complete::<tables::ProvinceBuildings>(frame)?;
        Ok(Self {
            wars,
            provinces,
            states: complete::<tables::States>(frame)?,
            regional_states,
            state_buildings: active_buildings(state_buildings),
            province_buildings: active_buildings(province_buildings),
            occupants: Occupants::read(frame)?,
        })
    }

    pub fn admit(&self, country: MissionCountry, region: u32, reached: &[u32]) -> Strategic {
        let unknown = Strategic {
            found: None,
            railway_gun: false,
        };
        let (Some(states), Some(provinces), Some(Some(regional))) = (
            self.states,
            self.provinces,
            self.regional_states.get(&region),
        ) else {
            return unknown;
        };
        let mut missing = false;
        let mut railway_gun = false;
        for &id in reached {
            let Some(province) = find(provinces, id, |row| row.id) else {
                missing = true;
                continue;
            };
            if !regional.contains(&province.state) {
                continue;
            }
            let Some(state) = find(states, province.state, |row| row.id) else {
                missing = true;
                continue;
            };
            let hostile = match self.wars.at_war(country, state.controller) {
                Some(false) => continue,
                state_war => match self.wars.at_war(country, province.controller) {
                    Some(false) => continue,
                    province_war => state_war == Some(true) && province_war == Some(true),
                },
            };
            let (Some(own), Some(shared)) = (&self.province_buildings, &self.state_buildings)
            else {
                missing = true;
                continue;
            };
            let own = own.get(&id).copied().unwrap_or_default();
            let shared = shared.get(&state.id).copied().unwrap_or_default();
            if own.counted || shared.counted {
                if hostile {
                    return Strategic {
                        found: Some(true),
                        railway_gun: false,
                    };
                }
                missing = true;
                continue;
            }
            if shared.railway_below_one {
                missing = true;
                continue;
            }
            if !own.railway_below_one {
                continue;
            }
            match self.occupants.order(id) {
                Some(order) => {
                    for &handle in &order.mixed {
                        match self.occupants.railway_gun(handle) {
                            Some(false) => {}
                            Some(true) if hostile => railway_gun = true,
                            Some(true) | None => missing = true,
                        }
                    }
                }
                None => missing = true,
            }
        }
        Strategic {
            found: (!missing && !railway_gun).then_some(false),
            railway_gun: railway_gun && !missing,
        }
    }
}

fn active_buildings(rows: Option<&[BuildingRow]>) -> Option<BTreeMap<u32, ActiveBuildings>> {
    rows.map(|rows| {
        let mut locations = BTreeMap::<u32, ActiveBuildings>::new();
        for row in rows.iter().filter(|row| row.active) {
            let location = locations.entry(row.location).or_default();
            if row.level >= 1 {
                location.counted = true;
            } else if row.building_type == cbuilding_template::RAIL_WAY_TOKEN {
                location.railway_below_one = true;
            }
        }
        locations
    })
}

pub struct ScoutTargetSources<'a> {
    states: Option<&'a [StateRow]>,
    families: Option<&'a Families>,
    wars: &'a Wars<'a>,
    links: &'a BTreeMap<u32, u32>,
    regional: Option<&'a BTreeMap<u32, Option<ScoutRegion>>>,
}

struct ScoutRegion {
    states: Vec<u32>,
    has_coverage: bool,
}

impl ScoutRegion {
    fn from_provinces(
        ids: &[u32],
        provinces: &[ProvinceRow],
        state_totals: &BTreeMap<u32, u64>,
    ) -> Option<Self> {
        let mut overlaps = BTreeMap::<u32, u8>::new();
        for &id in ids {
            let province = find(provinces, id, |row| row.id)?;
            if province.state != 0 {
                let count = overlaps.entry(province.state).or_default();
                *count = count.wrapping_add(1);
            }
        }
        let has_coverage = overlaps.iter().any(|(&state, &overlap)| {
            state_totals
                .get(&state)
                .is_some_and(|&total| overlap != 0 && (u64::from(overlap) * 100_000) / total > 0)
        });
        Some(Self {
            states: overlaps.into_keys().collect(),
            has_coverage,
        })
    }
}

impl<'a> ScoutTargetSources<'a> {
    pub fn read(
        frame: &'a DecodedFrame,
        links: &'a BTreeMap<u32, u32>,
        wars: &'a Wars<'a>,
        families: Option<&'a Families>,
        regions: &'a RegionStates,
    ) -> io::Result<Self> {
        Ok(Self {
            states: complete::<tables::States>(frame)?,
            families,
            wars,
            links,
            regional: regions.scout.as_ref(),
        })
    }

    fn foreign_state(&self, wing: &AirWingRow, controller: u32, flags: u64) -> Option<bool> {
        let effective = canonical(effective_raw(wing), self.links)?;
        let target = canonical(controller, self.links)?;
        if effective == target {
            return Some(false);
        }
        if self.families?.allied_for_fog(effective, target) {
            return Some(false);
        }
        if flags & AIR_MISSION_FOREIGN_TARGET_FLAG != 0 {
            return Some(true);
        }
        self.wars
            .at_war(MissionCountry { slot: effective }, controller)
    }

    pub fn admit(&self, wing: &AirWingRow, region: u32, flags: u64) -> Option<bool> {
        let states = self.states?;
        let regional = self.regional?.get(&region)?.as_ref()?;
        if !regional.has_coverage {
            return Some(false);
        }
        let mut unknown = false;
        for &state_id in &regional.states {
            let Some(state) = find(states, state_id, |row| row.id) else {
                unknown = true;
                continue;
            };
            match self.foreign_state(wing, state.controller, flags) {
                Some(true) => return Some(true),
                Some(false) => {}
                None => unknown = true,
            }
        }
        if unknown { None } else { Some(false) }
    }
}

pub struct NavalCombatTargetSources<'a> {
    combats: Option<BTreeMap<u32, Vec<&'a NavalCombatRow>>>,
    details: Option<&'a [NavalCombatDetailsRow]>,
    detected: Option<BTreeSet<(u64, usize)>>,
    families: Option<&'a Families>,
    wars: &'a Wars<'a>,
    links: &'a BTreeMap<u32, u32>,
}

impl<'a> NavalCombatTargetSources<'a> {
    pub fn read(
        frame: &'a DecodedFrame,
        links: &'a BTreeMap<u32, u32>,
        wars: &'a Wars<'a>,
        families: Option<&'a Families>,
    ) -> io::Result<Self> {
        let combats = complete::<tables::NavalCombats>(frame)?.map(|combats| {
            let mut by_province = BTreeMap::<u32, Vec<&NavalCombatRow>>::new();
            for combat in combats {
                by_province.entry(combat.province).or_default().push(combat);
            }
            by_province
        });
        let detected = complete::<tables::NavalCombatShips>(frame)?.map(|members| {
            members
                .iter()
                .filter(|member| {
                    member.member_source == 0
                        && member.group_index != u32::MAX
                        && member.state & 0x0b != 0
                        && member.detection_state != 0
                })
                .map(|member| (member.combat, usize::from(member.side)))
                .collect()
        });
        Ok(Self {
            combats,
            details: complete::<tables::NavalCombatDetails>(frame)?,
            detected,
            families,
            wars,
            links,
        })
    }

    pub fn admit_combat(&self, country: MissionCountry, reached: &[u32]) -> Option<bool> {
        let combats = self.combats.as_ref()?;
        let details = self.details?;
        let detected = self.detected.as_ref()?;
        let families = self.families?;
        let mut unknown = false;
        for combat in reached.iter().filter_map(|id| combats.get(id)).flatten() {
            let Some(detail) = find(details, combat.id, |row| row.combat) else {
                unknown = true;
                continue;
            };
            if detail.province != combat.province {
                unknown = true;
                continue;
            }
            let mut war = Arm::default();
            for &participant in detail.sides.iter().flat_map(|side| &side.summary.countries) {
                war.war(self.wars.at_war(country, participant));
                if war.found {
                    break;
                }
            }
            match war.result() {
                Some(true) => {}
                Some(false) => continue,
                None => {
                    unknown = true;
                    continue;
                }
            }
            let mut combat_unknown = false;
            for (side_index, side) in detail.sides.iter().enumerate() {
                if !detected.contains(&(combat.id, side_index)) {
                    continue;
                }
                for &participant in &side.summary.countries {
                    let shared = canonical(participant, self.links).and_then(|candidate| {
                        self.wars
                            .share_visibility(families, country.slot, candidate)
                    });
                    match shared {
                        Some(true) => return Some(true),
                        Some(false) => {}
                        None => combat_unknown = true,
                    }
                }
            }
            unknown |= combat_unknown;
        }
        if unknown { None } else { Some(false) }
    }
}

pub struct MineTargetSources<'a> {
    levels: Option<replay_derive::naval_mine_levels::Levels<'a>>,
}
impl<'a> MineTargetSources<'a> {
    pub fn read(frame: &'a DecodedFrame, links: &'a BTreeMap<u32, u32>) -> io::Result<Self> {
        Ok(Self {
            levels: replay_derive::naval_mine_levels::Levels::read(frame, links).ok(),
        })
    }
    pub fn admit(&self, wing: &AirWingRow, region: u32, maximum_raw: i64) -> Option<bool> {
        if region == 0 {
            return None;
        }
        Some(
            self.levels
                .as_ref()?
                .amount(effective_raw(wing), region)
                .ok()?
                < maximum_raw,
        )
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct NavalOccupants {
    pub task_force: Option<bool>,
    pub embarked: Option<bool>,
    pub railway_gun: Option<bool>,
}

#[derive(Clone, Copy, Default)]
struct Arm {
    found: bool,
    unknown: bool,
}

impl Arm {
    fn result(self) -> Option<bool> {
        if self.found {
            Some(true)
        } else if self.unknown {
            None
        } else {
            Some(false)
        }
    }

    fn war(&mut self, at_war: Option<bool>) {
        match at_war {
            Some(true) => self.found = true,
            Some(false) => {}
            None => self.unknown = true,
        }
    }
}

pub struct NavalOccupantSources<'a> {
    wars: &'a Wars<'a>,
    occupants: Occupants<'a>,
    movements: Option<&'a [DivisionMovementRow]>,
    province_kinds: &'a BTreeMap<u32, ProvinceKind>,
}

impl<'a> NavalOccupantSources<'a> {
    pub fn read(
        frame: &'a DecodedFrame,
        wars: &'a Wars<'a>,
        province_kinds: &'a BTreeMap<u32, ProvinceKind>,
    ) -> io::Result<Self> {
        Ok(Self {
            wars,
            occupants: Occupants::read(frame)?,
            movements: complete::<tables::DivisionMovement>(frame)?,
            province_kinds,
        })
    }

    pub fn admit(&self, country: MissionCountry, reached: &[u32]) -> NavalOccupants {
        let Some(province_units) = self.occupants.province_units else {
            return NavalOccupants::default();
        };
        let mut arms = [Arm::default(); 3];
        for &id in reached {
            let order = match self.province_kinds.get(&id) {
                Some(ProvinceKind::Sea) => find(province_units, id, |row| row.province),
                Some(_) => continue,
                None => None,
            };
            let Some(order) = order else {
                arms.iter_mut().for_each(|arm| arm.unknown = true);
                continue;
            };
            let [task_force, embarked, railway_gun] = &mut arms;
            for &handle in &order.naval {
                match self
                    .occupants
                    .task_forces
                    .map(|rows| find(rows, handle, |row: &TaskForceRow| row.id))
                {
                    Some(Some(row)) if row.province != id => task_force.unknown = true,
                    Some(Some(row)) if row.combat_references > 0 => {}
                    Some(Some(row)) => {
                        task_force.war(self.wars.at_war(country, row.logical_owner));
                    }
                    _ => task_force.unknown = true,
                }
            }
            for &handle in &order.mixed {
                match self.occupants.railway_gun(handle) {
                    None => {
                        embarked.unknown = true;
                        railway_gun.unknown = true;
                        break;
                    }
                    Some(true) => {
                        railway_gun.unknown = true;
                        continue;
                    }
                    Some(false) => {}
                }
                let Some(division) = self
                    .occupants
                    .divisions
                    .and_then(|rows| find(rows, handle, |row| row.id))
                else {
                    continue;
                };
                let movement = self
                    .movements
                    .and_then(|rows| find(rows, handle, |row| row.division));
                match movement {
                    _ if division.province != id => embarked.unknown = true,
                    None => embarked.unknown = true,
                    Some(movement)
                        if movement.naval_transfer == 0 || movement.combat_count != 0 => {}
                    Some(_) => {
                        embarked.war(self.wars.at_war(country, division.logical_owner));
                    }
                }
            }
        }
        let [task_force, embarked, railway_gun] = arms.map(Arm::result);
        NavalOccupants {
            task_force,
            embarked,
            railway_gun,
        }
    }
}

#[cfg(test)]
mod tests;
