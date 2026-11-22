
use dashboard_base::products::{BranchParams, LoadContext, Product};
use dashboard_base::{Source, catalogue::{self, Catalogue}, products::at_hour::{At, AtHour, Read}};
use dashboard_follow::live::{Cells, Source as Cell};
use dashboard_query::identity::{Holder, Holders};
use replay_data::rows::{AirTheatreGroupRow, AirTheatreMemberRow, AirTheatreRow, AirWingRow, CountryLinkRow, RaidAirScoreRow, TableId, tables};
use replay_frame::frame::DecodedFrame;
use serde::{Deserialize, Serialize};
use std::{
    collections::{BTreeMap, BTreeSet, HashMap, HashSet},
    io,
    sync::{Arc, atomic::AtomicBool},
};
#[cfg(any(test, feature = "export-types"))]
use ts_rs::TS;

pub use replay_derive::air_superiority::{assignment, compose};
pub mod index;
mod ranking;
pub mod timeline;

#[cfg(test)]
mod tests;

use assignment::AssignedSide;
use compose::{Compose, Operands, Pair, Roster, SidesCell, View};

pub const SCALE: i32 = 100_000;
pub const BAD_BELOW: i32 = 40_000;
pub const GOOD_ABOVE: i32 = 60_000;

#[derive(Clone, Copy, Debug, Default, Deserialize, Serialize, PartialEq, Eq)]
#[cfg_attr(any(test, feature = "export-types"), derive(TS))]
#[serde(rename_all = "lowercase")]
pub enum AirMode {
    #[default]
    Sides,
    Viewpoint,
}

#[derive(Clone, Debug, PartialEq, Default, Deserialize, Serialize)]
#[cfg_attr(any(test, feature = "export-types"), derive(TS))]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct AirSuperiorityParams {
    pub branch_id: String,
    #[cfg_attr(any(test, feature = "export-types"), ts(optional))]
    pub mode: Option<AirMode>,
    #[doc = " A side's key, or a country (its slot or tag) with `viewpointCountry`."]
    #[cfg_attr(any(test, feature = "export-types"), ts(optional))]
    pub viewpoint: Option<dashboard_base::products::Country>,
    #[cfg_attr(any(test, feature = "export-types"), ts(optional))]
    pub viewpoint_country: Option<bool>,
    #[doc = " The side keys compared; all when none."]
    #[cfg_attr(any(test, feature = "export-types"), ts(optional))]
    pub sides: Option<Vec<String>>,
    #[doc = " The regions shown; all active when none."]
    #[cfg_attr(any(test, feature = "export-types"), ts(optional))]
    pub regions: Option<Vec<u32>>,
    #[doc = " The air theatres whose regions are shown; all when none."]
    #[cfg_attr(any(test, feature = "export-types"), ts(optional))]
    pub theatres: Option<Vec<String>>,
}

pub(crate) fn viewpoint_name<'a>(viewpoint: Option<&'a dashboard_base::products::Country>, country: bool, holders: &'a Holders, hour: i32) -> &'a str {
    match viewpoint {
        Some(viewpoint) if country => holders.tag_at(viewpoint, hour).unwrap_or_default(),
        Some(viewpoint) => viewpoint.tag().unwrap_or_default(),
        None => "",
    }
}

impl AirSuperiorityParams {
    pub fn validate(&self) -> Result<(), &'static str> {
        self.branch_id.parse::<u64>().map_err(|_| "Invalid air branch")?;
        if self.mode == Some(AirMode::Viewpoint) && self.viewpoint.as_ref().is_none_or(|viewpoint| viewpoint.tag() == Some("")) {
            return Err("Viewpoint country or side is required");
        }
        Ok(())
    }

    fn side_keys(&self) -> Option<HashSet<&str>> {
        self.sides.as_ref().filter(|keys| !keys.is_empty()).map(|keys| keys.iter().map(String::as_str).collect())
    }

    fn theatre_ids(&self) -> Option<&[String]> {
        self.theatres.as_deref().filter(|ids| !ids.is_empty())
    }
}

