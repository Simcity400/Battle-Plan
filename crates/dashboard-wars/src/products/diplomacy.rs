
use dashboard_base::products::{Country, validate_country};
use dashboard_base::{products::{LoadContext, Product, at_hour::{At, AtHour, Keys, Owner, Read, ReadsAt}}, queries::country_ids::{Names, RunLinks}};
use dashboard_base::{Source, catalogue::{self, Catalogue}, span::HourSpan};
use dashboard_query::{
    Query,
    identity::{Holder, Holders},
    read,
    series::ranked::GroupSelection,
    timeline::{Join, Stretch},
};
use dashboard_world::products::group_members::GroupsAt;
use replay_recording::columnar_frames::Recording;
use replay_lanes::{lanes, points::Cell};
use replay_columnar::manifest::Status;
use replay_data::rows::{TableId, tables};
use replay_frame::frame::{DecodedFrame, TableView};
use serde::{Deserialize, Serialize};
use std::{
    collections::{BTreeMap, HashMap, HashSet},
    io,
    sync::{Arc, atomic::AtomicBool},
};
#[cfg(any(test, feature = "export-types"))]
use ts_rs::TS;

fn holder_ids(holders: &Holders, countries: &[Country], hour: i32) -> Vec<(String, Option<(u32, u32)>)> {
    countries
        .iter()
        .map(|country| {
            let id = match holders.at(country, hour).map(|run| run.holder) {
                Some(Holder::Found { slot, id }) => Some((slot, id)),
                _ => None,
            };
            (holders.name(country, hour), id)
        })
        .collect()
}

fn named(ids: &[(String, Option<(u32, u32)>)]) -> Vec<(&str, Option<(u32, u32)>)> {
    ids.iter().map(|(key, id)| (key.as_str(), *id)).collect()
}

fn aliases(frame: &DecodedFrame, ids: &[(&str, Option<(u32, u32)>)]) -> HashMap<u32, u32> {
    let slots: HashSet<u32> = ids.iter().filter_map(|&(_, id)| id.map(|(slot, _)| slot)).collect();
    let mut by_raw: HashMap<u32, u32> = ids.iter().filter_map(|&(_, id)| id.map(|(slot, raw)| (raw, slot))).collect();
    if let Ok(links) = frame.complete::<tables::CountryLinks>() {
        by_raw.extend(links.iter().filter(|link| slots.contains(&link.canonical_slot)).map(|link| (link.id, link.canonical_slot)));
    }
    by_raw
}

pub struct DiplomaticRelations;

#[derive(Clone, Copy, Debug, Default, Deserialize, Serialize, PartialEq, Eq)]
#[cfg_attr(any(test, feature = "export-types"), derive(TS))]
#[serde(rename_all = "lowercase")]
pub enum OpinionOrder {
    #[default]
    Faction,
    Name,
}

#[derive(Clone, Debug, PartialEq, Deserialize, Serialize)]
#[cfg_attr(any(test, feature = "export-types"), derive(TS))]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct DiplomaticRelationsParams {
    pub branch_id: String,
    #[serde(default)]
    pub countries: Vec<Country>,
    #[serde(default)]
    #[cfg_attr(any(test, feature = "export-types"), ts(optional))]
    pub top: Option<u32>,
    #[serde(default)]
    #[cfg_attr(any(test, feature = "export-types"), ts(optional))]
    pub order: Option<OpinionOrder>,
    #[doc = " A group: its members at the hour (or every member so far), resolved in Rust; replaces `countries` and `top`."]
    #[serde(default)]
    #[cfg_attr(any(test, feature = "export-types"), ts(optional))]
    pub group: Option<GroupSelection>,
}

#[derive(Clone, Debug, Serialize, PartialEq, Eq)]
#[cfg_attr(any(test, feature = "export-types"), derive(TS))]
#[serde(tag = "kind", rename_all = "camelCase")]
pub enum OpinionCell {
    Value { opinion: i32 },
    Missing { reason: String },
}

#[derive(Clone, Debug, Serialize, PartialEq, Eq)]
#[cfg_attr(any(test, feature = "export-types"), derive(TS))]
#[serde(rename_all = "camelCase")]
pub struct OpinionPair {
    #[doc = " The owner's and the target's index in `owners`."]
    pub row: u32,
    pub column: u32,
    pub cell: OpinionCell,
}

