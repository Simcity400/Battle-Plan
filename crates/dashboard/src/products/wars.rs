//! `wars`: each war a viewpoint sees over one timeline, with its names, sides, participants and
//! capitulations; `war-losses`: each of those wars' casualties at an hour; `war-contributions`
//! ([`contributions`]): each native group's contributions at an hour, rows at an hour.
//!
//! The game holds no war as such: the war overview groups the viewpoint's active relations into
//! native groups at each hour ([`native`]), and continuity across hours is the app's
//! ([`walk`]). Each run's inputs are read once as the changes of the overview tables' structural
//! columns ([`inputs::RunWars`]); a viewpoint's walk observes the hours they change, each hour's
//! rows replayed in memory, and extends the wars between. Names are dated at the first hour a
//! representative relation names a group for the viewpoint, from what stood then: the goals,
//! token texts and state names the run's inputs hold as histories, the countries' naming operands
//! from the run's looks, so naming reads no table at an hour. Casualties
//! are read from `CasualtyLifetimes` at the hour asked, by the relations each war held then.

pub mod contributions;
mod inputs;
mod native;
mod naming;
mod participation;
mod walk;
#[cfg(test)]
mod tests;

pub use inputs::{RunWars, run_wars};
pub use participation::{WarContributionCountry, WarContributionGroup};

use super::{LoadContext, Product, at_hour, groups::MembershipRun, styles::RunLooks};
use crate::{
    Source,
    catalogue::{self, Catalogue},
    span::HourSpan,
};
use dashboard_base::products::{check_reply_budget, validate_tag};
use dashboard_query::{
    Query,
    identity::{Holder, Holders},
    read,
};
use installed_content_country_names::country_names::{CountryNames, Operands};
use inputs::{Hour, Resolve, State, holds};
use native::{Blob, Pair};
use recorder_derive::looks::Look;
use replay_data::rows::TableId;
use serde::{Deserialize, Serialize};
use std::{
    collections::{BTreeMap, BTreeSet, HashMap},
    io,
    sync::{Arc, PoisonError, atomic::AtomicBool},
};
use ts_rs::TS;
use walk::{Seen, Viewed, Walk};

pub struct Wars;
pub struct WarLosses;

#[derive(Clone, Debug, Deserialize, Serialize, TS)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct WarsParams {
    pub branch_id: String,
    /// None follows the recording client's playing or observer country.
    pub viewpoint_tag: Option<String>,
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq, Hash, TS)]
#[serde(rename_all = "camelCase")]
pub struct WarName {
    /// Exact plain text, or "Not recorded" with a reason.
    pub text: String,
    pub known: bool,
    pub reason: Option<String>,
    pub key: Option<String>,
}

#[derive(Clone, Copy, Debug, Deserialize, Serialize, PartialEq, Eq, Hash, TS)]
#[serde(rename_all = "camelCase")]
pub enum WarSide {
    Attacker,
    Defender,
    Viewpoint,
    Opponent,
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq, TS)]
#[serde(rename_all = "camelCase")]
pub struct WarParticipantRun {
    pub tag: String,
    pub side: WarSide,
    pub first_hour: i32,
    pub last_hour: i32,
    pub joined_by: bool,
    pub left_by: bool,
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq, TS)]
#[serde(rename_all = "camelCase")]
pub struct WarCapitulation {
    pub tag: String,
    pub hour: i32,
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq, TS)]
#[serde(rename_all = "camelCase")]
pub struct WarObservation {
    pub first_hour: i32,
    pub last_hour: i32,
    pub viewpoint_tag: String,
    /// Native order, including any repeated occurrence.
    pub viewpoint_side: Vec<String>,
    pub opposing_side: Vec<String>,
    /// Orientation follows the representative's instigator, an app convention. None where the
    /// representative's operands are not recorded.
    pub attackers_are_viewpoint: Option<bool>,
    pub representative: Option<[String; 2]>,
    pub name: WarName,
}

