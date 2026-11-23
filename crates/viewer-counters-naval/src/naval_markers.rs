use viewer_counter_kit::gui_windows::NAVAL_RESULT_SPRITES;
use crate::base_windows;
use viewer_native_gui::native_windows::NAVAL_COMBAT_WINDOW;

use content_reader_gui::gui::native;
use content_reader_map::map::ProvinceKind;
use crate::naval_windows;
use crate::naval::{NavalContext, NavalSources, NavalUnit};
use replay_data::rows::tables;
use replay_frame::frame::DecodedFrame;
use std::io;
use std::borrow::Cow;
use std::collections::{BTreeMap, BTreeSet};
use std::sync::Arc;
use viewer_counter_kit::{
    CounterColorMode, CounterDefines, CounterFrame, CounterInputs, CounterReport, Countries, coarse,
    gui_windows, visible,
};
use viewer_counter_kit::flotilla::Flotilla;
use viewer_foundation::allegiance;
use viewer_foundation::allegiance::Allegiance;
use viewer_foundation::perspective::{Sight, Vision};
use viewer_foundation::tables::{Available, rows};
use viewer_foundation::unit_rules::{SHIP_IN_PORT, STANDSTILL};
#[cfg(test)]
use viewer_counter_kit::UiColors;
use replay_data::{
    rows::NavalCombatDetailsRow, rows::NavalProvinceCombatRow,
    rows::NavalResultRow,
};
use std::cell::OnceCell;
use viewer_native_gui::gui_resources::HitRectangle;
use viewer_native_gui::gui_resources::hit_rectangle;
use viewer_native_gui::gui_resources::viewport;
use viewer_counter_kit::combat_markers::{Held, missing_of, whole_of};
use viewer_engagements::naval_icon_frames::{
    Census,
    CensusMember,
    admitted_of,
    censuses_of,
    convoy_variant,
};
use {
    viewer_counter_kit::naval_combat_grid as grid, viewer_counter_kit::naval_combat_occurrences as occurrences,
    viewer_counter_kit::naval_combat_occurrences::Combat, viewer_counter_kit::naval_combat_occurrences::Direct,
    viewer_counter_kit::naval_combat_occurrences::Occurrence,
    viewer_counter_kit::naval_combat_occurrences::Sources, viewer_counter_kit::naval_combat_occurrences::Transfers,
    viewer_counter_kit::naval_combat_occurrences::Units,
};
use viewer_engagements::{
    engagement_history::CombatFlags,
    engaged::Engaged,
    icon_frames::{self, Kept},
};
use viewer_native_gui::native_gui;

pub struct MarkerContext<'a> {
    pub vision: &'a Vision,
    pub owner: &'a dyn Fn(u32) -> Option<u32>,
    pub naval: Result<&'a NavalContext<'a>, NavalSources<'a>>,
    pub countries: &'a Countries,
    pub flags: &'a CombatFlags,
}

#[derive(Clone, Copy)]
pub struct Viewer<'a> {
    pub vision: &'a Vision,
    pub owner: &'a dyn Fn(u32) -> Option<u32>,
}

impl Viewer<'_> {
    fn contains_viewer(&self, list: &[u32]) -> Option<bool> {
        let mut unresolved = false;
        for &raw in list {
            match (self.owner)(raw) {
                Some(slot) if slot == self.vision.ours() => return Some(true),
                Some(_) => {}
                None => unresolved = true,
            }
        }
        (!unresolved).then_some(false)
    }
}

impl MarkerContext<'_> {
    fn side_allegiance(&self, side: &str, countries: &[u32]) -> Result<Allegiance, String> {
        let ours = self.vision.ours();
        for &raw in countries {
            let slot = (self.owner)(raw)
                .ok_or_else(|| format!("{side} side country {raw} has no CountryLinks row"))?;
            if slot == ours {
                return Ok(Allegiance::Ours);
            }
            if self
                .vision
                .shares_vision_proven(slot)
                .map_err(|cause| cause.describe())?
            {
                return Ok(Allegiance::Allied);
            }
            if self
                .vision
                .at_war_proven(slot)
                .map_err(|cause| cause.describe())?
            {
                return Ok(Allegiance::Enemy);
            }
        }
        Ok(Allegiance::Other)
    }

    fn convoy_tint(
        &self,
        inputs: &CounterInputs<'_>,
        allegiance: Allegiance,
        countries: &[u32],
    ) -> Result<[f32; 4], String> {
        let first = countries.first().map(|&raw| (self.owner)(raw).ok_or(raw));
        convoy_color(
            inputs.settings.color_mode,
            &inputs.content.defines.tints,
            self.countries,
            allegiance,
            first,
        )
    }

    fn flip(&self, detail: &NavalCombatDetailsRow) -> Result<bool, String> {
        self.vision
            .turns_naval_progress(
                detail.sides[0]
                    .summary
                    .countries
                    .iter()
                    .map(|&raw| (self.owner)(raw)),
            )
            .map_err(|cause| cause.describe())
    }
}

fn convoy_color(
    mode: CounterColorMode,
    tints: &allegiance::Tints,
    countries: &Countries,
    allegiance: Allegiance,
    first: Option<Result<u32, u32>>,
) -> Result<[f32; 4], String> {
    match (mode, first) {
        (CounterColorMode::Allegiance, _) => Ok(tints.of(allegiance)),
        (CounterColorMode::Country, None) => Ok(tints.other),
        (CounterColorMode::Country, Some(Ok(slot))) => countries.ui_color(slot),
        (CounterColorMode::Country, Some(Err(raw))) => {
            Err(format!("its first country {raw} is not a linked country"))
        }
    }
}

pub fn naval_gap(
    report: &mut CounterReport,
    cause: String,
    combats: impl IntoIterator<Item = u64>,
) {
    report
        .combat_gaps
        .entry(("Naval", cause))
        .or_default()
        .extend(combats);
}

fn whole<T: replay_data::rows::TableSchema>(
    table: &Available<'_, T>,
    name: &str,
) -> Option<String> {
    whole_of(held(table), name)
}

fn held<'a, T: replay_data::rows::TableSchema>(table: &Available<'a, T>) -> Held<'a> {
    match table {
        Available::Complete(_) => Held::Complete,
        Available::Truncated { missing, .. } => Held::Truncated(*missing),
        Available::Unavailable { reason } => Held::Unavailable(reason),
        Available::NotRecorded { mode } => Held::NotRecorded(*mode),
    }
}

#[cfg(test)]
type ForceInput<'a> = Result<(&'a replay_data::rows::TaskForceRow, i32), String>;