#[derive(Clone, Debug, Serialize, PartialEq, Eq)]
#[cfg_attr(any(test, feature = "export-types"), derive(TS))]
#[serde(rename_all = "camelCase")]
pub struct OpinionOwner {
    pub tag: String,
    #[doc = " Its holder's raw id at the hour (the part a pair's `relation-opinions` line names it by);"]
    #[doc = " null where no country holds the tag then."]
    pub country_id: Option<u32>,
    #[doc = " Whether its opinions were read whole: its opinion of a shown country with no pair is exactly zero."]
    pub known: bool,
    #[doc = " Why it is not known: its opinions with no pair are not recorded."]
    pub reason: Option<String>,
    #[cfg_attr(any(test, feature = "export-types"), ts(optional))]
    pub read_hour: Option<i32>,
    #[cfg_attr(any(test, feature = "export-types"), ts(optional))]
    pub name: Option<String>,
    #[cfg_attr(any(test, feature = "export-types"), ts(optional))]
    pub name_missing: Option<String>,
    #[cfg_attr(any(test, feature = "export-types"), ts(optional))]
    pub color: Option<[u8; 3]>,
    #[cfg_attr(any(test, feature = "export-types"), ts(optional))]
    pub faction_id: Option<String>,
    #[cfg_attr(any(test, feature = "export-types"), ts(optional))]
    pub faction_name: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Serialize)]
#[cfg_attr(any(test, feature = "export-types"), derive(TS))]
#[serde(rename_all = "camelCase")]
pub struct DiplomaticRelationsReply {
    #[doc = " Why the countries to show are not known at this hour: factories, which rank a top, were not read whole or name an unresolved country, or a group's membership is not recorded. No owners then."]
    pub chosen_missing: Option<String>,
    pub owners: Vec<OpinionOwner>,
    #[doc = " The witnessed opinions between owners, and the pairs whose target no country holds, by row then column."]
    pub pairs: Vec<OpinionPair>,
    pub domain: [i32; 2],
    pub warnings: Vec<String>,
}

dashboard_base::reply_warnings!(DiplomaticRelationsReply);

const UNHELD: &str = "No country holds this tag at this hour";

fn witnessed<T: replay_data::rows::TableSchema>(frame: &DecodedFrame) -> (&[T::Row], Option<String>) {
    let view: &TableView = &frame.tables[T::ID.index()];
    let rows = match view {
        TableView::Complete(rows) | TableView::Truncated { rows, .. } => rows.rows::<T>().unwrap_or(&[]),
        TableView::NotRecorded { .. } | TableView::Unavailable { .. } => &[],
    };
    (rows, dashboard_text::scope::unread(T::ID, view))
}

pub(crate) fn opinions_matrix(frame: &DecodedFrame, ids: &[(&str, Option<(u32, u32)>)]) -> (Vec<OpinionOwner>, Vec<OpinionPair>, [i32; 2]) {
    let (rows, opinions_unread) = witnessed::<tables::RelationOpinions>(frame);
    let (reads, reads_unread) = witnessed::<tables::SliceReads>(frame);
    let table = TableId::RelationOpinions.index() as u32;
    let slot_of = aliases(frame, ids);
    let mut opinions: HashMap<(u32, u32), i32> = HashMap::new();
    for row in rows {
        if let (Some(&owner), Some(&target)) = (slot_of.get(&row.owner_country), slot_of.get(&row.target_country)) {
            opinions.insert((owner, target), row.opinion);
        }
    }
    let listed: HashSet<u32> = opinions.keys().map(|&(owner, _)| owner).collect();
    let mut read: HashMap<u32, i32> = HashMap::new();
    for row in reads.iter().filter(|row| row.table == table) {
        if let Some(&slot) = slot_of.get(&row.owner) {
            let latest = read.entry(slot).or_insert(row.sample_hour);
            *latest = (*latest).max(row.sample_hour);
        }
    }
    let owners: Vec<OpinionOwner> = ids
        .iter()
        .map(|&(tag, id)| {
            let (country_id, read_hour, reason) = match id {
                None => (None, None, Some(UNHELD.to_owned())),
                Some((slot, id)) => {
                    let seen = read.contains_key(&slot) || listed.contains(&slot);
                    let reason = match (&opinions_unread, seen) {
                        (Some(unread), _) => Some(unread.clone()),
                        (None, true) => None,
                        (None, false) => Some(reads_unread.clone().unwrap_or_else(|| "This country's opinions have not been read".to_owned())),
                    };
                    (Some(id), read.get(&slot).copied(), reason)
                }
            };
            OpinionOwner { tag: tag.to_owned(), country_id, known: reason.is_none(), reason, read_hour, name: None, name_missing: None, color: None, faction_id: None, faction_name: None }
        })
        .collect();
    let mut at_slot: HashMap<u32, Vec<u32>> = HashMap::new();
    for (index, &(_, id)) in ids.iter().enumerate() {
        if let Some((slot, _)) = id {
            at_slot.entry(slot).or_default().push(index as u32);
        }
    }
    let mut pairs = Vec::new();
    let (mut low, mut high) = (0_i32, 0_i32);
    for (&(owner, target), &opinion) in &opinions {
        let (Some(rows), Some(columns)) = (at_slot.get(&owner), at_slot.get(&target)) else { continue };
        for &row in rows {
            for &column in columns.iter().filter(|&&column| column != row) {
                low = low.min(opinion);
                high = high.max(opinion);
                pairs.push(OpinionPair { row, column, cell: OpinionCell::Value { opinion } });
            }
        }
    }
    let unheld: Vec<u32> = ids.iter().enumerate().filter(|(_, (_, id))| id.is_none()).map(|(index, _)| index as u32).collect();
    if !unheld.is_empty() {
        for row in owners.iter().enumerate().filter(|(_, owner)| owner.known).map(|(row, _)| row) {
            for &column in &unheld {
                pairs.push(OpinionPair { row: row as u32, column, cell: OpinionCell::Missing { reason: UNHELD.to_owned() } });
            }
        }
    }
    pairs.sort_unstable_by_key(|pair| (pair.row, pair.column));
    (owners, pairs, [low, high])
}

