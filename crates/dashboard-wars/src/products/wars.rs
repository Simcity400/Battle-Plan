
mod at;
pub mod contributions;
pub mod history;
mod inputs;
mod measures;
mod native;
mod naming;
mod participation;
mod walk;
#[cfg(test)]
mod tests;

pub use at::WarTimelineReply;
pub use inputs::{RunWars, run_wars};
pub use measures::{WarMeasure, WarMeasureKey, WarMeasureMember, WarMeasureSide};
pub use participation::{WarContributionCountry, WarContributionGroup};

use dashboard_base::products::{LoadContext, Product, at_hour};
use dashboard_world::products::{groups::MembershipRun, styles::RunLooks};
use dashboard_base::{Source, catalogue::{self, Catalogue}, span::HourSpan};
use dashboard_base::products::{Country, check_reply_budget, validate_country};
use dashboard_query::{
    Query,
    identity::{Holder, Holders},
};
use content_reader_country_names::country_names::{CountryNames, Operands};
use inputs::{Change, Countries, Hour, Input, Resolve, Row, Shown, State, holds};
use native::{Blob, Grouping, Pair, ScoreOrder};
use replay_looks::Look;
use serde::{Deserialize, Serialize};
use std::{
    collections::{BTreeMap, BTreeSet, HashMap},
    io,
    sync::{Arc, PoisonError, atomic::AtomicBool},
};
#[cfg(any(test, feature = "export-types"))]
use ts_rs::TS;
use walk::{Seen, Viewed, Walk};

pub struct Wars;
pub struct WarTimelineAt;

#[derive(Clone, Debug, PartialEq, Deserialize, Serialize)]
#[cfg_attr(any(test, feature = "export-types"), derive(TS))]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct WarTimelineParams {
    pub branch_id: String,
    #[doc = " None follows the recording client's playing or observer country."]
    pub viewpoint: Option<Country>,
    #[doc = " The hour answered: nothing recorded after it is read into the reply."]
    pub hour: i32,
}

#[derive(Clone, Debug, PartialEq, Deserialize, Serialize)]
#[cfg_attr(any(test, feature = "export-types"), derive(TS))]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct WarsParams {
    pub branch_id: String,
    #[doc = " None follows the recording client's playing or observer country."]
    pub viewpoint: Option<Country>,
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq, Hash)]
#[cfg_attr(any(test, feature = "export-types"), derive(TS))]
#[serde(rename_all = "camelCase")]
pub struct WarName {
    #[doc = r#" Exact plain text, or "Not recorded" with a reason."#]
    pub text: String,
    pub known: bool,
    pub reason: Option<String>,
    pub key: Option<String>,
}

#[derive(Clone, Copy, Debug, Deserialize, Serialize, PartialEq, Eq, Hash)]
#[cfg_attr(any(test, feature = "export-types"), derive(TS))]
#[serde(rename_all = "camelCase")]
pub enum WarSide {
    Attacker,
    Defender,
    Viewpoint,
    Opponent,
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[cfg_attr(any(test, feature = "export-types"), derive(TS))]
#[serde(rename_all = "camelCase")]
pub struct WarParticipantRun {
    pub tag: String,
    pub side: WarSide,
    pub first_hour: i32,
    pub last_hour: i32,
    pub joined_by: bool,
    pub left_by: bool,
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[cfg_attr(any(test, feature = "export-types"), derive(TS))]
#[serde(rename_all = "camelCase")]
pub struct WarCapitulation {
    pub tag: String,
    pub hour: i32,
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[cfg_attr(any(test, feature = "export-types"), derive(TS))]
#[serde(rename_all = "camelCase")]
pub struct WarObservation {
    pub first_hour: i32,
    pub last_hour: i32,
    pub viewpoint_tag: String,
    #[doc = " Native order, including any repeated occurrence."]
    pub viewpoint_side: Vec<String>,
    pub opposing_side: Vec<String>,
    #[doc = " Orientation follows the representative's instigator, an app convention. None where the"]
    #[doc = " representative's operands are not recorded."]
    pub attackers_are_viewpoint: Option<bool>,
    pub representative: Option<[String; 2]>,
    pub name: WarName,
    #[serde(skip_serializing_if = "Option::is_none")]
    #[cfg_attr(any(test, feature = "export-types"), ts(optional))]
    pub contribution_group: Option<[Vec<String>; 2]>,
}

#[derive(Clone, Debug, PartialEq, Deserialize, Serialize)]
#[cfg_attr(any(test, feature = "export-types"), derive(TS))]
#[serde(rename_all = "camelCase")]
pub struct WarTimeline {
    #[doc = " Timeline-local app identity; never a native war id."]
    pub id: String,
    pub first_hour: i32,
    pub last_hour: i32,
    pub started_by: bool,
    pub ended_by: bool,
    pub observations: Vec<WarObservation>,
    pub participants: Vec<WarParticipantRun>,
    pub capitulations: Vec<WarCapitulation>,
    #[serde(skip)]
    #[cfg_attr(any(test, feature = "export-types"), ts(skip))]
    pub(crate) relations: Vec<WarRelationMembership>,
    #[serde(skip)]
    #[cfg_attr(any(test, feature = "export-types"), ts(skip))]
    open_participants: HashMap<(String, WarSide), usize>,
    #[serde(skip)]
    #[cfg_attr(any(test, feature = "export-types"), ts(skip))]
    open_relations: HashMap<(WarRelationKey, bool), usize>,
}

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

