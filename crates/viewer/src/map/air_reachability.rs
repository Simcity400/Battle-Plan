//! Flying-aircraft mission admission from same-hour simulation operands.
//!
//! `GroupAirWingsByScenario@14122EB00` calls the mission's
//! `IsAirWingScenarioEligible@140F66100` law, then independently requires a
//! nonzero reached count (`CAirMission+0x44`). The reached IDs are
//! simulation-owned and can be stale after a transfer or an earlier mission:
//! they are never rebuilt from the current base, range or region, and never
//! region-filtered here.

use std::{
    collections::{BTreeMap, BTreeSet},
    io,
};

use engine::cbuilding_template;
use installed_content::map::ProvinceKind;
use replay::{
    frame::DecodedFrame,
    rows::{
        AIR_MISSION_BASE_EQUIPMENT_FLAGS, AIR_MISSION_FOREIGN_TARGET_FLAG, AIR_MISSION_LAY_MINES,
        AirBaseRow, AirWingRow, BuildingRow, DivisionMovementRow, DivisionRow, LandCombatRow,
        LandCombatSideRow, NavalBaseRow, NavalCombatDetailsRow, NavalCombatRow, NavalMineLevelRow,
        ProvinceRow, ProvinceUnitOrderRow, StateRow, TableSchema, TaskForceRow, tables,
    },
};

use super::{
    air_registry_sources::AirRegistrySources,
    tables::{Available, rows},
    visibility::Families,
};

/// The destination array 14122EB00 selects. The base-equipment branch builds
/// an aircraft at the air base, not a bomber flight route.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Category {
    Transport,
    Scout,
    BomberFlight,
    BaseEquipment,
    Fighter,
}

/// `last_target_province` is only tested for nonzero here; it need not be a
/// member of the (possibly stale) reached vector.
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

/// Why F66100's target chain stayed undecided: the missing operand of one
/// branch.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Cause {
    Tactical,
    Strategic,
    StrategicRetry,
    /// F61B40's railway-gun arm: an occupant that may be a railway gun.
    StrategicRailwayGun,
    Naval,
    /// F68410's embarked railway-gun arm.
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

/// Every undecided branch of one wing's target chain.
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

    /// The unrecorded railway-gun arms are the only undecided branches.
    pub fn railway_gun_only(self) -> bool {
        self.0 != 0 && self.iter().all(Cause::railway_gun)
    }
}

/// F66100's target OR chain. `Unknown` holds every branch an operand left
/// undecided, so the flight is omitted with those reasons.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TargetAdmission {
    Allowed,
    Blocked,
    Unknown(Causes),
}

/// F66100's existential target scans. `None` means an operand was
/// unavailable, not that the scan found no target.
#[derive(Clone, Copy, Debug, Default)]
pub struct TargetPredicates {
    pub tactical: Option<bool>,
    pub port: Option<bool>,
    pub scout: Option<bool>,
    pub naval_combat: Option<bool>,
    pub naval_occupants: NavalOccupants,
    pub mine: Option<bool>,
    pub strategic: Option<bool>,
    /// `strategic` is undecided only by F61B40's railway-gun arm.
    pub strategic_railway_gun: bool,
}

/// Static `CStrategicRegion+0x98/+0xA0` fixed-five province-kind fractions:
/// `CStrategicRegion::AddProvince@140E0F2B0` floors
/// `kind_count * 100000 / province_count`.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct RegionFractions {
    pub land_raw: i64,
    pub sea_raw: i64,
}

/// `land` and `sea` count disjoint listings of the `total`.
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

/// The mask bits whose target scans F66100 reaches: none past the bit-6
/// bypass, and none of bits 12..16 past the bit 9/10 bypass.
pub fn scanned_bits(mask: u32) -> u32 {
    if mask & (1 << 6) != 0 {
        0
    } else if mask & ((1 << 9) | (1 << 10)) != 0 {
        mask & !0x1_f000
    } else {
        mask
    }
}

/// Evaluates F66100's target branches; its common wing/base checks belong to
/// the aircraft planner. 14122EB00's reached-count gate is kept here so no
/// category bypasses it. The existential scans over the reached provinces
/// arrive as `targets`.
///
/// Decompiled order: bit 6 bypass; bits 1, 3, 4|7, 8; bits 9|10 bypass;
/// bits 12..16; then success when `(mask & 0x1f7fa) == 0`.
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

    // F66100 succeeds for these masks even after an earlier unresolved scan.
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

    // Unconditional success outside the target-dependent set; an earlier
    // unknown cannot negate it.
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
        Available::Truncated { .. } | Available::Unavailable { .. } => None,
    })
}

fn find<R, K: Ord>(rows: &[R], key: K, by: impl FnMut(&R) -> K) -> Option<&R> {
    rows.binary_search_by_key(&key, by)
        .ok()
        .map(|index| &rows[index])
}

fn canonical(raw: u32, links: &BTreeMap<u32, u32>) -> Option<u32> {
    if raw == 0 {
        Some(0)
    } else {
        links.get(&raw).copied()
    }
}

/// The wing's raw effective country: a positive override, else its owner.
fn effective_raw(wing: &AirWingRow) -> u32 {
    if wing.country_override > 0 {
        wing.country_override as u32
    } else {
        wing.owner
    }
}

fn war_pair(first: u32, second: u32) -> (u32, u32) {
    (first.min(second), first.max(second))
}

/// A mission country's canonical slot, the key of every same-hour join.
/// Native signed gates test the country's own reference ([`Wars`]) or the
/// candidate's raw one ([`MissionCountrySources::resolve`]).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) struct MissionCountry {
    pub slot: u32,
}

/// F4C430's sources. It reads no port or naval target table, so the sortie
/// count shares it with every target scan.
pub(super) struct MissionCountrySources<'a> {
    registry: &'a AirRegistrySources,
    provinces: Option<&'a [ProvinceRow]>,
    volunteer_hosts: Option<BTreeMap<u32, BTreeSet<u32>>>,
    links: &'a BTreeMap<u32, u32>,
}

impl<'a> MissionCountrySources<'a> {
    pub(super) fn read(
        frame: &'a DecodedFrame,
        links: &'a BTreeMap<u32, u32>,
        registry: &'a AirRegistrySources,
    ) -> io::Result<Self> {
        let volunteer_hosts = complete::<tables::VolunteerHosts>(frame)?.and_then(|hosts| {
            let mut by_country = BTreeMap::<u32, BTreeSet<u32>>::new();
            for row in hosts {
                let country = canonical(row.country, links)?;
                // `HasCountryLink1428@1406F0590` matches a raw-equal entry or
                // the same country. An unlinked host never matches; a zero
                // host still matches a null controller by raw equality, which
                // stops the base search there.
                if let Some(host) = canonical(row.host, links) {
                    by_country.entry(country).or_default().insert(host);
                }
            }
            Some(by_country)
        });
        Ok(Self {
            registry,
            provinces: complete::<tables::Provinces>(frame)?,
            volunteer_hosts,
            links,
        })
    }

    /// F4C480 tries the pool base's controller, then the transfer base's, and
    /// stops at the first that is one of the effective country's volunteer
    /// hosts. F4C430 accepts it only when its raw reference is signed-positive;
    /// otherwise (or with no eligible base) the effective country flies.
    pub(super) fn resolve(&self, wing: &AirWingRow) -> Option<MissionCountry> {
        let hosts = self.volunteer_hosts.as_ref()?;
        let fallback = MissionCountry {
            slot: canonical(effective_raw(wing), self.links)?,
        };
        let Some(hosts) = hosts.get(&fallback.slot).filter(|hosts| !hosts.is_empty()) else {
            return Some(fallback);
        };
        for handle in [wing.air_base, wing.base_override] {
            if handle == 0 {
                continue;
            }
            // A handle the complete registry does not resolve is a native miss.
            let Some(base) = self.registry.bases.get(&handle) else {
                continue;
            };
            let controller_raw = self.base_controller(base)?;
            let controller = canonical(controller_raw, self.links)?;
            if hosts.contains(&controller) {
                // No other base is tried after the first eligible one.
                return Some(if (controller_raw as i32) > 0 {
                    MissionCountry { slot: controller }
                } else {
                    fallback
                });
            }
        }
        Some(fallback)
    }

    /// `CAirBase::GetControllerTag@140C448F0`: a state base's province
    /// controller (+0x188 of `GetProvince`, which natively has no null case,
    /// so a base without a province is refused); a stateless base's host
    /// task-force owner, or zero ("Air base with no controller") when the
    /// host is null or does not resolve. A null task force is refused.
    fn base_controller(&self, base: &AirBaseRow) -> Option<u32> {
        let Some(state) = base.state else {
            return if base.host == 0 {
                Some(0)
            } else {
                self.registry.host_owner(base.id)
            };
        };
        let province = find(self.provinces?, base.province?, |row| row.id)?;
        (province.state == state).then_some(province.controller)
    }
}

/// `CCountry::viewer_relation@1406F3A00`'s war test from a mission country
/// toward a raw reference, over canonical active-war pairs.
///
/// A complete WarRelations proves absence too, from the writer evidence:
/// every non-null `CRelationStatus+0x2E8` is installed by
/// `ReplaceRelation@140D02DF0` (through
/// `CWarRelation::ReplaceLinkedRelation@140D18990`), and all 17 writers of
/// the status relation lists (it, the 15 `ReplaceRelationSlot` setters,
/// `SetRelationPointer@140D00850` and `SetSubjectRelation@140D00DB0`) add
/// the relation to the owned vector (`AddOwnedRelation@140D04DA0`) that
/// WarRelations walks in the same call; their removals drop both, and
/// `Reset@140D09D50` only clears. The statuses themselves are built only by
/// `CDiplomacyStatus::constructor@140D1E540`.
pub(super) struct Wars<'a> {
    pairs: Option<BTreeSet<(u32, u32)>>,
    /// Each slot's own CCountry reference (`CCountry+8`), which the native
    /// gate tests.
    own: Option<BTreeMap<u32, u32>>,
    links: &'a BTreeMap<u32, u32>,
    /// War tests answered, for the scaling test.
    #[cfg(test)]
    pub(super) tests: std::cell::Cell<usize>,
}