fn top_countries_at(frame: &DecodedFrame, holders: &Holders, hour: i32, limit: u32) -> Result<Vec<Country>, String> {
    let mut slot_to_tag: HashMap<u32, &str> = HashMap::new();
    let mut raw_to_slot: HashMap<u32, u32> = HashMap::new();
    for (tag, _) in &holders.tags {
        if let Some(Holder::Found { slot, id }) = holders.at(tag, hour).map(|r| r.holder) {
            slot_to_tag.insert(slot, tag.as_str());
            raw_to_slot.insert(id, slot);
        }
    }
    if let Ok(links) = frame.complete::<tables::CountryLinks>() {
        for link in links {
            raw_to_slot.insert(link.id, link.canonical_slot);
        }
    }
    let mut scores: HashMap<&str, i64> = slot_to_tag.values().map(|&tag| (tag, 0)).collect();
    let factories = dashboard_query::coverage::population::<tables::CountryFactories>(frame, &replay_frame::frame::Keys::All)?;
    for row in factories {
        let Some(&tag) = raw_to_slot.get(&row.country).and_then(|slot| slot_to_tag.get(slot)) else {
            return Err("Country factories name a country not known at this hour, so the largest countries are not known".into());
        };
        let total = i64::from(row.military_levels) + i64::from(row.naval_levels) + i64::from(row.civilian_levels);
        let entry = scores.entry(tag).or_insert(0);
        *entry = (*entry).max(total);
    }
    let mut ranked: Vec<(&str, i64)> = scores.into_iter().collect();
    ranked.sort_by(|a, b| b.1.cmp(&a.1).then(a.0.cmp(b.0)));
    ranked.truncate(limit as usize);
    Ok(ranked.into_iter().map(|(tag, _)| Country::from(tag)).collect())
}

fn chosen_at(frame: &DecodedFrame, params: &DiplomaticRelationsParams, holders: &Holders, hour: i32, groups: &GroupsAt) -> io::Result<Result<Vec<Country>, String>> {
    if let Some(group) = &params.group {
        let members = groups.members(group, hour)?;
        return Ok(match members.refused {
            Some(refused) => Err(refused),
            None => Ok(members.tags.iter().map(|tag| Country::from(tag.as_str())).collect()),
        });
    }
    Ok(match (params.countries.is_empty(), params.top) {
        (true, Some(limit)) => top_countries_at(frame, holders, hour, limit),
        (true, None) => Ok(Vec::new()),
        (false, _) => Ok(params.countries.clone()),
    })
}

fn ranks_top(params: &DiplomaticRelationsParams) -> bool {
    params.group.is_none() && params.countries.is_empty() && params.top.is_some()
}

pub struct DiplomacyContext {
    looks: Option<Arc<dashboard_world::products::styles::BranchLooks>>,
    factions: Arc<Vec<dashboard_world::products::groups::FactionGroup>>,
    faction_edges: Vec<i32>,
    warnings: Vec<String>,
    groups: GroupsAt,
}

impl AtHour for DiplomaticRelations {
    const KEY: &'static str = "diplomatic-relations";
    const TITLE: &'static str = "Diplomatic relations";
    const CONTENT_DEPENDENT: bool = false;
    type Params = DiplomaticRelationsParams;
    const FIELDS: dashboard_base::products::Fields = dashboard_base::fields![branchId, countries, "countries[]", top, order, group];
    type Reply = DiplomaticRelationsReply;
    type Context = DiplomacyContext;

    #[cfg(any(test, feature = "export-types"))]
    fn types() -> Vec<String> {
        vec![DiplomaticRelationsParams::decl(), OpinionOrder::decl(), OpinionCell::decl(), OpinionPair::decl(), OpinionOwner::decl(), DiplomaticRelationsReply::decl()]
    }

