//! Province fog rebuilt for the viewing country from recorded simulation. Walk admitted countries in
//! order, stamp neighbours and add radar/occupied-state terms with saturation, preserving addition
//! order.

use installed_content::{
    ResolvedLayers,
    map::{MapData, ProvinceKind},
};
use installed_content_save::naval_history::RailwayAdjacency;
use replay::{
    frame::DecodedFrame,
    rows::{
        CountryIntelRow, Recorded, RelationLinkRow, ReservedCountryNamingRow, ZeroSubjectInputs,
        tables,
    },
};
use std::{
    collections::{BTreeMap, BTreeSet},
    io,
};

/// The content half of the fog: province topology and the `NIntel` defines.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FogContent {
    pub topology: Topology,
    pub defines: Defines,
}

impl FogContent {
    pub fn load(
        layers: &ResolvedLayers,
        map: &MapData,
        defines: &installed_content::defines::Defines,
    ) -> io::Result<Self> {
        Ok(Self {
            topology: Topology::read(layers, map)?,
            defines: Defines::load(defines)?,
        })
    }
}

/// Province land/sea sets and native neighbour vectors.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Topology {
    pub land: BTreeSet<u32>,
    pub sea: BTreeSet<u32>,
    pub neighbors: BTreeMap<u32, Vec<u32>>,
}

impl Topology {
    pub fn read(layers: &ResolvedLayers, map: &MapData) -> io::Result<Self> {
        let adjacency = RailwayAdjacency::read(layers)?;
        let kind = |wanted| {
            map.definitions
                .values()
                .filter(move |p| p.kind == wanted)
                .map(|p| p.id)
        };
        Ok(Self {
            land: kind(ProvinceKind::Land).collect(),
            sea: kind(ProvinceKind::Sea).collect(),
            neighbors: map
                .definitions
                .keys()
                .filter(|&&p| p != 0)
                .map(|&p| {
                    adjacency
                        .neighbors(p)
                        .map(|n| (p, n.to_vec()))
                        .ok_or_else(|| {
                            io::Error::new(
                                io::ErrorKind::InvalidData,
                                format!("province {p} is missing from the native adjacency"),
                            )
                        })
                })
                .collect::<io::Result<_>>()?,
        })
    }
}

/// The `NIntel` and `NBuildings` defines the visibility laws read.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Defines {
    /// `COUNTRY_LEVEL_INTEL_MAXIMUMS`: civilian, army, navy, air.
    pub intel_maximums: [i64; 4],
    /// `NBuildings.RADAR_INTEL_EFFECT`, the native integer define.
    pub radar_effect: i32,
    /// `MAP_INTEL_VISIBILITY_CUTOFFS` threshold/value pairs.
    pub map_cutoffs: Vec<i64>,
    /// `AIR_MAPICON_SHOW_ALL_AIR_PORTS`.
    pub air_icon_threshold: i64,
    /// `AIR_MAPICON_AIR_PORT_VISIBILITY_DETAIL_THRESHOLDS`.
    pub air_detail_thresholds: [i64; 4],
    /// `AIR_MAPICON_AIR_PORT_PLANE_FUZZY_THRESHOLD`, the independent count spread.
    pub air_plane_fuzzy_range: i64,
    /// `NAVY_MAPICON_NAVAL_PORT_VISIBILITY_DETAIL_THRESHOLDS`; the engine asserts five.
    pub naval_detail_thresholds: [i64; 5],
}

impl Defines {
    pub fn load(defines: &installed_content::defines::Defines) -> io::Result<Self> {
        let four = |name: &str| -> io::Result<[i64; 4]> {
            defines
                .fixed5_array("NIntel", name)?
                .try_into()
                .map_err(|_| invalid(format!("NDefines.NIntel.{name} must hold four values")))
        };
        // Lua integer conversion truncates the double, then the loader keeps
        // the low signed dword.
        let radar = defines.number("NBuildings", "RADAR_INTEL_EFFECT")?;
        if !(-9.223_372_036_854_776e18..9.223_372_036_854_776e18).contains(&radar) {
            return Err(invalid(
                "NDefines.NBuildings.RADAR_INTEL_EFFECT is outside the native integer range",
            ));
        }
        Ok(Self {
            intel_maximums: four("COUNTRY_LEVEL_INTEL_MAXIMUMS")?,
            radar_effect: radar as i64 as i32,
            map_cutoffs: defines.fixed5_array("NIntel", "MAP_INTEL_VISIBILITY_CUTOFFS")?,
            air_icon_threshold: defines.fixed5("NIntel", "AIR_MAPICON_SHOW_ALL_AIR_PORTS")?,
            air_detail_thresholds: four("AIR_MAPICON_AIR_PORT_VISIBILITY_DETAIL_THRESHOLDS")?,
            air_plane_fuzzy_range: defines
                .fixed5("NIntel", "AIR_MAPICON_AIR_PORT_PLANE_FUZZY_THRESHOLD")?,
            naval_detail_thresholds: defines
                .fixed5_array(
                    "NIntel",
                    "NAVY_MAPICON_NAVAL_PORT_VISIBILITY_DETAIL_THRESHOLDS",
                )?
                .try_into()
                .map_err(|_| {
                    invalid(
                        "NDefines.NIntel.NAVY_MAPICON_NAVAL_PORT_VISIBILITY_DETAIL_THRESHOLDS \
                         must hold five values",
                    )
                })?,
        })
    }
}

fn invalid(message: impl Into<String>) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, message.into())
}

/// Faction, subject and attache visibility inputs in canonical slots, without wars. Subjects use
/// recorded installed typed pointers without retirement or date filtering; RelationLinks records
/// attache membership. Callers with their own war census can read these inputs independently.
#[derive(Clone, Default)]
pub struct Families {
    factions: BTreeMap<u32, u64>,
    subjects: BTreeSet<(u32, u32)>,
    attaches: BTreeSet<(u32, u32)>,
}

impl Families {
    /// Faction and subject inputs for full-country visibility.
    pub fn read(frame: &DecodedFrame, links: &BTreeMap<u32, u32>) -> Result<Self, &'static str> {
        let mut out = Self::default();
        out.read_factions(frame, links)?;
        out.read_subjects(frame, links)?;
        Ok(out)
    }

    pub(super) fn read_factions(
        &mut self,
        frame: &DecodedFrame,
        links: &BTreeMap<u32, u32>,
    ) -> Result<(), &'static str> {
        for row in complete::<tables::FactionMembers>(frame, "FactionMembers")? {
            // Faction rosters include genuine Country0 members. Recognize the literal only here; unknown
            // positive links remain failures.
            let country = if row.country == 0 {
                0
            } else {
                link(links, row.country, "FactionMembers country")?
            };
            self.factions.insert(country, row.faction);
        }
        Ok(())
    }

    pub(super) fn read_subjects(
        &mut self,
        frame: &DecodedFrame,
        links: &BTreeMap<u32, u32>,
    ) -> Result<(), &'static str> {
        for row in complete::<tables::Subjects>(frame, "Subjects")? {
            let (a, b) = (
                link(links, row.overlord, "Subjects country")?,
                link(links, row.subject, "Subjects country")?,
            );
            self.subjects.extend([(a, b), (b, a)]);
        }
        Ok(())
    }

    /// Relations where each pair shares a faction of its own.
    #[cfg(test)]
    pub(crate) fn factions_for_tests(pairs: &[(u32, u32)]) -> Self {
        let mut out = Self::default();
        for (faction, &(a, b)) in (1..).zip(pairs) {
            out.factions.extend([(a, faction), (b, faction)]);
        }
        out
    }

    /// Also admits recorded attache country slots.
    pub fn with_attaches(
        mut self,
        frame: &DecodedFrame,
        links: &BTreeMap<u32, u32>,
    ) -> Result<Self, &'static str> {
        for row in complete::<tables::RelationLinks>(frame, "RelationLinks")? {
            if row.links & RelationLinkRow::ATTACHE != 0 {
                let owner = *links.get(&row.country).ok_or("RelationLinks country")?;
                self.attaches.insert((owner, row.target_index));
            }
        }
        Ok(self)
    }

    /// The same country, a faction partner, an overlord or a subject, before the live-war veto.
    pub fn family(&self, a: u32, b: u32) -> bool {
        a == b || self.subjects.contains(&(a, b)) || self.same_faction(a, b)
    }

    /// one non-null faction.
    pub fn same_faction(&self, a: u32, b: u32) -> bool {
        self.factions
            .get(&a)
            .is_some_and(|&f| self.member_of_faction(b, f))
    }

    /// Scan the target's exact faction roster.
    pub(super) fn member_of_faction(&self, country: u32, faction: u64) -> bool {
        self.factions.get(&country) == Some(&faction)
    }

    /// Attache sharing has no war veto and uses the source's directed attache slot toward the viewer.
    pub fn allied_for_fog(&self, source: u32, viewer: u32) -> bool {
        self.family(source, viewer) || self.attaches.contains(&(source, viewer))
    }
}