#[derive(Clone, Debug, Serialize, PartialEq, Eq)]
#[cfg_attr(any(test, feature = "export-types"), derive(TS))]
#[serde(rename_all = "camelCase")]
pub struct AirAnchor {
    pub tag: String,
    pub hour: i32,
}

#[derive(Clone, Debug, Serialize, PartialEq, Eq)]
#[cfg_attr(any(test, feature = "export-types"), derive(TS))]
#[serde(rename_all = "camelCase")]
pub struct AirSide {
    pub key: String,
    #[doc = " Its anchor faction's id."]
    pub faction: Option<String>,
    #[doc = " The faction whose colour it shows: one a strict majority of its fighting members belong to."]
    pub colour_faction: Option<String>,
    #[doc = " Every faction merged into it, by id."]
    pub factions: Vec<String>,
    #[doc = " Its first member, for a side of no faction."]
    pub tag: Option<String>,
    pub leader: Option<String>,
    pub co_belligerents: usize,
    pub anchor: Option<AirAnchor>,
}

#[derive(Clone, Debug, Serialize, PartialEq, Eq)]
#[cfg_attr(any(test, feature = "export-types"), derive(TS))]
#[serde(rename_all = "camelCase")]
pub struct AirPart {
    pub side: u32,
    pub tag: String,
    pub live: String,
}

#[derive(Clone, Debug, Serialize, PartialEq, Eq)]
#[cfg_attr(any(test, feature = "export-types"), derive(TS))]
#[serde(tag = "kind", rename_all = "camelCase")]
pub enum AirCell {
    Reason { reason: String },
    Sides(AirSidesCell),
    View { side: Option<u32>, leader: Option<String>, live: Option<String> },
}

#[derive(Clone, Debug, Serialize, PartialEq, Eq)]
#[cfg_attr(any(test, feature = "export-types"), derive(TS))]
#[serde(rename_all = "camelCase")]
pub struct AirSidesCell {
    pub parts: Vec<AirPart>,
    #[doc = " A leaderless side and the country whose read it shows."]
    pub viewpoints: Vec<(u32, String)>,
    pub unknown: Vec<u32>,
    pub no_air_object: Vec<u32>,
    #[doc = " Only the chosen sides are compared."]
    pub filtered: bool,
    #[doc = " Sides of the fallback left out: no air object."]
    pub excluded: usize,
}

#[derive(Clone, Copy, Debug, Serialize, PartialEq, Eq)]
#[cfg_attr(any(test, feature = "export-types"), derive(TS))]
#[serde(rename_all = "camelCase")]
pub enum AirWings {
    Live,
    None,
    Unknown,
}

#[derive(Clone, Debug, Serialize, PartialEq, Eq)]
#[cfg_attr(any(test, feature = "export-types"), derive(TS))]
#[serde(rename_all = "camelCase")]
pub struct AirRegion {
    pub id: u32,
    pub name: Option<String>,
    #[doc = " Why the name is not shown."]
    pub name_missing: Option<String>,
    #[doc = " Any country scores in it."]
    pub active: bool,
    pub cell: AirCell,
    pub wings: AirWings,
}

#[derive(Clone, Debug, PartialEq, Serialize)]
#[cfg_attr(any(test, feature = "export-types"), derive(TS))]
#[serde(rename_all = "camelCase")]
pub struct AirSuperiorityReply {
    #[doc = " Why no comparison is shown at the hour."]
    pub missing: Option<String>,
    #[doc = " The sides the cells name, by index."]
    pub sides: Vec<AirSide>,
    #[doc = " The viewpoint side's index, when a side is the viewpoint and present."]
    pub viewpoint: Option<u32>,
    pub regions: Vec<AirRegion>,
    #[doc = " The comparison of a region no country scores in."]
    pub fallback: AirCell,
    pub scale: i32,
    pub bad_below: i32,
    pub good_above: i32,
    pub warnings: Vec<String>,
}