    fn validate(params: &Self::Params) -> Result<(), &'static str> {
        if params.branch_id.parse::<u64>().is_err() {
            return Err("Invalid branch");
        }
        params.countries.iter().try_for_each(validate_country)?;
        if params.group.is_some() && (params.top.is_some() || !params.countries.is_empty()) {
            return Err("A matrix names a group, a top or countries, one of them");
        }
        Ok(())
    }

    fn tables(params: &Self::Params) -> Vec<TableId> {
        let mut out = vec![TableId::RelationOpinions, TableId::SliceReads, TableId::CountryLinks];
        if ranks_top(params) {
            out.push(TableId::CountryFactories);
        }
        out
    }

    fn reads_in(params: &Self::Params, _owner: Option<Owner>, step: usize, held: &DecodedFrame, at: &ReadsAt<'_, Self::Context>) -> Vec<Read> {
        match step {
            0 => {
                let mut reads = vec![Read::all(TableId::CountryLinks)];
                if ranks_top(params) {
                    reads.push(Read::all(TableId::CountryFactories));
                }
                return reads;
            }
            1 => {}
            _ => return Vec::new(),
        }
        let chosen: Vec<Country> = match chosen_at(held, params, at.holders, at.hour, &at.context.groups) {
            Ok(Ok(chosen)) => chosen,
            Ok(Err(_)) | Err(_) => Vec::new(),
        };
        let mut ids: Vec<u32> = aliases(held, &named(&holder_ids(at.holders, &chosen, at.hour))).into_keys().collect();
        ids.sort_unstable();
        ids.dedup();
        let owners = |id: u32| (u128::from(id) << 32, u128::from(id) << 32 | u128::from(u32::MAX));
        let table = u128::from(TableId::RelationOpinions.index() as u32) << 32;
        vec![
            Read { table: TableId::RelationOpinions, keys: Keys::Ranges(ids.iter().map(|&id| owners(id)).collect()) },
            Read::these(TableId::SliceReads, ids.iter().map(|&id| table | u128::from(id))),
        ]
    }

    fn retained_reads(params: &Self::Params, owner: Option<Owner>, stage: usize, frame: &DecodedFrame, at: &ReadsAt<'_, Self::Context>) -> (Vec<Read>, i32) {
        let through = if stage == 1 { at.context.groups.valid_through(params.group.as_ref(), at.hour) } else { i32::MAX };
        (Self::reads_in(params, owner, stage, frame, at), through)
    }

    fn valid_through(_frame: &DecodedFrame, params: &Self::Params, at: &At<'_, Self::Context>) -> i32 {
        let group = at.context.groups.valid_through(params.group.as_ref(), at.hour);
        let look = at.context.looks.as_ref().map_or(i32::MAX, |looks| looks.next_change(at.hour).saturating_sub(1));
        let edges = &at.context.faction_edges;
        let faction = edges.get(edges.partition_point(|&edge| edge <= at.hour)).map_or(i32::MAX, |edge| edge - 1);
        group.min(look).min(faction)
    }

    fn context(_source: &Source, _load: &LoadContext) -> io::Result<Self::Context> {
        Ok(DiplomacyContext { looks: None, factions: Arc::new(Vec::new()), faction_edges: Vec::new(), warnings: Vec::new(), groups: GroupsAt::default() })
    }

    fn context_for(source: &Source, load: &LoadContext, params: &Self::Params) -> io::Result<Self::Context> {
        let mut warnings = Vec::new();
        let groups = GroupsAt::load(source, load, &params.branch_id, params.group.as_ref())?;
        let Some(branch) = params.branch_id.parse::<u64>().ok() else {
            return Ok(DiplomacyContext { looks: None, factions: Arc::new(Vec::new()), faction_edges: Vec::new(), warnings, groups });
        };
        let Ok(query) = dashboard_base::queries::query(source, branch) else {
            return Ok(DiplomacyContext { looks: None, factions: Arc::new(Vec::new()), faction_edges: Vec::new(), warnings, groups });
        };
        let looks = Some(Arc::new(dashboard_world::products::styles::BranchLooks::load(source, &query.timeline, load, &mut warnings)?));
        let factions = match dashboard_world::products::groups::shared(source, &query, load, &AtomicBool::new(false)) {
            Ok(reply) => Arc::new(reply.factions.clone()),
            Err(_) => Arc::new(Vec::new()),
        };
        let mut faction_edges: Vec<i32> = factions.iter().flat_map(|faction| {
            faction.names.iter().map(|run| (run.first_hour, run.last_hour))
                .chain(faction.members.iter().map(|run| (run.first_hour, run.last_hour)))
                .flat_map(|(first, last)| [Some(first), last.checked_add(1)].into_iter().flatten())
        }).collect();
        faction_edges.sort_unstable();
        faction_edges.dedup();
        Ok(DiplomacyContext { looks, factions, faction_edges, warnings, groups })
    }

    fn reply(frame: &DecodedFrame, params: &Self::Params, at: &At<'_, Self::Context>) -> io::Result<Self::Reply> {
        let (chosen, chosen_missing): (Vec<Country>, Option<String>) = match chosen_at(frame, params, at.holders, at.hour, &at.context.groups)? {
            Ok(chosen) => (chosen, None),
            Err(why) => (Vec::new(), Some(why)),
        };
        let mut warnings = at.context.warnings.clone();
        let mut metadata: HashMap<String, (Option<String>, Option<String>, Option<[u8; 3]>, Option<String>, Option<String>)> = HashMap::new();
        if let Some(looks) = &at.context.looks {
            let mut faction_by_tag: HashMap<String, (String, Option<String>)> = HashMap::new();
            for faction in at.context.factions.iter() {
                let (fname, _) = dashboard_world::products::groups::faction_name_at(faction, at.hour);
                for run in &faction.members {
                    if run.first_hour <= at.hour && at.hour <= run.last_hour {
                        faction_by_tag.entry(run.tag.clone()).or_insert_with(|| (faction.id.clone(), fname.clone()));
                    }
                }
            }
            for country in &chosen {
                let tag = at.holders.name(country, at.hour);
                let look = looks.at(&tag, at.hour);
                let (fid, fname) = faction_by_tag.get(&tag).map_or((None, None), |(id, name)| (Some(id.clone()), name.clone()));
                metadata.insert(tag, (look.name, look.name_missing, look.color, fid, fname));
            }
        }
        let mut ids = holder_ids(at.holders, &chosen, at.hour);
        if let Some(order) = params.order {
            ids.sort_by(|a, b| {
                let ma = metadata.get(&a.0);
                let mb = metadata.get(&b.0);
                if order == OpinionOrder::Faction {
                    let fa = ma.and_then(|m| m.3.as_deref());
                    let fb = mb.and_then(|m| m.3.as_deref());
                    if fa != fb {
                        return match (fa, fb) {
                            (Some(id_a), Some(id_b)) => {
                                let na = ma.and_then(|m| m.4.as_deref()).unwrap_or(id_a);
                                let nb = mb.and_then(|m| m.4.as_deref()).unwrap_or(id_b);
                                na.cmp(nb).then(id_a.cmp(id_b))
                            }
                            (Some(_), None) => std::cmp::Ordering::Less,
                            (None, Some(_)) => std::cmp::Ordering::Greater,
                            (None, None) => std::cmp::Ordering::Equal,
                        };
                    }
                }
                let na = ma.and_then(|m| m.0.as_deref()).unwrap_or(&a.0);
                let nb = mb.and_then(|m| m.0.as_deref()).unwrap_or(&b.0);
                na.cmp(nb).then(a.0.cmp(&b.0))
            });
        }
        let (mut owners, pairs, domain) = opinions_matrix(frame, &named(&ids));
        for owner in &mut owners {
            if owner.country_id.is_none()
                && let Some(reason) = at.holders.not_known_as(&owner.tag, at.hour)
            {
                owner.reason = Some(reason);
            }
            if let Some((name, name_missing, color, faction_id, faction_name)) = metadata.remove(&owner.tag) {
                owner.name = name;
                owner.name_missing = name_missing;
                owner.color = color;
                owner.faction_id = faction_id;
                owner.faction_name = faction_name;
            }
        }
        let unresolved = owners.iter().filter(|owner| owner.country_id.is_none()).count();
        if unresolved > 0 && at.holders.unknown_reason(at.hour).is_none() {
            warnings.push(format!("{unresolved} chosen countries have no holder at this hour; their opinions are not known."));
        }
        Ok(DiplomaticRelationsReply { chosen_missing, owners, pairs, domain, warnings })
    }
}