/// [`Families`] with the recorded wars whose retired byte is zero.
#[derive(Clone, Default)]
pub struct Relations {
    families: Families,
    wars: BTreeSet<(u32, u32)>,
}

impl Relations {
    /// Relations at peace where each pair shares a faction of its own.
    #[cfg(test)]
    pub(crate) fn factions_for_tests(pairs: &[(u32, u32)]) -> Self {
        Self {
            families: Families::factions_for_tests(pairs),
            wars: BTreeSet::new(),
        }
    }

    /// Faction, war and subject inputs for full-country visibility.
    pub fn read(frame: &DecodedFrame, links: &BTreeMap<u32, u32>) -> Result<Self, &'static str> {
        let mut out = Self::default();
        out.families.read_factions(frame, links)?;
        for row in complete::<tables::WarRelations>(frame, "WarRelations")? {
            let (a, b) = (
                link(links, row.first_country, "WarRelations country")?,
                link(links, row.second_country, "WarRelations country")?,
            );
            out.wars.extend([(a, b), (b, a)]);
        }
        out.families.read_subjects(frame, links)?;
        Ok(out)
    }

    /// Also admits recorded attache country slots.
    pub fn with_attaches(
        mut self,
        frame: &DecodedFrame,
        links: &BTreeMap<u32, u32>,
    ) -> Result<Self, &'static str> {
        self.families = self.families.with_attaches(frame, links)?;
        Ok(self)
    }

    /// Distinct countries with an active war relation.
    pub fn at_war(&self, a: u32, b: u32) -> bool {
        a != b && self.wars.contains(&(a, b))
    }

    /// The countries in an active war relation with `a`.
    pub fn enemies(&self, a: u32) -> impl Iterator<Item = u32> + '_ {
        self.wars.range((a, 0)..=(a, u32::MAX)).map(|&(_, b)| b)
    }

    /// See [`Families::same_faction`].
    pub fn same_faction(&self, a: u32, b: u32) -> bool {
        self.families.same_faction(a, b)
    }

    /// Full visibility is vetoed by a live war; otherwise it admits the same country, a faction
    /// partner, an overlord or a subject.
    pub fn shared(&self, a: u32, b: u32) -> bool {
        a == b || (!self.wars.contains(&(a, b)) && self.families.family(a, b))
    }

    /// See [`Families::allied_for_fog`].
    pub fn allied_for_fog(&self, source: u32, viewer: u32) -> bool {
        self.families.allied_for_fog(source, viewer)
    }
}

fn link(links: &BTreeMap<u32, u32>, raw: u32, what: &'static str) -> Result<u32, &'static str> {
    links.get(&raw).copied().ok_or(what)
}

fn complete<'a, T: replay::rows::TableSchema>(
    frame: &'a DecodedFrame,
    name: &'static str,
) -> Result<&'a [T::Row], &'static str> {
    frame.complete::<T>().map_err(|_| name)
}

fn saturate(current: u8, term: i32) -> u8 {
    i32::from(current).wrapping_add(term).clamp(0, 255) as u8
}
/// Fixed-point technology rounding.
fn rounded(value: i64) -> i32 {
    (value.wrapping_add(if value < 0 { -50_000 } else { 50_000 }) / 100_000) as i32
}
/// A zero maximum or reserved raw lane produces zero-extended -1, exceeding every cutoff.
fn intel_fraction(raw: i64, maximum: i64) -> Result<i64, &'static str> {
    if maximum == 0 || matches!(raw, i64::MAX | i64::MIN) || raw == i64::MIN + 1 {
        Ok(i64::from(u32::MAX))
    } else {
        raw.wrapping_mul(100_000)
            .checked_div(maximum)
            .ok_or("visibility intel division overflow")
    }
}

/// The air-base icon intel gates with the
/// loaded defines and the simulation's DLC feature `0x1C` (mask bit 5).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AirDefines {
    maximum: i64,
    icon_threshold: i64,
    detail: [i64; 4],
    fuzzy_range: i64,
    enabled: bool,
}

impl AirDefines {
    pub fn read(frame: &DecodedFrame, defines: &Defines) -> Result<Self, &'static str> {
        let [features] = complete::<tables::SimulationFeatures>(frame, "SimulationFeatures")?
        else {
            return Err("SimulationFeatures singleton");
        };
        Ok(Self {
            maximum: defines.intel_maximums[3],
            icon_threshold: defines.air_icon_threshold,
            detail: defines.air_detail_thresholds,
            fuzzy_range: defines.air_plane_fuzzy_range,
            enabled: features.dlc_mask & 0x20 != 0,
        })
    }
    fn fraction(&self, raw: Option<i64>, shared: bool, fog: bool) -> Option<i64> {
        if shared || !fog {
            Some(100_000)
        } else {
            intel_fraction(raw?, self.maximum).ok()
        }
    }
    /// Detail level zero through three. Search thresholds from the highest index downward; authored
    /// values need not be sorted.
    pub fn level(&self, raw: Option<i64>, shared: bool, fog: bool) -> Option<u8> {
        if !self.enabled {
            return Some(if shared || !fog { 3 } else { 0 });
        }
        let fraction = self.fraction(raw, shared, fog)?;
        Some(
            self.detail
                .iter()
                .rposition(|&v| v <= fraction)
                .unwrap_or(0) as u8,
        )
    }
    /// Whether the detail level is above zero.
    #[cfg(test)]
    pub fn port_detail(&self, raw: Option<i64>, shared: bool, fog: bool) -> Option<bool> {
        self.level(raw, shared, fog).map(|level| level > 0)
    }
    /// Use detail thresholds one and two with an independent fuzzy spread. The caller's seed combines
    /// the wrapping clock seed with its country or base numeric ID; the count wrapper adds its fixed
    /// jitter salt. This query is not DLC-gated.
    pub(crate) fn count_range(
        &self,
        count: i32,
        raw: Option<i64>,
        shared: bool,
        fog: bool,
        seed: u32,
        hour: i32,
    ) -> Option<([i32; 2], bool)> {
        let fraction = self.fraction(raw, shared, fog)?;
        let known = fraction >= self.detail[1];
        Some((
            super::province_tooltips::units::count_range_seeded(
                count,
                fraction,
                seed.wrapping_add(0x5E2),
                hour,
                [self.detail[1], self.detail[2], self.fuzzy_range],
            ),
            known,
        ))
    }
    /// The result is false without the simulation feature.
    pub fn show_ports(&self, raw: Option<i64>, shared: bool, fog: bool) -> Option<bool> {
        if !self.enabled {
            return Some(false);
        }
        self.fraction(raw, shared, fog)
            .map(|v| self.icon_threshold <= v)
    }
}

/// Naval-base port detail level zero through four, from the viewer's navy intel lane and simulation
/// feature 0x1C.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct NavalDefines {
    maximum: i64,
    detail: [i64; 5],
    enabled: bool,
}

impl NavalDefines {
    pub fn read(frame: &DecodedFrame, defines: &Defines) -> Result<Self, &'static str> {
        let [features] = complete::<tables::SimulationFeatures>(frame, "SimulationFeatures")?
        else {
            return Err("SimulationFeatures singleton");
        };
        Ok(Self {
            maximum: defines.intel_maximums[2],
            detail: defines.naval_detail_thresholds,
            enabled: features.dlc_mask & 0x20 != 0,
        })
    }

    /// Without the feature: 4 when the viewer shares vision with the owner or fog is off,
    /// else 0. With it: the highest threshold index at or below the intel fraction, searched
    /// from the top; authored values need not be sorted.
    pub fn level(&self, raw: Option<i64>, shared: bool, fog: bool) -> Option<u8> {
        if !self.enabled {
            return Some(if shared || !fog { 4 } else { 0 });
        }
        let fraction = if shared || !fog {
            100_000
        } else {
            intel_fraction(raw?, self.maximum).ok()?
        };
        Some(
            self.detail
                .iter()
                .rposition(|&threshold| threshold <= fraction)
                .unwrap_or(0) as u8,
        )
    }
}