#[derive(Clone, Debug, Serialize, PartialEq, Eq)]
#[cfg_attr(any(test, feature = "export-types"), derive(TS))]
#[serde(rename_all = "camelCase")]
pub struct AirChoice {
    pub id: String,
    pub name: String,
}

#[derive(Clone, Debug, PartialEq, Serialize)]
#[cfg_attr(any(test, feature = "export-types"), derive(TS))]
#[serde(rename_all = "camelCase")]
pub struct AirChoicesReply {
    #[doc = " Every strategic region of the recorded stack, by id."]
    pub regions: Vec<AirChoice>,
    #[doc = " Every side the timeline assigns, as last assigned."]
    pub sides: Vec<AirSide>,
    #[doc = " The air theatres at the timeline's last hour, by id."]
    pub theatres: Vec<AirChoice>,
    #[doc = " Why the theatres are not listed."]
    pub theatres_missing: Option<String>,
    #[doc = " Every tag held on the timeline."]
    pub countries: Vec<String>,
    pub warnings: Vec<String>,
}

dashboard_base::reply_warnings!(AirSuperiorityReply, AirChoicesReply);

pub struct Names {
    catalogue: Result<Arc<Catalogue>, String>,
    regions: Result<Arc<content_reader_regions::regions::StrategicRegions>, String>,
    warnings: Vec<String>,
}

fn names(source: &Source, load: &LoadContext) -> Names {
    let mut warnings = Vec::new();
    let cat = load.content().and_then(|options| catalogue::catalogue(source, options)).map_err(|error| format!("Recorded content stack unavailable; choose matching game and user folders ({error})"));
    let regions = cat.as_ref().map_err(Clone::clone).and_then(|cat| {
        warnings.extend(cat.verification.iter().cloned());
        warnings.extend(cat.warnings.iter().cloned());
        let content = dashboard_base::content::strategic_regions(&cat.content.layers).map_err(|error| error.to_string())?;
        warnings.extend(content.notes.iter().cloned());
        Ok(content)
    });
    if let Err(why) = &regions {
        warnings.push(format!("Air region names are not shown: {why}"));
    }
    Names { catalogue: cat, regions, warnings }
}

fn region_name(names: &Names, region: u32) -> (Option<String>, Option<String>) {
    let named = names.catalogue.as_ref().map_err(Clone::clone).and_then(|cat| {
        let regions = names.regions.as_ref().map_err(Clone::clone)?;
        dashboard_world::products::region_names::region_name(cat, regions, region)
    });
    match named {
        Ok(name) => (Some(name), None),
        Err(why) => (None, Some(why)),
    }
}

pub struct AirContext {
    names: Names,
    index: Result<Arc<index::AirIndex>, String>,
}

pub(crate) struct HourScores {
    pub operands: Operands,
    pub pairs: BTreeMap<u32, Vec<Pair>>,
    pub active: BTreeSet<u32>,
}

fn slot_tags(holders: &Holders, hour: i32) -> HashMap<u32, &str> {
    holders
        .tags
        .iter()
        .filter_map(|(tag, _)| match holders.at(tag, hour)?.holder {
            Holder::Found { slot, .. } => Some((slot, tag.as_str())),
            Holder::Unresolved => None,
        })
        .collect()
}