#[derive(Clone, Debug, Deserialize, Serialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct WarTimeline {
    /// Timeline-local app identity; never a native war id.
    pub id: String,
    pub first_hour: i32,
    pub last_hour: i32,
    pub started_by: bool,
    pub ended_by: bool,
    pub observations: Vec<WarObservation>,
    pub participants: Vec<WarParticipantRun>,
    pub capitulations: Vec<WarCapitulation>,
    /// The relation lifetimes the war held and when, for its casualties.
    #[serde(skip)]
    #[ts(skip)]
    pub(crate) relations: Vec<WarRelationMembership>,
    #[serde(skip)]
    #[ts(skip)]
    open_participants: HashMap<(String, WarSide), usize>,
    #[serde(skip)]
    #[ts(skip)]
    open_relations: HashMap<(WarRelationKey, bool), usize>,
}

/// A war relation's lifetime: its two country identities (canonical slot and raw id), the lesser
/// first, and its start hour, as `CasualtyLifetimes` keys it.
#[derive(Clone, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct WarRelationKey {
    pub first: (u32, u32),
    pub second: (u32, u32),
    pub start: i32,
}

impl WarRelationKey {
    pub(crate) fn new(a: (u32, u32), b: (u32, u32), start: i32) -> Self {
        let (first, second) = if a < b { (a, b) } else { (b, a) };
        Self { first, second, start }
    }

    /// Its `CasualtyLifetimes` key.
    pub fn lifetime(&self) -> u128 {
        u128::from(self.first.0) << 104 | u128::from(self.first.1) << 80 | u128::from(self.second.0) << 56 | u128::from(self.second.1) << 32 | u128::from(self.start as u32)
    }
}

/// A war's hold of one relation lifetime over consecutive hours.
#[derive(Clone, Debug)]
pub(crate) struct WarRelationMembership {
    pub key: WarRelationKey,
    /// Whether the key's first country is on the war's viewpoint side.
    pub oriented: bool,
    pub first_hour: i32,
    pub last_hour: i32,
}

#[derive(Clone, Debug, Deserialize, Serialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct WarsReply {
    pub branch_id: String,
    /// The war tables were read at some hour of the timeline.
    pub available: bool,
    /// The overview's viewpoint over consecutive read hours, including peace.
    pub viewpoints: Vec<MembershipRun>,
    /// Names of the countries the wars show, as read where they were named: for measuring labels
    /// only, never for naming a country at an hour.
    pub country_names: Vec<String>,
    pub wars: Vec<WarTimeline>,
    pub unobserved: Vec<HourSpan>,
    pub capitulations_unobserved: Vec<HourSpan>,
    pub warnings: Vec<String>,
}

#[derive(Clone, Debug, Deserialize, Serialize, TS)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct WarLossesParams {
    pub branch_id: String,
    pub viewpoint_tag: Option<String>,
    pub hour: i32,
}

/// [first hour, last hour, viewpoint side's known and unknown-attacker casualties, the opposing
/// side's]: what the war's relations had counted by then.
#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq, TS)]
pub struct WarCasualtyRun(pub i32, pub i32, pub String, pub String, pub String, pub String);

/// One war's casualties at the hour, or why they are not known.
#[derive(Clone, Debug, Deserialize, Serialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct WarCasualties {
    /// The war's id in the `wars` reply.
    pub id: String,
    pub run: Option<WarCasualtyRun>,
    pub reason: Option<String>,
}

#[derive(Clone, Debug, Deserialize, Serialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct WarLossesReply {
    pub branch_id: String,
    pub hour: i32,
    /// Each war the viewpoint saw at the hour, in the order of the wars.
    pub wars: Vec<WarCasualties>,
    pub warnings: Vec<String>,
}

dashboard_base::reply_warnings!(WarsReply, WarLossesReply);

/// Each canonical slot's holders over a timeline: the tag holding it and its country's raw id.
pub(crate) struct SlotNames {
    slots: HashMap<u32, Vec<(i32, i32, String, u32)>>,
}

impl SlotNames {
    pub fn new(holders: &Holders) -> Self {
        let mut slots: HashMap<u32, Vec<(i32, i32, String, u32)>> = HashMap::new();
        for (tag, runs) in &holders.tags {
            for run in runs {
                if let Holder::Found { slot, id } = run.holder {
                    slots.entry(slot).or_default().push((run.first, run.last, tag.clone(), id));
                }
            }
        }
        for runs in slots.values_mut() {
            runs.sort_unstable_by_key(|run| run.0);
        }
        Self { slots }
    }

