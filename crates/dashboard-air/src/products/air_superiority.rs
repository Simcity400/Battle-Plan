
use dashboard_base::products::{BranchParams, LoadContext, Product, regions::{ActiveRegions, NoRegions, no_regions}};
use dashboard_base::{Source, catalogue::{self, Catalogue}, products::at_hour::{self, At, AtHour, Read}};
use dashboard_follow::live::{Cells, Source as Cell};
use dashboard_query::identity::{Holder, Holders};
use dashboard_world::products::{groups::{self, FactionGroup}, styles::{BranchLooks, LookResolver, ResolvedCountryLook}};
use replay_data::rows::{AirTheatreGroupRow, AirTheatreMemberRow, AirTheatreRow, AirWingRow, CountryLinkRow, RaidAirScoreRow, TableId, tables};
use replay_frame::frame::DecodedFrame;
use serde::{Deserialize, Serialize};
use std::{
    collections::{BTreeMap, BTreeSet, HashMap, HashSet},
    io,
    sync::{Arc, atomic::AtomicBool},
};
#[cfg(any(test, feature = "export-types"))]
use dashboard_base::span::HourSpan;
#[cfg(any(test, feature = "export-types"))]
use ts_rs::TS;

pub use replay_derive::air_superiority::{assignment, compose};
pub mod index;
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

#[derive(Clone, Copy, Debug, Default, Deserialize, Serialize, PartialEq, Eq)]
#[cfg_attr(any(test, feature = "export-types"), derive(TS))]
#[serde(rename_all = "lowercase")]
pub enum AirRegionOrder {
    #[default]
    Contested,
    Active,
    Name,
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
    #[doc = " The user's regions, shown first in this order whether active or not."]
    #[cfg_attr(any(test, feature = "export-types"), ts(optional))]
    pub pinned_regions: Option<Vec<u32>>,
    #[doc = " Regions the user removed from the automatic set."]
    #[cfg_attr(any(test, feature = "export-types"), ts(optional))]
    pub hidden_regions: Option<Vec<u32>>,
    #[doc = " The air theatres whose regions are shown; all when none."]
    #[cfg_attr(any(test, feature = "export-types"), ts(optional))]
    pub theatres: Option<Vec<String>>,
    #[doc = " Region ordering; omitted preserves region-id order."]
    #[cfg_attr(any(test, feature = "export-types"), ts(optional))]
    pub order: Option<AirRegionOrder>,
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
pub struct AirCountryStyle {
    pub tag: String,
    pub name: Option<String>,
    pub name_missing: Option<String>,
    pub color: Option<[u8; 3]>,
    pub color_missing: Option<String>,
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
    #[cfg_attr(any(test, feature = "export-types"), ts(optional))]
    pub color: Option<[u8; 3]>,
    #[cfg_attr(any(test, feature = "export-types"), ts(optional))]
    pub color_missing: Option<String>,
    #[cfg_attr(any(test, feature = "export-types"), ts(optional))]
    pub name: Option<String>,
    #[cfg_attr(any(test, feature = "export-types"), ts(optional))]
    pub name_missing: Option<String>,
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
    #[doc = " One of the user's regions."]
    pub pinned: bool,
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
    pub legend: Vec<u32>,
    #[doc = " The viewpoint side's index, when a side is the viewpoint and present."]
    pub viewpoint: Option<u32>,
    #[cfg_attr(any(test, feature = "export-types"), ts(optional))]
    pub viewpoint_country_style: Option<AirCountryStyle>,
    pub regions: Vec<AirRegion>,
    #[doc = " Why a region not listed may still hold scores or wings: a read that lists regions was not whole at the hour."]
    pub unlisted: Option<String>,
    #[doc = " Why the comparison lists no region; set only when it is shown and `regions` is empty."]
    pub empty: Option<NoRegions>,
    pub active_unlisted: ActiveRegions,
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

pub(crate) trait SideResolver {
    fn country(&mut self, tag: &str, hour: i32) -> ResolvedCountryLook;
    fn faction_name(&self, faction_id: u64, hour: i32) -> (Option<String>, Option<String>);
}

#[derive(Default)]
pub(crate) struct NullSideResolver;

impl SideResolver for NullSideResolver {
    fn country(&mut self, _tag: &str, _hour: i32) -> ResolvedCountryLook {
        ResolvedCountryLook::default()
    }