pub(crate) fn hour_scores(rows: &[RaidAirScoreRow], complete: bool, links: Option<&[CountryLinkRow]>, holders: &Holders, present: &Arc<BTreeSet<String>>, hour: i32) -> HourScores {
    let mut operands = Operands { complete, present: Arc::clone(present), ..Operands::default() };
    let mut active = BTreeSet::new();
    for row in rows {
        if row.friendly > 0 || row.enemy > 0 {
            active.insert(row.region);
        }
    }
    let unknown = holders.unknown.iter().any(|(first, last, _)| *first <= hour && hour <= *last);
    let Some(links) = links.filter(|_| !unknown) else {
        return HourScores { operands, pairs: BTreeMap::new(), active };
    };
    operands.identity_known = true;
    let slots = slot_tags(holders, hour);
    let owners: HashMap<u32, &str> = links.iter().filter_map(|link| Some((link.id, *slots.get(&link.canonical_slot)?))).filter(|(_, tag)| present.contains(*tag)).collect();
    let mut supplied: BTreeMap<(u32, &str), &RaidAirScoreRow> = BTreeMap::new();
    for row in rows {
        let Some(&tag) = owners.get(&row.country) else {
            operands.unresolved = true;
            continue;
        };
        operands.witnessed.insert(tag.to_owned());
        if row.friendly < 0 || row.enemy < 0 {
            operands.invalid.insert(tag.to_owned());
            continue;
        }
        supplied.entry((row.region, tag)).and_modify(|old| if row.country > old.country { *old = row }).or_insert(row);
    }
    let mut pairs: BTreeMap<u32, Vec<Pair>> = BTreeMap::new();
    for ((region, tag), row) in supplied {
        pairs.entry(region).or_default().push(Pair { tag: tag.to_owned(), raw: row.country, friendly: row.friendly, enemy: row.enemy });
    }
    HourScores { operands, pairs, active }
}

pub(crate) fn theatre_regions(theatres: &[AirTheatreRow], groups: &[AirTheatreGroupRow], members: &[AirTheatreMemberRow], wings: &[AirWingRow]) -> Result<BTreeMap<String, BTreeSet<u32>>, String> {
    let groups: HashMap<u64, u64> = groups.iter().map(|group| (group.id, group.theatre)).collect();
    let regions: HashMap<u64, Option<u32>> = wings.iter().map(|wing| (wing.id, wing.mission_region)).collect();
    let mut out: BTreeMap<String, BTreeSet<u32>> = theatres.iter().map(|theatre| (theatre.id.to_string(), BTreeSet::new())).collect();
    for member in members {
        let theatre = groups.get(&member.group).ok_or("Air-theatre member's group is not recorded")?;
        let region = regions.get(&member.wing).ok_or("Air-theatre member's wing is not recorded")?;
        if let Some(region) = region {
            out.entry(theatre.to_string()).or_default().insert(*region);
        }
    }
    Ok(out)
}

const THEATRE_TABLES: [TableId; 3] = [TableId::AirTheatres, TableId::AirTheatreGroups, TableId::AirTheatreMembers];

fn member_wings(held: &DecodedFrame) -> Option<Read> {
    let members = held.complete::<tables::AirTheatreMembers>().ok()?;
    (!members.is_empty()).then(|| Read::these(TableId::AirWings, members.iter().map(|member| u128::from(member.wing))))
}

fn selected_regions(frame: &DecodedFrame, ids: &[String]) -> Result<HashSet<u32>, ()> {
    let read = || -> io::Result<_> { Ok((frame.complete::<tables::AirTheatres>()?, frame.complete::<tables::AirTheatreGroups>()?, frame.complete::<tables::AirTheatreMembers>()?)) };
    let (theatres, groups, members) = read().map_err(|_| ())?;
    let wings: &[AirWingRow] = if members.is_empty() { &[] } else { frame.complete::<tables::AirWings>().map_err(|_| ())? };
    let regions = theatre_regions(theatres, groups, members, wings).map_err(|_| ())?;
    Ok(ids.iter().filter_map(|id| regions.get(id)).flatten().copied().collect())
}

fn side_of(index: &index::AirIndex, side: &AssignedSide) -> AirSide {
    AirSide {
        key: side.key.clone(),
        faction: side.faction.map(|id| id.to_string()),
        colour_faction: side.colour_faction.map(|id| id.to_string()),
        factions: side.factions.iter().map(ToString::to_string).collect(),
        tag: side.factions.is_empty().then(|| side.members.first().cloned()).flatten(),
        leader: side.leader.clone(),
        co_belligerents: side.co_belligerents,
        anchor: index.anchor(side).map(|(tag, hour)| AirAnchor { tag, hour }),
    }
}