    pub fn lifetime(&self) -> u128 {
        u128::from(self.first.0) << 104 | u128::from(self.first.1) << 80 | u128::from(self.second.0) << 56 | u128::from(self.second.1) << 32 | u128::from(self.start as u32)
    }
}

#[derive(Clone, Debug, PartialEq)]
pub(crate) struct WarRelationMembership {
    pub key: WarRelationKey,
    pub oriented: bool,
    pub first_hour: i32,
    pub last_hour: i32,
}

#[derive(Clone, Debug, PartialEq, Deserialize, Serialize)]
#[cfg_attr(any(test, feature = "export-types"), derive(TS))]
#[serde(rename_all = "camelCase")]
pub struct WarsReply {
    pub branch_id: String,
    #[doc = " The war tables were read at some hour of the timeline."]
    pub available: bool,
    #[doc = " The overview's viewpoint over consecutive read hours, including peace."]
    pub viewpoints: Vec<MembershipRun>,
    #[doc = " Names of the countries the wars show, as read where they were named: for measuring labels"]
    #[doc = " only, never for naming a country at an hour."]
    pub country_names: Vec<String>,
    pub wars: Vec<WarTimeline>,
    pub unobserved: Vec<HourSpan>,
    #[serde(skip)]
    #[cfg_attr(any(test, feature = "export-types"), ts(skip))]
    pub(crate) partial_relations: Vec<(i32, i32)>,
    pub capitulations_unobserved: Vec<HourSpan>,
    pub warnings: Vec<String>,
}

dashboard_base::reply_warnings!(WarsReply);

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

    fn changes(&self, slot: u32) -> impl Iterator<Item = i32> + '_ {
        self.slots.get(&slot).into_iter().flatten().flat_map(|run| [run.0, run.1.saturating_add(1)])
    }
}

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

    fn country_slot(&self, country: &Country) -> Result<u32, String> {
        match self.holders.at(country, self.hour).map(|run| run.holder) {
            Some(Holder::Found { slot, .. }) => Ok(slot),
            _ => Err(self.holders.not_known(country, self.hour).unwrap_or_else(|| country.tag().map_or_else(|| "Not in this recording: viewpoint".to_owned(), |tag| format!("Not in this recording: viewpoint {tag}")))),
        }
    }
}

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

struct Dated<'r> {
    inputs: &'r RunWars,
    looks: &'r RunNaming,
    prefix: Option<&'r [u8]>,
    at: i32,
}

struct Names<'a> {
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
            let reads: Vec<at_hour::Read> = content_reader_country_names::country_names::TABLES.iter().map(|&table| at_hour::Read::all(table)).collect();
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

#[derive(Default)]
struct ObservationBoundary {
    links_changed: bool,
    recheck: bool,
}