impl<'a> Wars<'a> {
    pub(super) fn read(frame: &DecodedFrame, links: &'a BTreeMap<u32, u32>) -> io::Result<Self> {
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
            #[cfg(test)]
            tests: Default::default(),
        })
    }

    /// The mission country's own reference (`CCountry+8`) is compared with
    /// the target's raw reference and must be signed-positive; the same
    /// canonical country is never at war.
    fn at_war(&self, first: MissionCountry, second_raw: u32) -> Option<bool> {
        #[cfg(test)]
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

    /// `HasFullCountryVisibility(a, b, 0, 0)` between canonical slots: a live
    /// war vetoes the faction, overlord and subject relations.
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

/// The recorded identities of a province's mixed occupants (`CProvince+0xE0`,
/// armies, TaskForces the fleets, and any other occupant can be a railway gun.
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

    /// Neither a recorded division nor a recorded task force; `None`
    /// without the Divisions census.
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

/// F68C50's port-strike scan: a naval base (any level) at a reached province
/// whose controller is at war with the mission country, holding a task force
/// whose logical owner is too.
pub struct PortTargetSources<'a> {
    provinces: Option<&'a [ProvinceRow]>,
    naval_bases: Option<&'a [NavalBaseRow]>,
    province_units: Option<&'a [ProvinceUnitOrderRow]>,
    task_forces: Option<&'a [TaskForceRow]>,
    wars: &'a Wars<'a>,
}

impl<'a> PortTargetSources<'a> {
    pub(super) fn read(frame: &'a DecodedFrame, wars: &'a Wars<'a>) -> io::Result<Self> {
        Ok(Self {
            provinces: complete::<tables::Provinces>(frame)?,
            naval_bases: complete::<tables::NavalBases>(frame)?,
            province_units: complete::<tables::ProvinceUnitOrder>(frame)?,
            task_forces: complete::<tables::TaskForces>(frame)?,
            wars,
        })
    }