const FIRST: usize = 0;
const SECOND: usize = 1;
const START: usize = 2;
const END: usize = 3;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct PactRun {
    pub(crate) first: i32,
    pub(crate) last: i32,
    pub(crate) endpoints: (u32, u32),
    pub(crate) dates: (i32, i32),
}

#[derive(Default)]
pub struct RunPacts {
    pub(crate) rows: Vec<(u128, Vec<PactRun>)>,
    pub(crate) kinds: HashMap<u32, String>,
    pub(crate) unproven: Vec<(i32, i32)>,
}

fn pact_runs(steps: &[lanes::Step], to: i32) -> Vec<PactRun> {
    let number = |cell: &Cell| match cell {
        Cell::Number(value) => Some(*value),
        _ => None,
    };
    let mut out: Vec<PactRun> = Vec::new();
    for (at, step) in steps.iter().enumerate() {
        let last = steps.get(at + 1).map_or(to, |next| next.hour - 1);
        let Some(cells) = &step.cells else { continue };
        let (Some(first), Some(second), Some(start), Some(end)) = (number(&cells[0]), number(&cells[1]), number(&cells[2]), number(&cells[3])) else { continue };
        let run = PactRun { first: step.hour, last, endpoints: (first as u32, second as u32), dates: (start as i32, end as i32) };
        match out.last_mut() {
            Some(held) if (held.endpoints, held.dates) == (run.endpoints, run.dates) && i64::from(held.last) + 1 == i64::from(run.first) => held.last = last,
            _ => out.push(run),
        }
    }
    out
}