fn observed_hours(inputs: &RunWars, links: &dashboard_base::queries::country_ids::RunLinks, names: &SlotNames, holders: &Holders, first: i32, last: i32) -> Vec<(i32, ObservationBoundary)> {
    let mut hours = BTreeMap::from([(first, ObservationBoundary { links_changed: true, recheck: true })]);
    let mut add = |hour: i32, links_changed: bool, recheck: bool| {
        if first < hour && hour <= last {
            let boundary = hours.entry(hour).or_default();
            boundary.links_changed |= links_changed;
            boundary.recheck |= recheck;
        }
    };
    for change in &inputs.changes {
        add(change.hour, false, false);
    }
    for runs in [&inputs.read, &inputs.capitulations] {
        for &(a, b) in runs.iter() {
            add(a, false, true);
            add(b.saturating_add(1), false, true);
        }
    }
    let mut slots = BTreeSet::new();
    for raw in &inputs.raws {
        for &(a, b, slot) in links.slots.get(raw).into_iter().flatten() {
            add(a, true, true);
            add(b.saturating_add(1), true, true);
            if let Some(slot) = slot {
                slots.insert(slot as u32);
            }
        }
    }
    for slot in slots {
        for hour in names.changes(slot) {
            add(hour, false, true);
        }
    }
    for &(a, b, _) in &holders.unknown {
        add(a, false, true);
        add(b.saturating_add(1), false, true);
    }
    hours.into_iter().collect()
}

struct ObservationInputs {
    countries: Arc<Countries>,
    scores: HashMap<u32, Option<(u32, i32)>>,
    orders: Vec<ScoreOrder>,
    hour: i32,
    viewpoint: u32,
    through: i32,
}

impl ObservationInputs {
    fn keeps(&mut self, change: &Change, state: &State) -> bool {
        let (Input::Score, Shown::Row(Row::Score(next))) = (change.input, &change.shown) else { return false };
        let Ok(raw) = u32::try_from(change.key) else { return false };
        let (Some(before), Some(slot)) = (state.scores.get(&raw), self.countries.raw_to_slot.get(&raw)) else { return false };
        let Some(held) = self.scores.get_mut(slot) else { return true };
        if let Some((owner, total)) = held
            && *owner == raw
        {
            *total = next.divisions.wrapping_add(next.factories);
        }
        before.score_available == next.score_available && before.major == next.major && before.view_subject == next.view_subject
    }

    fn same_order(&self) -> bool {
        let total = |slot| self.scores.get(&slot).and_then(|&held| held.map(|(_, score)| score));
        self.orders.iter().all(|check| {
            total(check.first).zip(total(check.second)).is_some_and(|(a, b)| a.cmp(&b) == check.order)
        })
    }
}

fn changes_grouping(change: &Change, group: &Grouping, link: impl Fn(u32) -> Option<u32>) -> bool {
    let relevant = |raw| link(raw).is_none_or(|slot| group.countries.contains(&slot));
    let first = (change.key >> 32) as u32;
    let second = change.key as u32;
    match change.input {
        Input::Relation => relevant(first) || relevant(second),
        Input::Owned => relevant(first),
        Input::Faction => relevant(second),
        Input::Score | Input::Rule | Input::Local | Input::Capitulation => false,
    }
}