#[cfg(test)]
fn hq_frame<'a>(
    viewer: Viewer<'_>,
    detail: &NavalCombatDetailsRow,
    members: impl IntoIterator<Item = &'a replay_data::rows::NavalCombatShipRow>,
    ship_forces: &BTreeMap<u64, u64>,
    forces: &BTreeMap<u64, ForceInput<'_>>,
) -> Result<bool, String> {
    let members: Vec<CensusMember> = members.into_iter().map(CensusMember::of).collect();
    let ship_force = |ship: u64| ship_forces.get(&ship).copied();
    let force = |force: u64| {
        forces
            .get(&force)
            .cloned()
            .unwrap_or_else(|| Err(format!("the TaskForces table has no row for task force {force}")))
            .map(|(row, headquarters)| (row.roster_owner, headquarters))
    };
    hq_frame_of(viewer, &DetailFacts::of(detail).countries, members.iter(), &HqForces { ship_force: &ship_force, force: &force })
}

fn hq_frame_of<'a>(
    viewer: Viewer<'_>,
    countries: &[Vec<u32>; 2],
    members: impl IntoIterator<Item = &'a CensusMember>,
    hq: &HqForces<'_>,
) -> Result<bool, String> {
    let mut side = None;
    for (index, candidate) in countries.iter().enumerate() {
        let contains = viewer
            .contains_viewer(candidate)
            .ok_or_else(|| {
                let raw = candidate
                    .iter()
                    .find(|raw| (viewer.owner)(**raw).is_none());
                format!(
                    "side {index} country {} has no CountryLinks row",
                    raw.copied().unwrap_or_default()
                )
            })?;
        if contains {
            side = Some(index as u8);
            break;
        }
    }
    let Some(side) = side else {
        return Ok(false);
    };
    let mut unresolved = None;
    for member in members
        .into_iter()
        .filter(|member| member.side == side && member.live_ship != 0)
    {
        let qualifies = (|| {
            let force = (hq.ship_force)(member.ship)
                .ok_or_else(|| format!("the Ships table has no row for ship {}", member.ship))?;
            let (roster_owner, headquarters) = (hq.force)(force)?;
            let owner = (viewer.owner)(roster_owner).ok_or_else(|| {
                format!(
                    "task force {force}'s roster owner {roster_owner} has no CountryLinks row"
                )
            })?;
            Ok::<_, String>(owner == viewer.vision.ours() && headquarters > 0)
        })();
        match qualifies {
            Ok(true) => return Ok(true),
            Ok(false) => {}
            Err(cause) => {
                unresolved.get_or_insert(cause);
            }
        }
    }
    unresolved.map_or(Ok(false), Err)
}

pub type HqFrames = Result<BTreeMap<u64, Result<bool, String>>, String>;

fn rotates(home: u32, occurrence: &Occurrence) -> bool {
    home == occurrence.anchor_province
}