    fn faction_name(&self, _faction_id: u64, _hour: i32) -> (Option<String>, Option<String>) {
        (None, Some("Faction name was not recorded at this hour".to_owned()))
    }
}

pub(crate) struct RecordedSideResolver<'a> {
    looks: LookResolver<'a>,
    factions: HashMap<u64, FactionGroup>,
}

impl<'a> RecordedSideResolver<'a> {
    pub(crate) fn new(
        source: &'a Source,
        query: &'a dashboard_query::Query<'a>,
        load: &LoadContext,
        cancelled: &AtomicBool,
        warnings: &mut Vec<String>,
    ) -> io::Result<Self> {
        let looks = LookResolver::new(source, &query.timeline, load, warnings)?;
        let reply = groups::shared(source, query, load, cancelled)?;
        let factions = reply
            .factions
            .iter()
            .filter_map(|faction| Some((faction.id.parse::<u64>().ok()?, faction.clone())))
            .collect();
        Ok(Self { looks, factions })
    }
}

impl SideResolver for RecordedSideResolver<'_> {
    fn country(&mut self, tag: &str, hour: i32) -> ResolvedCountryLook {
        self.looks.at(tag, hour).unwrap_or_else(|error| {
            let reason = error.to_string();
            ResolvedCountryLook { name: None, color: None, name_missing: Some(reason.clone()), color_missing: Some(reason) }
        })
    }

    fn faction_name(&self, faction_id: u64, hour: i32) -> (Option<String>, Option<String>) {
        match self.factions.get(&faction_id) {
            Some(group) => groups::faction_name_at(group, hour),
            None => (None, Some("Faction name was not recorded at this hour".to_owned())),
        }
    }
}

pub(crate) struct BranchSideResolver {
    looks: BranchLooks,
    factions: HashMap<u64, FactionGroup>,
}

impl BranchSideResolver {
    fn load(
        source: &Source,
        query: &dashboard_query::Query<'_>,
        load: &LoadContext,
        cancelled: &AtomicBool,
        warnings: &mut Vec<String>,
    ) -> io::Result<Self> {
        let looks = BranchLooks::load(source, &query.timeline, load, warnings)?;
        let reply = groups::shared(source, query, load, cancelled)?;
        let factions = reply
            .factions
            .iter()
            .filter_map(|faction| Some((faction.id.parse::<u64>().ok()?, faction.clone())))
            .collect();
        Ok(Self { looks, factions })
    }

    fn through(&self, hour: i32, sides: &[AirSide], viewpoint: Option<&AirCountryStyle>) -> i32 {
        let mut next = viewpoint.and_then(|style| self.looks.next_for(&style.tag, hour));
        for side in sides {
            if (side.factions.is_empty() || side.anchor.is_none())
                && let Some(tag) = side.tag.as_ref().or(side.leader.as_ref()) {
                next = next.into_iter().chain(self.looks.next_for(tag, hour)).min();
            }
            for group in side.factions.iter().filter_map(|id| self.factions.get(&id.parse::<u64>().ok()?)) {
                let at = group.names.partition_point(|name| name.first_hour <= hour);
                let last = at.checked_sub(1).map(|i| group.names[i].last_hour).filter(|&last| last >= hour);
                let date = group.names.get(at).map(|name| name.first_hour);
                next = next.into_iter().chain(last.and_then(|h| h.checked_add(1))).chain(date).min();
            }
        }
        at_hour::before(next)
    }
}

impl SideResolver for &BranchSideResolver {
    fn country(&mut self, tag: &str, hour: i32) -> ResolvedCountryLook {
        self.looks.at(tag, hour)
    }