    pub fn at(&self, slot: u32, hour: i32) -> Option<(&str, u32)> {
        let runs = self.slots.get(&slot)?;
        let at = runs.partition_point(|run| run.1 < hour);
        runs.get(at).filter(|run| run.0 <= hour).map(|run| (run.2.as_str(), run.3))
    }

    /// The hours `slot`'s holder changes.
    fn changes(&self, slot: u32) -> impl Iterator<Item = i32> + '_ {
        self.slots.get(&slot).into_iter().flatten().flat_map(|run| [run.0, run.1.saturating_add(1)])
    }
}

/// An hour's resolution: raw ids by `links`, slots by their holders, tags by the identity rule.
pub(crate) struct Resolved<'a> {
    pub links: &'a dyn Fn(u32) -> Option<u32>,
    pub names: &'a SlotNames,
    pub holders: &'a Holders,
    pub hour: i32,
}

impl Resolve for Resolved<'_> {
    fn slot(&self, raw: u32) -> Option<u32> {
        (self.links)(raw)
    }

    fn holder(&self, slot: u32) -> Option<(&str, u32)> {
        self.names.at(slot, self.hour)
    }

    fn tag_slot(&self, tag: &str) -> Option<u32> {
        match self.holders.at(tag, self.hour)?.holder {
            Holder::Found { slot, .. } => Some(slot),
            Holder::Unresolved => None,
        }
    }
}

/// What a name read for a group depends on that the walk holds: the representative relation and
/// its instigator, its countries' tags, the owned row its goals are read through, and the viewpoint
/// and what decides whether it is allied to either country.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
struct NameKey {
    relation: WarRelationKey,
    instigator: u8,
    tags: (String, String),
    owned: Option<(u32, u32, u32, u32)>,
    view: u32,
    recorded: bool,
    allied: [(bool, Option<u64>, Option<(u8, u8)>); 2],
}

const LOOKS_UNKNOWN: &str = "the country tables are not complete at this hour";

/// One run's looks, with each tag's spans of looks: a country's naming operands at an hour are
/// its holder's look then, looked up rather than read from the country tables.
struct RunNaming {
    looks: Arc<RunLooks>,
    by_tag: HashMap<Arc<str>, Vec<(i32, i32, u32)>>,
}

impl RunNaming {
    fn new(looks: Arc<RunLooks>) -> Self {
        let mut by_tag: HashMap<Arc<str>, Vec<(i32, i32, u32)>> = HashMap::new();
        for &(number, look, first, last) in &looks.spans {
            if let Some(tag) = looks.tags.get(&number) {
                by_tag.entry(Arc::clone(tag)).or_default().push((first, last, look));
            }
        }
        for spans in by_tag.values_mut() {
            spans.sort_unstable();
        }
        Self { looks, by_tag }
    }

    fn look(&self, tag: &str, hour: i32) -> Option<&Look> {
        let spans = self.by_tag.get(tag)?;
        let at = spans.partition_point(|span| span.1 < hour);
        spans.get(at).filter(|span| span.0 <= hour).and_then(|span| self.looks.looks.get(&span.2))
    }

    /// The naming operands of `slots` at `hour` (each slot's holder's look, with every overlord its
    /// name reaches), and which of slot zero's names its reserved row has built where one reaches it.
    fn operands(&self, tags: &HashMap<u32, String>, slots: &BTreeSet<u32>, hour: i32) -> (BTreeMap<u32, Result<Operands, String>>, Option<u8>) {
        let unknown = self.looks.unknown.iter().any(|&(a, b)| a <= hour && hour <= b);
        let (mut operands, mut built) = (BTreeMap::new(), None);
        for &slot in slots {
            match tags.get(&slot).and_then(|tag| self.look(tag, hour)).filter(|look| look.slot == slot) {
                Some(look) => {
                    built = built.or(look.built);
                    for (reached, held) in &look.chain {
                        operands.entry(*reached).or_insert_with(|| held.clone());
                    }
                }
                None if unknown => {
                    operands.entry(slot).or_insert_with(|| Err(LOOKS_UNKNOWN.to_owned()));
                }
                None => {}
            }
        }
        (operands, built)
    }
}

/// What a name is read from beside the walk's hour: the run's naming histories and looks, and the
/// naming rule's prefix at the hour.
struct Dated<'r> {
    inputs: &'r RunWars,
    looks: &'r RunNaming,
    prefix: Option<&'r [u8]>,
    at: i32,
}