fn two_engaged(
    flags: &CombatFlags,
    province: u32,
    row: Option<&NavalProvinceCombatRow>,
    complete: bool,
) -> Result<bool, String> {
    flags
        .engaged_entries(row, complete, 2)
        .ok_or_else(|| match row {
            None => format!(
                "province {province}'s combat vector is unknown (the NavalProvinceCombats table \
                 is incomplete)"
            ),
            Some(_) => {
                format!("an engaged flag in province {province}'s combat vector is undecided")
            }
        })
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FrameSource {
    Engaged,
    Kept(Kept),
    Spillover,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Decision {
    variant: u32,
    progress: bool,
    frame: FrameSource,
}

fn decide(
    rotates: bool,
    engaged: Engaged,
    census: Result<[Census; 2], &str>,
    kept: Option<Kept>,
) -> Result<Decision, String> {
    if !rotates {
        return Ok(Decision {
            variant: 0,
            progress: false,
            frame: FrameSource::Spillover,
        });
    }
    match engaged {
        Engaged::No => {
            let kept = kept.unwrap_or(Kept::Undecided);
            Ok(Decision {
                variant: match kept {
                    Kept::Frame(frame) => frame,
                    _ => 0,
                },
                progress: false,
                frame: FrameSource::Kept(kept),
            })
        }
        Engaged::Yes => Ok(Decision {
            variant: u32::from(convoy_variant(
                census.map_err(|cause| format!("convoy variant undecided: {cause}"))?,
            )),
            progress: true,
            frame: FrameSource::Engaged,
        }),
        Engaged::Ambiguous(cause) => Err(cause.reason().into()),
        Engaged::Unknown => Err("the combat's engaged flag for the viewer is unknown".into()),
    }
}

pub struct NavalMarkerTables {
    combats: Option<CombatTables>,
    results: Arc<ResultTables>,
    vectors: BTreeSet<u32>,
    direct_complete: bool,
    live: BTreeSet<u64>,
    live_undecided: Option<String>,
    gated: BTreeSet<u64>,
}

#[derive(Debug, PartialEq)]
struct Icon {
    combat: u64,
    province: u32,
    rotates: bool,
    home: u32,
    anchor: [f32; 3],
    ordinal: u32,
    decision: Decision,
    hq: Result<bool, String>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Omitted {
    pub combat: u64,
    pub province: u32,
    pub rotates: bool,
    pub place: Option<([f32; 3], u32)>,
    pub cause: String,
}

struct CombatTables {
    icons: Vec<Result<Icon, Omitted>>,
    placement: Vec<(String, Vec<u64>)>,
    unplaced: usize,
    undecided: Vec<Omitted>,
    hq: HqFrames,
    provenance: Option<&'static str>,
}

impl NavalMarkerTables {
    pub fn read(
        frame: &DecodedFrame,
        viewer: Viewer<'_>,
        flags: &CombatFlags,
        kept: &BTreeMap<u64, Kept>,
        province_kind: &dyn Fn(u32) -> Option<ProvinceKind>,
        anchor: &dyn Fn(u32) -> Option<[f32; 3]>,
    ) -> io::Result<Self> {
        let direct = rows::<tables::NavalProvinceCombats>(frame)?;
        let details_table = rows::<tables::NavalCombatDetails>(frame)?;
        let combats = rows::<tables::NavalCombats>(frame)?;
        let combat_refs: Vec<Combat> = combats
            .rows()
            .iter()
            .map(|row| Combat {
                id: row.id,
                home: row.province,
                registry_ordinal: row.registry_ordinal,
            })
            .collect();
        let direct_refs: Vec<Direct> = direct
            .rows()
            .iter()
            .map(|row| Direct {
                province: row.province,
                combats: row
                    .combats
                    .iter()
                    .zip(&row.kinds)
                    .filter(|(_, kind)| **kind == NavalProvinceCombatRow::NAVAL)
                    .map(|(combat, _)| *combat)
                    .collect(),
            })
            .collect();
        let details: BTreeMap<u64, DetailFacts> = details_table.rows().iter().map(|row| (row.combat, DetailFacts::of(row))).collect();
        let transfers = rows::<tables::NavalCombatTransfers>(frame)?;
        let units = rows::<tables::NavalTransferUnits>(frame)?;
        let transfer_refs: Vec<Transfers> = transfers
            .rows()
            .iter()
            .map(|row| Transfers {
                combat: row.combat,
                transfers: row.transfers.clone(),
            })
            .collect();
        let unit_refs: Vec<Units> = units
            .rows()
            .iter()
            .map(|row| Units {
                transfer: row.transfer,
                units: row.units.clone(),
            })
            .collect();
        let empty_census = combats.is_complete() && combats.rows().is_empty();
        let locations = if empty_census { None } else { unit_locations(frame, &unit_refs)? };
        let members = rows::<tables::NavalCombatShips>(frame)?;
        let census: Vec<CensusMember> = members.rows().iter().map(CensusMember::of).collect();
        let ships = rows::<tables::Ships>(frame)?;
        let forces = rows::<tables::TaskForces>(frame)?;
        let inputs = rows::<tables::TaskForceCounterInputs>(frame)?;
        let ship_force = |ship: u64| {
            let rows = ships.rows();
            rows.binary_search_by_key(&ship, |row| row.id).ok().map(|found| rows[found].task_force)
        };
        let force = |id: u64| -> Result<(u32, i32), String> {
            let (forces, inputs) = (forces.rows(), inputs.rows());
            let force = forces
                .binary_search_by_key(&id, |row| row.id)
                .map_err(|_| format!("the TaskForces table has no row for task force {id}"))?;
            let input = inputs
                .binary_search_by_key(&id, |row| row.task_force)
                .map_err(|_| format!("the TaskForceCounterInputs table has no row for task force {id}"))?;
            Ok((forces[force].roster_owner, inputs[input].attached_headquarters))
        };
        let hq = match whole(&ships, "Ships").or_else(|| whole(&forces, "TaskForces")).or_else(|| whole(&inputs, "TaskForceCounterInputs")) {
            Some(cause) => Err(cause),
            None => Ok(HqForces { ship_force: &ship_force, force: &force }),
        };
        let results = rows::<tables::NavalResults>(frame)?;
        let result_members = rows::<tables::NavalResultMembers>(frame)?;
        let result_countries = rows::<tables::NavalResultCountries>(frame)?;
        let result_fields: Vec<ResultFields> = results.rows().iter().map(ResultFields::of).collect();
        let countries = result_country_lists(result_countries.rows().iter().map(|row| (row.result, row.list, row.ordinal, row.country)));
        let present: BTreeSet<u64> = result_members.rows().iter().filter(|member| member.presence_value != 0).map(|member| member.result).collect();
        let flag = |id: u64| flags.entry(id, NavalProvinceCombatRow::NAVAL);
        let kept_of = |id: u64| kept.get(&id).copied();
        let values = naval_marker_values(&NavalMarkerInputs {
            vision: viewer.vision,
            owner: viewer.owner,
            combats: &combat_refs,
            combats_held: held(&combats),
            direct: &direct_refs,
            direct_complete: direct.is_complete(),
            details: &details,
            details_held: held(&details_table),
            transfers: transfers.is_complete().then_some(transfer_refs.as_slice()),
            units: units.is_complete().then_some(unit_refs.as_slice()),
            locations: locations.as_ref(),
            members: match whole(&members, "NavalCombatShips") {
                Some(cause) => Err(cause),
                None => Ok(&census),
            },
            hq,
            flag: &flag,
            provenance: flags.naval_provenance.limitation(),
            kept: &kept_of,
            province_kind,
            anchor,
            results: &result_fields,
            result_countries: &countries,
            present: &present,
            results_complete: results.is_complete() && result_members.is_complete() && result_countries.is_complete(),
        });
        Ok(Self {
            combats: values.combats.map(|combats| CombatTables {
                icons: combats
                    .icons
                    .into_iter()
                    .map(|icon| {
                        icon.map(|icon| Icon {
                            combat: icon.combat,
                            province: icon.province,
                            rotates: icon.rotates,
                            home: icon.home,
                            anchor: icon.anchor,
                            ordinal: icon.ordinal,
                            decision: icon.decision,
                            hq: icon.hq,
                        })
                    })
                    .collect(),
                placement: combats.placement,
                unplaced: combats.unplaced,
                undecided: combats.undecided,
                hq: combats.hq,
                provenance: combats.provenance,
            }),
            results: Arc::new(values.results),
            vectors: direct.rows().iter().map(|row| row.province).collect(),
            direct_complete: direct.is_complete(),
            live: values.live,
            live_undecided: values.live_undecided,
            gated: values.gated,
        })
    }

    pub fn empty() -> Self {
        Self {
            combats: None,
            results: Arc::new(ResultTables { provinces: BTreeMap::new(), facts: BTreeMap::new(), undecided: None }),
            vectors: BTreeSet::new(),
            direct_complete: false,
            live: BTreeSet::new(),
            live_undecided: None,
            gated: BTreeSet::new(),
        }
    }

    pub fn of_values(values: &NavalMarkerValues, frame: &DecodedFrame) -> io::Result<Self> {
        let direct = rows::<tables::NavalProvinceCombats>(frame)?;
        Ok(Self {
            combats: values.combats.as_ref().map(|combats| CombatTables {
                icons: combats
                    .icons
                    .iter()
                    .map(|icon| match icon {
                        Ok(icon) => Ok(Icon {
                            combat: icon.combat,
                            province: icon.province,
                            rotates: icon.rotates,
                            home: icon.home,
                            anchor: icon.anchor,
                            ordinal: icon.ordinal,
                            decision: icon.decision,
                            hq: icon.hq.clone(),
                        }),
                        Err(omitted) => Err(omitted.clone()),
                    })
                    .collect(),
                placement: combats.placement.clone(),
                unplaced: combats.unplaced,
                undecided: combats.undecided.clone(),
                hq: combats.hq.clone(),
                provenance: combats.provenance,
            }),
            results: Arc::new(values.results.clone()),
            vectors: direct.rows().iter().map(|row| row.province).collect(),
            direct_complete: direct.is_complete(),
            live: values.live.clone(),
            live_undecided: values.live_undecided.clone(),
            gated: values.gated.clone(),
        })
    }

    pub fn values(&self) -> NavalMarkerValues {
        NavalMarkerValues {
            combats: self.combats.as_ref().map(|combats| CombatValues {
                icons: combats
                    .icons
                    .iter()
                    .map(|icon| match icon {
                        Ok(icon) => Ok(IconValues {
                            combat: icon.combat,
                            province: icon.province,
                            rotates: icon.rotates,
                            home: icon.home,
                            anchor: icon.anchor,
                            ordinal: icon.ordinal,
                            decision: icon.decision,
                            hq: icon.hq.clone(),
                        }),
                        Err(omitted) => Err(omitted.clone()),
                    })
                    .collect(),
                placement: combats.placement.clone(),
                unplaced: combats.unplaced,
                undecided: combats.undecided.clone(),
                hq: combats.hq.clone(),
                provenance: combats.provenance,
            }),
            results: (*self.results).clone(),
            live: self.live.clone(),
            live_undecided: self.live_undecided.clone(),
            gated: self.gated.clone(),
        }
    }
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct DetailFacts {
    pub countries: [Vec<u32>; 2],
}

impl DetailFacts {
    pub fn of(row: &NavalCombatDetailsRow) -> Self {
        Self { countries: [row.sides[0].summary.countries.clone(), row.sides[1].summary.countries.clone()] }
    }

    fn admitted(facts: Option<&Self>) -> Option<bool> {
        admitted_of(facts.map(|facts| [facts.countries[0].as_slice(), facts.countries[1].as_slice()]))
    }
}

pub struct HqForces<'a> {
    pub ship_force: &'a dyn Fn(u64) -> Option<u64>,
    pub force: &'a dyn Fn(u64) -> Result<(u32, i32), String>,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct ResultFields {
    pub id: u64,
    pub province: u32,
    pub province_ordinal: u32,
    pub flag_a: u8,
    pub flag_b: u8,
    pub subtype: u8,
    pub readiness: u8,
    pub winning_side: u8,
    pub losing_side: u8,
}

impl ResultFields {
    fn of(row: &NavalResultRow) -> Self {
        Self {
            id: row.id,
            province: row.province,
            province_ordinal: row.province_ordinal,
            flag_a: row.flag_a,
            flag_b: row.flag_b,
            subtype: row.subtype,
            readiness: row.readiness,
            winning_side: row.winning_side,
            losing_side: row.losing_side,
        }
    }
}

pub fn result_country_lists(rows: impl IntoIterator<Item = (u64, u8, u32, u32)>) -> BTreeMap<(u64, u8), Vec<u32>> {
    let mut countries: BTreeMap<(u64, u8), Vec<(u32, u32)>> = BTreeMap::new();
    for (result, list, ordinal, country) in rows {
        countries.entry((result, list)).or_default().push((ordinal, country));
    }
    countries
        .into_iter()
        .map(|(key, mut list)| {
            list.sort_unstable();
            (key, list.into_iter().map(|(_, country)| country).collect())
        })
        .collect()
}

pub struct NavalMarkerInputs<'a> {
    pub vision: &'a Vision,
    pub owner: &'a dyn Fn(u32) -> Option<u32>,
    pub combats: &'a [Combat],
    pub combats_held: Held<'a>,
    pub direct: &'a [Direct],
    pub direct_complete: bool,
    pub details: &'a BTreeMap<u64, DetailFacts>,
    pub details_held: Held<'a>,
    pub transfers: Option<&'a [Transfers]>,
    pub units: Option<&'a [Units]>,
    pub locations: Option<&'a BTreeMap<u64, u32>>,
    pub members: Result<&'a [CensusMember], String>,
    pub hq: Result<HqForces<'a>, String>,
    pub flag: &'a dyn Fn(u64) -> Engaged,
    pub provenance: Option<&'static str>,
    pub kept: &'a dyn Fn(u64) -> Option<Kept>,
    pub province_kind: &'a dyn Fn(u32) -> Option<ProvinceKind>,
    pub anchor: &'a dyn Fn(u32) -> Option<[f32; 3]>,
    pub results: &'a [ResultFields],
    pub result_countries: &'a BTreeMap<(u64, u8), Vec<u32>>,
    pub present: &'a BTreeSet<u64>,
    pub results_complete: bool,
}

#[derive(Clone, Debug, PartialEq)]
pub struct IconValues {
    pub combat: u64,
    pub province: u32,
    pub rotates: bool,
    pub home: u32,
    pub anchor: [f32; 3],
    pub ordinal: u32,
    pub decision: Decision,
    pub hq: Result<bool, String>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct CombatValues {
    pub icons: Vec<Result<IconValues, Omitted>>,
    pub placement: Vec<(String, Vec<u64>)>,
    pub unplaced: usize,
    pub undecided: Vec<Omitted>,
    pub hq: HqFrames,
    pub provenance: Option<&'static str>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct NavalMarkerValues {
    pub combats: Option<CombatValues>,
    pub results: ResultTables,
    pub live: BTreeSet<u64>,
    pub live_undecided: Option<String>,
    pub gated: BTreeSet<u64>,
}

pub fn naval_marker_values(inputs: &NavalMarkerInputs<'_>) -> NavalMarkerValues {
    let viewer = Viewer { vision: inputs.vision, owner: inputs.owner };
    NavalMarkerValues {
        combats: combat_values(inputs, viewer),
        results: result_values(viewer, inputs.results, inputs.result_countries, inputs.present, inputs.results_complete),
        live: inputs.combats.iter().map(|combat| combat.id).collect(),
        live_undecided: whole_of(inputs.combats_held, "NavalCombats"),
        gated: inputs.details.iter().filter(|(_, detail)| DetailFacts::admitted(Some(detail)) == Some(false)).map(|(&combat, _)| combat).collect(),
    }
}

fn combat_values(inputs: &NavalMarkerInputs<'_>, viewer: Viewer<'_>) -> Option<CombatValues> {
    let combats_complete = matches!(inputs.combats_held, Held::Complete);
    let empty_census = combats_complete && inputs.combats.is_empty();
    if empty_census && inputs.direct.iter().all(|row| row.combats.is_empty()) {
        return None;
    }
    let detail = |id: u64| inputs.details.get(&id);
    let placed = occurrences::derive(
        Sources {
            combats: inputs.combats,
            combats_complete,
            direct: inputs.direct,
            direct_complete: inputs.direct_complete,
            transfers: inputs.transfers,
            units: inputs.units,
            unit_locations: if empty_census { None } else { inputs.locations },
            province_kind: inputs.province_kind,
        },
        &|id| DetailFacts::admitted(detail(id)),
        &|id| icon_frames::engaged_seen((inputs.flag)(id)),
    );
    let mut placement = Vec::new();
    for &(province, combat) in &placed.direct_gaps {
        let key = format!("combat {combat}, which province {province} names");
        placement.push((
            format!("icon omitted: {}", missing_of(inputs.combats_held, "NavalCombats", key)),
            vec![combat],
        ));
    }
    let mut undecided = Vec::new();
    for &(province, combat) in &placed.admission_gaps {
        let key = format!("combat {combat}, which province {province} names");
        let cause = format!(
            "icon omitted: its side countries are unknown: {}",
            missing_of(inputs.details_held, "NavalCombatDetails", key)
        );
        undecided.push(Omitted {
            combat,
            province,
            rotates: inputs.combats
                .iter()
                .any(|row| row.id == combat && row.home == province),
            place: None,
            cause: cause.clone(),
        });
        placement.push((cause, vec![combat]));
    }
    if let Some(reason) = placed.spillover_gap {
        placement.push((
            format!("spillover icons omitted: {reason}"),
            inputs.combats.iter().map(|combat| combat.id).collect(),
        ));
    }
    let forward: Result<BTreeMap<u64, Vec<&CensusMember>>, String> = inputs.members.as_ref().map_err(Clone::clone).map(|members| {
        let mut forward: BTreeMap<u64, Vec<&CensusMember>> = BTreeMap::new();
        for member in members.iter().filter(|member| member.member_source == 0 && member.group_index != viewer_foundation::unit_rules::PAIRED_ONLY_GROUP) {
            forward.entry(member.combat).or_default().push(member);
        }
        forward
    });
    let censuses = inputs.members.as_ref().map_err(Clone::clone).map(|members| censuses_of(members.iter().copied()));
    let hq: HqFrames = match (&forward, &inputs.hq) {
        (Err(cause), _) => Err(cause.clone()),
        (_, Err(cause)) => Err(cause.clone()),
        (Ok(forward), Ok(forces)) => Ok(inputs
            .details
            .iter()
            .map(|(&combat, detail)| (combat, hq_frame_of(viewer, &detail.countries, forward.get(&combat).into_iter().flatten().copied(), forces)))
            .collect()),
    };
    let homes: BTreeMap<u64, u32> = inputs.combats.iter().map(|row| (row.id, row.home)).collect();
    let mut icons = Vec::new();
    for occurrence in placed.by_province.values().flatten() {
        let Some(&home) = homes.get(&occurrence.combat) else {
            continue;
        };
        let Some(facts) = detail(occurrence.combat) else {
            continue;
        };
        if DetailFacts::admitted(Some(facts)) != Some(true) {
            continue;
        }
        let combat = occurrence.combat;
        let rotating = rotates(home, occurrence);
        let ordinal = occurrence.ordinal.ok_or_else(|| {
            format!(
                "its place in province {}'s icon stack is unknown",
                occurrence.province
            )
        });
        let anchored = (inputs.anchor)(occurrence.anchor_province).ok_or_else(|| {
            format!(
                "the content places no Standstill unit-stack anchor for province {}",
                occurrence.anchor_province
            )
        });
        let place = anchored.clone().ok().zip(ordinal.clone().ok());
        let icon = (|| {
            let ordinal = ordinal?;
            let anchor = anchored?;
            let engaged = if rotating {
                (inputs.flag)(combat)
            } else {
                Engaged::No
            };
            let census = match &censuses {
                Err(cause) => Err(cause.as_str()),
                Ok(censuses) => match censuses.get(&combat) {
                    Some(Err(cause)) => Err(cause.as_str()),
                    Some(Ok(census)) => Ok(*census),
                    None => Ok(Default::default()),
                },
            };
            Ok(IconValues {
                combat,
                province: occurrence.province,
                rotates: rotating,
                home,
                anchor,
                ordinal,
                decision: decide(rotating, engaged, census, (inputs.kept)(combat))?,
                hq: match &hq {
                    Err(cause) => Err(cause.clone()),
                    Ok(hq) => hq[&combat].clone(),
                },
            })
        })();
        icons.push(icon.map_err(|cause: String| Omitted {
            combat,
            province: occurrence.province,
            rotates: rotating,
            place,
            cause: format!("icon omitted: {cause}"),
        }));
    }
    Some(CombatValues {
        icons,
        placement,
        unplaced: placed.direct_gaps.len() + placed.admission_gaps.len(),
        undecided,
        hq,
        provenance: inputs.provenance,
    })
}

impl CombatTables {
    fn report_omitted(&self, refused: Option<&str>, report: &mut CounterReport) {
        report.unavailable_naval_combats += self.unplaced;
        for (cause, combats) in &self.placement {
            naval_gap(report, cause.clone(), combats.iter().copied());
        }
        for icon in &self.icons {
            let (combat, cause) = match (icon, refused) {
                (Ok(icon), Some(refused)) => (icon.combat, refused.to_owned()),
                (Err(omitted), Some(refused)) => (omitted.combat, refused.to_owned()),
                (Err(omitted), None) => (omitted.combat, omitted.cause.clone()),
                (Ok(icon), None) => {
                    if let Some(limit) = self.provenance {
                        report
                            .combat_limits
                            .entry(("Naval", limit.to_owned()))
                            .or_default()
                            .insert(icon.combat);
                    }
                    continue;
                }
            };
            report.unavailable_naval_combats += 1;
            naval_gap(report, cause, [combat]);
        }
    }
}

fn unit_locations(frame: &DecodedFrame, units: &[Units]) -> io::Result<Option<BTreeMap<u64, u32>>> {
    let divisions = rows::<tables::Divisions>(frame)?;
    let forces = rows::<tables::TaskForces>(frame)?;
    let guns = rows::<tables::RailwayGunCounterInputs>(frame)?;
    if !(divisions.is_complete() && forces.is_complete() && guns.is_complete()) {
        return Ok(None);
    }
    let (divisions, forces, guns) = (divisions.rows(), forces.rows(), guns.rows());
    let locate = |unit: u64| {
        if let Ok(index) = guns.binary_search_by_key(&unit, |row| row.gun) {
            return Some(guns[index].province);
        }
        if let Ok(index) = forces.binary_search_by_key(&unit, |row| row.id) {
            return Some(forces[index].province);
        }
        let index = divisions.binary_search_by_key(&unit, |row| row.id).ok()?;
        Some(divisions[index].province)
    };
    Ok(Some(
        units
            .iter()
            .flat_map(|row| &row.units)
            .filter_map(|&unit| Some((unit, locate(unit)?)))
            .collect(),
    ))
}

pub fn naval_markers(
    inputs: &CounterInputs<'_>,
    frame: &DecodedFrame,
    context: &MarkerContext<'_>,
    tables: &NavalMarkerTables,
    out: &mut CounterFrame<impl Sized, base_windows::PreparedBases, impl Sized, naval_windows::Plan>,
) -> io::Result<()> {
    combat_icons(inputs, frame, context, tables, out)?;
    result_pass(inputs, tables, out);
    Ok(())
}

fn combat_pass(height: f32, defines: &CounterDefines) -> bool {
    visible(height, defines.naval_combat_cutoff)
}

fn gap<K: Ord + From<&'static str>>(gaps: &mut BTreeMap<K, usize>, reason: &'static str) {
    *gaps.entry(reason.into()).or_default() += 1;
}

fn unconstructed(reason: &str) -> String {
    format!("icon omitted: {NAVAL_COMBAT_WINDOW} does not lay out: {reason}")
}

fn combat_icons(
    inputs: &CounterInputs<'_>,
    frame: &DecodedFrame,
    context: &MarkerContext<'_>,
    tables: &NavalMarkerTables,
    out: &mut CounterFrame<impl Sized, base_windows::PreparedBases, impl Sized, naval_windows::Plan>,
) -> io::Result<()> {
    let windows = inputs.windows;
    let window = windows.naval_combat();
    let resources = viewer_counter_kit::gui_windows::gui_assets(inputs, None);
    let defines = &inputs.content.defines;
    if !inputs.visibility.province_icons || !inputs.passes(|height| combat_pass(height, defines)) {
        return Ok(());
    }
    let window_frame = match windows.naval_combat_frame(viewport(inputs.viewport, inputs.ui_scale))
    {
        Ok(window_frame) => window_frame,
        Err(reason) => {
            if let Some(combats) = &tables.combats {
                combats.report_omitted(Some(&unconstructed(&reason)), &mut out.report);
            }
            out.naval.combats_undecided = Some(unconstructed(&reason).into());
            return Ok(());
        }
    };
    let mut pass = naval_windows::CombatPass {
        icons: Vec::new(),
        gated: tables.gated.clone(),
        live: tables.live.clone(),
        live_undecided: tables.live_undecided.as_ref().map(|why| {
            format!("icon omitted: whether its combat resolves is undecided: {why}").into()
        }),
        hq: Ok(BTreeMap::new()),
        size: window_frame.size,
    };
    let Some(combats) = &tables.combats else {
        out.naval.combat = Some(pass);
        return Ok(());
    };
    combats.report_omitted(None, &mut out.report);
    pass.hq = combats.hq.clone();
    for omitted in &combats.undecided {
        pass.icons.push(naval_windows::CombatIcon::omitted(
            (omitted.province, omitted.combat),
            None,
            omitted.rotates,
            &omitted.cause,
        ));
    }
    let height = window_frame.size[1] as f32;
    let details = rows::<tables::NavalCombatDetails>(frame)?.rows();
    let direct = rows::<tables::NavalProvinceCombats>(frame)?.rows();
    let details_of: BTreeMap<u64, &NavalCombatDetailsRow> = details.iter().map(|row| (row.combat, row)).collect();
    let vector_of: BTreeMap<u32, &NavalProvinceCombatRow> = direct.iter().map(|row| (row.province, row)).collect();
    let members = rows::<tables::NavalCombatShips>(frame)?;
    let ships = rows::<tables::Ships>(frame)?;
    let forces = rows::<tables::TaskForces>(frame)?;
    let inputs_rows = rows::<tables::TaskForceCounterInputs>(frame)?;
    let own_naval: OnceCell<NavalContext<'_>> = OnceCell::new();
    let mut grid_index: Option<grid::Index<'_>> = None;
    for icon in &combats.icons {
        let icon = match icon {
            Ok(icon) => icon,
            Err(omitted) => {
                pass.icons.push(naval_windows::CombatIcon::omitted(
                    (omitted.province, omitted.combat),
                    omitted.place,
                    omitted.rotates,
                    &omitted.cause,
                ));
                continue;
            }
        };
        if occurrences::screen_y_offset(icon.ordinal, height).is_none() {
            let cause =
                "icon omitted: the naval combat window height is not a whole number of pixels";
            out.report.unavailable_naval_combats += 1;
            naval_gap(&mut out.report, cause.into(), [icon.combat]);
            pass.icons.push(naval_windows::CombatIcon::omitted(
                (icon.province, icon.combat),
                None,
                icon.rotates,
                cause,
            ));
            continue;
        }
        let Some(detail) = details_of.get(&icon.combat).copied() else {
            let cause = "icon omitted: the hour on screen holds no NavalCombatDetails row for the combat the naval markers layer placed";
            out.report.unavailable_naval_combats += 1;
            naval_gap(&mut out.report, cause.into(), [icon.combat]);
            pass.icons.push(naval_windows::CombatIcon::omitted((icon.province, icon.combat), Some((icon.anchor, icon.ordinal)), icon.rotates, cause));
            continue;
        };
        let engaged = icon.rotates && icon.decision.progress;
        let small = if !engaged || coarse(inputs) {
            Ok(true)
        } else {
            two_engaged(
                context.flags,
                icon.home,
                vector_of.get(&icon.home).copied(),
                tables.direct_complete,
            )
        };
        let progress = || match (&small, context.flip(detail)) {
            (Ok(small), Ok(flip)) => Ok(gui_windows::CombatProgress {
                small: *small,
                progress: detail.progress,
                flip,
            }),
            (Err(cause), _) => Err(cause.clone()),
            (_, Err(cause)) => Err(cause),
        };
        let state = gui_windows::CombatState {
            rotation: icon.rotates.then(|| gui_windows::Rotation {
                engaged: icon.decision.progress.then(|| gui_windows::Engaged {
                    variant: icon.decision.variant,
                    progress: progress(),
                }),
            }),
            hq: icon.hq.clone(),
        };
        let mut update = gui_windows::combat_calls(window, &state, &resources);
        update.wrote.push(naval_windows::PLACE);
        let grids = match small {
            _ if !engaged => Ok(Vec::new()),
            Err(cause) => Err(cause),
            Ok(true) => Ok(Vec::new()),
            Ok(false) => {
                let naval = match context.naval {
                    Ok(naval) => naval,
                    Err(sources) => match own_naval.get() {
                        Some(naval) => naval,
                        None => {
                            let built = NavalContext::build(frame, sources)?;
                            own_naval.get_or_init(|| built)
                        }
                    },
                };
                let index = grid_index.get_or_insert_with(|| {
                    grid::Index::new(grid::Sources {
                        combat_ships: members.is_complete().then(|| members.rows()),
                        ships: ships.is_complete().then(|| grid::ShipCensus {
                            task_forces: Cow::Borrowed(&naval.ship_task_forces),
                            counts: Cow::Borrowed(&naval.ships),
                            rows: ships.rows().len(),
                        }),
                        task_forces: forces.is_complete().then_some(forces.rows()),
                        counter_inputs: inputs_rows.is_complete().then_some(inputs_rows.rows()),
                        aggression: Some(inputs.content.aggression.as_slice()),
                    })
                });
                let tiles = grid::derive(
                    index,
                    icon.combat,
                    context.vision,
                    &context.owner,
                    &context.countries.sides,
                )
                .and_then(|sides| Ok((combat_tiles(context, naval, &sides)?, sides)));
                tiles
                    .map(|tiles| grid_fills(inputs, context, window, detail, &tiles, out))
                    .map_err(|cause| cause.to_string())
            }
        };
        let grids = match grids {
            Ok(grids) => {
                if icon.rotates {
                    update.wrote.push(gui_windows::GRIDS);
                }
                grids
            }
            Err(cause) => {
                update.unknown.push((
                    gui_windows::GRIDS,
                    format!("icon omitted: its grids are undecided: {cause}"),
                ));
                Vec::new()
            }
        };
        pass.icons.push(naval_windows::CombatIcon {
            owner: (icon.province, icon.combat),
            place: Some((icon.anchor, icon.ordinal)),
            update,
            rotates: icon.rotates,
            grids,
            frame: Some(icon.decision.frame),
            omitted: false,
        });
    }
    out.naval.combat = Some(pass);
    Ok(())
}

pub fn combat_footprints(
    window: &native::Element,
    layout: &native_gui::Layout,
    origin: [i32; 2],
    resources: &dyn native_gui::Resources,
    notes: &mut BTreeMap<String, usize>,
) -> Vec<(HitRectangle, bool)> {
    let mut hits: Vec<_> = [
        (native::Kind::Button, "bg_btn", true),
        (native::Kind::Button, "frame_btn", true),
        (native::Kind::Icon, "progress", false),
        (native::Kind::Icon, "progress_small", false),
    ]
    .into_iter()
    .filter_map(|(kind, name, clickable)| {
        let control = window.find(kind, name)?;
        let hit = hit_rectangle(control, layout, origin, resources, notes)?;
        Some((hit, clickable))
    })
    .collect();
    hits.sort_by_key(|(hit, _)| hit.order);
    hits
}

fn grid_fills(
    inputs: &CounterInputs<'_>,
    context: &MarkerContext<'_>,
    window: &native::Element,
    detail: &NavalCombatDetailsRow,
    (tiles, sides): &([Vec<Flotilla>; 2], [grid::SideGrid<'_>; 2]),
    out: &mut CounterFrame<impl Sized, base_windows::PreparedBases, impl Sized, naval_windows::Plan>,
) -> Vec<naval_windows::GridFill> {
    let mut fills = Vec::new();
    let navy = inputs.windows.navy();
    let art = gui_windows::CounterArt {
        assets: inputs.assets,
        countries: context.countries,
    };
    let resources = viewer_counter_kit::gui_windows::gui_assets(inputs, None);
    let ctx = gui_windows::NavyContext {
        settings: inputs.settings,
        selected: inputs.selected,
        defines: &inputs.content.defines,
        art: &art,
        gui: &resources,
    };
    for (side, roles) in [
        [GridRole::Tiles(0), GridRole::Adds(0)],
        [GridRole::Tiles(1), GridRole::Adds(1)],
    ]
    .into_iter()
    .enumerate()
    {
        for role in roles {
            let Some(name) = GRID_ROLES
                .iter()
                .find(|(_, own)| *own == role)
                .map(|(name, _)| *name)
            else {
                continue;
            };
            let Some(element) = window.find(native::Kind::Grid, name) else {
                continue;
            };
            match role {
                GridRole::Tiles(_) => {
                    let items = tiles[side]
                        .iter()
                        .map(|tile| {
                            Ok(gui_windows::navy_calls(
                                &ctx,
                                navy,
                                &gui_windows::NavyState {
                                    flotilla: tile,
                                    countries: context.countries,
                                },
                                &mut out.report.navy_counter_gaps,
                            ))
                        })
                        .collect();
                    fills.push(naval_windows::GridFill {
                        grid: element.path.clone(),
                        side: Some(side),
                        items,
                    });
                }
                GridRole::Adds(_) => {
                    let convoys = sides[side].convoys;
                    if convoys == 0 {
                        continue;
                    }
                    let countries = &detail.sides[side].summary.countries;
                    let name = if side == 0 { "attacking" } else { "defending" };
                    let item = context
                        .side_allegiance(name, countries)
                        .and_then(|allegiance| context.convoy_tint(inputs, allegiance, countries))
                        .map(|tint| {
                            viewer_counter_kit::gui_windows::Update::calls(match inputs.windows.tiny() {
                                Some(tiny) => gui_windows::convoy_calls(
                                    &inputs.assets.gui,
                                    tiny,
                                    convoys,
                                    tint,
                                    &mut out.report.naval_grid_gaps,
                                ),
                                None => Vec::new(),
                            })
                        })
                        .map_err(|reason| {
                            Cow::Owned(format!("a convoy addition is not drawn: {reason}"))
                        });
                    fills.push(naval_windows::GridFill {
                        grid: element.path.clone(),
                        side: None,
                        items: vec![item],
                    });
                }
            }
        }
    }
    fills
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum GridRole {
    Adds(usize),
    Tiles(usize),
}

const GRID_ROLES: [(&str, GridRole); 4] = [
    ("attacker_adds_grid", GridRole::Adds(0)),
    ("defender_adds_grid", GridRole::Adds(1)),
    ("attacker_grid", GridRole::Tiles(0)),
    ("defender_grid", GridRole::Tiles(1)),
];

pub const GRID_FULL: &str =
    "a naval combat grid item is past its grid's slot limits, which do not add it; not drawn";

fn combat_tiles(
    context: &MarkerContext<'_>,
    naval: &NavalContext<'_>,
    sides: &[grid::SideGrid<'_>; 2],
) -> Result<[Vec<Flotilla>; 2], &'static str> {
    let mut tiles = [Vec::new(), Vec::new()];
    for (side, grid) in sides.iter().enumerate() {
        for group in &grid.groups {
            let units = group
                .task_forces
                .iter()
                .map(|member| {
                    let force = member.force;
                    Ok(NavalUnit {
                        force,
                        ordinal: 0,
                        slot: member.country,
                        logical: (context.owner)(force.logical_owner),
                        flag_owner: if (force.expeditionary_owner as i32) > 0 {
                            (context.owner)(force.expeditionary_owner)
                                .ok_or("a grid task force's expeditionary owner is unresolved")?
                        } else {
                            member.country
                        },
                        sight: if member.visibility >= 250 {
                            Sight::Full
                        } else {
                            Sight::Partial
                        },
                        byte: member.visibility,
                        ships: Some(member.ships),
                        position: None,
                        strength: naval
                            .ships
                            .get(&force.id)
                            .map_or(0, |&(_, strength)| strength),
                    })
                })
                .collect::<Result<Vec<_>, &'static str>>()?;
            tiles[side].push(naval.flotilla(&units.iter().collect::<Vec<_>>(), true, false));
        }
    }
    Ok(tiles)
}

fn result_list_contains(
    viewer: Viewer<'_>,
    countries: &BTreeMap<(u64, u8), Vec<u32>>,
    result: u64,
    list: u8,
) -> Option<bool> {
    viewer.contains_viewer(countries.get(&(result, list)).map_or(&[], Vec::as_slice))
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct ResultFacts {
    lists: [Option<bool>; 3],
    flag_a: bool,
    flag_b: bool,
    subtype: bool,
    readiness: bool,
    winning_side: u8,
    losing_side: u8,
    present: bool,
}

impl ResultFacts {
    fn visible(&self) -> Option<bool> {
        if !self.lists[2]? {
            return Some(false);
        }
        if !(self.lists[0]? || self.lists[1]?) {
            return Some(false);
        }
        Some(self.flag_a || self.flag_b || self.present)
    }

    pub fn admitted(&self) -> Option<bool> {
        self.lists[2]
    }
}

pub fn result_presentation(
    results: &[ResultFacts],
    held: usize,
) -> Result<(&'static str, u32), &'static str> {
    let sprite = if results.iter().all(|result| result.flag_b) {
        NAVAL_RESULT_SPRITES[0]
    } else if results.iter().all(|result| result.flag_a) {
        NAVAL_RESULT_SPRITES[1]
    } else if results.iter().filter(|result| result.subtype).count() <= held / 2 {
        NAVAL_RESULT_SPRITES[2]
    } else {
        NAVAL_RESULT_SPRITES[3]
    };
    let frame = if results.iter().all(|result| result.readiness) {
        2
    } else {
        let (mut won, mut lost) = (true, true);
        for result in results {
            let side = |selector: u8| match selector {
                1 | 2 => result.lists[usize::from(selector - 1)],
                _ => Some(false),
            };
            match (side(result.winning_side), side(result.losing_side)) {
                (Some(winner), Some(loser)) if winner != loser => {
                    if winner {
                        lost = false;
                    } else {
                        won = false;
                    }
                }
                (Some(_), Some(_)) => {}
                _ => return Err("a retained naval result's country is unresolved"),
            }
        }
        match (won, lost) {
            (false, true) => 0,
            (true, false) => 1,
            _ => 2,
        }
    };
    Ok((sprite, frame))
}

#[cfg(test)]
pub fn convoy_result_for_test() -> ResultFacts {
    ResultFacts {
        lists: [Some(true), Some(false), Some(true)],
        subtype: true,
        winning_side: 1,
        losing_side: 2,
        present: true,
        ..Default::default()
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct ResultTables {
    pub provinces: BTreeMap<u32, Vec<u64>>,
    pub facts: BTreeMap<u64, ResultFacts>,
    pub undecided: Option<&'static str>,
}

impl ResultTables {
    pub fn visible(&self, province: u32) -> Result<Vec<u64>, &'static str> {
        if let Some(why) = self.undecided {
            return Err(why);
        }
        let mut visible = Vec::new();
        for id in self.provinces.get(&province).into_iter().flatten() {
            match self.facts.get(id).and_then(ResultFacts::visible) {
                Some(true) => visible.push(*id),
                Some(false) => {}
                None => return Err("a retained naval result's country is unresolved"),
            }
        }
        Ok(visible)
    }
}

pub fn result_tables(frame: &DecodedFrame, viewer: Viewer<'_>) -> io::Result<ResultTables> {
    let results = rows::<tables::NavalResults>(frame)?;
    let members = rows::<tables::NavalResultMembers>(frame)?;
    let country_rows = rows::<tables::NavalResultCountries>(frame)?;
    let countries = result_country_lists(country_rows.rows().iter().map(|row| (row.result, row.list, row.ordinal, row.country)));
    let present: BTreeSet<u64> = members
        .rows()
        .iter()
        .filter(|member| member.presence_value != 0)
        .map(|member| member.result)
        .collect();
    let fields: Vec<ResultFields> = results.rows().iter().map(ResultFields::of).collect();
    Ok(result_values(viewer, &fields, &countries, &present, results.is_complete() && members.is_complete() && country_rows.is_complete()))
}

fn result_values(
    viewer: Viewer<'_>,
    results: &[ResultFields],
    countries: &BTreeMap<(u64, u8), Vec<u32>>,
    present: &BTreeSet<u64>,
    complete: bool,
) -> ResultTables {
    let mut by_province: BTreeMap<u32, Vec<&ResultFields>> = BTreeMap::new();
    for result in results {
        by_province.entry(result.province).or_default().push(result);
    }
    let facts = results
        .iter()
        .map(|result| {
            let list = |list| result_list_contains(viewer, countries, result.id, list);
            (
                result.id,
                ResultFacts {
                    lists: [list(0), list(1), list(2)],
                    flag_a: result.flag_a != 0,
                    flag_b: result.flag_b != 0,
                    subtype: result.subtype != 0,
                    readiness: result.readiness != 0,
                    winning_side: result.winning_side,
                    losing_side: result.losing_side,
                    present: present.contains(&result.id),
                },
            )
        })
        .collect();
    ResultTables {
        provinces: by_province
            .into_iter()
            .map(|(province, mut results)| {
                results.sort_unstable_by_key(|result| result.province_ordinal);
                (province, results.iter().map(|result| result.id).collect())
            })
            .collect(),
        facts,
        undecided: (!complete).then_some("the retained naval results or their members or countries are incomplete"),
    }
}

const REFUSED_RESULT_WINDOW: &str =
    "the naval result window or its buttonType bg_btn is refused or not declared";

pub fn result_anchor(
    inputs: &CounterInputs<'_>,
    raised: &BTreeSet<u32>,
    whole: bool,
    province: u32,
) -> Result<[f32; 3], &'static str> {
    match inputs.content.province_kind(province) {
        Some(ProvinceKind::Land) => inputs
            .content
            .anchor(province, SHIP_IN_PORT)
            .ok_or("the content places no unit-stack slot 19 in the result's land province"),
        Some(_) => {
            let raised = raised.contains(&province);
            if !raised && !whole {
                return Err("a sea province's combat vector is unknown");
            }
            inputs
                .content
                .anchor(province, STANDSTILL)
                .map(|mut anchor| {
                    if raised {
                        anchor[2] += 5.0;
                    }
                    anchor
                })
                .ok_or("the content places no Standstill anchor in the result's province")
        }
        None => Err("the map does not define the result's province"),
    }
}

fn refused_results(tables: &ResultTables, report: &mut CounterReport) {
    for &province in tables.provinces.keys() {
        if tables
            .visible(province)
            .is_ok_and(|visible| visible.is_empty())
        {
            continue;
        }
        report.unavailable_naval_results += 1;
        gap(&mut report.naval_result_gaps, REFUSED_RESULT_WINDOW);
    }
}

fn result_pass(inputs: &CounterInputs<'_>, tables: &NavalMarkerTables, out: &mut CounterFrame<impl Sized, base_windows::PreparedBases, impl Sized, naval_windows::Plan>) {
    if !inputs.visibility.province_icons
        || tables.results.provinces.is_empty() && tables.results.undecided.is_none()
    {
        return;
    }
    let declared = inputs
        .windows
        .naval_result()
        .is_some_and(|window| window.find(native::Kind::Button, "bg_btn").is_some());
    if !declared {
        refused_results(&tables.results, &mut out.report);
        return;
    }
    let size = match inputs
        .windows
        .naval_result_frame(viewport(inputs.viewport, inputs.ui_scale))
    {
        None => {
            refused_results(&tables.results, &mut out.report);
            return;
        }
        Some(Err(reason)) => {
            *out.report
                .naval_window_notes
                .entry(format!("the naval result icons are not placed: {reason}").into())
                .or_default() += 1;
            out.naval.results_undecided = Some(reason.into());
            return;
        }
        Some(Ok(frame)) => frame.size,
    };
    out.naval.results = Some(naval_windows::ResultPass {
        tables: Arc::clone(&tables.results),
        raised: tables.vectors.iter().copied().collect(),
        vectors_whole: tables.direct_complete,
        size,
    });
}

#[cfg(test)]
mod tests;