struct Dictionary<'a> {
    sides: &'a [AssignedSide],
    keep: Option<HashSet<&'a str>>,
    at: BTreeMap<u32, u32>,
}

impl<'a> Dictionary<'a> {
    fn kept(&self, position: u32) -> bool {
        self.keep.as_ref().is_none_or(|keys| keys.contains(self.sides[position as usize].key.as_str()))
    }

    fn note(&mut self, position: u32) {
        self.at.insert(position, 0);
    }

    fn select(&mut self, cell: compose::Cell) -> compose::Cell {
        let filtered = self.keep.is_some();
        match cell {
            compose::Cell::Sides(cell) => {
                let SidesCell { parts, viewpoints, unknown, no_air_object, excluded, .. } = cell;
                let cell = SidesCell {
                    parts: parts.into_iter().filter(|(position, ..)| self.kept(*position)).collect(),
                    viewpoints: viewpoints.into_iter().filter(|(position, _)| self.kept(*position)).collect(),
                    unknown: unknown.into_iter().filter(|position| self.kept(*position)).collect(),
                    no_air_object: no_air_object.into_iter().filter(|position| self.kept(*position)).collect(),
                    filtered,
                    excluded,
                };
                for position in cell.parts.iter().map(|(position, ..)| *position).chain(cell.unknown.iter().copied()).chain(cell.no_air_object.iter().copied()) {
                    self.note(position);
                }
                compose::Cell::Sides(cell)
            }
            compose::Cell::View { side: Some(position), leader, row } => {
                self.note(position);
                compose::Cell::View { side: Some(position), leader, row }
            }
            other => other,
        }
    }

    fn finish(&mut self) {
        for (at, index) in self.at.values_mut().enumerate() {
            *index = at as u32;
        }
    }

    fn index(&self, position: u32) -> u32 {
        self.at[&position]
    }
}

fn live_row(tag: &str, region: u32) -> String {
    format!("{tag}@{region}")
}

fn shown(cell: compose::Cell, region: u32, dictionary: &Dictionary<'_>, live: &Cells, columns: (usize, usize), enemy: bool) -> AirCell {
    let score = |tag: &str, raw: u32| {
        let row = live_row(tag, region);
        let key = u128::from(raw) << 32 | u128::from(region);
        live.value(&row, "friendly", Cell { table: TableId::RaidAirScores, column: columns.0, key }, 1.0);
        if enemy {
            live.value(&row, "enemy", Cell { table: TableId::RaidAirScores, column: columns.1, key }, 1.0);
        }
        row
    };
    match cell {
        compose::Cell::Reason(reason) => AirCell::Reason { reason: reason.to_owned() },
        compose::Cell::Sides(cell) => AirCell::Sides(AirSidesCell {
            parts: cell.parts.iter().map(|(position, tag, raw)| AirPart { side: dictionary.index(*position), tag: tag.clone(), live: score(tag, *raw) }).collect(),
            viewpoints: cell.viewpoints.iter().filter(|(position, _)| dictionary.at.contains_key(position)).map(|(position, tag)| (dictionary.index(*position), tag.clone())).collect(),
            unknown: cell.unknown.iter().map(|position| dictionary.index(*position)).collect(),
            no_air_object: cell.no_air_object.iter().map(|position| dictionary.index(*position)).collect(),
            filtered: cell.filtered,
            excluded: cell.excluded,
        }),
        compose::Cell::View { side, leader, row } => AirCell::View {
            side: side.map(|position| dictionary.index(position)),
            live: row.zip(leader.as_deref()).map(|(raw, tag)| score(tag, raw)),
            leader,
        },
    }
}