/// The names a walk gives its groups, each read once at the first hour its inputs name it.
struct Names<'a> {
    /// Only for slot zero's operands, which no look holds.
    frames: at_hour::Frames<'a>,
    cat: Option<&'a Catalogue>,
    held: HashMap<NameKey, WarName>,
    countries: BTreeSet<String>,
}

impl Names<'_> {
    fn key(hour: &Hour, p: Pair) -> NameKey {
        let world = &hour.world;
        let relation = &world.relations[&p];
        let owned = hour.goal_owned.get(&p).map(|r| (r.owner, r.ordinal, r.first_goal_count, r.second_goal_count));
        let allied = |target: u32| {
            let faction = world.factions.get(&target).copied().filter(|faction| *faction != 0 && world.factions.get(&hour.view) == Some(faction));
            let scored = world.scores.get(&target).map(|score| (score.score_available, score.view_subject));
            (world.active(hour.view, target), faction, scored)
        };
        NameKey {
            relation: WarRelationKey::new(hour.identities[&p.0], hour.identities[&p.1], relation.start_hour),
            instigator: relation.first_was_instigator,
            tags: (hour.tags[&relation.first_country].clone(), hour.tags[&relation.second_country].clone()),
            owned,
            view: hour.view,
            recorded: hour.recorded_view == Some(hour.view),
            allied: [allied(relation.first_country), allied(relation.second_country)],
        }
    }

    /// The name `hour`'s viewpoint gives group `blob`, named for relation `p`, as it stood at
    /// `dated.at`.
    fn name(&mut self, hour: &Hour, p: Pair, blob: &Blob, dated: &Dated<'_>) -> io::Result<WarName> {
        let Some(cat) = self.cat else {
            return Ok(naming::unknown("Not recorded: the recorded content stack is not resolved"));
        };
        let key = Self::key(hour, p);
        if let Some(name) = self.held.get(&key) {
            return Ok(name.clone());
        }
        let relation = &hour.world.relations[&p];
        let mut slots: BTreeSet<u32> = blob.first.iter().chain(&blob.second).copied().collect();
        slots.extend([hour.view, relation.first_country, relation.second_country]);
        let goal = naming::first_goal(dated.inputs, hour, p, dated.at).ok().flatten();
        slots.extend(goal.and_then(|goal| naming::goal_country(hour, &goal).ok().flatten()));
        let (mut operands, mut built) = dated.looks.operands(&hour.tags, &slots, dated.at);
        if slots.contains(&0) && !operands.contains_key(&0) {
            let reads: Vec<at_hour::Read> = installed_content_country_names::country_names::TABLES.iter().map(|&table| at_hour::Read::all(table)).collect();
            let frame = self.frames.frame(dated.at, &reads, "War name")?;
            let reserved = CountryNames::read_selected(&frame, &cat.localisation, [0]);
            built = built.or(reserved.reserved_built());
            for (slot, held) in reserved.into_operands() {
                operands.entry(slot).or_insert(held);
            }
        }
        let names = CountryNames::recorded(&cat.localisation, operands, built);
        let facts = naming::Facts { cat, inputs: dated.inputs, prefix: dated.prefix, at: dated.at };
        let name = naming::name(&facts, &names, hour, p);
        for &slot in blob.first.iter().chain(&blob.second).chain([&hour.view]) {
            if let Some(country) = naming::country(&names, slot) {
                self.countries.insert(country);
            }
        }
        self.held.insert(key, name.clone());
        Ok(name)
    }
}

/// The hours a stretch's walk observes: its first, and each hour in it an input row, the hours
/// read, a link or a holder of a country the inputs name changes.
fn observed_hours(inputs: &RunWars, links: &super::country_ids::RunLinks, names: &SlotNames, holders: &Holders, first: i32, last: i32) -> Vec<i32> {
    let mut hours = BTreeSet::from([first]);
    let mut add = |hour: i32| {
        if first < hour && hour <= last {
            hours.insert(hour);
        }
    };
    for change in &inputs.changes {
        add(change.hour);
    }
    for runs in [&inputs.read, &inputs.capitulations] {
        for &(a, b) in runs.iter() {
            add(a);
            add(b.saturating_add(1));
        }
    }
    let mut slots = BTreeSet::new();
    for raw in &inputs.raws {
        for &(a, b, slot) in links.slots.get(raw).into_iter().flatten() {
            add(a);
            add(b.saturating_add(1));
            if let Some(slot) = slot {
                slots.insert(slot as u32);
            }
        }
    }
    for slot in slots {
        for hour in names.changes(slot) {
            add(hour);
        }
    }
    for &(a, b, _) in &holders.unknown {
        add(a);
        add(b.saturating_add(1));
    }
    hours.into_iter().collect()
}