/// Directed military access and docking rights from `RelationLinks`, in canonical `(holder, target)`
/// order.
#[derive(Clone, Debug, Default)]
pub struct AccessLinks {
    military: BTreeSet<(u32, u32)>,
    docking: BTreeSet<(u32, u32)>,
}

impl AccessLinks {
    pub fn read(frame: &DecodedFrame, links: &BTreeMap<u32, u32>) -> Result<Self, &'static str> {
        let mut out = Self::default();
        for row in complete::<tables::RelationLinks>(frame, "RelationLinks")? {
            let holder = *links.get(&row.country).ok_or("RelationLinks country")?;
            if row.links & RelationLinkRow::MILITARY_ACCESS != 0 {
                out.military.insert((holder, row.target_index));
            }
            if row.links & RelationLinkRow::DOCKING_RIGHTS != 0 {
                out.docking.insert((holder, row.target_index));
            }
        }
        Ok(out)
    }

    pub fn military(&self, holder: u32, target: u32) -> bool {
        self.military.contains(&(holder, target))
    }

    pub fn docking(&self, holder: u32, target: u32) -> bool {
        self.docking.contains(&(holder, target))
    }
}

impl Relations {
    /// A live war vetoes visibility. Otherwise admit military access or docking rights from the viewer
    /// toward the owner, faction membership, or overlord/subject relations.
    pub fn known_port(&self, access: &AccessLinks, viewer: u32, owner: u32) -> bool {
        viewer == owner
            || (!self.wars.contains(&(viewer, owner))
                && (access.military(viewer, owner)
                    || access.docking(viewer, owner)
                    || self.families.family(viewer, owner)))
    }

    /// Province access excludes lakes. It admits sea provinces; unowned states; the viewer's own states
    /// or controlled provinces; war, military access or docking rights toward the controller; and
    /// shared faction or direct subject relations with the controller.
    pub fn province_access(
        &self,
        access: &AccessLinks,
        viewer: u32,
        province: ProvinceAccess,
    ) -> bool {
        let ProvinceAccess {
            lake,
            sea,
            state_owner,
            controller,
        } = province;
        if lake {
            return false;
        }
        if state_owner.is_none_or(|owner| owner == viewer) || controller == Some(viewer) {
            return true;
        }
        let Some(controller) = controller else {
            return sea;
        };
        self.at_war(viewer, controller)
            || access.military(viewer, controller)
            || access.docking(viewer, controller)
            || sea
            || self.same_faction(viewer, controller)
            || self.families.subjects.contains(&(viewer, controller))
    }
}

/// Province access inputs: definition flags, state owner (`None` for no owner), and controller
/// (`None` for none).
#[derive(Clone, Copy, Debug, Default)]
pub struct ProvinceAccess {
    pub lake: bool,
    pub sea: bool,
    pub state_owner: Option<u32>,
    pub controller: Option<u32>,
}

fn network_term(cutoff: i32, modifier: i64) -> i32 {
    (i64::from(cutoff.wrapping_add(255))
        .wrapping_mul(100_000)
        .wrapping_mul(modifier)
        / 10_000_000_000) as i32
}

/// Apply the unit-intel modifier to the base byte.
pub fn unit_byte(base: u16, modifier: i64) -> u16 {
    let value = i64::from(base)
        .wrapping_mul(modifier.wrapping_add(100_000))
        .wrapping_mul(100_000)
        / 10_000_000_000;
    (value as i32).clamp(0, 255) as u16
}

/// One owner's `CountryIntel` rows. Decoded rows are in key order
/// (owner reference, then target slot), so a binary search finds the slice.
pub(crate) fn owner_rows(rows: &[CountryIntelRow], owner: u32) -> &[CountryIntelRow] {
    let start = rows.partition_point(|row| row.owner_country < owner);
    let end = rows.partition_point(|row| row.owner_country <= owner);
    &rows[start..end]
}

/// Raw country references per canonical slot, for reading keyed rows.
pub(crate) fn raw_references(links: &BTreeMap<u32, u32>) -> BTreeMap<u32, Vec<u32>> {
    let mut out: BTreeMap<u32, Vec<u32>> = BTreeMap::new();
    for (&raw, &slot) in links {
        out.entry(slot).or_default().push(raw);
    }
    out
}

/// Army lanes read on demand from the key-sorted table, never copied.
struct Intel<'a> {
    rows: Result<&'a [CountryIntelRow], &'static str>,
    raws: BTreeMap<u32, Vec<u32>>,
}

impl Intel<'_> {
    fn army(&self, owner: u32, target: u32) -> Result<i64, &'static str> {
        let rows = self.rows?;
        Ok(self
            .raws
            .get(&owner)
            .into_iter()
            .flatten()
            .find_map(|&raw| {
                let rows = owner_rows(rows, raw);
                rows.binary_search_by_key(&target, |row| row.target_index)
                    .ok()
                    .map(|index| rows[index].army)
            })
            .unwrap_or(0))
    }

    /// Actual receiver0's native getter, not a positive alias's keyed rows.
    /// The signed raw gate precedes its daily sample and vector demand.
    fn zero_army(
        &self,
        frame: &DecodedFrame,
        raw_target: u32,
        target: u32,
    ) -> Result<i64, &'static str> {
        if raw_target as i32 <= 0 {
            return Ok(0);
        }
        if target > i32::MAX as u32 {
            return Err("CountryIntel owner0 target outside native index range");
        }
        let reserved = reserved(frame)?;
        let sample = match &reserved.country_intel_sample {
            Recorded::NotRecorded => {
                return Err("ReservedCountryNaming.country_intel_sample not recorded");
            }
            Recorded::Value(None) => {
                return Err("ReservedCountryNaming.country_intel_sample capture refused");
            }
            Recorded::Value(Some(sample)) => sample,
        };
        if sample.sample_hour > frame.hour {
            return Err("ReservedCountryNaming.country_intel_sample is later than frame");
        }
        if sample.vector_count > i32::MAX as u32 {
            return Err("ReservedCountryNaming.country_intel_sample count outside native range");
        }
        let rows = owner_rows(self.rows?, 0);
        if rows
            .iter()
            .any(|row| row.target_index >= sample.vector_count)
            || rows
                .windows(2)
                .any(|pair| pair[0].target_index >= pair[1].target_index)
        {
            return Err("CountryIntel owner0 keys outside admitted vector or duplicated");
        }
        if sample.vector_count > 0
            && rows.last().map(|row| row.target_index) != Some(sample.vector_count - 1)
        {
            return Err("CountryIntel owner0 final sentinel missing");
        }
        if target >= sample.vector_count {
            return Ok(0);
        }
        Ok(rows
            .binary_search_by_key(&target, |row| row.target_index)
            .ok()
            .map_or(0, |index| rows[index].army))
    }
}

fn reserved(frame: &DecodedFrame) -> Result<&ReservedCountryNamingRow, &'static str> {
    let [row] = complete::<tables::ReservedCountryNaming>(frame, "ReservedCountryNaming")? else {
        return Err("ReservedCountryNaming singleton");
    };
    Ok(row)
}

/// Only the native MapIntel boundary admits literal0. General link readers
/// remain strict, and an unrecorded positive alias never becomes Country0.
fn map_intel_slot(links: &BTreeMap<u32, u32>, raw: u32) -> Result<u32, &'static str> {
    if raw == 0 {
        Ok(0)
    } else {
        link(links, raw, "visibility country link")
    }
}

fn map_intel_equal(links: &BTreeMap<u32, u32>, a: u32, b: u32) -> Result<bool, &'static str> {
    Ok(a == b || (a != 0 && b != 0 && map_intel_slot(links, a)? == map_intel_slot(links, b)?))
}

fn active_war(war: [u8; 2]) -> Result<bool, &'static str> {
    let [present, retired] = war;
    if present > 1 || (present == 0 && retired != 0) {
        return Err("visibility war has invalid native presence");
    }
    Ok(present == 1 && retired == 0)
}