fn timeline_wars(source: &Source, query: &Query<'_>, view: Option<&Country>, cat: Option<&Catalogue>, cancelled: &AtomicBool) -> io::Result<WarsReply> {
    #[cfg(test)]
    tests::count_walk();
    let timeline = &query.timeline;
    let names = SlotNames::new(&query.holders);
    let mut walk = Walk::new(timeline.id, timeline.first);
    let mut naming = Names { frames: at_hour::Frames::new(query)?, cat, held: HashMap::new(), countries: BTreeSet::new() };
    let mut looks: HashMap<u32, Arc<RunNaming>> = HashMap::new();
    for stretch in &timeline.stretches {
        let inputs = crate::products::wars::cached_wars(source, stretch.run)?;
        let links = dashboard_base::queries::links(source, stretch.run)?;
        let run_looks = match (cat, looks.get(&stretch.run)) {
            (None, _) => None,
            (Some(_), Some(held)) => Some(Arc::clone(held)),
            (Some(_), None) => {
                let made = Arc::new(RunNaming::new(dashboard_world::products::styles::cached_looks(source, stretch.run)?));
                looks.insert(stretch.run, Arc::clone(&made));
                Some(made)
            }
        };
        walk.reply.available |= inputs.read.iter().any(|&(a, b)| a <= stretch.last && stretch.first <= b);
        walk.reply.partial_relations.extend(inputs.partial_relations.iter().filter_map(|&(first, last)| {
            let (first, last) = (first.max(stretch.first), last.min(stretch.last));
            (first <= last).then_some((first, last))
        }));
        let hours = observed_hours(&inputs, &links, &names, &query.holders, stretch.first, stretch.last);
        let zero = |hour| links.slot_at(0, hour).map(|slot| (slot, names.at(slot, hour)));
        let mut state = State::default();
        let mut applied = 0;
        let mut grouping = None::<Grouping>;
        let mut zero_link = None;
        let mut observation = None::<ObservationInputs>;
        for (index, &(hour, ObservationBoundary { links_changed, recheck })) in hours.iter().enumerate() {
            dashboard_text::metadata::check_cancel(cancelled)?;
            let before = applied;
            let mut reuse = !recheck && walk.can_repeat_observation() && observation.as_ref().is_some_and(|held| {
                hour <= held.through && zero(hour) == zero(held.hour) && names.at(held.viewpoint, hour) == names.at(held.viewpoint, held.hour)
            });
            while let Some(change) = inputs.changes.get(applied).filter(|change| change.hour <= hour) {
                if reuse { reuse = observation.as_mut().is_some_and(|held| held.keeps(change, &state)); }
                state.apply(change);
                applied += 1;
            }
            let through = hours.get(index + 1).map_or(stretch.last, |(next, _)| next - 1);
            if reuse && observation.as_ref().is_some_and(ObservationInputs::same_order) {
                walk.extend(through);
                continue;
            }
            observation = None;
            let read = if holds(&inputs.read, hour) {
                let link = |raw: u32| links.slot_at(raw, hour);
                let resolve = Resolved { links: &link, names: &names, holders: &query.holders, hour };
                #[cfg(test)]
                tests::count_projection();
                match Hour::build(&state, &resolve, view) {
                    Ok(read) => {
                        let current_zero = read.raw_to_slot.get(&0).copied();
                        if links_changed || current_zero != zero_link || grouping.as_ref().is_some_and(|held| {
                            held.view != read.view || inputs.changes[before..applied].iter().any(|change| changes_grouping(change, held, link))
                        }) {
                            grouping = None;
                        }
                        zero_link = current_zero;
                        let groups = grouping.get_or_insert_with(|| {
                            #[cfg(test)]
                            tests::count_grouping(hour);
                            read.world.grouping(read.view)
                        });
                        let mut observed = Vec::new();
                        let mut scores = HashMap::new();
                        let mut orders = Vec::new();
                        for (blob, pairs) in &groups.blobs {
                            let named = match read.world.representative(blob, pairs, &mut orders) {
                                Ok(p) => {
                                    let name = match &run_looks {
                                        Some(looks) => {
                                            let prefix = state.rules.get(&0).map(|(prefix, _)| prefix.as_slice());
                                            naming.name(&read, p, blob, &Dated { inputs: &inputs, looks, prefix, at: hour })?
                                        }
                                        None => naming::unknown("Not recorded: the recorded content stack is not resolved"),
                                    };
                                    Ok((p, name))
                                }
                                Err(reason) => Err(reason),
                            };
                            scores.extend(blob.first.iter().chain(&blob.second).map(|&slot| (slot, None)));
                            observed.push(walk::observed(&read.world, (&read.tags, &read.identities), read.view, (blob, pairs), named, hour));
                        }
                        let countries = Arc::new(read.countries());
                        if !orders.is_empty() {
                            for (&raw, score) in &state.scores {
                                if let Some(held) = countries.raw_to_slot.get(&raw).and_then(|slot| scores.get_mut(slot)) {
                                    *held = Some((raw, score.divisions.wrapping_add(score.factories)));
                                }
                            }
                        }
                        let through = view.map_or(i32::MAX, |country| query.holders.at(country, hour).map_or(hour, |held| held.last));
                        observation = Some(ObservationInputs { countries: Arc::clone(&countries), scores, orders, hour, viewpoint: read.view, through });
                        Ok((Viewed { countries, view: read.view }, observed))
                    }
                    Err(reason) => Err(reason),
                }
            } else {
                Err("Not recorded: the war tables were not read at this hour".to_owned())
            };
            if read.is_err() {
                grouping = None;
            }
            let capitulated = state.capitulated();
            if capitulated.iter().any(|&(_, at)| at > hour) { observation = None; }
            let capitulations = if holds(&inputs.capitulations, hour) { Ok(capitulated.as_slice()) } else { Err(()) };
            walk.observe(Seen { hour, read, capitulations });
            walk.extend(through);
        }
    }
    let mut reply = walk.finish(timeline.last);
    reply.country_names = naming.countries.into_iter().collect();
    if cat.is_none() {
        reply.warnings.push("War names are Not recorded until the recorded content stack is resolved.".into());
    }
    Ok(reply)
}