/// `view`'s wars over `query`'s timeline, named through `cat`.
fn timeline_wars(source: &Source, query: &Query<'_>, view: Option<&str>, cat: Option<&Catalogue>, cancelled: &AtomicBool) -> io::Result<WarsReply> {
    let timeline = &query.timeline;
    let names = SlotNames::new(&query.holders);
    let mut walk = Walk::new(timeline.id, timeline.first);
    let mut naming = Names { frames: at_hour::Frames::new(query)?, cat, held: HashMap::new(), countries: BTreeSet::new() };
    let mut looks: HashMap<u32, Arc<RunNaming>> = HashMap::new();
    for stretch in &timeline.stretches {
        let inputs = crate::answer::wars(source, stretch.run)?;
        let links = crate::answer::links(source, stretch.run)?;
        let run_looks = match (cat, looks.get(&stretch.run)) {
            (None, _) => None,
            (Some(_), Some(held)) => Some(Arc::clone(held)),
            (Some(_), None) => {
                let made = Arc::new(RunNaming::new(crate::answer::looks(source, stretch.run)?));
                looks.insert(stretch.run, Arc::clone(&made));
                Some(made)
            }
        };
        walk.reply.available |= inputs.read.iter().any(|&(a, b)| a <= stretch.last && stretch.first <= b);
        let hours = observed_hours(&inputs, &links, &names, &query.holders, stretch.first, stretch.last);
        let mut state = State::default();
        let mut applied = 0;
        for (index, &hour) in hours.iter().enumerate() {
            crate::metadata::check_cancel(cancelled)?;
            while let Some(change) = inputs.changes.get(applied).filter(|change| change.hour <= hour) {
                state.apply(change);
                applied += 1;
            }
            let read = if holds(&inputs.read, hour) {
                let link = |raw: u32| links.slot_at(raw, hour);
                let resolve = Resolved { links: &link, names: &names, holders: &query.holders, hour };
                match Hour::build(&state, &resolve, view) {
                    Ok(read) => {
                        let mut observed = Vec::new();
                        for blob in read.world.blobs(read.view) {
                            let named = match read.world.representative(&blob) {
                                Ok(p) => {
                                    let name = match &run_looks {
                                        Some(looks) => {
                                            let prefix = state.rules.get(&0).map(|(prefix, _)| prefix.as_slice());
                                            naming.name(&read, p, &blob, &Dated { inputs: &inputs, looks, prefix, at: hour })?
                                        }
                                        None => naming::unknown("Not recorded: the recorded content stack is not resolved"),
                                    };
                                    Ok((p, name))
                                }
                                Err(reason) => Err(reason),
                            };
                            observed.push(walk::observed(&read.world, (&read.tags, &read.identities), read.view, &blob, named, hour));
                        }
                        Ok((Viewed { countries: Arc::new(read.countries()), view: read.view }, observed))
                    }
                    Err(reason) => Err(reason),
                }
            } else {
                Err("Not recorded: the war tables were not read at this hour".to_owned())
            };
            let capitulated = state.capitulated();
            let capitulations = if holds(&inputs.capitulations, hour) { Ok(capitulated.as_slice()) } else { Err(()) };
            walk.observe(Seen { hour, read, capitulations });
            walk.extend(hours.get(index + 1).map_or(stretch.last, |next| next - 1));
        }
    }
    let mut reply = walk.finish(timeline.last);
    reply.country_names = naming.countries.into_iter().collect();
    if cat.is_none() {
        reply.warnings.push("War names are Not recorded until the recorded content stack is resolved.".into());
    }
    Ok(reply)
}