pub fn run_pacts(recording: &Recording, run: u32) -> io::Result<RunPacts> {
    let held = recording.run(run)?;
    let (first, last) = (held.first_hour, held.first_hour + held.ordinals as i32 - 1);
    let history = dashboard_query::cost::storage(|| lanes::history(recording, run, TableId::RelationTuples, &[FIRST, SECOND, START, END], lanes::Keys::All, first, last))?;
    let rows: Vec<(u128, Vec<PactRun>)> = history.keys.iter().map(|(key, steps)| (*key, pact_runs(steps, history.to))).filter(|(_, runs)| !runs.is_empty()).collect();
    let mut types: Vec<u128> = rows.iter().map(|(key, _)| u128::from((key >> 16) as u32)).collect();
    types.sort_unstable();
    types.dedup();
    let mut kinds = HashMap::new();
    if !types.is_empty() {
        let reads = [read::Read { table: TableId::LexicalTokens, columns: None, keys: read::Keys::These(&types) }];
        let held = dashboard_query::cost::storage(|| read::at(recording, run, &reads, last))?;
        let view = held[0].view();
        for &(key, slot) in &held[0].rows {
            if let Some(text) = read::text(&view, 0, slot).and_then(|bytes| std::str::from_utf8(bytes).ok()) {
                kinds.insert(key as u32, text.to_owned());
            }
        }
    }
    let unproven = dashboard_query::cost::storage(|| lanes::statuses(recording, run, TableId::RelationTuples, first, last))?
        .into_iter()
        .filter(|held| held.status != Status::Complete)
        .map(|held| (held.first, held.last))
        .collect();
    Ok(RunPacts { rows, kinds, unproven })
}

pub struct DiplomaticPacts;

#[derive(Clone, Debug, PartialEq, Deserialize, Serialize)]
#[cfg_attr(any(test, feature = "export-types"), derive(TS))]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct DiplomaticPactsParams {
    pub branch_id: String,
    pub country: Country,
    #[doc = " The hour answered: nothing recorded after it is read into the reply."]
    pub hour: i32,
}

#[derive(Clone, Debug, Serialize, PartialEq, Eq)]
#[cfg_attr(any(test, feature = "export-types"), derive(TS))]
#[serde(rename_all = "camelCase")]
pub struct DiplomacyKind {
    #[doc = " The relation's id as the game spells it."]
    pub kind: String,
    pub count: u32,
    #[doc = " Its name as the game shows it; null when not read, with why."]
    pub name: Option<String>,
    pub reason: Option<String>,
}

#[derive(Clone, Debug, Serialize, PartialEq, Eq)]
#[cfg_attr(any(test, feature = "export-types"), derive(TS))]
#[serde(rename_all = "camelCase")]
pub struct DiplomacyPact {
    pub kind: String,
    #[doc = " Its endpoints as the game orders them; null where no tag names the country then."]
    pub first: Option<String>,
    pub second: Option<String>,
    #[doc = " The hours the relation began and ends as the game dates them; null where not written."]
    pub start_hour: Option<i32>,
    pub end_hour: Option<i32>,
    #[doc = " The first and last hour the chosen country's row held it, never after the reply's hour."]
    pub first_seen: i32,
    pub last_seen: i32,
    #[doc = " The hour by which its row was gone: a read of the chosen country no longer listed it."]
    pub gone_by: Option<i32>,
    #[doc = " Held at the reply's hour."]
    pub held: bool,
    #[doc = " Its row's hours ended at a gap in the recording: whether it ended then is not known."]
    pub ended_by: bool,
}

#[derive(Clone, Debug, PartialEq, Serialize)]
#[cfg_attr(any(test, feature = "export-types"), derive(TS))]
#[serde(rename_all = "camelCase")]
pub struct DiplomacyPactsReply {
    #[doc = " The hour answered, never after the branch's last hour."]
    pub hour: i32,
    #[doc = " The first hour after `hour` at which a pact, its end, a partner's tag or an unread stretch changes; null"]
    #[doc = " when none does."]
    pub next_change: Option<i32>,
    #[doc = " Every relation the pacts by the hour name, with its count by then."]
    pub kinds: Vec<DiplomacyKind>,
    pub pacts: Vec<DiplomacyPact>,
    pub unobserved: Vec<HourSpan>,
    pub warnings: Vec<String>,
}

dashboard_base::reply_warnings!(DiplomacyPactsReply);

fn tag_ids(holders: &Holders, tag: &Country) -> Vec<(u32, i32, i32)> {
    holders
        .runs(tag)
        .iter()
        .filter_map(|run| match run.holder {
            Holder::Found { id, .. } => Some((id, run.first, run.last)),
            Holder::Unresolved => None,
        })
        .collect()
}