#[derive(Default)]
struct Walks(HashMap<(u64, Option<Country>), (Option<Arc<Catalogue>>, Arc<WarsReply>)>);

fn forget_walks(source: &Source) {
    source.state::<Walks>().lock().unwrap_or_else(PoisonError::into_inner).0.clear();
}

#[cfg(feature = "test-support")]
pub fn has_cached_walks(source: &Source) -> bool {
    !source.state::<Walks>().lock().unwrap_or_else(PoisonError::into_inner).0.is_empty()
}

fn catalogue_of(source: &Source, context: &LoadContext) -> io::Result<Option<Arc<Catalogue>>> {
    context.content.as_ref().map(|options| catalogue::catalogue(source, options)).transpose()
}

fn walked(source: &Source, branch: u64, view: Option<&Country>, cat: Option<Arc<Catalogue>>, cancelled: &AtomicBool) -> io::Result<Arc<WarsReply>> {
    let key = (branch, view.cloned());
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
    let query = dashboard_base::queries::query(source, branch)?;
    let reply = Arc::new(timeline_wars(source, &query, view, cat.as_deref(), cancelled)?);
    source.state::<Walks>().lock().unwrap_or_else(PoisonError::into_inner).0.insert(key, (cat, Arc::clone(&reply)));
    Ok(reply)
}

pub(crate) fn viewpoint_wars(source: &Source, branch: u64, view: &Country, context: &LoadContext, cancelled: &AtomicBool) -> io::Result<Arc<WarsReply>> {
    walked(source, branch, Some(view), catalogue_of(source, context)?, cancelled)
}

fn branch(id: &str) -> io::Result<u64> {
    id.parse().map_err(|_| dashboard_core::invalid("Invalid wars branch"))
}

impl Product for Wars {
    const KEY: &'static str = "wars";
    const CONTENT_DEPENDENT: bool = true;
    type Params = WarsParams;
    const FIELDS: dashboard_base::products::Fields = dashboard_base::fields![branchId, viewpoint];
    type Reply = WarsReply;

    #[cfg(any(test, feature = "export-types"))]
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
        params.viewpoint.as_ref().map_or(Ok(()), validate_country)
    }

    fn load(source: &Source, params: &Self::Params, context: &LoadContext, cancelled: &AtomicBool) -> io::Result<Self::Reply> {
        let cat = catalogue_of(source, context)?;
        let reply = walked(source, branch(&params.branch_id)?, params.viewpoint.as_ref(), cat, cancelled)?;
        check_reply_budget(&*reply)?;
        Ok((*reply).clone())
    }
}

impl Product for WarTimelineAt {
    const KEY: &'static str = "war-timeline";
    const CONTENT_DEPENDENT: bool = true;
    type Params = WarTimelineParams;
    const FIELDS: dashboard_base::products::Fields = dashboard_base::fields![branchId, viewpoint, hour: Read];
    type Reply = WarTimelineReply;

    #[cfg(any(test, feature = "export-types"))]
    fn types() -> Vec<String> {
        vec![WarTimelineParams::decl(), WarTimelineReply::decl()]
    }

    fn validate(params: &Self::Params) -> Result<(), &'static str> {
        params.branch_id.parse::<u64>().map_err(|_| "Invalid wars branch")?;
        params.viewpoint.as_ref().map_or(Ok(()), validate_country)
    }

    fn load(source: &Source, params: &Self::Params, context: &LoadContext, cancelled: &AtomicBool) -> io::Result<Self::Reply> {
        let branch = branch(&params.branch_id)?;
        let cat = catalogue_of(source, context)?;
        let walked = walked(source, branch, params.viewpoint.as_ref(), cat, cancelled)?;
        let last = dashboard_base::queries::query(source, branch)?.timeline.last;
        let reply = at::at(&walked, params.hour.min(last));
        check_reply_budget(&reply)?;
        Ok(reply)
    }
}

enum RunKey {}

type Runs = dashboard_base::queries::RunCache<RunKey, RunWars>;

pub(crate) fn forget(source: &dashboard_base::Source) {
    forget_walks(source);
    Runs::forget(source);
}

pub fn cached_wars(source: &dashboard_base::Source, run: u32) -> io::Result<std::sync::Arc<RunWars>> {
    Runs::held(&source.state::<Runs>(), run, || run_wars(source.replay_ref().recording(), run))
}