/// Each timeline and viewpoint's walk, with the catalogue it was named through: kept while the
/// recording is open, every request of the viewpoint answered from it.
#[derive(Default)]
struct Walks(HashMap<(u64, Option<String>), (Option<Arc<Catalogue>>, Arc<WarsReply>)>);

/// Forgets every walk of `source`: the next request walks again.
pub(crate) fn forget(source: &Source) {
    source.state::<Walks>().lock().unwrap_or_else(PoisonError::into_inner).0.clear();
}

/// The content's catalogue a reply names through, when the request has content.
fn catalogue_of(source: &Source, context: &LoadContext) -> io::Result<Option<Arc<Catalogue>>> {
    context.content.as_ref().map(|options| catalogue::catalogue(source, options)).transpose()
}

/// `view`'s walk of timeline `branch`, walked once per timeline, viewpoint and catalogue.
fn walked(source: &Source, branch: u64, view: Option<&str>, cat: Option<Arc<Catalogue>>, cancelled: &AtomicBool) -> io::Result<Arc<WarsReply>> {
    let key = (branch, view.map(str::to_owned));
    let same = |held: &Option<Arc<Catalogue>>| match (held, &cat) {
        (Some(a), Some(b)) => Arc::ptr_eq(a, b),
        (None, None) => true,
        _ => false,
    };
    if let Some((held, reply)) = source.state::<Walks>().lock().unwrap_or_else(PoisonError::into_inner).0.get(&key)
        && same(held)
    {
        return Ok(Arc::clone(reply));
    }
    let query = crate::answer::query(source, branch)?;
    let reply = Arc::new(timeline_wars(source, &query, view, cat.as_deref(), cancelled)?);
    source.state::<Walks>().lock().unwrap_or_else(PoisonError::into_inner).0.insert(key, (cat, Arc::clone(&reply)));
    Ok(reply)
}

/// `view`'s wars of timeline `branch`, named through the request's content: the walk `wars`
/// answers that viewpoint from.
pub(crate) fn viewpoint_wars(source: &Source, branch: u64, view: &str, context: &LoadContext, cancelled: &AtomicBool) -> io::Result<Arc<WarsReply>> {
    walked(source, branch, Some(view), catalogue_of(source, context)?, cancelled)
}

fn branch(id: &str) -> io::Result<u64> {
    id.parse().map_err(|_| crate::invalid("Invalid wars branch"))
}

impl Product for Wars {
    const KEY: &'static str = "wars";
    const CONTENT_DEPENDENT: bool = true;
    type Params = WarsParams;
    type Reply = WarsReply;

    fn types() -> Vec<String> {
        vec![
            WarsParams::decl(),
            WarName::decl(),
            WarSide::decl(),
            WarParticipantRun::decl(),
            WarCapitulation::decl(),
            WarObservation::decl(),
            WarTimeline::decl(),
            HourSpan::decl(),
            MembershipRun::decl(),
            WarsReply::decl(),
        ]
    }

    fn validate(params: &Self::Params) -> Result<(), &'static str> {
        params.branch_id.parse::<u64>().map_err(|_| "Invalid wars branch")?;
        params.viewpoint_tag.as_deref().map_or(Ok(()), validate_tag)
    }

    fn load(source: &Source, params: &Self::Params, context: &LoadContext, cancelled: &AtomicBool) -> io::Result<Self::Reply> {
        let cat = catalogue_of(source, context)?;
        let reply = walked(source, branch(&params.branch_id)?, params.viewpoint_tag.as_deref(), cat, cancelled)?;
        check_reply_budget(&*reply)?;
        Ok((*reply).clone())
    }
}

/// The relation lifetimes `war` held at `hour`.
fn held(war: &WarTimeline, hour: i32) -> Vec<&WarRelationMembership> {
    war.relations.iter().filter(|membership| membership.first_hour <= hour && hour <= membership.last_hour).collect()
}

/// `CasualtyLifetimes`' counters, as the schema numbers them.
fn lifetime_columns() -> io::Result<[usize; 4]> {
    let field = |name| dashboard_declare::field(TableId::CasualtyLifetimes, name);
    Ok([field("taken_a")?, field("taken_b")?, field("unknown_a")?, field("unknown_b")?])
}