/// Complete same-hour membership is demanded only after the war veto and a
/// nonnull source faction. Missing target membership then means native null.
fn map_intel_faction(
    frame: &DecodedFrame,
    links: &BTreeMap<u32, u32>,
    target: u32,
) -> Result<Option<u64>, &'static str> {
    let mut faction = None;
    for row in complete::<tables::FactionMembers>(frame, "FactionMembers")? {
        let country = map_intel_slot(links, row.country)?;
        if row.faction == 0 {
            return Err("FactionMembers has null faction identity");
        }
        if country == target {
            if faction.is_some_and(|previous| previous != row.faction) {
                return Err("FactionMembers has inconsistent faction identity");
            }
            faction = Some(row.faction);
        }
    }
    Ok(faction)
}

fn reserved_faction(row: &ReservedCountryNamingRow) -> Result<u64, &'static str> {
    match row.faction {
        Recorded::NotRecorded => Err("ReservedCountryNaming.faction not recorded"),
        Recorded::Value(None) => Err("ReservedCountryNaming.faction capture refused"),
        Recorded::Value(Some(faction)) => Ok(faction),
    }
}

/// Test subject parties in order against the reached cell's raw target. Null or wrong-type
/// subjects decide before requiring that target. A known second match can preserve truth after an
/// unavailable first party.
fn map_intel_subject(
    subject: &ZeroSubjectInputs,
    links: &BTreeMap<u32, u32>,
    target: Result<u32, &'static str>,
) -> Result<bool, &'static str> {
    let parties = match (subject.relation_type, subject.parties) {
        (None, None) => return Ok(false),
        (Some(kind), None) if kind != engine::csubject_relation::TYPE_VALUE => return Ok(false),
        (Some(kind), Some(parties)) if kind == engine::csubject_relation::TYPE_VALUE => parties,
        _ => return Err("visibility subject has inconsistent native branches"),
    };
    let matches = |party: Option<u32>| {
        map_intel_equal(
            links,
            party.ok_or("visibility subject party capture refused")?,
            target?,
        )
    };
    let first = matches(parties[0]);
    if first == Ok(true) {
        return Ok(true);
    }
    let second = matches(parties[1]);
    if second == Ok(true) {
        return Ok(true);
    }
    first?;
    second
}

/// Physical receiver0, : no symmetric relation-table inference.
fn reverse_zero_shared(
    frame: &DecodedFrame,
    links: &BTreeMap<u32, u32>,
    target: u32,
) -> Result<bool, &'static str> {
    let reserved = reserved(frame)?;
    let packet = match &reserved.reverse_visibility {
        Recorded::NotRecorded => {
            return Err("ReservedCountryNaming.reverse_visibility not recorded");
        }
        Recorded::Value(None) => {
            return Err("ReservedCountryNaming.reverse_visibility capture refused");
        }
        Recorded::Value(Some(packet)) => packet,
    };
    if !packet.sample_hour.fresh_at(frame.hour) {
        return Err("ReservedCountryNaming.reverse_visibility sample hour mismatch");
    }
    let cell = packet
        .cells
        .get(target as usize)
        .and_then(Option::as_ref)
        .ok_or("ReservedCountryNaming.reverse_visibility cell capture refused")?;
    if active_war(
        cell.war
            .ok_or("ReservedCountryNaming.reverse_visibility war capture refused")?,
    )? {
        return Ok(false);
    }
    let faction = reserved_faction(reserved)?;
    if faction != 0 && (target == 0 || map_intel_faction(frame, links, target)? == Some(faction)) {
        return Ok(true);
    }
    let cell_target = cell
        .target
        .ok_or("ReservedCountryNaming.reverse_visibility target capture refused");
    let subject = cell
        .subject
        .as_ref()
        .ok_or("ReservedCountryNaming.reverse_visibility subject capture refused")?;
    map_intel_subject(subject, links, cell_target)
}

/// Prefer the current positive receiver's directed status0 packet. Only older
/// field absence keeps the established exact-zero publication guard; a fresh
/// refused packet never falls back to the independent legacy observations.
fn forward_zero_shared(
    frame: &DecodedFrame,
    links: &BTreeMap<u32, u32>,
    owner: u32,
) -> Result<bool, &'static str> {
    let mut rows = complete::<tables::CountryCounterInputs>(frame, "CountryCounterInputs")?
        .iter()
        .filter(|row| links.get(&row.country) == Some(&owner));
    let row = rows
        .next()
        .ok_or("CountryCounterInputs visibility owner missing")?;
    if rows.next().is_some() {
        return Err("CountryCounterInputs visibility owner duplicated");
    }
    if !matches!(row.forward_visibility, Recorded::NotRecorded) {
        return forward_visibility_shared(frame, links, owner, &row.forward_visibility, false);
    }
    let war = match row.zero_war {
        Recorded::NotRecorded => return Err("CountryCounterInputs.zero_war not recorded"),
        Recorded::Value(None) => return Err("CountryCounterInputs.zero_war capture refused"),
        Recorded::Value(Some(war)) => war,
    };
    if active_war(war)? {
        return Ok(false);
    }
    if let Some(faction) = map_intel_faction(frame, links, owner)?
        && reserved_faction(reserved(frame)?)? == faction
    {
        return Ok(true);
    }
    let subject = match &row.zero_subject {
        Recorded::NotRecorded => return Err("CountryCounterInputs.zero_subject not recorded"),
        Recorded::Value(None) => return Err("CountryCounterInputs.zero_subject capture refused"),
        Recorded::Value(Some(subject)) => subject,
    };
    map_intel_subject(subject, links, Ok(0))
}

/// Counters test raw self-reference before the directed shared-visibility veto, which has no signed
/// receiver gate. Zero endpoints retain recorded per-cell subject operands.
pub(super) fn counter_shared(
    frame: &DecodedFrame,
    links: &BTreeMap<u32, u32>,
    owner: &super::directed_wars::Owner<'_>,
    raw_target: u32,
    families: Result<&Families, &str>,
) -> Result<bool, String> {
    if owner.same_country(raw_target)? {
        return Ok(true);
    }
    let target = map_intel_slot(links, raw_target).map_err(str::to_owned)?;
    if owner.pointer_active_slot(target)? {
        return Ok(false);
    }
    if owner.slot() == 0 {
        return reverse_zero_shared(frame, links, target).map_err(str::to_owned);
    }
    if target == 0 {
        return forward_zero_shared(frame, links, owner.slot()).map_err(str::to_owned);
    }
    let families = families.map_err(str::to_owned)?;
    Ok(families.subjects.contains(&(owner.slot(), target))
        || families.factions.get(&owner.slot()).is_some_and(|faction| {
            *faction != 0 && families.factions.get(&target) == Some(faction)
        }))
}

/// A positive receiver's visibility matrix from this complete hourly table. The cell's raw target
/// includes positive aliases of Country0. War and military admission precede any demand for the
/// target or subject.
fn forward_visibility_shared(
    frame: &DecodedFrame,
    links: &BTreeMap<u32, u32>,
    owner: u32,
    inputs: &Recorded<Option<replay::rows::ReverseZeroVisibilityInputs>>,
    military: bool,
) -> Result<bool, &'static str> {
    let cell = match inputs {
        Recorded::NotRecorded => {
            return Err("CountryCounterInputs.forward_visibility not recorded");
        }
        Recorded::Value(None) => {
            return Err("CountryCounterInputs.forward_visibility capture refused");
        }
        Recorded::Value(Some(cell)) => cell,
    };
    if active_war(
        cell.war
            .ok_or("CountryCounterInputs.forward_visibility war capture refused")?,
    )? {
        return Ok(false);
    }
    let access = if military {
        match cell.military_access {
            Some(1) => Ok(true),
            Some(0) => Ok(false),
            None => Err("CountryCounterInputs.forward_visibility military access capture refused"),
            Some(_) => {
                return Err("CountryCounterInputs.forward_visibility military access invalid");
            }
        }
    } else {
        Ok(false)
    };
    event_or(access, || {
        let faction = (|| {
            let Some(faction) = map_intel_faction(frame, links, owner)? else {
                return Ok(false);
            };
            Ok(reserved_faction(reserved(frame)?)? == faction)
        })();
        event_or(faction, || {
            let target = cell
                .target
                .ok_or("CountryCounterInputs.forward_visibility target capture refused");
            let subject = cell
                .subject
                .as_ref()
                .ok_or("CountryCounterInputs.forward_visibility subject capture refused")?;
            map_intel_subject(subject, links, target)
        })
    })
}