    /// `None` is Unknown; a later reached province may still prove success.
    /// Without a naval base in any reached province the scan is false.
    pub(super) fn admit(&self, country: MissionCountry, reached: &[u32]) -> Option<bool> {
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

/// `CountTacticalCombatTargets@140F69480`'s scan, as F66100 calls it (no
/// output, recon off, stop on first): a land combat at a reached province
/// (`CProvince+0x170`) with a primary country (`side+0x38`, attacker then
/// defender) at war with the mission country
/// (`HasParticipantAtWarWithCountry@1413CD4F0`). A wing with an attached
/// orders group (`CAirWing+0x9B8`) also needs an attacking or defending army
/// in that group (`CArmy+0xC0`).
pub struct TacticalTargetSources<'a> {
    wars: &'a Wars<'a>,
    /// Each province's land combats; `None` without complete LandCombats.
    combats: Option<BTreeMap<u32, Vec<&'a LandCombatRow>>>,
    sides: Option<&'a [LandCombatSideRow]>,
    divisions: Option<&'a [DivisionRow]>,
}

impl<'a> TacticalTargetSources<'a> {
    pub(super) fn read(frame: &'a DecodedFrame, wars: &'a Wars<'a>) -> io::Result<Self> {
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

    /// `None` is Unknown; a later combat may still prove success.
    pub(super) fn admit(
        &self,
        wing: &AirWingRow,
        mission_country: MissionCountry,
        reached: &[u32],
    ) -> Option<bool> {
        let combats = self.combats.as_ref()?;
        // The capture authenticated the group against its native pointer, so
        // handle equality is the native pointer equality. None is a group that
        // did not authenticate.
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

/// F69110's strategic scan: reached provinces in a state the region touches
/// (the province itself may lie outside the region), with hostile state and
/// province controllers. F69110 hands F61B40 the active building vectors
/// (`CBuildingStatus+0x50`, at `CProvince+0x1E0` and `CState+0x170`), not the
/// full `+0x38` lists, and F61B40 counts an active building of level 1 or
/// more. One below level 1 counts only when its type token is the rail way
/// template's (`cbuilding_template::RAIL_WAY_TOKEN`, which `CBuilding+0x08`
/// copies from its template) and a hostile type-0xD unit (a railway gun) is in
/// `get_province(building)+0xE0`; any other type below level 1 is skipped.
/// That is the reached province for a province building. A state building's
/// status has no province (flag bit 0 clear, so
/// `CBuildingStatus::get_province@141162320` is null), so an active state rail
/// way below level 1 is refused as undecided. Every mixed occupant being a
/// recorded division or task force proves no railway gun; an unidentified one
/// leaves the arm undecided.
pub struct StrategicTargetSources<'a> {
    wars: &'a Wars<'a>,
    provinces: Option<&'a [ProvinceRow]>,
    states: Option<&'a [StateRow]>,
    /// Empty without a complete Provinces table; `None` for a region with a
    /// province missing from it.
    regional_states: BTreeMap<u32, Option<BTreeSet<u32>>>,
    state_buildings: Option<BTreeMap<u32, ActiveBuildings>>,
    province_buildings: Option<BTreeMap<u32, ActiveBuildings>>,
    occupants: Occupants<'a>,
}

/// The levels in one location's active building vector, as F61B40 splits
/// them.
#[derive(Clone, Copy, Debug, Default)]
struct ActiveBuildings {
    /// Level 1 or more: a target.
    counted: bool,
    /// A rail way below level 1: a target only through the railway-gun arm.
    railway_below_one: bool,
}

/// F69110's result for one wing.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Strategic {
    pub found: Option<bool>,
    /// `found` is undecided only by the railway-gun arm.
    pub railway_gun: bool,
}

impl<'a> StrategicTargetSources<'a> {
    pub(super) fn read(
        frame: &'a DecodedFrame,
        wars: &'a Wars<'a>,
        region_provinces: &BTreeMap<u32, Vec<u32>>,
    ) -> io::Result<Self> {
        let provinces = complete::<tables::Provinces>(frame)?;
        let regional_states = provinces.map_or_else(BTreeMap::new, |provinces| {
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

    pub(super) fn admit(&self, country: MissionCountry, region: u32, reached: &[u32]) -> Strategic {
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
            // F69110 tests the state controller, then (only when at war)
            // the province controller. Both gate the province, so a proved
            // peace in either decides it even when the other is undecided.
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

/// Building censuses are hourly, so a complete one proves this hour's active
/// vectors.
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

/// F66020's bit-14 land-scout branch reads the region's state controllers,
/// not the reached provinces'. Families are the hourly diplomacy of the
/// wing's effective country; there is no viewpoint or fog input.
pub struct ScoutTargetSources<'a> {
    states: Option<&'a [StateRow]>,
    /// With the attache slots `allied_for_fog` reads.
    families: Option<&'a Families>,
    wars: &'a Wars<'a>,
    links: &'a BTreeMap<u32, u32>,
    regional: Option<BTreeMap<u32, Option<ScoutRegion>>>,
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
        // AddProvince does not append a duplicate listing but still counts it.
        for &id in ids {
            let province = find(provinces, id, |row| row.id)?;
            if province.state != 0 {
                let count = overlaps.entry(province.state).or_default();
                *count = count.wrapping_add(1);
            }
        }
        // AddProvince counts region/state overlap in a byte and builds each
        // +0x80 coverage entry from it and the state's full province count.
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
    pub(super) fn read(
        frame: &'a DecodedFrame,
        links: &'a BTreeMap<u32, u32>,
        wars: &'a Wars<'a>,
        families: Option<&'a Families>,
        region_provinces: &BTreeMap<u32, Vec<u32>>,
    ) -> io::Result<Self> {
        let regional = complete::<tables::Provinces>(frame)?.map(|provinces| {
            let mut state_totals = BTreeMap::<u32, u64>::new();
            for province in provinces.iter().filter(|province| province.state != 0) {
                *state_totals.entry(province.state).or_default() += 1;
            }
            region_provinces
                .iter()
                .map(|(&region, ids)| {
                    (
                        region,
                        ScoutRegion::from_provinces(ids, provinces, &state_totals),
                    )
                })
                .collect()
        });
        Ok(Self {
            states: complete::<tables::States>(frame)?,
            families,
            wars,
            links,
            regional,
        })
    }

    /// F70F60 excludes the same country and its directional allied-for-fog
    /// relation, then accepts air-role bit 18 or `viewer_relation`'s war.
    /// The effective country does not take the volunteer base controller here.
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

    /// F66020 tests whether *any* coverage entry is positive, not that the
    /// foreign state's own entry is.
    pub fn admit(&self, wing: &AirWingRow, region: u32, flags: u64) -> Option<bool> {
        let states = self.states?;
        let regional = self.regional.as_ref()?.get(&region)?.as_ref()?;
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

/// The combat arm of F68410's naval scan. Its other arms are separate
/// predicates, so their absence cannot turn a negative combat result into a
/// negative naval result. CNavalCombat's CCombat base is at offset 0, so the
/// recorded sides (`CNavalCombat+0x28/+0x30`, country list at
/// `CNavalCombatant+0x38`) are the `CCombat+0x28/+0x30` lists
/// `HasParticipantAtWarWithCountry@1413CD4F0` walks.
pub struct NavalCombatTargetSources<'a> {
    /// Combats by province, in NavalCombats order.
    combats: Option<BTreeMap<u32, Vec<&'a NavalCombatRow>>>,
    details: Option<&'a [NavalCombatDetailsRow]>,
    /// `(combat, side)` pairs with an active, detected CFEXGroup member.
    detected: Option<BTreeSet<(u64, usize)>>,
    /// Families only: [`Wars`] holds the hour's one war census.
    families: Option<&'a Families>,
    wars: &'a Wars<'a>,
    links: &'a BTreeMap<u32, u32>,
}

impl<'a> NavalCombatTargetSources<'a> {
    pub(super) fn read(
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

    /// F66100 passes null output, seed zero, recon off and stop-on-first, so
    /// no spotting roll runs. A combat at a reached province qualifies when a
    /// primary participant is at war with the mission country
    /// (`HasParticipantAtWarWithCountry@1413CD4F0` resolves the tag and tests
    /// `viewer_relation`, whose own-reference gate [`Wars`] applies) and a side
    /// with an active, detected CFEXGroup member has a primary country that
    /// shares visibility with it.
    pub(super) fn admit_combat(&self, country: MissionCountry, reached: &[u32]) -> Option<bool> {
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
            // One participant at war decides the war test, whatever others
            // leave undecided.
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

/// `CAirMission::CanLayMinesInRegion@140F65E40`: the effective country's raw
/// `CStrategicNavy+0xC8` mines in the mission region against the content cap.
/// The table is targeted, so a missing pair is Unknown, not zero.
pub struct MineTargetSources<'a> {
    levels: Option<&'a [NavalMineLevelRow]>,
    links: &'a BTreeMap<u32, u32>,
}

impl<'a> MineTargetSources<'a> {
    pub fn read(frame: &'a DecodedFrame, links: &'a BTreeMap<u32, u32>) -> io::Result<Self> {
        Ok(Self {
            levels: complete::<tables::NavalMineLevels>(frame)?,
            links,
        })
    }

    /// `maximum_raw` is the loader-clamped content cap; the comparison is
    /// the native one for any value.
    pub fn admit(&self, wing: &AirWingRow, region: u32, maximum_raw: i64) -> Option<bool> {
        if region == 0 {
            return None;
        }
        let country = canonical(effective_raw(wing), self.links).filter(|&country| country != 0)?;
        let level = find(self.levels?, (country, region), |row| {
            (row.country, row.region)
        })?;
        Some(level.raw_mines < maximum_raw)
    }
}

/// F68410's occupant arms, each an existential scan over the reached sea
/// provinces. `None` means an operand was unavailable.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct NavalOccupants {
    /// Task forces (`CProvince+0xF8`).
    pub task_force: Option<bool>,
    /// F5DE10 over embarked armies (+0x110).
    pub embarked: Option<bool>,
    /// F5DE10 over embarked railway guns (+0x128), which are not recorded.
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

/// F68410 scans each reached province's task forces (`CProvince+0xF8`),
/// embarked armies (+0x110) and railway guns (+0x128). As F66100 calls it
/// (no output, seed zero, `CanTargetForeignCountry` off) a task force
/// qualifies on sea with no combat and a logical owner at war with the
/// mission country (`viewer_relation`); F5DE10 admits an army or railway gun
/// whose own location is a sea province, with a naval transfer, no combat
/// and a hostile owner. The recording has ProvinceUnitOrder's naval list and
/// the mixed list ([`Occupants`]).
pub struct NavalOccupantSources<'a> {
    wars: &'a Wars<'a>,
    occupants: Occupants<'a>,
    movements: Option<&'a [DivisionMovementRow]>,
    province_kinds: &'a BTreeMap<u32, ProvinceKind>,
}

impl<'a> NavalOccupantSources<'a> {
    pub(super) fn read(
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

    pub(super) fn admit(&self, country: MissionCountry, reached: &[u32]) -> NavalOccupants {
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
                    // `CUnit::has_combat@140BF0AD0`.
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
mod tests {
    use super::*;
    use replay::{
        frame::{Metrics, TableView},
        rows::{
            AIR_MISSION_MAP_FLAGS, AIR_MISSION_ZERO_PLANE_FLAGS, CountryRow, FactionMemberRow,
            NavalCombatShipRow, RelationLinkRow, TableId, TableRows, VolunteerHostRow,
            WarRelationRow,
        },
    };

    const LAND: RegionFractions = RegionFractions {
        land_raw: 100_000,
        sea_raw: 0,
    };
    const SEA: RegionFractions = RegionFractions {
        land_raw: 0,
        sea_raw: 100_000,
    };
    const MIXED: RegionFractions = RegionFractions {
        land_raw: 50_000,
        sea_raw: 50_000,
    };

    /// Every table complete and empty but Countries, whose slots 1..=20 own
    /// their slot number as reference.
    fn empty_frame() -> DecodedFrame {
        let mut frame = DecodedFrame {
            branch: 0,
            hour: 100,
            tables: std::array::from_fn(|i| {
                TableView::Complete(TableRows::empty(TableId::ALL[i])).into()
            }),
            metrics: Metrics::default(),
        };
        set::<tables::Countries>(
            &mut frame,
            (1..=20)
                .map(|slot| CountryRow {
                    id: slot,
                    canonical_slot: slot,
                    ..Default::default()
                })
                .collect(),
        );
        frame
    }

    fn set<T: TableSchema>(frame: &mut DecodedFrame, rows: Vec<T::Row>) {
        *frame.tables[T::ID.index()] = TableView::Complete(TableRows::from_rows::<T>(rows));
    }

    fn truncate<T: TableSchema>(frame: &mut DecodedFrame) {
        *frame.tables[T::ID.index()] = TableView::Truncated {
            rows: TableRows::empty(T::ID),
            missing: 1,
        };
    }

    fn table<T: TableSchema>(frame: &mut DecodedFrame) -> &mut Vec<T::Row> {
        match &mut *frame.tables[T::ID.index()] {
            TableView::Complete(rows) => rows.rows_mut::<T>().unwrap(),
            _ => unreachable!(),
        }
    }

    fn war(first_country: u32, second_country: u32) -> WarRelationRow {
        WarRelationRow {
            first_country,
            second_country,
            ..Default::default()
        }
    }

    fn province(id: u32, state: u32, controller: u32) -> ProvinceRow {
        ProvinceRow {
            id,
            state,
            controller,
            ..Default::default()
        }
    }

    fn admission(
        mask: u32,
        reached_count: i32,
        fractions: Option<RegionFractions>,
        dlc_mask: Option<u32>,
    ) -> TargetAdmission {
        target_admission(
            mask,
            reached_count,
            fractions,
            dlc_mask,
            TargetPredicates::default(),
        )
    }

    struct PortFixture {
        wing: AirWingRow,
        base: AirBaseRow,
        air_bases: Vec<AirBaseRow>,
        host_owners: BTreeMap<u64, u32>,
        provinces: Vec<ProvinceRow>,
        naval_bases: Vec<NavalBaseRow>,
        province_units: Vec<ProvinceUnitOrderRow>,
        task_forces: Vec<TaskForceRow>,
        wars: Vec<WarRelationRow>,
        volunteer_hosts: Vec<VolunteerHostRow>,
        links: BTreeMap<u32, u32>,
    }

    impl PortFixture {
        fn new() -> Self {
            let base = AirBaseRow {
                id: 11,
                state: Some(7),
                province: Some(100),
                ..Default::default()
            };
            Self {
                wing: AirWingRow {
                    id: 1,
                    owner: 1,
                    air_base: base.id,
                    mission_mask: 1 << 8,
                    ..Default::default()
                },
                air_bases: vec![base.clone()],
                base,
                host_owners: BTreeMap::new(),
                provinces: vec![province(100, 7, 2), province(412, 8, 13)],
                // Hash membership counts even at level zero.
                naval_bases: vec![NavalBaseRow {
                    province: 412,
                    country: 1,
                    level: 0,
                    ..Default::default()
                }],
                province_units: vec![
                    ProvinceUnitOrderRow {
                        province: 100,
                        ..Default::default()
                    },
                    ProvinceUnitOrderRow {
                        province: 412,
                        naval: vec![42],
                        ..Default::default()
                    },
                ],
                task_forces: vec![TaskForceRow {
                    id: 42,
                    province: 412,
                    roster_owner: 1,
                    logical_owner: 3,
                    ..Default::default()
                }],
                wars: vec![war(1, 3)],
                volunteer_hosts: Vec::new(),
                links: BTreeMap::from([(1, 1), (2, 2), (3, 3), (12, 2), (13, 3)]),
            }
        }

        /// Country 1's volunteer hosts, in vector order.
        fn volunteer(&mut self, hosts: &[u32]) {
            self.volunteer_hosts = hosts
                .iter()
                .zip(0..)
                .map(|(&host, position)| VolunteerHostRow {
                    country: 1,
                    position,
                    host,
                    ..Default::default()
                })
                .collect();
        }

        fn frame(&self) -> DecodedFrame {
            let mut frame = empty_frame();
            set::<tables::Provinces>(&mut frame, self.provinces.clone());
            set::<tables::NavalBases>(&mut frame, self.naval_bases.clone());
            set::<tables::ProvinceUnitOrder>(&mut frame, self.province_units.clone());
            set::<tables::TaskForces>(&mut frame, self.task_forces.clone());
            set::<tables::WarRelations>(&mut frame, self.wars.clone());
            set::<tables::VolunteerHosts>(&mut frame, self.volunteer_hosts.clone());
            frame
        }

        fn registry(&self) -> AirRegistrySources {
            AirRegistrySources {
                bases: self
                    .air_bases
                    .iter()
                    .map(|base| (base.id, base.clone()))
                    .collect(),
                host_owners: self.host_owners.clone(),
                ..Default::default()
            }
        }

        fn resolve_in(&self, frame: &DecodedFrame) -> Option<u32> {
            let registry = self.registry();
            let mission = MissionCountrySources::read(frame, &self.links, &registry).unwrap();
            mission.resolve(&self.wing).map(|country| country.slot)
        }

        fn resolve(&self) -> Option<u32> {
            self.resolve_in(&self.frame())
        }

        fn admit_in(&self, frame: &DecodedFrame, reached: &[u32]) -> Option<bool> {
            let registry = self.registry();
            let mission = MissionCountrySources::read(frame, &self.links, &registry).unwrap();
            let wars = Wars::read(frame, &self.links).unwrap();
            let sources = PortTargetSources::read(frame, &wars).unwrap();
            sources.admit(mission.resolve(&self.wing)?, reached)
        }

        fn admit(&self, reached: &[u32]) -> Option<bool> {
            self.admit_in(&self.frame(), reached)
        }
    }

    #[test]
    fn port_strike_uses_base_hash_controller_and_naval_logical_owner() {
        let mut fixture = PortFixture::new();
        // The reached province need not be in this wing's current region.
        assert_eq!(fixture.admit(&[412]), Some(true));
        fixture.naval_bases.clear();
        assert_eq!(fixture.admit(&[412]), Some(false));
        fixture.naval_bases.push(NavalBaseRow {
            province: 412,
            ..Default::default()
        });
        fixture.provinces[1].controller = 1;
        assert_eq!(fixture.admit(&[412]), Some(false));
        fixture.provinces[1].controller = 3;
        fixture.task_forces[0].logical_owner = 2;
        assert_eq!(fixture.admit(&[412]), Some(false));
        fixture.task_forces[0].logical_owner = 3;
        fixture.province_units[1].naval.clear();
        assert_eq!(fixture.admit(&[412]), Some(false));
    }

    #[test]
    fn tactical_target_needs_a_hostile_primary_country_and_the_attached_group() {
        let fixture = PortFixture::new();
        let mut frame = fixture.frame();
        let army = |id: u64| (id << 32) | 53;
        set::<tables::LandCombats>(
            &mut frame,
            vec![LandCombatRow {
                id: 5,
                province: 412,
                attackers: vec![army(1)],
                defenders: vec![army(2)],
                ..Default::default()
            }],
        );
        set::<tables::LandCombatSides>(
            &mut frame,
            vec![LandCombatSideRow {
                combat: 5,
                attacker_primary: vec![2],
                defender_primary: vec![13],
                ..Default::default()
            }],
        );
        set::<tables::Divisions>(
            &mut frame,
            vec![
                DivisionRow {
                    id: army(1),
                    orders_group: army(9),
                    ..Default::default()
                },
                DivisionRow {
                    id: army(2),
                    orders_group: army(8),
                    ..Default::default()
                },
            ],
        );
        let registry = fixture.registry();
        let admit = |frame: &DecodedFrame, wing: &AirWingRow, reached: &[u32]| {
            let mission = MissionCountrySources::read(frame, &fixture.links, &registry).unwrap();
            let country = mission.resolve(wing).unwrap();
            let wars = Wars::read(frame, &fixture.links).unwrap();
            TacticalTargetSources::read(frame, &wars)
                .unwrap()
                .admit(wing, country, reached)
        };
        let mut wing = AirWingRow {
            attached_orders_group: Some(0),
            ..fixture.wing.clone()
        };
        // The defender's primary alias 13 is at war with country 1.
        assert_eq!(admit(&frame, &wing, &[412]), Some(true));
        assert_eq!(admit(&frame, &wing, &[100]), Some(false));
        // An attached group must hold one of the combat's armies.
        wing.attached_orders_group = Some(army(7));
        assert_eq!(admit(&frame, &wing, &[412]), Some(false));
        wing.attached_orders_group = Some(army(8));
        assert_eq!(admit(&frame, &wing, &[412]), Some(true));
        // A group that did not authenticate cannot be decided, unless no
        // hostile combat is reached.
        wing.attached_orders_group = None;
        assert_eq!(admit(&frame, &wing, &[412]), None);
        assert_eq!(admit(&frame, &wing, &[100]), Some(false));
        wing.attached_orders_group = Some(army(8));
        // Secondary roster owners are not tested.
        table::<tables::LandCombatSides>(&mut frame)[0] = LandCombatSideRow {
            combat: 5,
            attacker_primary: vec![2],
            attacker_secondary: vec![3],
            ..Default::default()
        };
        assert_eq!(admit(&frame, &wing, &[412]), Some(false));
        // An unknown group does not matter without a hostile combat.
        wing.attached_orders_group = None;
        assert_eq!(admit(&frame, &wing, &[412]), Some(false));
        wing.attached_orders_group = Some(army(8));
        // A combat without its side row cannot be decided.
        table::<tables::LandCombatSides>(&mut frame).clear();
        assert_eq!(admit(&frame, &wing, &[412]), None);
        truncate::<tables::LandCombats>(&mut frame);
        assert_eq!(admit(&frame, &wing, &[412]), None);
    }

    #[test]
    fn port_strike_mission_country_follows_volunteer_base_controller() {
        let mut fixture = PortFixture::new();
        fixture.wars = vec![war(2, 3)];
        assert_eq!(fixture.admit(&[412]), Some(false));
        // Host alias 12 matches base controller 2; target alias 13 and
        // task-force owner 3 match the same war.
        fixture.volunteer(&[12]);
        assert_eq!(fixture.admit(&[412]), Some(true));
        // A state base without a province has no native controller: refused.
        fixture.base.province = None;
        fixture.air_bases[0].province = None;
        assert_eq!(fixture.admit(&[412]), None);
        fixture.volunteer(&[]);
        fixture.wars = vec![war(1, 3)];
        assert_eq!(fixture.admit(&[412]), Some(true));
    }

    #[test]
    fn base_controller_needs_the_province_row_in_the_base_state() {
        let mut fixture = PortFixture::new();
        fixture.volunteer(&[2]);
        assert_eq!(fixture.resolve(), Some(2));
        let mut frame = fixture.frame();
        truncate::<tables::Provinces>(&mut frame);
        assert_eq!(fixture.resolve_in(&frame), None);
        let mut frame = fixture.frame();
        truncate::<tables::VolunteerHosts>(&mut frame);
        assert_eq!(fixture.resolve_in(&frame), None);
        fixture.provinces[0].state = 8;
        assert_eq!(fixture.resolve(), None);
        fixture.provinces.remove(0);
        assert_eq!(fixture.resolve(), None);
        // Without hosts no base controller is consulted.
        fixture.volunteer(&[]);
        assert_eq!(fixture.resolve(), Some(1));
    }

    #[test]
    fn mission_country_rejects_nonpositive_raw_volunteer_controller() {
        let mut fixture = PortFixture::new();
        fixture.provinces[0].controller = 0x8000_0002;
        fixture.links.insert(0x8000_0002, 2);
        fixture.volunteer(&[2]);
        fixture.wars = vec![war(2, 3)];
        assert_eq!(fixture.admit(&[412]), Some(false));
        fixture.wars = vec![war(1, 3)];
        assert_eq!(fixture.admit(&[412]), Some(true));
    }

    #[test]
    fn viewer_relation_gates_on_the_mission_countrys_own_reference() {
        let fixture = PortFixture::new();
        let mut frame = fixture.frame();
        assert_eq!(fixture.admit_in(&frame, &[412]), Some(true));
        // `CCountry+8`, not the wing's raw reference, is tested.
        table::<tables::Countries>(&mut frame)[0].id = 0x8000_0001;
        assert_eq!(fixture.admit_in(&frame, &[412]), Some(false));
        truncate::<tables::Countries>(&mut frame);
        assert_eq!(fixture.admit_in(&frame, &[412]), None);
    }

    #[test]
    fn carrier_mission_country_uses_host_task_force_roster_owner() {
        let mut fixture = PortFixture::new();
        fixture.base = AirBaseRow {
            id: 11,
            host: 88,
            ..Default::default()
        };
        fixture.air_bases[0] = fixture.base.clone();
        fixture.host_owners.insert(11, 2);
        fixture.volunteer(&[2]);
        fixture.wars = vec![war(2, 3)];
        assert_eq!(fixture.admit(&[412]), Some(true));
        fixture.host_owners.insert(11, 1);
        assert_eq!(fixture.admit(&[412]), Some(false));
        // An unresolved host is the null controller: a zero host matches it,
        // and its nonpositive raw reference falls back to the owner.
        fixture.host_owners.clear();
        fixture.volunteer(&[0, 2]);
        assert_eq!(fixture.resolve(), Some(1));
        assert_eq!(fixture.admit(&[412]), Some(false));
    }

    #[test]
    fn a_null_host_task_force_refuses_only_a_wing_that_needs_its_controller() {
        let mut fixture = PortFixture::new();
        fixture.base = AirBaseRow {
            id: 11,
            host: 88,
            ..Default::default()
        };
        fixture.air_bases[0] = fixture.base.clone();
        let registry = |fixture: &PortFixture| AirRegistrySources {
            hosts_without_task_force: BTreeSet::from([11]),
            ..fixture.registry()
        };
        let resolve = |fixture: &PortFixture| {
            let frame = fixture.frame();
            let registry = registry(fixture);
            let mission = MissionCountrySources::read(&frame, &fixture.links, &registry).unwrap();
            mission.resolve(&fixture.wing).map(|country| country.slot)
        };
        // Without volunteer hosts the controller is never read.
        assert_eq!(resolve(&fixture), Some(1));
        fixture.volunteer(&[2]);
        assert_eq!(resolve(&fixture), None);
    }

    #[test]
    fn mission_country_tries_transfer_base_after_ineligible_pool_base() {
        let mut fixture = PortFixture::new();
        fixture.wing.base_override = 22;
        fixture.provinces.push(province(101, 7, 4));
        fixture.provinces.sort_by_key(|row| row.id);
        fixture.air_bases.push(AirBaseRow {
            id: 22,
            state: Some(7),
            province: Some(101),
            ..Default::default()
        });
        fixture.links.insert(4, 4);
        fixture.volunteer(&[4]);
        assert_eq!(fixture.resolve(), Some(4));
        // An eligible pool controller stops the search; its negative raw
        // reference falls back to the owner, not to the transfer base.
        fixture.provinces[0].controller = 0x8000_0002;
        fixture.links.insert(0x8000_0002, 2);
        fixture.volunteer(&[2, 4]);
        assert_eq!(fixture.resolve(), Some(1));
        // So does a zero host matching the pool base's zero controller.
        fixture.provinces[0].controller = 0;
        fixture.volunteer(&[0, 4]);
        assert_eq!(fixture.resolve(), Some(1));
    }

    #[test]
    fn mission_country_reader_does_not_require_port_or_war_tables() {
        let fixture = PortFixture::new();
        let mut frame = DecodedFrame {
            tables: std::array::from_fn(|_| {
                TableView::Unavailable {
                    reason: "synthetic gap".into(),
                }
                .into()
            }),
            ..empty_frame()
        };
        set::<tables::Provinces>(&mut frame, fixture.provinces.clone());
        set::<tables::VolunteerHosts>(&mut frame, Vec::new());
        // Wrong row variants: reading any of them would be a schema error.
        for id in [
            TableId::NavalBases,
            TableId::TaskForces,
            TableId::WarRelations,
            TableId::RelationTuples,
        ] {
            *frame.tables[id.index()] = TableView::Complete(TableRows::empty(TableId::AirBases));
        }
        assert_eq!(fixture.resolve_in(&frame), Some(1));
    }

    #[test]
    fn incomplete_port_sources_or_unjoined_occupants_stay_unknown() {
        let mut fixture = PortFixture::new();
        let mut frame = fixture.frame();
        truncate::<tables::NavalBases>(&mut frame);
        assert_eq!(fixture.admit_in(&frame, &[412]), None);
        let mut frame = fixture.frame();
        truncate::<tables::WarRelations>(&mut frame);
        assert_eq!(fixture.admit_in(&frame, &[412]), None);
        fixture.province_units[1].naval = vec![41, 42];
        assert_eq!(fixture.admit(&[412]), Some(true));
        fixture.province_units[1].naval = vec![41];
        assert_eq!(fixture.admit(&[412]), None);
    }

    #[test]
    fn port_wars_join_aliases_and_a_complete_war_census_proves_peace() {
        let mut fixture = PortFixture::new();
        fixture.wars = vec![war(2, 13)];
        fixture.volunteer(&[12]);
        let mut frame = fixture.frame();
        assert_eq!(fixture.admit_in(&frame, &[412]), Some(true));
        // RelationTuples is no witness either way.
        *frame.tables[TableId::RelationTuples.index()] = TableView::Unavailable {
            reason: "tuple census absent".into(),
        };
        assert_eq!(fixture.admit_in(&frame, &[412]), Some(true));
        truncate::<tables::ProvinceUnitOrder>(&mut frame);
        assert_eq!(fixture.admit_in(&frame, &[412]), None);
        set::<tables::WarRelations>(&mut frame, Vec::new());
        assert_eq!(fixture.admit_in(&frame, &[412]), Some(false));
        truncate::<tables::WarRelations>(&mut frame);
        assert_eq!(fixture.admit_in(&frame, &[412]), None);
    }

    #[test]
    fn port_scan_without_a_reached_naval_base_needs_no_war_census() {
        let fixture = PortFixture::new();
        let mut frame = fixture.frame();
        truncate::<tables::WarRelations>(&mut frame);
        truncate::<tables::ProvinceUnitOrder>(&mut frame);
        assert_eq!(fixture.admit_in(&frame, &[100]), Some(false));
        assert_eq!(fixture.admit_in(&frame, &[100, 412]), None);
    }

    #[test]
    fn port_result_respects_static_gate_and_other_f66100_branches() {
        let port = |port| TargetPredicates {
            port,
            ..Default::default()
        };
        assert_eq!(
            target_admission(
                (1 << 1) | (1 << 8),
                1,
                Some(LAND),
                Some(0),
                port(Some(true))
            ),
            TargetAdmission::Allowed
        );
        assert_eq!(
            target_admission(1 << 8, 1, Some(SEA), Some(0), port(Some(true))),
            TargetAdmission::Blocked
        );
        assert_eq!(
            target_admission(1 << 8, 1, Some(LAND), Some(0), port(Some(false))),
            TargetAdmission::Blocked
        );
        assert!(matches!(
            target_admission(1 << 8, 1, None, Some(0), port(Some(true))),
            TargetAdmission::Unknown(_)
        ));
        assert_eq!(
            admission((1 << 8) | (1 << 10), 1, Some(LAND), Some(0)),
            TargetAdmission::Allowed
        );
    }

    #[test]
    fn tactical_static_gate_keeps_later_native_or_branches() {
        let tactical = TargetPredicates {
            tactical: Some(true),
            ..Default::default()
        };
        assert_eq!(
            target_admission(1 << 1, 1, Some(LAND), Some(0), tactical),
            TargetAdmission::Allowed
        );
        assert_eq!(
            target_admission(1 << 1, 1, Some(SEA), Some(0), tactical),
            TargetAdmission::Blocked
        );
        let TargetAdmission::Unknown(causes) = target_admission(1 << 1, 1, None, Some(0), tactical)
        else {
            panic!("undecided tactical branch");
        };
        assert!(causes.iter().eq([Cause::Tactical]));
        assert_eq!(
            admission((1 << 1) | (1 << 9), 1, None, Some(0)),
            TargetAdmission::Allowed
        );
    }

    fn scout_frame(controller: u32) -> DecodedFrame {
        let mut frame = empty_frame();
        set::<tables::Provinces>(&mut frame, vec![province(100, 4, 0), province(101, 4, 0)]);
        set::<tables::States>(
            &mut frame,
            vec![StateRow {
                id: 4,
                controller,
                ..Default::default()
            }],
        );
        frame
    }

    #[test]
    fn land_scout_uses_state_controller_and_directional_attache() {
        let wing = AirWingRow {
            owner: 1,
            ..Default::default()
        };
        let links = BTreeMap::from([(1, 1), (2, 2), (3, 3), (12, 2)]);
        let regions = BTreeMap::from([(7, vec![100, 101])]);
        let admit = |frame: &DecodedFrame| {
            let wars = Wars::read(frame, &links).unwrap();
            let families = fog_families(frame, &links);
            let sources =
                ScoutTargetSources::read(frame, &links, &wars, families.as_ref(), &regions)
                    .unwrap();
            sources.admit(&wing, 7, 0)
        };
        let mut frame = scout_frame(12);
        set::<tables::WarRelations>(&mut frame, vec![war(1, 2)]);
        assert_eq!(admit(&frame), Some(true));
        assert_eq!(
            target_admission(
                1 << 14,
                1,
                Some(LAND),
                Some(0),
                TargetPredicates {
                    scout: admit(&frame),
                    ..Default::default()
                },
            ),
            TargetAdmission::Allowed
        );
        // The wing country's attache slot toward the controller makes it
        // allied for fog, which excludes the state even at war.
        let attache = |country, target_index| RelationLinkRow {
            country,
            target_index,
            links: RelationLinkRow::ATTACHE,
        };
        set::<tables::RelationLinks>(&mut frame, vec![attache(1, 2)]);
        assert_eq!(admit(&frame), Some(false));
        set::<tables::RelationLinks>(&mut frame, vec![attache(2, 1)]);
        assert_eq!(admit(&frame), Some(true));
    }

    #[test]
    fn land_scout_air_role_accepts_foreign_neutral_but_not_same_state() {
        let wing = AirWingRow {
            owner: 1,
            ..Default::default()
        };
        let links = BTreeMap::from([(1, 1), (2, 2)]);
        let regions = BTreeMap::from([(7, vec![100])]);
        let admit = |frame: &DecodedFrame, flags| {
            let wars = Wars::read(frame, &links).unwrap();
            let families = fog_families(frame, &links);
            let sources =
                ScoutTargetSources::read(frame, &links, &wars, families.as_ref(), &regions)
                    .unwrap();
            sources.admit(&wing, 7, flags)
        };
        let frame = scout_frame(2);
        assert_eq!(admit(&frame, 0), Some(false));
        assert_eq!(admit(&frame, 1 << 18), Some(true));
        let frame = scout_frame(1);
        assert_eq!(admit(&frame, 1 << 18), Some(false));
        // A complete war census proves peace; without it only bit 18 decides.
        let mut frame = scout_frame(2);
        truncate::<tables::RelationTuples>(&mut frame);
        assert_eq!(admit(&frame, 0), Some(false));
        truncate::<tables::WarRelations>(&mut frame);
        set::<tables::RelationLinks>(&mut frame, Vec::new());
        assert_eq!(admit(&frame, 0), None);
    }

    #[test]
    fn land_scout_missing_relations_and_native_zero_coverage_remain_distinct() {
        let wing = AirWingRow {
            owner: 1,
            ..Default::default()
        };
        let links = BTreeMap::from([(1, 1), (2, 2)]);
        let mut frame = scout_frame(2);
        truncate::<tables::FactionMembers>(&mut frame);
        let wars = Wars::read(&frame, &links).unwrap();
        let families = fog_families(&frame, &links);
        let regions = BTreeMap::from([(7, vec![100])]);
        let sources =
            ScoutTargetSources::read(&frame, &links, &wars, families.as_ref(), &regions).unwrap();
        assert_eq!(sources.admit(&wing, 7, 1 << 18), None);
        let empty = BTreeMap::from([(7, Vec::new())]);
        let sources =
            ScoutTargetSources::read(&frame, &links, &wars, families.as_ref(), &empty).unwrap();
        assert_eq!(sources.admit(&wing, 7, 1 << 18), Some(false));
    }

    fn fog_families(frame: &DecodedFrame, links: &BTreeMap<u32, u32>) -> Option<Families> {
        Families::read(frame, links)
            .and_then(|families| families.with_attaches(frame, links))
            .ok()
    }

    fn test_wing() -> AirWingRow {
        AirWingRow {
            id: 1,
            owner: 1,
            air_base: 11,
            ..Default::default()
        }
    }

    fn test_base() -> AirBaseRow {
        AirBaseRow {
            id: 11,
            state: Some(4),
            province: Some(100),
            ..Default::default()
        }
    }

    fn test_registry() -> AirRegistrySources {
        AirRegistrySources {
            bases: BTreeMap::from([(11, test_base())]),
            ..Default::default()
        }
    }

    fn naval_combat_frame() -> DecodedFrame {
        let mut frame = scout_frame(2);
        table::<tables::Provinces>(&mut frame)[0].controller = 2;
        set::<tables::WarRelations>(&mut frame, vec![war(1, 3)]);
        set::<tables::NavalCombats>(
            &mut frame,
            vec![NavalCombatRow {
                id: 55,
                province: 900,
                ..Default::default()
            }],
        );
        let mut detail = NavalCombatDetailsRow {
            combat: 55,
            province: 900,
            ..Default::default()
        };
        detail.sides[0].summary.countries = vec![3];
        detail.sides[1].summary.countries = vec![1];
        set::<tables::NavalCombatDetails>(&mut frame, vec![detail]);
        set::<tables::NavalCombatShips>(
            &mut frame,
            vec![NavalCombatShipRow {
                combat: 55,
                side: 1,
                member_source: 0,
                group_index: 0,
                state: 1,
                detection_state: -1,
                ..Default::default()
            }],
        );
        frame
    }

    fn naval_member(frame: &mut DecodedFrame) -> &mut NavalCombatShipRow {
        &mut table::<tables::NavalCombatShips>(frame)[0]
    }

    fn naval_side(frame: &mut DecodedFrame) -> &mut Vec<u32> {
        &mut table::<tables::NavalCombatDetails>(frame)[0].sides[1]
            .summary
            .countries
    }

    #[test]
    fn naval_combat_target_needs_war_and_self_disclosed_active_detected_member() {
        let mut frame = naval_combat_frame();
        let links = BTreeMap::from([(1, 1), (2, 2), (3, 3)]);
        let (wing, registry) = (test_wing(), test_registry());
        let admit = |frame: &DecodedFrame, reached: &[u32]| {
            let mission = MissionCountrySources::read(frame, &links, &registry).unwrap();
            let wars = Wars::read(frame, &links).unwrap();
            let families = Families::read(frame, &links).ok();
            let sources =
                NavalCombatTargetSources::read(frame, &links, &wars, families.as_ref()).unwrap();
            sources.admit_combat(mission.resolve(&wing)?, reached)
        };
        let naval = |naval_combat| TargetPredicates {
            naval_combat,
            ..Default::default()
        };
        assert_eq!(admit(&frame, &[900]), Some(true));
        assert_eq!(
            target_admission(1 << 4, 1, Some(SEA), Some(0), naval(admit(&frame, &[900]))),
            TargetAdmission::Allowed
        );
        assert_eq!(admit(&frame, &[901]), Some(false));
        naval_member(&mut frame).state = 4; // On the way is outside native mask 0x0b.
        assert_eq!(admit(&frame, &[900]), Some(false));
        // Undecided occupant arms leave a negative combat arm unknown.
        assert!(matches!(
            target_admission(1 << 4, 1, Some(SEA), Some(0), naval(admit(&frame, &[900]))),
            TargetAdmission::Unknown(_)
        ));
        naval_member(&mut frame).state = 1;
        naval_member(&mut frame).detection_state = 0;
        assert_eq!(admit(&frame, &[900]), Some(false));
        // An unlinked participant beside one at war leaves nothing undecided.
        table::<tables::NavalCombatDetails>(&mut frame)[0].sides[0]
            .summary
            .countries = vec![99, 3];
        assert_eq!(admit(&frame, &[900]), Some(false));
        table::<tables::NavalCombatDetails>(&mut frame)[0].sides[0]
            .summary
            .countries = vec![99];
        assert_eq!(admit(&frame, &[900]), None);
        table::<tables::NavalCombatDetails>(&mut frame)[0].sides[0]
            .summary
            .countries = vec![3];
        naval_member(&mut frame).detection_state = -1;
        naval_member(&mut frame).member_source = 1; // Retired member vector.
        assert_eq!(admit(&frame, &[900]), Some(false));
        naval_member(&mut frame).member_source = 0;
        naval_member(&mut frame).group_index = u32::MAX;
        assert_eq!(admit(&frame, &[900]), Some(false));
        naval_member(&mut frame).group_index = 0;
        assert_eq!(admit(&frame, &[900]), Some(true));
        assert_eq!(
            target_admission(1 << 4, 1, Some(LAND), Some(0), naval(admit(&frame, &[900]))),
            TargetAdmission::Blocked
        );
    }

    #[test]
    fn naval_combat_primary_countries_and_volunteer_mission_country_are_distinct() {
        let mut frame = naval_combat_frame();
        let links = BTreeMap::from([(1, 1), (2, 2), (3, 3), (12, 2), (13, 3)]);
        let (wing, registry) = (test_wing(), test_registry());
        let admit = |frame: &DecodedFrame| {
            let mission = MissionCountrySources::read(frame, &links, &registry).unwrap();
            let wars = Wars::read(frame, &links).unwrap();
            let families = Families::read(frame, &links).ok();
            let sources =
                NavalCombatTargetSources::read(frame, &links, &wars, families.as_ref()).unwrap();
            sources.admit_combat(mission.resolve(&wing)?, &[900])
        };
        naval_side(&mut frame).clear();
        assert_eq!(admit(&frame), Some(false));

        // A faction partner shares visibility unless the one war census
        // vetoes it.
        *naval_side(&mut frame) = vec![2];
        assert_eq!(admit(&frame), Some(false));
        let member = |country| FactionMemberRow {
            country,
            faction: 5,
            ordinal: country,
        };
        set::<tables::FactionMembers>(&mut frame, vec![member(1), member(2)]);
        assert_eq!(admit(&frame), Some(true));
        table::<tables::WarRelations>(&mut frame).push(war(1, 2));
        assert_eq!(admit(&frame), Some(false));
        set::<tables::FactionMembers>(&mut frame, Vec::new());
        set::<tables::WarRelations>(&mut frame, vec![war(1, 3)]);

        // The mission country becomes the volunteer base controller 2.
        *naval_side(&mut frame) = vec![12];
        set::<tables::WarRelations>(&mut frame, vec![war(2, 13)]);
        set::<tables::VolunteerHosts>(
            &mut frame,
            vec![VolunteerHostRow {
                country: 1,
                host: 12,
                ..Default::default()
            }],
        );
        assert_eq!(admit(&frame), Some(true));
        set::<tables::WarRelations>(&mut frame, Vec::new());
        assert_eq!(admit(&frame), Some(false));
        truncate::<tables::NavalCombatShips>(&mut frame);
        assert_eq!(admit(&frame), None);
    }

    #[test]
    fn mine_laying_reads_the_targeted_effective_country_region_level() {
        let mut frame = empty_frame();
        let links = BTreeMap::from([(1, 1), (2, 2), (12, 2)]);
        let mut wing = AirWingRow {
            owner: 1,
            ..Default::default()
        };
        let admit = |frame: &DecodedFrame, wing: &AirWingRow, region: u32| {
            let sources = MineTargetSources::read(frame, &links).unwrap();
            sources.admit(wing, region, 100_000_000)
        };
        // A pair the collector did not record is unknown, not zero.
        assert_eq!(admit(&frame, &wing, 7), None);
        let level = |country, raw_mines| NavalMineLevelRow {
            country,
            region: 7,
            raw_mines,
        };
        set::<tables::NavalMineLevels>(&mut frame, vec![level(1, 0), level(2, 100_000_000)]);
        assert_eq!(admit(&frame, &wing, 7), Some(true));
        wing.country_override = 12; // Alias of country 2 is the effective owner.
        assert_eq!(admit(&frame, &wing, 7), Some(false));
        assert_eq!(admit(&frame, &wing, 8), None);
        table::<tables::NavalMineLevels>(&mut frame)[1].raw_mines -= 1;
        assert_eq!(admit(&frame, &wing, 7), Some(true));
        truncate::<tables::NavalMineLevels>(&mut frame);
        assert_eq!(admit(&frame, &wing, 7), None);
    }

    #[test]
    fn mine_laying_obeys_sea_dlc_and_or_branch_priority() {
        let admitted = |mask, fraction, dlc, mine| {
            target_admission(
                mask,
                1,
                fraction,
                dlc,
                TargetPredicates {
                    mine,
                    ..Default::default()
                },
            )
        };
        assert_eq!(
            admitted(AIR_MISSION_LAY_MINES, Some(SEA), Some(0x10), Some(true)),
            TargetAdmission::Allowed
        );
        assert_eq!(
            admitted(AIR_MISSION_LAY_MINES, Some(LAND), Some(0x10), Some(true)),
            TargetAdmission::Blocked
        );
        assert_eq!(
            admitted(AIR_MISSION_LAY_MINES, Some(SEA), Some(0), Some(true)),
            TargetAdmission::Blocked
        );
        assert_eq!(
            admitted(AIR_MISSION_LAY_MINES, Some(SEA), Some(0x10), Some(false)),
            TargetAdmission::Blocked
        );
        assert!(matches!(
            admitted(AIR_MISSION_LAY_MINES, Some(SEA), Some(0x10), None),
            TargetAdmission::Unknown(_)
        ));
        assert_eq!(
            admitted(
                (1 << 1) | AIR_MISSION_LAY_MINES,
                Some(MIXED),
                Some(0x10),
                Some(true)
            ),
            TargetAdmission::Allowed
        );
    }

    fn embarked_frame() -> DecodedFrame {
        let mut frame = naval_combat_frame();
        set::<tables::ProvinceUnitOrder>(
            &mut frame,
            vec![ProvinceUnitOrderRow {
                province: 900,
                mixed: vec![40],
                ..Default::default()
            }],
        );
        set::<tables::Divisions>(
            &mut frame,
            vec![DivisionRow {
                id: 40,
                logical_owner: 3,
                province: 900,
                ..Default::default()
            }],
        );
        set::<tables::DivisionMovement>(
            &mut frame,
            vec![DivisionMovementRow {
                division: 40,
                naval_transfer: 1,
                ..Default::default()
            }],
        );
        frame
    }

    fn embarked_movement(frame: &mut DecodedFrame) -> &mut DivisionMovementRow {
        &mut table::<tables::DivisionMovement>(frame)[0]
    }

    #[test]
    fn embarked_target_uses_current_sea_location_transfer_combat_and_war() {
        let mut frame = embarked_frame();
        let links = BTreeMap::from([(1, 1), (2, 2), (3, 3)]);
        let (wing, registry) = (test_wing(), test_registry());
        let kinds = BTreeMap::from([(100, ProvinceKind::Land), (900, ProvinceKind::Sea)]);
        let admit = |frame: &DecodedFrame, kinds: &BTreeMap<u32, ProvinceKind>| {
            let mission = MissionCountrySources::read(frame, &links, &registry).unwrap();
            let wars = Wars::read(frame, &links).unwrap();
            let sources = NavalOccupantSources::read(frame, &wars, kinds).unwrap();
            sources.admit(mission.resolve(&wing).unwrap(), &[900])
        };
        let embarked = |frame: &DecodedFrame, kinds| admit(frame, kinds).embarked;
        assert_eq!(embarked(&frame, &kinds), Some(true));
        assert_eq!(
            target_admission(
                1 << 4,
                1,
                Some(SEA),
                Some(0),
                TargetPredicates {
                    naval_combat: Some(false),
                    naval_occupants: admit(&frame, &kinds),
                    ..Default::default()
                },
            ),
            TargetAdmission::Allowed
        );
        let land = BTreeMap::from([(900, ProvinceKind::Land)]);
        assert_eq!(embarked(&frame, &land), Some(false));
        embarked_movement(&mut frame).naval_transfer = 0;
        assert_eq!(embarked(&frame, &kinds), Some(false));
        embarked_movement(&mut frame).naval_transfer = 1;
        embarked_movement(&mut frame).combat_count = 1;
        assert_eq!(embarked(&frame, &kinds), Some(false));
        embarked_movement(&mut frame).combat_count = 0;
        table::<tables::Divisions>(&mut frame)[0].logical_owner = 2;
        assert_eq!(embarked(&frame, &kinds), Some(false));
        table::<tables::Divisions>(&mut frame)[0].logical_owner = 3;
        assert_eq!(embarked(&frame, &kinds), Some(true));
    }

    #[test]
    fn task_force_target_needs_sea_location_no_combat_and_war() {
        let mut frame = embarked_frame();
        table::<tables::ProvinceUnitOrder>(&mut frame)[0] = ProvinceUnitOrderRow {
            province: 900,
            mixed: vec![50],
            naval: vec![50],
        };
        set::<tables::TaskForces>(
            &mut frame,
            vec![TaskForceRow {
                id: 50,
                province: 900,
                logical_owner: 3,
                ..Default::default()
            }],
        );
        let links = BTreeMap::from([(1, 1), (2, 2), (3, 3)]);
        let (wing, registry) = (test_wing(), test_registry());
        let kinds = BTreeMap::from([(900, ProvinceKind::Sea)]);
        let admit = |frame: &DecodedFrame| {
            let mission = MissionCountrySources::read(frame, &links, &registry).unwrap();
            let wars = Wars::read(frame, &links).unwrap();
            let sources = NavalOccupantSources::read(frame, &wars, &kinds).unwrap();
            sources.admit(mission.resolve(&wing).unwrap(), &[900])
        };
        // The task force is also a mixed occupant, not a railway gun.
        assert_eq!(
            admit(&frame),
            NavalOccupants {
                task_force: Some(true),
                embarked: Some(false),
                railway_gun: Some(false),
            }
        );
        table::<tables::TaskForces>(&mut frame)[0].combat_references = 1;
        assert_eq!(admit(&frame).task_force, Some(false));
        let occupants = admit(&frame);
        assert_eq!(
            target_admission(
                1 << 7,
                1,
                Some(SEA),
                Some(0),
                TargetPredicates {
                    naval_combat: Some(false),
                    naval_occupants: occupants,
                    ..Default::default()
                },
            ),
            TargetAdmission::Blocked
        );
        table::<tables::TaskForces>(&mut frame)[0].combat_references = 0;
        table::<tables::TaskForces>(&mut frame)[0].logical_owner = 2;
        assert_eq!(admit(&frame).task_force, Some(false));
        // Without a complete war census silence does not prove peace.
        truncate::<tables::WarRelations>(&mut frame);
        assert_eq!(admit(&frame).task_force, None);
        truncate::<tables::TaskForces>(&mut frame);
        let occupants = admit(&frame);
        assert_eq!((occupants.task_force, occupants.railway_gun), (None, None));
    }

    #[test]
    fn embarked_target_preserves_unknown_and_counts_railway_guns() {
        let mut frame = embarked_frame();
        let links = BTreeMap::from([(1, 1), (3, 3)]);
        let (wing, registry) = (test_wing(), test_registry());
        let kinds = BTreeMap::from([(900, ProvinceKind::Sea)]);
        let admit = |frame: &DecodedFrame, kinds: &BTreeMap<u32, ProvinceKind>| {
            let mission = MissionCountrySources::read(frame, &links, &registry).unwrap();
            let wars = Wars::read(frame, &links).unwrap();
            let sources = NavalOccupantSources::read(frame, &wars, kinds).unwrap();
            sources.admit(mission.resolve(&wing).unwrap(), &[900])
        };
        assert_eq!(admit(&frame, &kinds).embarked, Some(true));
        // Neither a division nor a task force: an unrecorded railway gun.
        table::<tables::ProvinceUnitOrder>(&mut frame)[0].mixed = vec![77];
        let occupants = admit(&frame, &kinds);
        assert_eq!(
            occupants,
            NavalOccupants {
                task_force: Some(false),
                embarked: Some(false),
                railway_gun: None,
            }
        );
        assert_eq!(
            target_admission(
                1 << 4,
                1,
                Some(SEA),
                Some(0),
                TargetPredicates {
                    naval_combat: Some(false),
                    naval_occupants: occupants,
                    ..Default::default()
                },
            ),
            TargetAdmission::Unknown(Causes(1 << Cause::NavalRailwayGun as u16))
        );
        table::<tables::ProvinceUnitOrder>(&mut frame)[0].mixed = vec![40];
        assert_eq!(admit(&frame, &BTreeMap::new()), NavalOccupants::default());
        truncate::<tables::DivisionMovement>(&mut frame);
        assert_eq!(admit(&frame, &kinds).embarked, None);
        // An undecided combat or occupant arm is not the railway-gun gap,
        // but the railway-gun arm stays undecided beside it.
        let TargetAdmission::Unknown(causes) =
            target_admission(1 << 4, 1, Some(SEA), Some(0), TargetPredicates::default())
        else {
            panic!("undecided naval arms");
        };
        assert!(!causes.railway_gun_only());
        assert!(causes.iter().eq([Cause::Naval, Cause::NavalRailwayGun]));
        // A proved arm leaves only the region fraction undecided.
        let TargetAdmission::Unknown(causes) = target_admission(
            1 << 4,
            1,
            None,
            Some(0),
            TargetPredicates {
                naval_combat: Some(true),
                ..Default::default()
            },
        ) else {
            panic!("undecided sea fraction");
        };
        assert!(causes.iter().eq([Cause::Naval]));
    }

    fn strategic_frame() -> DecodedFrame {
        let mut frame = empty_frame();
        set::<tables::Provinces>(
            &mut frame,
            vec![
                province(100, 4, 1),
                province(400, 7, 2),
                province(401, 7, 3),
                province(900, 8, 3),
            ],
        );
        let state = |id, controller| StateRow {
            id,
            controller,
            ..Default::default()
        };
        set::<tables::States>(&mut frame, vec![state(7, 2), state(8, 3)]);
        set::<tables::WarRelations>(&mut frame, vec![war(1, 2), war(1, 3)]);
        set::<tables::ProvinceBuildings>(
            &mut frame,
            vec![BuildingRow {
                location: 401,
                level: 1,
                active: true,
                ..Default::default()
            }],
        );
        set::<tables::ProvinceUnitOrder>(
            &mut frame,
            vec![ProvinceUnitOrderRow {
                province: 401,
                mixed: vec![40],
                ..Default::default()
            }],
        );
        set::<tables::Divisions>(
            &mut frame,
            vec![DivisionRow {
                id: 40,
                province: 401,
                ..Default::default()
            }],
        );
        frame
    }

    #[test]
    fn strategic_witness_uses_full_region_states_and_active_building_vectors() {
        let mut frame = strategic_frame();
        let links = BTreeMap::from([(1, 1), (2, 2), (3, 3)]);
        let regions = BTreeMap::from([(7, vec![400])]);
        let (wing, registry) = (test_wing(), test_registry());
        let full = |frame: &DecodedFrame, reached: &[u32]| {
            let mission = MissionCountrySources::read(frame, &links, &registry).unwrap();
            let wars = Wars::read(frame, &links).unwrap();
            let sources = StrategicTargetSources::read(frame, &wars, &regions).unwrap();
            sources.admit(mission.resolve(&wing).unwrap(), 7, reached)
        };
        let admit = |frame: &DecodedFrame, reached: &[u32]| full(frame, reached).found;
        // Province 401 is outside region 7 but belongs to its state 7.
        assert_eq!(admit(&frame, &[401]), Some(true));
        assert_eq!(admit(&frame, &[900]), Some(false));
        // An undecided state controller does not stop the province
        // controller's test; a proved peace there decides the province.
        table::<tables::States>(&mut frame)[0].controller = 99;
        assert_eq!(admit(&frame, &[401]), None);
        table::<tables::Provinces>(&mut frame)[2].controller = 1;
        assert_eq!(admit(&frame, &[401]), Some(false));
        table::<tables::Provinces>(&mut frame)[2].controller = 3;
        table::<tables::States>(&mut frame)[0].controller = 2;
        for bit in [3, 16] {
            assert_eq!(
                target_admission(
                    (1 << 1) | (1 << bit),
                    1,
                    Some(LAND),
                    Some(0),
                    TargetPredicates {
                        strategic: admit(&frame, &[401]),
                        ..Default::default()
                    },
                ),
                TargetAdmission::Allowed
            );
        }
        // F61B40 walks the active vector, not the full building list.
        table::<tables::ProvinceBuildings>(&mut frame)[0].active = false;
        assert_eq!(admit(&frame, &[401]), Some(false));
        const RAIL_WAY: u32 = cbuilding_template::RAIL_WAY_TOKEN;
        const OTHER: u32 = RAIL_WAY + 1;
        table::<tables::ProvinceBuildings>(&mut frame)[0].level = -1;
        table::<tables::ProvinceBuildings>(&mut frame)[0].building_type = RAIL_WAY;
        table::<tables::ProvinceUnitOrder>(&mut frame)[0].mixed = vec![77];
        assert_eq!(admit(&frame, &[401]), Some(false));
        table::<tables::ProvinceUnitOrder>(&mut frame)[0].mixed = vec![40];
        table::<tables::ProvinceBuildings>(&mut frame)[0].active = true;
        // F61B40's railway-gun arm: every mixed occupant is a recorded
        // division or task force, so there is none.
        assert_eq!(admit(&frame, &[401]), Some(false));
        table::<tables::ProvinceUnitOrder>(&mut frame)[0].mixed = vec![77];
        assert_eq!(
            full(&frame, &[401]),
            Strategic {
                found: None,
                railway_gun: true
            }
        );
        // F61B40 skips any other type below level 1, whatever the occupants,
        // so the strategic scan blocks the mission.
        table::<tables::ProvinceBuildings>(&mut frame)[0].building_type = OTHER;
        assert_eq!(admit(&frame, &[401]), Some(false));
        for bit in [3, 16] {
            assert_eq!(
                target_admission(
                    1 << bit,
                    1,
                    Some(LAND),
                    Some(0),
                    TargetPredicates {
                        strategic: admit(&frame, &[401]),
                        ..Default::default()
                    },
                ),
                TargetAdmission::Blocked
            );
        }
        table::<tables::ProvinceBuildings>(&mut frame)[0].building_type = RAIL_WAY;
        let railway = |bit: u32| {
            target_admission(
                1 << bit,
                1,
                Some(LAND),
                Some(0),
                TargetPredicates {
                    strategic: None,
                    strategic_railway_gun: true,
                    ..Default::default()
                },
            )
        };
        for bit in [3, 16] {
            assert!(
                matches!(railway(bit), TargetAdmission::Unknown(causes) if causes.railway_gun_only())
            );
        }
        table::<tables::ProvinceUnitOrder>(&mut frame)[0].mixed = vec![40];
        assert_eq!(admit(&frame, &[401]), Some(false));
        truncate::<tables::Divisions>(&mut frame);
        assert!(!full(&frame, &[401]).railway_gun);
        assert_eq!(admit(&frame, &[401]), None);
        let building = BuildingRow {
            location: 7,
            level: 2,
            active: true,
            ..Default::default()
        };
        set::<tables::StateBuildings>(&mut frame, vec![building]);
        assert_eq!(admit(&frame, &[401]), Some(true));
        assert_eq!(admit(&frame, &[900]), Some(false)); // State buildings need a candidate.
        // An active state rail way below level 1 would reach the railway arm
        // with no province.
        table::<tables::StateBuildings>(&mut frame)[0].level = 0;
        table::<tables::StateBuildings>(&mut frame)[0].building_type = RAIL_WAY;
        assert_eq!(
            full(&frame, &[401]),
            Strategic {
                found: None,
                railway_gun: false
            }
        );
        // Other types below level 1 are skipped in both vectors.
        table::<tables::StateBuildings>(&mut frame)[0].building_type = OTHER;
        table::<tables::ProvinceBuildings>(&mut frame)[0].building_type = OTHER;
        assert_eq!(admit(&frame, &[401]), Some(false));
        truncate::<tables::StateBuildings>(&mut frame);
        assert_eq!(admit(&frame, &[401]), None);
    }

    #[test]
    fn railway_gun_omissions_need_every_undecided_branch_to_be_a_railway_arm() {
        let targets = TargetPredicates {
            strategic_railway_gun: true,
            naval_combat: Some(false),
            naval_occupants: NavalOccupants {
                task_force: Some(false),
                embarked: Some(false),
                railway_gun: None,
            },
            ..Default::default()
        };
        let causes = |mask| match target_admission(mask, 1, Some(MIXED), Some(0x10), targets) {
            TargetAdmission::Unknown(causes) => causes,
            other => panic!("{other:?}"),
        };
        let both = causes((1 << 3) | (1 << 4));
        assert!(both.railway_gun_only());
        assert!(
            both.iter()
                .eq([Cause::StrategicRailwayGun, Cause::NavalRailwayGun])
        );
        // An undecided tactical branch is not a railway-gun omission.
        let mixed = causes((1 << 1) | (1 << 3));
        assert!(!mixed.railway_gun_only());
        assert_eq!(mixed.iter().count(), 2);
        assert!(!Causes::default().railway_gun_only());
    }

    #[test]
    fn bypasses_end_the_scans_f66100_reads() {
        assert_eq!(scanned_bits((1 << 6) | (1 << 8)), 0);
        let transport = (1 << 10) | (1 << 3) | (1 << 8) | 0x1_f000;
        assert_eq!(scanned_bits(transport), (1 << 10) | (1 << 3) | (1 << 8));
        assert_eq!(scanned_bits((1 << 9) | (1 << 16)), 1 << 9);
        assert_eq!(scanned_bits(0x1_f000), 0x1_f000);
    }

    #[test]
    fn category_priority_matches_native_target_branches() {
        assert_eq!(
            category((1 << 10) | (1 << 14) | (1 << 17), 1, 0),
            Some(Category::Transport)
        );
        assert_eq!(category((1 << 14) | 0x85, 1, 0), Some(Category::Scout));
        assert_eq!(category(0x2, 1, 0), Some(Category::BomberFlight));
        assert_eq!(
            category(0x2, 1, 0x6_0001_0000),
            Some(Category::BaseEquipment)
        );
        assert_eq!(
            category((1 << 10) | (1 << 14) | (1 << 17), 0, 0),
            Some(Category::BaseEquipment)
        );
        assert_eq!(category(0x80 | 0x2, 1, 0), Some(Category::Fighter));
        assert_eq!(category(1 << 11, 1, 0), None);
    }

    #[test]
    fn projected_air_definition_bits_preserve_native_map_predicates() {
        let mask = AIR_MISSION_MAP_FLAGS;
        assert_eq!(mask, 0x1e_0005_0000);
        for raw in [0, u64::MAX, mask, !mask]
            .into_iter()
            .chain((0..64).map(|bit| 1_u64 << bit))
        {
            let projected = raw & mask;
            assert_eq!(
                raw & AIR_MISSION_ZERO_PLANE_FLAGS != 0,
                projected & AIR_MISSION_ZERO_PLANE_FLAGS != 0
            );
            assert_eq!(category(0x2, 7, raw), category(0x2, 7, projected));
            assert_eq!(
                raw & AIR_MISSION_FOREIGN_TARGET_FLAG != 0,
                projected & AIR_MISSION_FOREIGN_TARGET_FLAG != 0
            );
        }
    }

    #[test]
    fn reached_count_gates_every_mask_and_stale_membership_is_used_as_is() {
        // Province 412 can belong to an earlier region and target 913 can be
        // stale independently; neither is part of 14122EB00's checks.
        let none = TargetPredicates::default();
        assert_eq!(
            target_admission(1 << 10, 1, Some(LAND), Some(0), none),
            TargetAdmission::Allowed
        );
        assert_eq!(category(1 << 10, 913, 0), Some(Category::Transport));
        assert_eq!(
            admission(1 << 10, 0, Some(LAND), Some(0)),
            TargetAdmission::Blocked
        );
        assert_eq!(
            admission(1 << 6, 0, Some(LAND), Some(0)),
            TargetAdmission::Blocked
        );
        // The count is the gate; IDs are recorded only for membership scans.
        assert_eq!(
            admission(1 << 2, 1, Some(LAND), Some(0)),
            TargetAdmission::Allowed
        );
        assert_eq!(
            target_admission(1 << 2, 0, Some(LAND), Some(0), none),
            TargetAdmission::Blocked
        );
    }

    #[test]
    fn later_proved_true_bypass_resolves_earlier_unknown() {
        assert!(matches!(
            admission(1 << 1, 1, Some(LAND), Some(0)),
            TargetAdmission::Unknown(_)
        ));
        assert_eq!(
            admission((1 << 1) | (1 << 10), 1, Some(LAND), Some(0)),
            TargetAdmission::Allowed
        );
        assert_eq!(
            admission((1 << 8) | (1 << 6), 1, Some(LAND), Some(0)),
            TargetAdmission::Allowed
        );
    }

    #[test]
    fn unknown_target_predicates_do_not_become_false() {
        for bit in [1, 3, 4, 7, 8, 16] {
            assert!(matches!(
                admission(1 << bit, 1, Some(MIXED), Some(0x10)),
                TargetAdmission::Unknown(_)
            ));
        }
        assert!(matches!(
            admission(1 << 14, 1, Some(LAND), Some(0x10)),
            TargetAdmission::Unknown(_)
        ));
    }

    #[test]
    fn zero_region_fraction_blocks_only_the_impossible_target_branches() {
        for bit in [1, 3, 8, 16] {
            assert_eq!(
                admission(1 << bit, 1, Some(SEA), Some(0x10)),
                TargetAdmission::Blocked
            );
        }
        for bit in [4, 7] {
            assert_eq!(
                admission(1 << bit, 1, Some(LAND), Some(0x10)),
                TargetAdmission::Blocked
            );
        }
        assert_eq!(
            admission((1 << 1) | (1 << 15), 1, Some(SEA), Some(0x10)),
            TargetAdmission::Allowed
        );
        assert!(matches!(
            admission((1 << 3) | (1 << 4), 1, Some(LAND), Some(0x10)),
            TargetAdmission::Unknown(_)
        ));
        assert!(matches!(
            admission(1 << 4, 1, None, Some(0x10)),
            TargetAdmission::Unknown(_)
        ));
    }

    #[test]
    fn proved_sea_branches_can_resolve_earlier_unknown() {
        assert_eq!(region_fractions(2, 1, 1), MIXED);
        assert_eq!(region_fractions(100_001, 100_000, 1).sea_raw, 0);
        assert_eq!(region_fractions(0, 0, 0), RegionFractions::default());
        assert_eq!(
            admission((1 << 1) | (1 << 13), 1, Some(MIXED), Some(0x10)),
            TargetAdmission::Allowed
        );
        assert_eq!(
            admission(1 << 13, 1, Some(MIXED), Some(0)),
            TargetAdmission::Blocked
        );
        assert_eq!(
            admission(1 << 14, 1, Some(MIXED), Some(0)),
            TargetAdmission::Allowed
        );
        assert_eq!(
            admission(1 << 15, 1, Some(MIXED), None),
            TargetAdmission::Allowed
        );
        assert!(matches!(
            admission(1 << 13, 1, None, Some(0x10)),
            TargetAdmission::Unknown(_)
        ));
    }
}