fn moment(frame: &DecodedFrame, params: &AirSuperiorityParams, holders: &Holders, hour: i32, context: &AirContext, live: &Cells) -> io::Result<AirSuperiorityReply> {
    let mut warnings = context.names.warnings.clone();
    let index = context.index.as_ref().map_err(|why| dashboard_core::refused(why.clone()))?;
    let reply = |missing: String, warnings: Vec<String>| AirSuperiorityReply {
        missing: Some(missing),
        sides: Vec::new(),
        viewpoint: None,
        regions: Vec::new(),
        fallback: AirCell::Reason { reason: "scores-unavailable".into() },
        scale: SCALE,
        bad_below: BAD_BELOW,
        good_above: GOOD_ABOVE,
        warnings,
    };
    let rows = match dashboard_text::metadata::recorded_rows::<tables::RaidAirScores>(frame) {
        Ok(rows) => rows,
        Err(error) => return Ok(reply(format!("Air scores are not recorded at this hour: {error}"), warnings)),
    };
    let complete = frame.complete::<tables::RaidAirScores>().is_ok();
    if !complete && rows.is_empty() {
        return Ok(reply("Air scores were not available at this hour".into(), warnings));
    }
    let epoch = index.epoch(hour);
    let empty = Arc::default();
    let present = epoch.map_or(&empty, |epoch| &epoch.present);
    let scores = hour_scores(rows, complete, frame.complete::<tables::CountryLinks>().ok(), holders, present, hour);
    let sides: &[AssignedSide] = epoch.and_then(|epoch| epoch.sides.as_deref()).map_or(&[], Vec::as_slice);
    let roster = epoch.and_then(|epoch| epoch.sides.as_ref()).map(|sides| Roster::new(sides));
    let viewpoint = viewpoint_name(params.viewpoint.as_ref(), params.viewpoint_country == Some(true), holders, hour);
    let view = match params.mode.unwrap_or_default() {
        AirMode::Sides => View::Sides,
        AirMode::Viewpoint if params.viewpoint_country == Some(true) => View::Country(viewpoint),
        AirMode::Viewpoint => View::Side(sides.iter().position(|side| side.key == viewpoint).map(|at| at as u32)),
    };
    let compose = Compose::new(roster.as_ref(), &scores.operands, view);
    let theatres = params.theatre_ids().map(|ids| selected_regions(frame, ids));
    let mut dictionary = Dictionary { sides, keep: params.side_keys(), at: BTreeMap::new() };
    let viewpoint_side = match view {
        View::Side(Some(position)) => {
            dictionary.note(position);
            Some(position)
        }
        _ => None,
    };
    let wings_rows = frame.complete::<tables::AirRegionWings>();
    let wings: Option<BTreeMap<u32, i64>> = wings_rows.as_ref().ok().map(|rows| rows.iter().map(|row| (row.region, row.wings)).collect());
    let chosen: Option<BTreeSet<u32>> = params.regions.as_ref().map(|regions| regions.iter().copied().collect());
    let mut ids: BTreeSet<u32> = scores.pairs.keys().chain(&scores.active).copied().collect();
    ids.extend(wings.iter().flat_map(|wings| wings.iter().filter(|(_, count)| **count != 0).map(|(region, _)| *region)));
    if let Some(chosen) = &chosen {
        ids.retain(|region| chosen.contains(region));
        ids.extend(chosen.iter().copied());
    }
    let filter = |region: Option<u32>| match &theatres {
        None => None,
        Some(Err(())) => Some("theatres-not-recorded"),
        Some(Ok(selected)) if region.is_none_or(|region| !selected.contains(&region)) => Some("outside-theatres"),
        Some(Ok(_)) => None,
    };
    let mut cells = Vec::with_capacity(ids.len());
    for &region in &ids {
        let cell = match filter(Some(region)) {
            Some(reason) => compose::Cell::Reason(reason),
            None => dictionary.select(compose.cell(scores.pairs.get(&region).map(Vec::as_slice))),
        };
        cells.push((region, cell));
    }
    let fallback = match filter(None) {
        Some(reason) => compose::Cell::Reason(reason),
        None => dictionary.select(compose.cell(None)),
    };
    dictionary.finish();
    let columns = (dashboard_declare::field(TableId::RaidAirScores, "friendly")?, dashboard_declare::field(TableId::RaidAirScores, "enemy")?);
    let wings_column = dashboard_declare::field(TableId::AirRegionWings, "wings")?;
    let enemy = !matches!(view, View::Sides);
    let regions = cells
        .into_iter()
        .map(|(region, cell)| {
            let (name, name_missing) = region_name(&context.names, region);
            let wings = match wings.as_ref().map(|wings| wings.get(&region)) {
                Some(Some(_)) => {
                    live.value(&region.to_string(), "wings", Cell { table: TableId::AirRegionWings, column: wings_column, key: u128::from(region) }, 1.0);
                    AirWings::Live
                }
                Some(None) => AirWings::None,
                None => AirWings::Unknown,
            };
            AirRegion { id: region, name, name_missing, active: scores.active.contains(&region), cell: shown(cell, region, &dictionary, live, columns, enemy), wings }
        })
        .collect();
    let fallback = shown(fallback, 0, &dictionary, live, columns, enemy);
    if let Err(error) = &wings_rows {
        warnings.push(format!("Air wings by region are not known at this hour: {error}"));
    }
    Ok(AirSuperiorityReply {
        missing: None,
        sides: dictionary.at.keys().map(|position| side_of(index, &sides[*position as usize])).collect(),
        viewpoint: viewpoint_side.map(|position| dictionary.index(position)),
        regions,
        fallback,
        scale: SCALE,
        bad_below: BAD_BELOW,
        good_above: GOOD_ABOVE,
        warnings,
    })
}