/// Map-text visibility admits paradrops through directed military access and disables docking.
/// Receiver comparison uses the resolved object's own ID. Actual Country0 therefore requires its
/// first matrix for a positive target alias of Country0, even when incoming aliases are equal.
pub(crate) fn event_shared(
    frame: &DecodedFrame,
    links: &BTreeMap<u32, u32>,
    receiver: u32,
    raw_target: u32,
    military: bool,
) -> Result<bool, &'static str> {
    let owner = map_intel_slot(links, receiver)?;
    let own_id = if owner == 0 {
        0
    } else {
        let mut rows = complete::<tables::Countries>(frame, "Countries")?
            .iter()
            .filter(|row| row.canonical_slot == owner);
        let row = rows
            .next()
            .ok_or("map text receiver Countries row missing")?;
        if rows.next().is_some() {
            return Err("map text receiver Countries row duplicated");
        }
        row.id
    };
    if own_id == raw_target {
        return Ok(true);
    }
    let target = map_intel_slot(links, raw_target)?;
    if own_id != 0 && raw_target != 0 && owner == target {
        return Ok(true);
    }
    if owner == 0 {
        let row = reserved(frame)?;
        let packet = match &row.reverse_visibility {
            Recorded::NotRecorded => {
                return Err("ReservedCountryNaming.reverse_visibility not recorded");
            }
            Recorded::Value(None) => {
                return Err("ReservedCountryNaming.reverse_visibility capture refused");
            }
            Recorded::Value(Some(packet)) => packet,
        };
        if !packet.sample_hour.fresh_at(frame.hour) {
            return Err("ReservedCountryNaming.reverse_visibility sample hour mismatch");
        }
        let cell = packet
            .cells
            .get(target as usize)
            .and_then(Option::as_ref)
            .ok_or("ReservedCountryNaming.reverse_visibility cell capture refused")?;
        if active_war(
            cell.war
                .ok_or("ReservedCountryNaming.reverse_visibility war capture refused")?,
        )? {
            return Ok(false);
        }
        let access = if military {
            match cell.military_access {
                Some(1) => Ok(true),
                Some(0) => Ok(false),
                Some(_) => {
                    return Err("ReservedCountryNaming.reverse_visibility military access invalid");
                }
                None => {
                    Err("ReservedCountryNaming.reverse_visibility military access capture refused")
                }
            }
        } else {
            Ok(false)
        };
        return event_or(access, || {
            let faction = (|| {
                let faction = reserved_faction(row)?;
                Ok(faction != 0
                    && (target == 0 || map_intel_faction(frame, links, target)? == Some(faction)))
            })();
            event_or(faction, || {
                let target = cell
                    .target
                    .ok_or("ReservedCountryNaming.reverse_visibility target capture refused");
                let subject = cell
                    .subject
                    .as_ref()
                    .ok_or("ReservedCountryNaming.reverse_visibility subject capture refused")?;
                map_intel_subject(subject, links, target)
            })
        });
    }
    if target == 0 {
        if !military {
            return forward_zero_shared(frame, links, owner);
        }
        let mut rows = complete::<tables::CountryCounterInputs>(frame, "CountryCounterInputs")?
            .iter()
            .filter(|row| links.get(&row.country) == Some(&owner));
        let row = rows
            .next()
            .ok_or("CountryCounterInputs visibility owner missing")?;
        if rows.next().is_some() {
            return Err("CountryCounterInputs visibility owner duplicated");
        }
        return forward_visibility_shared(frame, links, owner, &row.forward_visibility, true);
    }
    for row in complete::<tables::WarRelations>(frame, "WarRelations")? {
        let a = map_intel_slot(links, row.first_country)?;
        let b = map_intel_slot(links, row.second_country)?;
        if (a, b) == (owner, target) || (a, b) == (target, owner) {
            return Ok(false);
        }
    }
    let access = if military {
        (|| {
            for row in complete::<tables::RelationLinks>(frame, "RelationLinks")? {
                if row.target_index == target
                    && map_intel_slot(links, row.country)? == owner
                    && row.links & RelationLinkRow::MILITARY_ACCESS != 0
                {
                    return Ok(true);
                }
            }
            Ok(false)
        })()
    } else {
        Ok(false)
    };
    event_or(access, || {
        let faction = (|| {
            let Some(faction) = map_intel_faction(frame, links, owner)? else {
                return Ok(false);
            };
            Ok(map_intel_faction(frame, links, target)? == Some(faction))
        })();
        event_or(faction, || {
            for row in complete::<tables::Subjects>(frame, "Subjects")? {
                let a = map_intel_slot(links, row.overlord)?;
                let b = map_intel_slot(links, row.subject)?;
                if (a, b) == (owner, target) || (a, b) == (target, owner) {
                    return Ok(true);
                }
            }
            Ok(false)
        })
    })
}

/// Ordered native OR after a known clear war veto. A skipped/refused earlier
/// operand cannot erase an independently proven positive later branch.
fn event_or(
    first: Result<bool, &'static str>,
    second: impl FnOnce() -> Result<bool, &'static str>,
) -> Result<bool, &'static str> {
    if first == Ok(true) {
        return Ok(true);
    }
    let second = second();
    if second == Ok(true) {
        return Ok(true);
    }
    first?;
    second
}

struct Rebuild<'a> {
    frame: &'a DecodedFrame,
    links: &'a BTreeMap<u32, u32>,
    topology: &'a Topology,
    controllers: BTreeMap<u32, u32>,
    intel: Intel<'a>,
    relations: &'a Relations,
    maximum: i64,
    cutoffs: &'a [i64],
    bytes: BTreeMap<u32, u8>,
}

impl Rebuild<'_> {
    /// Army intel cutoff: choose the value of the first pair whose threshold exceeds the intel
    /// fraction, otherwise the last value.
    fn cutoff(&self, raw_a: u32, raw_b: u32) -> Result<i32, &'static str> {
        if self.cutoffs.len() < 2 {
            return Ok(0);
        }
        let a = map_intel_slot(self.links, raw_a)?;
        let b = map_intel_slot(self.links, raw_b)?;
        // The resolved receiver's own ID is zero, even for a positive source
        // alias. A positive target alias0 still queries matrix[0].
        let shared = if a == 0 {
            raw_b == 0 || reverse_zero_shared(self.frame, self.links, b)?
        } else if b == 0 {
            forward_zero_shared(self.frame, self.links, a)?
        } else {
            self.relations.shared(a, b)
        };
        let fraction = if shared {
            100_000
        } else {
            let army = if a == 0 {
                self.intel.zero_army(self.frame, raw_b, b)?
            } else {
                self.intel.army(a, b)?
            };
            intel_fraction(army, self.maximum)?
        };
        let mut result = 0;
        // An odd trailing value is dropped, as the native loader warns.
        for pair in self.cutoffs.chunks_exact(2) {
            result = rounded(pair[1]);
            if pair[0] > fraction {
                break;
            }
        }
        Ok(result)
    }
    fn neighbors(&mut self, province: u32, base: i32) -> Result<(), &'static str> {
        let source = *self
            .controllers
            .get(&province)
            .ok_or("visibility source province")?;
        let neighbors = self
            .topology
            .neighbors
            .get(&province)
            .ok_or("visibility province topology")?;
        for &next in neighbors {
            if !self.topology.land.contains(&next) {
                continue;
            }
            let other = *self
                .controllers
                .get(&next)
                .ok_or("visibility neighbour controller")?;
            if map_intel_equal(self.links, other, source)? {
                continue;
            }
            let value = base.wrapping_add(self.cutoff(source, other)?).clamp(0, 255) as u8;
            let byte = self
                .bytes
                .get_mut(&next)
                .ok_or("visibility neighbour province")?;
            *byte = (*byte).max(value);
        }
        Ok(())
    }
    fn reveal(&mut self, province: u32, base: i32) -> Result<(), &'static str> {
        *self
            .bytes
            .get_mut(&province)
            .ok_or("visibility revealed province")? = 255;
        self.neighbors(province, base)
    }
    fn add(&mut self, province: u32, term: i32) -> Result<(), &'static str> {
        let land = self.topology.land.contains(&province);
        let byte = self
            .bytes
            .get_mut(&province)
            .ok_or("visibility additive province")?;
        *byte = if land { saturate(*byte, term) } else { 0 };
        Ok(())
    }
}

