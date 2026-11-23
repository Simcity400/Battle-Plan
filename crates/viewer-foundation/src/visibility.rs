
use content_reader_layers::ResolvedLayers;
use content_reader_map::map::{MapData, ProvinceKind};
use content_reader_save::naval_history::RailwayAdjacency;
use replay_data::rows::tables;
#[cfg(test)]
use replay_data::rows::{
    CountryIntelRow, Recorded, RelationLinkRow, ReservedCountryNamingRow, ZeroSubjectInputs,
};
pub use replay_derive::visibility::{event_shared, forward_zero_shared_of};
pub(crate) use replay_derive::visibility::owner_rows;
#[cfg(test)]
use replay_derive::visibility::*;
pub use replay_derive::visibility::{AccessLinks, Families, ProvinceAccess, Relations};
use replay_derive::visibility::{
    Intel, forward_zero_shared, intel_fraction, map_intel_equal, map_intel_slot, raw_references,
    reverse_zero_shared,
};
use replay_frame::frame::DecodedFrame;
use std::{
    collections::{BTreeMap, BTreeSet},
    io,
};

pub mod template_reveal;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FogContent {
    pub topology: Topology,
    pub defines: Defines,
}

impl FogContent {
    pub fn loaded(content: &Result<Self, String>) -> Result<&Self, &'static str> {
        content
            .as_ref()
            .map_err(|_| "fog content (topology or NIntel defines) failed to load")
    }

    pub fn load(
        layers: &ResolvedLayers,
        map: &MapData,
        defines: &content_reader_defines::defines::Defines,
    ) -> io::Result<Self> {
        Ok(Self {
            topology: Topology::read(layers, map)?,
            defines: Defines::load(defines)?,
        })
    }
}

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

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Defines {
    pub intel_maximums: [i64; 4],
    pub radar_effect: i32,
    pub map_cutoffs: Vec<i64>,
    pub air_icon_threshold: i64,
    pub air_detail_thresholds: [i64; 4],
    pub air_plane_fuzzy_range: i64,
    pub naval_detail_thresholds: [i64; 5],
}

impl Defines {
    pub fn load(defines: &content_reader_defines::defines::Defines) -> io::Result<Self> {
        let four = |name: &str| -> io::Result<[i64; 4]> {
            defines
                .fixed5_array("NIntel", name)?
                .try_into()
                .map_err(|_| invalid(format!("NDefines.NIntel.{name} must hold four values")))
        };
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

fn complete<'a, T: replay_data::rows::TableSchema>(
    frame: &'a DecodedFrame,
    name: &'static str,
) -> Result<&'a [T::Row], &'static str> {
    frame.complete::<T>().map_err(|_| name)
}

pub(crate) fn saturate(current: u8, term: i32) -> u8 {
    i32::from(current).wrapping_add(term).clamp(0, 255) as u8
}
pub(crate) fn rounded(value: i64) -> i32 {
    (value.wrapping_add(if value < 0 { -50_000 } else { 50_000 }) / 100_000) as i32
}
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
        Ok(Self::of(defines, features.dlc_mask))
    }
    pub fn of(defines: &Defines, dlc_mask: u32) -> Self {
        Self {
            maximum: defines.intel_maximums[3],
            icon_threshold: defines.air_icon_threshold,
            detail: defines.air_detail_thresholds,
            fuzzy_range: defines.air_plane_fuzzy_range,
            enabled: dlc_mask & 0x20 != 0,
        }
    }
    fn fraction(&self, raw: Option<i64>, shared: bool, fog: bool) -> Option<i64> {
        if shared || !fog {
            Some(100_000)
        } else {
            intel_fraction(raw?, self.maximum).ok()
        }
    }
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
    #[cfg(any(test, feature = "test-support"))]
    pub fn port_detail(&self, raw: Option<i64>, shared: bool, fog: bool) -> Option<bool> {
        self.level(raw, shared, fog).map(|level| level > 0)
    }
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
            viewer_runtime::count_math::count_range_seeded(
                count,
                fraction,
                seed.wrapping_add(0x5E2),
                hour,
                [self.detail[1], self.detail[2], self.fuzzy_range],
            ),
            known,
        ))
    }
    pub fn show_ports(&self, raw: Option<i64>, shared: bool, fog: bool) -> Option<bool> {
        if !self.enabled {
            return Some(false);
        }
        self.fraction(raw, shared, fog)
            .map(|v| self.icon_threshold <= v)
    }
}

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
        Ok(Self::of(defines, features.dlc_mask))
    }
    pub fn of(defines: &Defines, dlc_mask: u32) -> Self {
        Self {
            maximum: defines.intel_maximums[2],
            detail: defines.naval_detail_thresholds,
            enabled: dlc_mask & 0x20 != 0,
        }
    }

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

pub(crate) fn network_term(cutoff: i32, modifier: i64) -> i32 {
    (i64::from(cutoff.wrapping_add(255))
        .wrapping_mul(100_000)
        .wrapping_mul(modifier)
        / 10_000_000_000) as i32
}

pub fn unit_byte(base: u16, modifier: i64) -> u16 {
    let value = i64::from(base)
        .wrapping_mul(modifier.wrapping_add(100_000))
        .wrapping_mul(100_000)
        / 10_000_000_000;
    (value as i32).clamp(0, 255) as u16
}

pub fn counter_shared(
    frame: &DecodedFrame,
    links: &BTreeMap<u32, u32>,
    owner: &replay_derive::directed_wars::Owner<'_>,
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
    Ok(families.counter_shared(owner.slot(), target))
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
    fn cutoff(&self, raw_a: u32, raw_b: u32) -> Result<i32, &'static str> {
        if self.cutoffs.len() < 2 {
            return Ok(0);
        }
        let a = map_intel_slot(self.links, raw_a)?;
        let b = map_intel_slot(self.links, raw_b)?;
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

#[cfg(any(test, feature = "test-support"))]
pub fn test_defines() -> Defines {
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
    use replay_data::{
        rows::CountryRow, rows::DivisionRow, rows::FactionMemberRow, rows::ProvinceRow,
        rows::RadarCoverageRow, rows::SimulationFeatureRow, rows::StateRow,
        rows::StateVisibilityRow, rows::TableId, rows::TableRows, rows::WarRelationRow,
    };
    use replay_frame::{frame::Metrics, frame::TableView};

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
                weather_suspend: replay_data::rows::Recorded::Value(None),
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

    fn set<T: replay_data::rows::TableSchema>(frame: &mut DecodedFrame, rows: Vec<T::Row>) {
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
                radius: None,
            }],
        );
        let bytes = run(&frame, &links, 1, &content).unwrap();
        assert_eq!((bytes[&3], bytes[&5], bytes[&4]), (255, 25, 0));
        set::<tables::Divisions>(&mut frame, vec![division(2)]);
        assert_eq!(run(&frame, &links, 1, &content).unwrap()[&3], 50);
    }

    #[test]
    fn an_occupied_owned_state_adds_its_network_term_and_absence_adds_none() {
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
        assert_eq!(
            d.count_range(0, Some(29_999), false, true, 123, hour),
            Some(([0; 2], false))
        );
    }
}