pub struct AirSuperiorityAt;

impl AtHour for AirSuperiorityAt {
    const KEY: &'static str = "air-superiority";
    const TITLE: &'static str = "Air superiority";
    const CONTENT_DEPENDENT: bool = true;
    type Params = AirSuperiorityParams;
    const FIELDS: dashboard_base::products::Fields = dashboard_base::fields![branchId, mode, viewpoint, viewpointCountry, sides, regions, theatres, "regions[]"];
    type Reply = AirSuperiorityReply;
    type Context = AirContext;

    fn validate(params: &Self::Params) -> Result<(), &'static str> {
        params.validate()
    }

    #[cfg(any(test, feature = "export-types"))]
    fn types() -> Vec<String> {
        vec![
            AirSuperiorityParams::decl(),
            AirSuperiorityReply::decl(),
            AirMode::decl(),
            AirAnchor::decl(),
            AirSide::decl(),
            AirPart::decl(),
            AirCell::decl(),
            AirSidesCell::decl(),
            AirWings::decl(),
            AirRegion::decl(),
        ]
    }

    fn tables(_params: &Self::Params) -> Vec<TableId> {
        vec![
            TableId::RaidAirScores,
            TableId::AirRegionWings,
            TableId::CountryLinks,
            TableId::AirTheatres,
            TableId::AirTheatreGroups,
            TableId::AirTheatreMembers,
            TableId::AirWings,
        ]
    }

    fn reads(params: &Self::Params, _owner: Option<dashboard_base::products::at_hour::Owner>, step: usize, held: &DecodedFrame) -> Vec<Read> {
        let theatres = params.theatre_ids().is_some();
        match step {
            0 => {
                let mut reads = vec![Read::all(TableId::RaidAirScores), Read::all(TableId::AirRegionWings), Read::all(TableId::CountryLinks)];
                if theatres {
                    reads.extend(THEATRE_TABLES.into_iter().map(Read::all));
                }
                reads
            }
            1 if theatres => member_wings(held).into_iter().collect(),
            _ => Vec::new(),
        }
    }

    fn columns(table: TableId) -> Option<Vec<usize>> {
        (table == TableId::RaidAirScores).then(|| ["friendly", "enemy"].iter().filter_map(|name| dashboard_declare::field(table, name).ok()).collect())
    }

    fn context(source: &Source, load: &LoadContext) -> io::Result<Self::Context> {
        Ok(AirContext { names: names(source, load), index: Err("Air superiority needs its timeline".into()) })
    }

    fn context_for(source: &Source, load: &LoadContext, params: &Self::Params) -> io::Result<Self::Context> {
        let branch = params.branch_id.parse::<u64>().map_err(|_| dashboard_core::invalid("Invalid air branch"))?;
        Ok(AirContext { names: names(source, load), index: index::index(source, branch).map_err(|error| format!("Air sides are not known: {error}")) })
    }

    fn reply(frame: &DecodedFrame, params: &Self::Params, at: &At<'_, Self::Context>) -> io::Result<Self::Reply> {
        moment(frame, params, at.holders, at.hour, at.context, at.live)
    }
}