pub(crate) fn pacts(stretches: &[Stretch], held: &HashMap<u32, (Arc<RunPacts>, Arc<RunLinks>)>, names: &Names, ids: &[(u32, i32, i32)], hour: i32) -> (Vec<DiplomacyPact>, BTreeMap<String, u32>) {
    let mut out: Vec<DiplomacyPact> = Vec::new();
    for (index, stretch) in stretches.iter().enumerate().take_while(|(_, stretch)| stretch.first <= hour) {
        let Some((pacts, links)) = held.get(&stretch.run) else { continue };
        let next_breaks = stretches.get(index + 1).is_none_or(|next| next.join == Join::Break || next.first > hour);
        for &(id, held_first, held_last) in ids {
            let (from, to) = (held_first.max(stretch.first), held_last.min(stretch.last).min(hour));
            if from > to {
                continue;
            }
            let mine = |key: u128| (key >> 80) as u32 == id || (key >> 48) as u32 == id;
            for (key, runs) in pacts.rows.iter().filter(|(key, _)| mine(*key)) {
                let Some(kind) = pacts.kinds.get(&((key >> 16) as u32)) else { continue };
                for run in runs {
                    let (a, b) = (run.first.max(from), run.last.min(to));
                    if a > b {
                        continue;
                    }
                    let start_hour = (run.dates.0 > 0).then_some(run.dates.0);
                    let end_hour = (run.dates.1 > run.dates.0).then_some(run.dates.1);
                    let held_now = b == hour;
                    let ended_by = b == stretch.last && !held_now && next_breaks;
                    let proved = !pacts.unproven.iter().any(|&(first, last)| first <= b + 1 && b + 1 <= last);
                    let gone_by = (b == run.last && b < stretch.last && b < hour && proved).then_some(b + 1);
                    let first = names.tag_at(links, run.endpoints.0, b).map(str::to_owned);
                    let second = names.tag_at(links, run.endpoints.1, b).map(str::to_owned);
                    out.push(DiplomacyPact { kind: kind.clone(), first, second, start_hour, end_hour, first_seen: a, last_seen: b, gone_by, held: held_now, ended_by });
                }
            }
        }
    }
    out.sort_by(|a, b| (&a.kind, &a.first, &a.second, a.start_hour, a.first_seen).cmp(&(&b.kind, &b.first, &b.second, b.start_hour, b.first_seen)));
    let mut joined: Vec<DiplomacyPact> = Vec::with_capacity(out.len());
    for pact in out {
        match joined.last_mut() {
            Some(held) if (&held.kind, &held.first, &held.second, held.start_hour) == (&pact.kind, &pact.first, &pact.second, pact.start_hour) && pact.last_seen <= held.last_seen => {}
            Some(held)
                if (&held.kind, &held.first, &held.second, held.start_hour) == (&pact.kind, &pact.first, &pact.second, pact.start_hour)
                    && i64::from(held.last_seen) + 1 >= i64::from(pact.first_seen) =>
            {
                held.last_seen = pact.last_seen;
                held.end_hour = pact.end_hour;
                held.gone_by = pact.gone_by;
                held.held = pact.held;
                held.ended_by = pact.ended_by;
            }
            _ => joined.push(pact),
        }
    }
    let mut counts: BTreeMap<String, u32> = BTreeMap::new();
    for pact in &joined {
        *counts.entry(pact.kind.clone()).or_default() += 1;
    }
    let mut out = joined;
    out.sort_by(|a, b| (a.first_seen, &a.kind, &a.first, &a.second).cmp(&(b.first_seen, &b.kind, &b.first, &b.second)));
    (out, counts)
}

fn next_change(query: &Query<'_>, held: &HashMap<u32, (Arc<RunPacts>, Arc<RunLinks>)>, ids: &[(u32, i32, i32)], unobserved: &[HourSpan], hour: i32) -> Option<i32> {
    let mut next = i32::MAX;
    let mut after = |at: i32| {
        if at > hour && at < next {
            next = at;
        }
    };
    for gap in unobserved {
        after(gap.first_hour);
        after(gap.last_hour.saturating_add(1));
    }
    for runs in query.holders.tags.values() {
        for run in runs {
            after(run.first);
            after(run.last.saturating_add(1));
        }
    }
    for stretch in &query.timeline.stretches {
        after(stretch.first);
        after(stretch.last.saturating_add(1));
        let Some((pacts, _)) = held.get(&stretch.run) else { continue };
        for &(id, _, _) in ids {
            let mine = |key: u128| (key >> 80) as u32 == id || (key >> 48) as u32 == id;
            for run in pacts.rows.iter().filter(|(key, _)| mine(*key)).flat_map(|(_, runs)| runs) {
                after(run.first.max(stretch.first));
                after(run.last.min(stretch.last).saturating_add(1));
            }
        }
    }
    (next != i32::MAX).then_some(next)
}

pub(crate) fn unproven(stretches: &[Stretch], held: &HashMap<u32, (Arc<RunPacts>, Arc<RunLinks>)>) -> Vec<HourSpan> {
    let mut out = Vec::new();
    for stretch in stretches {
        let Some((pacts, _)) = held.get(&stretch.run) else { continue };
        for &(first, last) in &pacts.unproven {
            let (first, last) = (first.max(stretch.first), last.min(stretch.last));
            if first <= last {
                out.push(HourSpan { first_hour: first, last_hour: last });
            }
        }
    }
    out
}