/// Rows grouped by a key, in their table order.
fn group<'a, T, K: Ord>(
    rows: impl IntoIterator<Item = &'a T>,
    mut key: impl FnMut(&T) -> Result<Option<K>, &'static str>,
) -> Result<BTreeMap<K, Vec<&'a T>>, &'static str> {
    let mut out: BTreeMap<K, Vec<&'a T>> = BTreeMap::new();
    for row in rows {
        if let Some(key) = key(row)? {
            out.entry(key).or_default().push(row);
        }
    }
    Ok(out)
}

/// The viewing country's province bytes. Every input is indexed by owner
/// once, so the country loop reads only what each country contributes. Any
/// missing or unresolvable input fails with its name.
pub fn derive(
    frame: &DecodedFrame,
    links: &BTreeMap<u32, u32>,
    ours: u32,
    content: &FogContent,
    relations: &Relations,
) -> Result<BTreeMap<u32, u8>, &'static str> {
    macro_rules! input {
        ($table:ident) => {
            complete::<tables::$table>(frame, stringify!($table))?
        };
    }
    let (topology, defines) = (&content.topology, &content.defines);
    let countries = input!(Countries);
    let provinces = input!(Provinces);
    let divisions = input!(Divisions);
    let tasks = input!(TaskForces);
    let networks: BTreeMap<_, _> = input!(StateVisibility)
        .iter()
        .map(|r| (r.state, r))
        .collect();
    let canonical = |raw: u32| -> Result<u32, &'static str> {
        if raw == 0 {
            Ok(0)
        } else {
            links.get(&raw).copied().ok_or("visibility country link")
        }
    };
    let owned = |raw: u32| canonical(raw).map(|slot| (slot != 0).then_some(slot));
    if provinces
        .iter()
        .any(|p| !topology.neighbors.contains_key(&p.id))
    {
        return Err("visibility province topology");
    }
    let controlled = group(provinces, |p| owned(p.controller))?;
    let by_state = group(provinces, |p| Ok((p.state != 0).then_some(p.state)))?;
    let occupied = group(input!(States), |s| {
        Ok((canonical(s.owner)? != canonical(s.controller)?)
            .then_some(owned(s.owner)?)
            .flatten())
    })?;
    let rosters = group(divisions, |d| owned(d.roster_owner))?;
    let fleets = group(input!(Fleets), |f| owned(f.owner))?;
    let fleet_tasks = group(tasks, |t| Ok((t.province != 0).then_some(t.fleet)))?;
    let theatres = input!(LandTheatres);
    let own_volunteer_theatres = group(theatres, |t| {
        Ok((t.volunteers_theatre > 0)
            .then_some(owned(t.owner)?)
            .flatten())
    })?;
    let volunteers_here = group(theatres, |t| {
        Ok(
            (t.volunteers_theatre > 0 && canonical(t.volunteers_theatre as u32)? == ours)
                .then_some(owned(t.owner)?)
                .flatten(),
        )
    })?;
    let areas = group(input!(TheatreAreas), |a| Ok(Some(a.theatre)))?;
    let stationed = group(input!(StationedUnits), |u| Ok(Some(u.theatre)))?;
    let hosts = group(input!(VolunteerHosts), |h| owned(h.country))?;
    let radar = group(input!(RadarCoverage), |r| owned(r.country))?;
    let mut rebuild = Rebuild {
        frame,
        links,
        topology,
        controllers: provinces.iter().map(|p| (p.id, p.controller)).collect(),
        intel: Intel {
            rows: complete::<tables::CountryIntel>(frame, "CountryIntel"),
            raws: raw_references(links),
        },
        relations,
        maximum: defines.intel_maximums[1],
        cutoffs: &defines.map_cutoffs,
        bytes: provinces.iter().map(|p| (p.id, 0)).collect(),
    };
    // Divisions and task forces are key-sorted by id.
    let unit = |id: u64| {
        divisions
            .binary_search_by_key(&id, |d| d.id)
            .map(|i| (divisions[i].logical_owner, divisions[i].province))
            .or_else(|_| {
                tasks
                    .binary_search_by_key(&id, |t| t.id)
                    .map(|i| (tasks[i].logical_owner, tasks[i].province))
            })
            .map_err(|_| "StationedUnits unit")
    };
    let mut order: Vec<_> = countries.iter().map(|c| (c.canonical_slot, c.id)).collect();
    order.sort_unstable();
    let Some(&viewer) = order.iter().find(|&&(slot, _)| slot == ours) else {
        return Err("visibility viewing country");
    };
    order.retain(|&(slot, _)| slot != ours);
    order.insert(0, viewer);
    for (country, raw_country) in order {
        let own = country == ours;
        if !own && !relations.allied_for_fog(country, ours) {
            // Volunteers of another country stationed in our theatre.
            let theatre = volunteers_here
                .get(&country)
                .and_then(|theatres| theatres.iter().min_by_key(|t| t.ordinal));
            if let Some(theatre) = theatre {
                for unit_row in stationed.get(&theatre.id).into_iter().flatten() {
                    let (owner, province) = unit(unit_row.unit)?;
                    if canonical(owner)? == ours && province != 0 {
                        rebuild.reveal(province, 250)?;
                    }
                }
            }
            continue;
        }
        for province in controlled.get(&country).into_iter().flatten() {
            rebuild.reveal(province.id, 150)?;
        }
        if own {
            for theatre in own_volunteer_theatres.get(&country).into_iter().flatten() {
                let mut ordered = areas.get(&theatre.id).cloned().unwrap_or_default();
                ordered.sort_by_key(|a| a.ordinal);
                for area in ordered {
                    // The native iterator stops at the first empty area.
                    if area.provinces.is_empty() {
                        break;
                    }
                    for &p in &area.provinces {
                        rebuild.reveal(p, 150)?;
                    }
                }
            }
            for host in hosts.get(&country).into_iter().flatten() {
                for province in controlled.get(&canonical(host.host)?).into_iter().flatten() {
                    rebuild.reveal(province.id, 150)?;
                }
            }
        }
        for state in occupied.get(&country).into_iter().flatten() {
            // Absent from the sparse table: a zero modifier adds nothing.
            let Some(network) = networks.get(&state.id) else {
                continue;
            };
            let term = network_term(
                rebuild.cutoff(raw_country, state.controller)?,
                network.network_modifier,
            );
            for province in by_state.get(&state.id).into_iter().flatten() {
                rebuild.add(province.id, term)?;
            }
        }
        for division in rosters.get(&country).into_iter().flatten() {
            if division.reveal_skip == 0 && division.province != 0 {
                rebuild.reveal(division.province, 250)?;
            }
        }
        for fleet in fleets.get(&country).into_iter().flatten() {
            for task in fleet_tasks.get(&fleet.id).into_iter().flatten() {
                rebuild.neighbors(task.province, 250)?;
            }
        }
        let mut groups = radar.get(&country).cloned().unwrap_or_default();
        groups.sort_by_key(|r| r.ordinal);
        for group in groups {
            for &p in &group.provinces {
                let controller = *rebuild
                    .controllers
                    .get(&p)
                    .ok_or("RadarCoverage province")?;
                if !map_intel_equal(links, controller, raw_country)? {
                    let term = defines
                        .radar_effect
                        .wrapping_add(rebuild.cutoff(raw_country, controller)?);
                    rebuild.add(p, term)?;
                }
            }
        }
    }
    Ok(rebuild.bytes)
}

/// Synthetic defines for viewer fixtures.
#[cfg(test)]
pub(crate) fn test_defines() -> Defines {
    Defines {
        intel_maximums: [10_000_000, 100_000, 10_000_000, 10_000_000],
        radar_effect: 25,
        map_cutoffs: Vec::new(),
        air_icon_threshold: 30_000,
        air_detail_thresholds: [0, 30_000, 70_000, 100_000],
        air_plane_fuzzy_range: 100_000,
        naval_detail_thresholds: [0, 30_000, 50_000, 70_000, 100_000],
    }
}