pub struct AirSuperiorityChoices;

impl Product for AirSuperiorityChoices {
    const KEY: &'static str = "air-superiority-choices";
    const CONTENT_DEPENDENT: bool = true;
    type Params = BranchParams;
    const FIELDS: dashboard_base::products::Fields = dashboard_base::fields![branchId];
    type Reply = AirChoicesReply;

    #[cfg(any(test, feature = "export-types"))]
    fn types() -> Vec<String> {
        vec![AirChoice::decl(), AirChoicesReply::decl()]
    }

    fn validate(params: &Self::Params) -> Result<(), &'static str> {
        params.validate()
    }

    fn load(source: &Source, params: &Self::Params, context: &LoadContext, cancelled: &AtomicBool) -> io::Result<Self::Reply> {
        let branch = params.branch();
        let query = dashboard_base::queries::query(source, branch)?;
        let index = index::index(source, branch)?;
        dashboard_text::metadata::check_cancel(cancelled)?;
        let names = names(source, context);
        let mut warnings = names.warnings.clone();
        let regions = match &names.regions {
            Ok(regions) => regions.regions.keys().map(|&id| AirChoice { id: id.to_string(), name: region_name(&names, id).0.unwrap_or_else(|| format!("Region {id}")) }).collect(),
            Err(_) => Vec::new(),
        };
        let mut last: BTreeMap<&str, &AssignedSide> = BTreeMap::new();
        for side in index.epochs.iter().filter_map(|epoch| epoch.sides.as_deref()).flatten() {
            last.insert(&side.key, side);
        }
        let sides = last.values().map(|side| side_of(&index, side)).collect();
        let (theatres, theatres_missing) = match theatres_at_end(&query, &names) {
            Ok(theatres) => (theatres, None),
            Err(why) => (Vec::new(), Some(why)),
        };
        if let Some(why) = &theatres_missing {
            warnings.push(why.clone());
        }
        Ok(AirChoicesReply { regions, sides, theatres, theatres_missing, countries: query.holders.tags.keys().cloned().collect(), warnings })
    }
}

fn theatres_at_end(query: &dashboard_query::Query<'_>, names: &Names) -> Result<Vec<AirChoice>, String> {
    let frame = dashboard_base::products::at_hour::frame(query, query.timeline.last, &[Read::all(TableId::AirTheatres)], "Air theatres").map_err(|error| error.to_string())?;
    let theatres = frame.complete::<tables::AirTheatres>().map_err(|error| format!("Air theatres are not recorded at the timeline's end: {error}"))?;
    let mut theatres: Vec<&AirTheatreRow> = theatres.iter().collect();
    theatres.sort_by_key(|theatre| (theatre.ordinal, theatre.id));
    Ok(theatres
        .into_iter()
        .map(|theatre| AirChoice {
            id: theatre.id.to_string(),
            name: super::air_missions::stored_in(&names.catalogue, &theatre.name, "Air theatre name").unwrap_or_else(|_| format!("Air theatre {} (name not recorded)", theatre.id)),
        })
        .collect())
}