    fn faction_name(&self, faction_id: u64, hour: i32) -> (Option<String>, Option<String>) {
        match self.factions.get(&faction_id) {
            Some(group) => groups::faction_name_at(group, hour),
            None => (None, Some("Faction name was not recorded at this hour".to_owned())),
        }
    }
}

pub struct AirContext {
    names: Names,
    index: Result<Arc<index::AirIndex>, String>,
    resolver: Option<Arc<BranchSideResolver>>,
    resolver_warnings: Vec<String>,
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

pub(crate) fn country_style_at(tag: &str, hour: i32, resolver: &mut dyn SideResolver) -> AirCountryStyle {
    let look = resolver.country(tag, hour);
    AirCountryStyle {
        tag: tag.to_owned(),
        name: look.name,
        name_missing: look.name_missing,
        color: look.color,
        color_missing: look.color_missing,
    }
}

pub(crate) fn side_of(index: &index::AirIndex, side: &AssignedSide, hour: i32, resolver: &mut dyn SideResolver) -> AirSide {
    let anchor = index.anchor(side).map(|(tag, hour)| AirAnchor { tag, hour });
    let tag = side.factions.is_empty().then(|| side.members.first().cloned()).flatten();
    let (color, color_missing) = if let Some(anchor) = &anchor {
        let look = resolver.country(&anchor.tag, anchor.hour);
        (look.color, look.color_missing)
    } else if let Some(country) = tag.as_deref().or(side.leader.as_deref()) {
        let look = resolver.country(country, hour);
        (look.color, look.color_missing)
    } else {
        (None, Some("Side has no recorded colour anchor".to_owned()))
    };
    let (name, name_missing) = if !side.factions.is_empty() {
        let mut parts = Vec::with_capacity(side.factions.len());
        let mut missing = None;
        for &faction_id in &side.factions {
            let (resolved, reason) = resolver.faction_name(faction_id, hour);
            match resolved {
                Some(text) => parts.push(text),
                None => {
                    missing = Some(reason.unwrap_or_else(|| "Faction name was not recorded at this hour".to_owned()));
                    break;
                }
            }
        }
        match missing {
            Some(reason) => (None, Some(reason)),
            None => (Some(parts.join(" / ")), None),
        }
    } else if let Some(country) = tag.as_deref().or(side.leader.as_deref()) {
        let look = resolver.country(country, hour);
        match look.name {
            Some(text) if side.key.starts_with("w:") && side.co_belligerents > 0 => (Some(format!("{text} and allies")), None),
            Some(text) => (Some(text), None),
            None => (None, look.name_missing),
        }
    } else {
        (None, Some("Side has no recorded name".to_owned()))
    };
    AirSide {
        key: side.key.clone(),
        faction: side.faction.map(|id| id.to_string()),
        colour_faction: side.colour_faction.map(|id| id.to_string()),
        factions: side.factions.iter().map(ToString::to_string).collect(),
        tag,
        leader: side.leader.clone(),
        co_belligerents: side.co_belligerents,
        anchor,
        color,
        color_missing,
        name,
        name_missing,
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

fn side_ranks(sides: &[AirSide]) -> Vec<usize> {
    let mut factions: Vec<u128> = sides
        .iter()
        .filter_map(|side| side.faction.as_deref().and_then(|id| id.parse::<u128>().ok()))
        .collect();
    factions.sort_unstable();
    factions.dedup();
    let mut seen: HashMap<&str, usize> = HashMap::with_capacity(sides.len());
    let mut ranks = Vec::with_capacity(sides.len());
    for side in sides {
        let rank = if let Some(&held) = seen.get(side.key.as_str()) {
            held
        } else {
            let next = match side.faction.as_deref().and_then(|id| id.parse::<u128>().ok()).and_then(|id| factions.binary_search(&id).ok()) {
                Some(index) => index,
                None => factions.len() + seen.len(),
            };
            seen.insert(side.key.as_str(), next);
            next
        };
        ranks.push(rank);
    }
    ranks
}

fn region_lead_and_total(cell: &compose::Cell, pairs: Option<&[Pair]>) -> (f64, i64) {
    let Some(pairs) = pairs else {
        return (f64::INFINITY, 0);
    };
    let by_raw: HashMap<u32, &Pair> = pairs.iter().map(|pair| (pair.raw, pair)).collect();
    match cell {
        compose::Cell::View { row: Some(raw), .. } => {
            let Some(pair) = by_raw.get(raw) else {
                return (f64::INFINITY, 0);
            };
            let friendly = pair.friendly.max(0);
            let enemy = pair.enemy.max(0);
            let total = friendly.saturating_add(enemy);
            if total <= 0 {
                return (f64::INFINITY, 0);
            }
            let lead = (friendly - enemy).abs() as f64 / total as f64;
            (lead, total)
        }
        compose::Cell::Sides(cell) if cell.unknown.is_empty() => {
            let mut total: i64 = 0;
            let mut first: i64 = 0;
            let mut second: i64 = 0;
            for (_, _, raw) in &cell.parts {
                let Some(pair) = by_raw.get(raw) else {
                    return (f64::INFINITY, 0);
                };
                let score = pair.friendly.max(0);
                if score <= 0 {
                    continue;
                }
                total = total.saturating_add(score);
                if score > first {
                    second = first;
                    first = score;
                } else if score > second {
                    second = score;
                }
            }
            if total <= 0 {
                return (f64::INFINITY, 0);
            }
            ((first - second) as f64 / total as f64, total)
        }
        _ => (f64::INFINITY, 0),
    }
}

fn shown(cell: compose::Cell, region: u32, dictionary: &Dictionary<'_>, live: &Cells, columns: (usize, usize), enemy: bool, register_total: bool) -> AirCell {
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
        compose::Cell::Sides(cell) => {
            if register_total && !cell.parts.is_empty() {
                let sources: Vec<Cell> = cell
                    .parts
                    .iter()
                    .map(|(_, _, raw)| Cell { table: TableId::RaidAirScores, column: columns.0, key: u128::from(*raw) << 32 | u128::from(region) })
                    .collect();
                live.sum(&region.to_string(), "total", &sources, 1.0);
            }
            AirCell::Sides(AirSidesCell {
                parts: cell.parts.iter().map(|(position, tag, raw)| AirPart { side: dictionary.index(*position), tag: tag.clone(), live: score(tag, *raw) }).collect(),
                viewpoints: cell.viewpoints.iter().filter(|(position, _)| dictionary.at.contains_key(position)).map(|(position, tag)| (dictionary.index(*position), tag.clone())).collect(),
                unknown: cell.unknown.iter().map(|position| dictionary.index(*position)).collect(),
                no_air_object: cell.no_air_object.iter().map(|position| dictionary.index(*position)).collect(),
                filtered: cell.filtered,
                excluded: cell.excluded,
            })
        }
        compose::Cell::View { side, leader, row } => {
            if register_total
                && let Some(raw) = row
            {
                let key = u128::from(raw) << 32 | u128::from(region);
                let sources = [
                    Cell { table: TableId::RaidAirScores, column: columns.0, key },
                    Cell { table: TableId::RaidAirScores, column: columns.1, key },
                ];
                live.sum(&region.to_string(), "total", &sources, 1.0);
            }
            AirCell::View {
                side: side.map(|position| dictionary.index(position)),
                live: row.zip(leader.as_deref()).map(|(raw, tag)| score(tag, raw)),
                leader,
            }
        }
    }
}

fn moment(frame: &DecodedFrame, params: &AirSuperiorityParams, holders: &Holders, hour: i32, context: &AirContext, live: &Cells, resolver: &mut dyn SideResolver) -> io::Result<AirSuperiorityReply> {
    let mut warnings = context.names.warnings.clone();
    let index = context.index.as_ref().map_err(|why| dashboard_core::refused(why.clone()))?;
    let reply = |missing: String, warnings: Vec<String>| AirSuperiorityReply {
        missing: Some(missing),
        sides: Vec::new(),
        legend: Vec::new(),
        viewpoint: None,
        viewpoint_country_style: None,
        regions: Vec::new(),
        unlisted: None,
        empty: None,
        active_unlisted: ActiveRegions::default(),
        fallback: AirCell::Reason { reason: "scores-unavailable".into() },
        scale: SCALE,
        bad_below: BAD_BELOW,
        good_above: GOOD_ABOVE,
        warnings,
    };
    let (rows, complete) = match dashboard_text::metadata::recorded_rows::<tables::RaidAirScores>(frame)? {
        dashboard_text::metadata::RecordedRows::Complete(rows) => (rows, true),
        dashboard_text::metadata::RecordedRows::Truncated { rows, missing } => {
            if rows.is_empty() {
                return Ok(reply(format!("Air scores were read only in part at this hour ({missing} rows not read)"), warnings));
            }
            (rows, false)
        }
        dashboard_text::metadata::RecordedRows::Unavailable { reason } => {
            let message = if reason.is_empty() {
                "Air scores were not available at this hour".to_owned()
            } else {
                format!("Air scores were not available at this hour: {reason}")
            };
            return Ok(reply(message, warnings));
        }
        dashboard_text::metadata::RecordedRows::NotRecorded { mode } => {
            return Ok(reply(
                format!("Air scores are not recorded at this hour: {}", dashboard_text::metadata::not_recorded(hour, TableId::RaidAirScores, mode)),
                warnings,
            ));
        }
    };
    let epoch = index.epoch(hour);
    let empty = Arc::default();
    let present = epoch.map_or(&empty, |epoch| &epoch.present);
    let scores = hour_scores(rows, complete, frame.complete::<tables::CountryLinks>().ok(), holders, present, hour);
    let sides: &[AssignedSide] = epoch.and_then(|epoch| epoch.sides.as_deref()).map_or(&[], Vec::as_slice);
    let roster = epoch.and_then(|epoch| epoch.sides.as_ref()).map(|sides| Roster::new(sides));
    let viewpoint = viewpoint_name(params.viewpoint.as_ref(), params.viewpoint_country == Some(true), holders, hour);
    if params.mode == Some(AirMode::Viewpoint)
        && params.viewpoint_country == Some(true)
        && viewpoint.is_empty()
        && let Some(reason) = params.viewpoint.as_ref().and_then(|country| holders.not_known(country, hour))
    {
        return Ok(reply(reason, warnings));
    }
    let view = match params.mode.unwrap_or_default() {
        AirMode::Sides => View::Sides,
        AirMode::Viewpoint if params.viewpoint_country == Some(true) => View::Country(viewpoint),
        AirMode::Viewpoint => View::Side(sides.iter().position(|side| side.key == viewpoint).map(|at| at as u32)),
    };
    let viewpoint_country_style = (params.mode == Some(AirMode::Viewpoint) && params.viewpoint_country == Some(true) && !viewpoint.is_empty()).then(|| country_style_at(viewpoint, hour, resolver));
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
    let (wings, wings_whole): (Option<BTreeMap<u32, i64>>, Option<String>) =
        match dashboard_text::metadata::recorded_rows::<tables::AirRegionWings>(frame)?.into_rows(TableId::AirRegionWings, hour) {
            Ok((rows, None)) => (
                Some(rows.iter().map(|row| (row.region, row.wings)).collect()),
                dashboard_query::coverage::population::<tables::AirRegionWings>(frame, &replay_frame::frame::Keys::All).err(),
            ),
            Ok((rows, Some(why))) => (Some(rows.iter().map(|row| (row.region, row.wings)).collect()), Some(why)),
            Err(why) => (None, Some(why)),
        };
    let unlisted = dashboard_query::coverage::population::<tables::RaidAirScores>(frame, &replay_frame::frame::Keys::All).err().or_else(|| wings_whole.clone());
    let pinned: Vec<u32> = {
        let mut seen = HashSet::new();
        params.pinned_regions.iter().flatten().copied().filter(|id| seen.insert(*id)).collect()
    };
    let pin_rank: HashMap<u32, usize> = pinned.iter().enumerate().map(|(at, &id)| (id, at)).collect();
    let hidden: HashSet<u32> = params.hidden_regions.iter().flatten().copied().filter(|id| !pin_rank.contains_key(id)).collect();
    let mut ids: BTreeSet<u32> = scores.pairs.keys().chain(&scores.active).copied().collect();
    ids.extend(wings.iter().flat_map(|wings| wings.iter().filter(|(_, count)| **count != 0).map(|(region, _)| *region)));
    let hidden_ids: Vec<u32> = ids.iter().copied().filter(|region| hidden.contains(region) && (scores.active.contains(region) || wings.as_ref().is_some_and(|wings| wings.get(region).is_some_and(|count| *count != 0)))).collect();
    let hidden_active = hidden_ids.len() as u32;
    ids.retain(|region| !hidden.contains(region));
    ids.extend(pinned.iter().copied());
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
    let ordered = params.order.is_some();
    let resolved_sides: Vec<AirSide> = dictionary.at.keys().map(|position| side_of(index, &sides[*position as usize], hour, resolver)).collect();
    let ranks = ordered.then(|| side_ranks(&resolved_sides));
    let mut ordered_regions: Vec<(AirRegion, f64, i64, i64)> = Vec::with_capacity(cells.len());
    let mut outside = 0u32;
    for (region, cell) in cells {
        let (lead, total) = if ordered { region_lead_and_total(&cell, scores.pairs.get(&region).map(Vec::as_slice)) } else { (f64::INFINITY, 0) };
        let wing_count = wings.as_ref().and_then(|map| map.get(&region)).copied().unwrap_or(0);
        let active = scores.active.contains(&region);
        let is_pinned = pin_rank.contains_key(&region);
        if params.order.is_some() {
            let idle = !active && total <= 0 && wing_count <= 0;
            if !is_pinned && matches!(cell, compose::Cell::Reason("outside-theatres")) {
                outside += u32::from(active || wing_count != 0);
                continue;
            }
            if !is_pinned && idle {
                continue;
            }
        }
        let (name, name_missing) = region_name(&context.names, region);
        let wings_kind = match dashboard_query::coverage::witnessed::<tables::AirRegionWings>(frame, u128::from(region)) {
            dashboard_query::coverage::Witness::Row(_) => {
                live.value(&region.to_string(), "wings", Cell { table: TableId::AirRegionWings, column: wings_column, key: u128::from(region) }, 1.0);
                AirWings::Live
            }
            dashboard_query::coverage::Witness::Absent => AirWings::None,
            dashboard_query::coverage::Witness::NotRecorded(_) => AirWings::Unknown,
        };
        let mut shown_cell = shown(cell, region, &dictionary, live, columns, enemy, ordered);
        if let (Some(ranks), AirCell::Sides(sides_cell)) = (&ranks, &mut shown_cell) {
            sides_cell.parts.sort_by_key(|part| ranks.get(part.side as usize).copied().unwrap_or(usize::MAX));
        }
        ordered_regions.push((
            AirRegion { id: region, name, name_missing, active, pinned: is_pinned, cell: shown_cell, wings: wings_kind },
            lead,
            total,
            wing_count,
        ));
    }
    if let Some(order) = params.order {
        ordered_regions.sort_by(|a, b| match order {
            AirRegionOrder::Name => {
                let an = a.0.name.clone().unwrap_or_else(|| format!("Region {}", a.0.id));
                let bn = b.0.name.clone().unwrap_or_else(|| format!("Region {}", b.0.id));
                an.to_lowercase().cmp(&bn.to_lowercase()).then_with(|| an.cmp(&bn)).then_with(|| a.0.id.cmp(&b.0.id))
            }
            AirRegionOrder::Contested => a.1.total_cmp(&b.1).then_with(|| b.2.cmp(&a.2)).then_with(|| a.0.id.cmp(&b.0.id)),
            AirRegionOrder::Active => b.2.cmp(&a.2).then_with(|| b.3.cmp(&a.3)).then_with(|| a.0.id.cmp(&b.0.id)),
        });
    }
    ordered_regions.sort_by_key(|(region, ..)| pin_rank.get(&region.id).copied().unwrap_or(usize::MAX));
    let regions: Vec<AirRegion> = ordered_regions.into_iter().map(|(region, ..)| region).collect();
    let empty = no_regions(!regions.is_empty(), unlisted.as_deref(), hidden_active, outside);
    let fallback = shown(fallback, 0, &dictionary, live, columns, enemy, false);
    if let Some(error) = &wings_whole {
        warnings.push(format!("Air wings by region are not all known at this hour: {error}"));
    }
    let viewpoint_index = viewpoint_side.map(|position| dictionary.index(position));
    let legend = match params.mode.unwrap_or_default() {
        AirMode::Viewpoint => viewpoint_index.into_iter().collect(),
        AirMode::Sides => {
            let mut seen: BTreeMap<&str, u32> = BTreeMap::new();
            for region in &regions {
                if let AirCell::Sides(cell) = &region.cell {
                    for side in cell.parts.iter().map(|part| part.side).chain(cell.unknown.iter().copied()).chain(cell.no_air_object.iter().copied()) {
                        seen.insert(&resolved_sides[side as usize].key, side);
                    }
                }
            }
            seen.into_values().collect()
        }
    };
    Ok(AirSuperiorityReply {
        missing: None,
        sides: resolved_sides,
        legend,
        viewpoint: viewpoint_index,
        viewpoint_country_style,
        regions,
        active_unlisted: ActiveRegions { ids: hidden_ids, missing: unlisted.clone() },
        unlisted,
        empty,
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
    const CLOCK: dashboard_base::products::at_hour::Clock = dashboard_base::products::at_hour::Clock::Boundaries;
    type Params = AirSuperiorityParams;
    const FIELDS: dashboard_base::products::Fields = dashboard_base::fields![branchId, mode, viewpoint, viewpointCountry, sides, pinnedRegions, hiddenRegions, theatres, order, "pinnedRegions[]", "hiddenRegions[]"];
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
            AirRegionOrder::decl(),
            AirAnchor::decl(),
            AirCountryStyle::decl(),
            AirSide::decl(),
            AirPart::decl(),
            AirCell::decl(),
            AirSidesCell::decl(),
            AirWings::decl(),
            AirRegion::decl(),
            NoRegions::decl(),
            ActiveRegions::decl(),
            HourSpan::decl(),
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
                let mut reads = vec![Read::all(TableId::RaidAirScores), Read::all(TableId::AirRegionWings)];
                if theatres {
                    reads.extend(THEATRE_TABLES.into_iter().map(Read::all));
                }
                reads
            }
            1 => {
                let countries: BTreeSet<_> = match dashboard_text::metadata::recorded_rows::<tables::RaidAirScores>(held) {
                    Ok(dashboard_text::metadata::RecordedRows::Complete(rows) | dashboard_text::metadata::RecordedRows::Truncated { rows, .. }) => {
                        rows.iter().map(|row| u128::from(row.country)).collect()
                    }
                    _ => BTreeSet::new(),
                };
                let mut reads = vec![Read::these(TableId::CountryLinks, countries)];
                if theatres { reads.extend(member_wings(held)); }
                reads
            }
            _ => Vec::new(),
        }
    }

    fn columns(table: TableId) -> Option<Vec<usize>> {
        (table == TableId::RaidAirScores).then(|| ["friendly", "enemy"].iter().filter_map(|name| dashboard_declare::field(table, name).ok()).collect())
    }

    fn projection(_params: &Self::Params, table: TableId, _plan: &[(Vec<Read>, i32)], _context: &AirContext) -> Option<Vec<usize>> {
        match table {
            TableId::AirTheatres => Some(Vec::new()),
            TableId::RaidAirScores => Self::columns(table),
            _ => crate::products::air_missions::theatre_columns(table),
        }
    }

    fn context(source: &Source, load: &LoadContext) -> io::Result<Self::Context> {
        Ok(AirContext {
            names: names(source, load),
            index: Err("Air superiority needs its timeline".into()),
            resolver: None,
            resolver_warnings: Vec::new(),
        })
    }

    fn context_for(source: &Source, load: &LoadContext, params: &Self::Params) -> io::Result<Self::Context> {
        let branch = params.branch_id.parse::<u64>().map_err(|_| dashboard_core::invalid("Invalid air branch"))?;
        let mut resolver_warnings = Vec::new();
        let resolver = match dashboard_base::queries::query(source, branch) {
            Ok(query) => {
                let cancelled = AtomicBool::new(false);
                Some(Arc::new(BranchSideResolver::load(source, &query, load, &cancelled, &mut resolver_warnings)?))
            }
            Err(_) => None,
        };
        Ok(AirContext {
            names: names(source, load),
            index: index::index(source, branch).map_err(|error| format!("Air sides are not known: {error}")),
            resolver,
            resolver_warnings,
        })
    }

    fn valid_reply_through(frame: &DecodedFrame, _params: &Self::Params, at: &At<'_, Self::Context>, reply: &Self::Reply, _partial: bool) -> i32 {
        if matches!(frame.tables[TableId::RaidAirScores.index()].held(), replay_frame::frame::TableView::NotRecorded { .. }) {
            return at.hour;
        }
        let through = valid_through(at.context, at.hour).min(at_hour::before(at.holders.next_change(at.hour)));
        at.context.resolver.as_ref().map_or(through, |resolver| through.min(resolver.through(at.hour, &reply.sides, reply.viewpoint_country_style.as_ref())))
    }

    fn reply(frame: &DecodedFrame, params: &Self::Params, at: &At<'_, Self::Context>) -> io::Result<Self::Reply> {
        if let Some(resolver) = &at.context.resolver {
            let mut active: &BranchSideResolver = resolver;
            let mut reply = moment(frame, params, at.holders, at.hour, at.context, at.live, &mut active)?;
            reply.warnings.extend(at.context.resolver_warnings.iter().cloned());
            return Ok(reply);
        }
        let mut null = NullSideResolver;
        moment(frame, params, at.holders, at.hour, at.context, at.live, &mut null)
    }
}

fn valid_through(context: &AirContext, hour: i32) -> i32 {
    let Ok(index) = &context.index else { return hour };
    epoch_through(index, hour)
}

fn epoch_through(index: &index::AirIndex, hour: i32) -> i32 {
    let at = index.epochs.partition_point(|epoch| epoch.last < hour);
    match index.epochs.get(at) {
        Some(epoch) if epoch.first <= hour => epoch.last,
        Some(next) => next.first.saturating_sub(1),
        None => i32::MAX,
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
        let mut last: BTreeMap<&str, &AssignedSide> = BTreeMap::new();
        for side in index.epochs.iter().filter_map(|epoch| epoch.sides.as_deref()).flatten() {
            last.insert(&side.key, side);
        }
        let mut resolver = RecordedSideResolver::new(source, &query, context, cancelled, &mut warnings)?;
        let sides = last.values().map(|side| side_of(&index, side, query.timeline.last, &mut resolver)).collect();
        let (theatres, theatres_missing) = match theatres_at_end(&query, &names) {
            Ok(theatres) => (theatres, None),
            Err(why) => (Vec::new(), Some(why)),
        };
        if let Some(why) = &theatres_missing {
            warnings.push(why.clone());
        }
        Ok(AirChoicesReply { sides, theatres, theatres_missing, countries: query.holders.tags.keys().cloned().collect(), warnings })
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