/// A war's casualties at an hour from the counters of the lifetimes it held then: each side's
/// known losses summed over its relations, and its unknown-attacker losses taken once per country
/// (the game copies them into each of the country's relations) at their largest. None where a
/// lifetime's counters are not known.
pub(crate) fn losses(held: &[(&WarRelationMembership, Option<[i64; 4]>)]) -> Option<[i128; 4]> {
    let mut sums = [0_i128; 4];
    let mut unknown: HashMap<((u32, u32), usize), i64> = HashMap::new();
    for (membership, counters) in held {
        let [taken_a, taken_b, unknown_a, unknown_b] = (*counters)?;
        let (a, b) = if membership.oriented { (0, 2) } else { (2, 0) };
        sums[a] += i128::from(taken_a);
        sums[b] += i128::from(taken_b);
        for (country, side, value) in [(membership.key.first, a + 1, unknown_a), (membership.key.second, b + 1, unknown_b)] {
            let most = unknown.entry((country, side)).or_insert(value);
            *most = (*most).max(value);
        }
    }
    for ((_, side), value) in unknown {
        sums[side] += i128::from(value);
    }
    Some(sums)
}

impl Product for WarLosses {
    const KEY: &'static str = "war-losses";
    const CONTENT_DEPENDENT: bool = true;
    const INTERACTIVE: bool = true;
    type Params = WarLossesParams;
    type Reply = WarLossesReply;

    fn types() -> Vec<String> {
        vec![WarLossesParams::decl(), WarCasualtyRun::decl(), WarCasualties::decl(), WarLossesReply::decl()]
    }

    fn validate(params: &Self::Params) -> Result<(), &'static str> {
        params.branch_id.parse::<u64>().map_err(|_| "Invalid wars branch")?;
        params.viewpoint_tag.as_deref().map_or(Ok(()), validate_tag)
    }

    fn load(source: &Source, params: &Self::Params, context: &LoadContext, cancelled: &AtomicBool) -> io::Result<Self::Reply> {
        let id = branch(&params.branch_id)?;
        let hour = params.hour;
        let cat = catalogue_of(source, context)?;
        let reply = walked(source, id, params.viewpoint_tag.as_deref(), cat, cancelled)?;
        let observed = |war: &&WarTimeline| {
            let at = war.observations.partition_point(|run| run.last_hour < hour);
            war.observations.get(at).is_some_and(|run| run.first_hour <= hour)
        };
        let wars: Vec<&WarTimeline> = reply.wars.iter().filter(observed).collect();
        let mut out = WarLossesReply { branch_id: params.branch_id.clone(), hour, wars: Vec::new(), warnings: Vec::new() };
        if wars.is_empty() {
            return Ok(out);
        }
        let query = crate::answer::query(source, id)?;
        let run = query.timeline.at(hour).ok_or_else(|| dashboard_core::refused("War losses hour is not recorded"))?.run;
        let mut keys: Vec<u128> = wars.iter().flat_map(|war| held(war, hour)).map(|membership| membership.key.lifetime()).collect();
        keys.sort_unstable();
        keys.dedup();
        let columns = lifetime_columns()?;
        let asked = [read::Read { table: TableId::CasualtyLifetimes, columns: None, keys: read::Keys::These(&keys) }];
        let snapshot = read::at(source.replay_ref().recording(), run, &asked, hour)?;
        let view = snapshot[0].view();
        let counters: HashMap<u128, Option<[i64; 4]>> = snapshot[0]
            .rows
            .iter()
            .map(|&(key, slot)| {
                let cell = |column: usize| view.number(column, slot);
                (key, (|| Some([cell(columns[0])?, cell(columns[1])?, cell(columns[2])?, cell(columns[3])?]))())
            })
            .collect();
        for war in wars {
            let lifetimes: Vec<_> = held(war, hour).into_iter().map(|membership| (membership, counters.get(&membership.key.lifetime()).copied().flatten())).collect();
            let entry = match losses(&lifetimes) {
                Some(sums) => {
                    let [a, b, c, d] = sums.map(|sum| sum.to_string());
                    WarCasualties { id: war.id.clone(), run: Some(WarCasualtyRun(hour, hour, a, b, c, d)), reason: None }
                }
                None => WarCasualties { id: war.id.clone(), run: None, reason: Some("Not recorded: a relation's casualties were not recorded at this hour".into()) },
            };
            out.wars.push(entry);
        }
        Ok(out)
    }
}