pub(crate) fn kind_titles(counts: BTreeMap<String, u32>, tokens: &HashMap<&str, u32>, catalogue: Result<Option<&Catalogue>, String>) -> (Vec<DiplomacyKind>, Vec<String>) {
    let mut warnings = Vec::new();
    let kinds = counts
        .into_iter()
        .map(|(kind, count)| {
            let token = tokens.get(kind.as_str()).copied().unwrap_or(0);
            let named = content_reader_politics::diplomacy::relation_title_key(token, &kind)
                .map_err(|reason| reason.replace("(count: 1)", &format!("(count: {count})")))
                .and_then(|key| match &catalogue {
                    Ok(Some(catalogue)) => catalogue.localised(key, catalogue::View::Plain).ok_or_else(|| format!("Relation {kind}: title {key} is unavailable (count: {count})")),
                    Ok(None) => Err(format!("Relation {kind}: recorded content is not configured (count: {count})")),
                    Err(error) => Err(format!("Relation {kind}: {error} (count: {count})")),
                });
            let (name, reason) = match named {
                Ok(name) => (Some(name), None),
                Err(reason) => {
                    warnings.push(format!("Relation title unavailable for {count} occurrences: {reason}"));
                    (None, Some(reason))
                }
            };
            DiplomacyKind { kind, count, name, reason }
        })
        .collect();
    (kinds, warnings)
}

impl Product for DiplomaticPacts {
    const KEY: &'static str = "diplomatic-pacts";
    const CONTENT_DEPENDENT: bool = true;
    type Params = DiplomaticPactsParams;
    const FIELDS: dashboard_base::products::Fields = dashboard_base::fields![branchId, country, hour: Read];
    type Reply = DiplomacyPactsReply;

    #[cfg(any(test, feature = "export-types"))]
    fn types() -> Vec<String> {
        vec![DiplomaticPactsParams::decl(), DiplomacyKind::decl(), DiplomacyPact::decl(), DiplomacyPactsReply::decl()]
    }

    fn validate(params: &Self::Params) -> Result<(), &'static str> {
        if params.branch_id.parse::<u64>().is_err() {
            return Err("Invalid branch");
        }
        validate_country(&params.country)
    }

    fn load(source: &Source, params: &Self::Params, context: &LoadContext, cancelled: &AtomicBool) -> io::Result<Self::Reply> {
        let branch = params.branch_id.parse::<u64>().map_err(|_| dashboard_core::invalid("Invalid branch"))?;
        let query = dashboard_base::queries::query(source, branch)?;
        let names = Names::new(&query.holders);
        let mut held = HashMap::new();
        for stretch in &query.timeline.stretches {
            dashboard_text::metadata::check_cancel(cancelled)?;
            if let std::collections::hash_map::Entry::Vacant(slot) = held.entry(stretch.run) {
                slot.insert((crate::products::diplomacy::cached_pacts(source, stretch.run)?, dashboard_base::queries::links(source, stretch.run)?));
            }
        }
        let ids = tag_ids(&query.holders, &params.country);
        let hour = params.hour.min(query.timeline.last);
        let (pacts, counts) = pacts(&query.timeline.stretches, &held, &names, &ids, hour);
        let tokens: HashMap<&str, u32> = held.values().flat_map(|(run, _)| run.kinds.iter().map(|(token, kind)| (kind.as_str(), *token))).collect();
        let catalogue = context.content.as_ref().map(|options| catalogue::catalogue(source, options)).transpose().map_err(|error| error.to_string());
        let (kinds, mut warnings) = kind_titles(counts, &tokens, catalogue.as_ref().map(|held| held.as_deref()).map_err(Clone::clone));
        if let Ok(Some(catalogue)) = &catalogue {
            warnings.extend(catalogue.refusal_lines());
        }
        let mut unobserved = dashboard_world::products::subjects::unknown_hours(&query);
        unobserved.extend(unproven(&query.timeline.stretches, &held));
        let unobserved = dashboard_world::products::subjects::settle(unobserved);
        let next_change = next_change(&query, &held, &ids, &unobserved, hour);
        let unobserved = unobserved.into_iter().filter(|gap| gap.first_hour <= hour).map(|gap| HourSpan { first_hour: gap.first_hour, last_hour: gap.last_hour.min(hour) }).collect();
        Ok(DiplomacyPactsReply { hour, next_change, kinds, pacts, unobserved, warnings })
    }
}

#[cfg(test)]
mod tests;

enum RunKey {}

type Runs = dashboard_base::queries::RunCache<RunKey, RunPacts>;

pub(crate) fn forget(source: &dashboard_base::Source) {
    Runs::forget(source);
}

pub fn cached_pacts(source: &dashboard_base::Source, run: u32) -> io::Result<std::sync::Arc<RunPacts>> {
    Runs::held(&source.state::<Runs>(), run, || run_pacts(source.replay_ref().recording(), run))
}