#[cfg(test)]
#[path = "visibility/reverse_zero_tests.rs"]
mod reverse_zero_tests;

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use replay::{
        frame::{Metrics, TableView},
        rows::{
            CountryRow, DivisionRow, FactionMemberRow, ProvinceRow, RadarCoverageRow,
            SimulationFeatureRow, StateRow, StateVisibilityRow, TableId, TableRows, WarRelationRow,
        },
    };

    /// Three countries (slots 1..=3, raw 101..=103), provinces 1..=5 with
    /// province 4 at sea; every input table complete and empty.
    pub(crate) fn fixture() -> (DecodedFrame, BTreeMap<u32, u32>, FogContent) {
        let mut tables = std::array::from_fn(|_| TableView::Unavailable {
            reason: "unused in fixture".into(),
        });
        macro_rules! rows {
            ($name:ident, $rows:expr) => {
                tables[TableId::$name.index()] = TableView::Complete(TableRows::$name($rows));
            };
        }
        rows!(
            Countries,
            (1..=3)
                .map(|c| CountryRow {
                    id: 100 + c,
                    canonical_slot: c,
                    tag: format!("T{c}").into_bytes(),
                    ..Default::default()
                })
                .collect()
        );
        rows!(
            Provinces,
            [(1, 101), (2, 102), (3, 103), (4, 0), (5, 103)]
                .into_iter()
                .map(|(id, controller)| ProvinceRow {
                    id,
                    controller,
                    ..Default::default()
                })
                .collect()
        );
        rows!(States, vec![]);
        rows!(Divisions, vec![]);
        rows!(StateVisibility, vec![]);
        rows!(Fleets, vec![]);
        rows!(TaskForces, vec![]);
        rows!(LandTheatres, vec![]);
        rows!(TheatreAreas, vec![]);
        rows!(StationedUnits, vec![]);
        rows!(VolunteerHosts, vec![]);
        rows!(RadarCoverage, vec![]);
        rows!(CountryIntel, vec![]);
        rows!(FactionMembers, vec![]);
        rows!(WarRelations, vec![]);
        rows!(Subjects, vec![]);
        rows!(RelationLinks, vec![]);
        rows!(UnitIntel, vec![]);
        rows!(CountryVisibility, vec![]);
        rows!(DivisionMovement, vec![]);
        rows!(
            SimulationFeatures,
            vec![SimulationFeatureRow {
                dlc_mask: 0x20,
                human_ai: 0,
                weather_suspend: replay::rows::Recorded::Value(None),
                ..Default::default()
            }]
        );
        (
            DecodedFrame {
                branch: 1,
                hour: 0,
                tables: tables.map(Into::into),
                metrics: Metrics::default(),
            },
            (1..=3).map(|i| (100 + i, i)).collect(),
            FogContent {
                topology: Topology {
                    land: [1, 2, 3, 5].into(),
                    sea: [4].into(),
                    neighbors: [
                        (1, vec![2]),
                        (2, vec![1]),
                        (3, vec![]),
                        (4, vec![3]),
                        (5, vec![]),
                    ]
                    .into(),
                },
                defines: test_defines(),
            },
        )
    }

    fn run(
        frame: &DecodedFrame,
        links: &BTreeMap<u32, u32>,
        ours: u32,
        content: &FogContent,
    ) -> Result<BTreeMap<u32, u8>, &'static str> {
        let relations = Relations::read(frame, links)?.with_attaches(frame, links)?;
        derive(frame, links, ours, content, &relations)
    }

    fn set<T: replay::rows::TableSchema>(frame: &mut DecodedFrame, rows: Vec<T::Row>) {
        *frame.tables[T::ID.index()] = TableView::Complete(T::wrap(rows));
    }

    #[test]
    fn native_signed_saturation_and_modifier_boundaries() {
        assert_eq!(saturate(250, 20), 255);
        assert_eq!(saturate(10, -20), 0);
        assert_eq!(saturate(250, i32::MAX), 0);
        assert_eq!(network_term(-50, 50_000), 102);
        assert_eq!(unit_byte(250, -50_000), 125);
        assert_eq!(unit_byte(150, 100_000), 255);
        assert_eq!(rounded(-150_000), -2);
    }

    #[test]
    fn each_viewer_rebuilds_its_own_bytes_from_the_same_hour() {
        engine::executable_tables::initialize_for_tests()
            .expect("BATTLE_PLAN_GAME_DIR must name the supported installation for runtime table tests");
        let (frame, links, content) = fixture();
        assert_eq!(
            run(&frame, &links, 1, &content).unwrap(),
            [(1, 255), (2, 150), (3, 0), (4, 0), (5, 0)].into()
        );
        assert_eq!(
            run(&frame, &links, 3, &content).unwrap(),
            [(1, 0), (2, 0), (3, 255), (4, 0), (5, 255)].into()
        );
    }

    #[test]
    fn an_attache_shares_province_fog_one_way_but_not_unit_intel() {
        engine::executable_tables::initialize_for_tests()
            .expect("BATTLE_PLAN_GAME_DIR must name the supported installation for runtime table tests");
        let (mut frame, links, content) = fixture();
        set::<tables::RelationLinks>(
            &mut frame,
            vec![RelationLinkRow {
                country: 103,
                target_index: 1,
                links: RelationLinkRow::ATTACHE,
                transport_subject: Default::default(),
            }],
        );
        assert_eq!(run(&frame, &links, 1, &content).unwrap()[&3], 255);
        assert_eq!(run(&frame, &links, 3, &content).unwrap()[&1], 0);
        let relations = Relations::read(&frame, &links)
            .unwrap()
            .with_attaches(&frame, &links)
            .unwrap();
        assert!(!relations.shared(3, 1));
    }

    #[test]
    fn a_war_vetoes_shared_intel_but_not_the_allied_rebuild() {
        engine::executable_tables::initialize_for_tests()
            .expect("BATTLE_PLAN_GAME_DIR must name the supported installation for runtime table tests");
        let (mut frame, links, content) = fixture();
        set::<tables::FactionMembers>(
            &mut frame,
            [101, 103]
                .into_iter()
                .enumerate()
                .map(|(ordinal, country)| FactionMemberRow {
                    country,
                    faction: 7,
                    ordinal: ordinal as u32,
                })
                .collect(),
        );
        set::<tables::WarRelations>(
            &mut frame,
            vec![WarRelationRow {
                first_country: 101,
                second_country: 103,
                ..Default::default()
            }],
        );
        let relations = Relations::read(&frame, &links).unwrap();
        assert!(relations.at_war(1, 3) && !relations.shared(1, 3));
        assert_eq!(run(&frame, &links, 1, &content).unwrap()[&3], 255);
    }

    #[test]
    fn families_need_no_war_census_but_relations_do() {
        engine::executable_tables::initialize_for_tests()
            .expect("BATTLE_PLAN_GAME_DIR must name the supported installation for runtime table tests");
        let (mut frame, links, _) = fixture();
        set::<tables::FactionMembers>(
            &mut frame,
            vec![
                FactionMemberRow {
                    country: 101,
                    faction: 7,
                    ordinal: 0,
                },
                FactionMemberRow {
                    country: 103,
                    faction: 7,
                    ordinal: 1,
                },
            ],
        );
        *frame.tables[TableId::WarRelations.index()] = TableView::Unavailable {
            reason: "not recorded".into(),
        };
        let families = Families::read(&frame, &links).unwrap();
        assert!(families.family(1, 3) && !families.family(1, 2));
        assert_eq!(Relations::read(&frame, &links).err(), Some("WarRelations"));
    }

    #[test]
    fn reserved_faction_members_preserve_positive_families_and_unknown_links() {
        engine::executable_tables::initialize_for_tests()
            .expect("BATTLE_PLAN_GAME_DIR must name the supported installation for runtime table tests");
        let (mut frame, links, _) = fixture();
        assert!(!links.contains_key(&0));
        set::<tables::FactionMembers>(
            &mut frame,
            [0, 101, 103]
                .into_iter()
                .enumerate()
                .map(|(ordinal, country)| FactionMemberRow {
                    country,
                    faction: 7,
                    ordinal: ordinal as u32,
                })
                .collect(),
        );
        let families = Families::read(&frame, &links).unwrap();
        assert!(families.same_faction(1, 3));
        assert!(families.same_faction(0, 1));
        assert!(!families.same_faction(1, 2));
        let relations = Relations::read(&frame, &links).unwrap();
        assert!(relations.shared(1, 3));

        set::<tables::FactionMembers>(
            &mut frame,
            vec![FactionMemberRow {
                country: 999,
                faction: 7,
                ordinal: 0,
            }],
        );
        assert_eq!(
            Families::read(&frame, &links).err(),
            Some("FactionMembers country")
        );
    }

    #[test]
    fn reveal_skip_radar_and_sea_targets_follow_the_native_order() {
        engine::executable_tables::initialize_for_tests()
            .expect("BATTLE_PLAN_GAME_DIR must name the supported installation for runtime table tests");
        let (mut frame, links, content) = fixture();
        let division = |reveal_skip| DivisionRow {
            id: 7,
            roster_owner: 101,
            logical_owner: 101,
            province: 4,
            reveal_skip,
            ..Default::default()
        };
        set::<tables::Divisions>(&mut frame, vec![division(0)]);
        set::<tables::RadarCoverage>(
            &mut frame,
            vec![RadarCoverageRow {
                country: 101,
                ordinal: 0,
                state: None,
                provinces: vec![3, 3, 5, 4],
            }],
        );
        let bytes = run(&frame, &links, 1, &content).unwrap();
        assert_eq!((bytes[&3], bytes[&5], bytes[&4]), (255, 25, 0));
        set::<tables::Divisions>(&mut frame, vec![division(2)]);
        assert_eq!(run(&frame, &links, 1, &content).unwrap()[&3], 50);
    }

    #[test]
    fn an_occupied_owned_state_adds_its_network_term_and_absence_adds_none() {
        engine::executable_tables::initialize_for_tests()
            .expect("BATTLE_PLAN_GAME_DIR must name the supported installation for runtime table tests");
        let (mut frame, links, content) = fixture();
        set::<tables::States>(
            &mut frame,
            vec![StateRow {
                id: 9,
                owner: 101,
                controller: 103,
                movement_infrastructure: 0,
                ..Default::default()
            }],
        );
        if let TableView::Complete(TableRows::Provinces(rows)) =
            &mut *frame.tables[TableId::Provinces.index()]
        {
            rows.iter_mut().find(|p| p.id == 3).unwrap().state = 9;
        }
        assert_eq!(run(&frame, &links, 1, &content).unwrap()[&3], 0);
        set::<tables::StateVisibility>(
            &mut frame,
            vec![StateVisibilityRow {
                state: 9,
                country: 101,
                network_modifier: 50_000,
            }],
        );
        assert_eq!(run(&frame, &links, 1, &content).unwrap()[&3], 127);
    }

    #[test]
    fn a_missing_input_is_named_instead_of_dropping_a_term() {
        engine::executable_tables::initialize_for_tests()
            .expect("BATTLE_PLAN_GAME_DIR must name the supported installation for runtime table tests");
        let (mut frame, links, mut content) = fixture();
        *frame.tables[TableId::RadarCoverage.index()] = TableView::Unavailable {
            reason: "read fault".into(),
        };
        assert_eq!(run(&frame, &links, 1, &content), Err("RadarCoverage"));
        let (frame, _, _) = fixture();
        content.topology.neighbors.remove(&5);
        assert_eq!(
            run(&frame, &links, 1, &content),
            Err("visibility province topology")
        );
    }

    #[test]
    fn cutoffs_keep_strict_thresholds_odd_tail_and_invalid_fraction() {
        engine::executable_tables::initialize_for_tests()
            .expect("BATTLE_PLAN_GAME_DIR must name the supported installation for runtime table tests");
        let (frame, links, content) = fixture();
        let relations = Relations::read(&frame, &links).unwrap();
        let cutoffs = [50_000, -100_000, 100_000, 200_000, 999];
        let rows = |army| {
            vec![CountryIntelRow {
                owner_country: 101,
                target_index: 2,
                army,
                ..Default::default()
            }]
        };
        let (half, below) = (rows(50_000), rows(49_999));
        let mut r = Rebuild {
            frame: &frame,
            links: &links,
            topology: &content.topology,
            controllers: BTreeMap::new(),
            intel: Intel {
                rows: Ok(&half),
                raws: raw_references(&links),
            },
            relations: &relations,
            maximum: 100_000,
            cutoffs: &cutoffs,
            bytes: BTreeMap::new(),
        };
        assert_eq!(r.cutoff(101, 102), Ok(2));
        r.intel.rows = Ok(&below);
        assert_eq!(r.cutoff(101, 102), Ok(-1));
        assert_eq!(intel_fraction(i64::MIN, 100_000), Ok(i64::from(u32::MAX)));
        r.maximum = 0;
        assert_eq!(r.cutoff(101, 102), Ok(2));
    }

    #[test]
    fn air_gates_use_authored_thresholds_and_the_simulation_feature() {
        let mut d = AirDefines {
            maximum: 20_000_000,
            icon_threshold: 75_000,
            detail: [0, 90_000, 5_000, 120_000],
            fuzzy_range: 100_000,
            enabled: true,
        };
        assert_eq!(d.level(Some(999_999), false, true), Some(0));
        assert_eq!(d.level(Some(1_000_000), false, true), Some(2));
        assert_eq!(d.port_detail(Some(1_000_000), false, true), Some(true));
        // Shared vision supplies full intel, which still takes the authored
        // threshold search. An unsorted threshold above full intel is not reached.
        assert_eq!(d.level(None, true, true), Some(2));
        assert_eq!(d.level(None, false, false), Some(2));
        assert_eq!(d.level(None, false, true), None);
        d.detail[3] = 100_000;
        assert_eq!(d.level(None, true, true), Some(3));
        assert_eq!(d.show_ports(Some(14_999_999), false, true), Some(false));
        assert_eq!(d.show_ports(Some(15_000_000), false, true), Some(true));
        assert_eq!(d.show_ports(None, false, true), None);
        d.enabled = false;
        assert_eq!(d.level(None, false, true), Some(0));
        assert_eq!(d.level(None, true, true), Some(3));
        assert_eq!(d.level(None, false, false), Some(3));
        assert_eq!(d.port_detail(None, false, true), Some(false));
        assert_eq!(d.port_detail(None, true, true), Some(true));
        assert_eq!(d.show_ports(None, true, false), Some(false));
    }

    #[test]
    fn air_count_estimates_keep_midpoint_thresholds_and_independent_spread() {
        let mut d = AirDefines {
            maximum: 100_000,
            icon_threshold: 75_000,
            detail: [0, 30_000, 70_000, 100_000],
            fuzzy_range: 100_000,
            enabled: true,
        };
        let hour = 60_827_914;
        assert_eq!(
            d.count_range(100, Some(70_000), false, true, 123, hour),
            Some(([100; 2], true))
        );
        let (low, known) = d
            .count_range(100, Some(30_000), false, true, 123, hour)
            .unwrap();
        assert!(known);
        assert!(low[0] < 100 && low[1] > 100);
        d.fuzzy_range = 0;
        assert_eq!(
            d.count_range(100, Some(30_000), false, true, 123, hour),
            Some(([100; 2], true))
        );
        // The independent count spread does not alter the four-tier law.
        assert_eq!(d.level(Some(30_000), false, true), Some(1));
        d.detail[2] = d.detail[1];
        d.fuzzy_range = 100_000;
        assert_eq!(
            d.count_range(100, Some(0), false, true, 123, hour),
            Some(([100; 2], false))
        );
    }

    #[test]
    fn air_count_estimates_keep_shared_fog_unknown_and_dlc_laws_distinct() {
        let mut d = AirDefines {
            maximum: 100_000,
            icon_threshold: 30_000,
            detail: [0, 30_000, 70_000, 100_000],
            fuzzy_range: 100_000,
            enabled: false,
        };
        let hour = 60_827_914;
        assert_eq!(d.count_range(100, None, false, true, 123, hour), None);
        assert_eq!(
            d.count_range(100, None, true, true, 123, hour),
            Some(([100; 2], true))
        );
        assert_eq!(
            d.count_range(100, None, false, false, 123, hour),
            Some(([100; 2], true))
        );
        let estimate = d.count_range(100, Some(30_000), false, true, 123, hour);
        d.enabled = true;
        assert_eq!(
            d.count_range(100, Some(30_000), false, true, 123, hour),
            estimate
        );
        assert_eq!(
            d.count_range(-1, Some(30_000), false, true, 123, hour),
            Some(([0; 2], true))
        );
        // Native decides its known byte before returning on a nonpositive count.
        assert_eq!(
            d.count_range(0, Some(29_999), false, true, 123, hour),
            Some(([0; 2], false))
        );
    }
}
