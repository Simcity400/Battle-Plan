//! Native, viewpoint-dependent war blobs with explicitly app-owned continuity.
//! The game stores no group identity or participant join/leave dates.
mod labels;
mod naming;
mod native;
pub(crate) mod prepared;
#[cfg(test)]
pub(crate) mod tests;

use super::{
    LoadContext, MAX_PRODUCT_REPLY_BYTES, Product,
    country_groups::{HourSpan, MembershipRun},
    validate_tag,
};
#[cfg(test)]
use crate::metadata::validate_campaign;
use crate::{
    Source,
    catalogue::{self, Catalogue},
    invalid,
    metadata::branch_bounds,
};
use native::{Pair, World, pair};
#[cfg(test)]
use replay::{container::Replay, frame::FrameDecoder};
use replay::{
    frame::{DecodedFrame, TableView, ViewIdentity},
    rows::{self, TableId, tables},
};
use serde::{Deserialize, Serialize};
#[cfg(test)]
use std::sync::{Arc, atomic::Ordering};
use std::{
    collections::{BTreeSet, HashMap, HashSet},
    io,
    sync::atomic::AtomicBool,
};
use ts_rs::TS;

pub struct Wars;
#[derive(Clone, Debug, Deserialize, Serialize, TS)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct WarsParams {
    pub branch_id: String,
    /// None follows the recording client's playing/observer country.
    pub viewpoint_tag: Option<String>,
}
#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq, TS)]
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
    /// Orientation follows the representative's instigator, an app convention.
    /// None if the representative's operands are unavailable.
    pub attackers_are_viewpoint: Option<bool>,
    pub representative: Option<[String; 2]>,
    pub name: WarName,
}
/// [first hour, last hour, viewpoint known/unknown, opponent known/unknown].
/// Exact observed runs; a gap always starts a new run.
#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq, TS)]
pub struct WarCasualtyRun(
    pub i32,
    pub i32,
    pub String,
    pub String,
    pub String,
    pub String,
);
#[derive(Clone, Debug, Deserialize, Serialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct WarTimeline {
    /// Branch-local app identity; never a native war id.
    pub id: String,
    pub first_hour: i32,
    pub last_hour: i32,
    pub started_by: bool,
    pub ended_by: bool,
    pub observations: Vec<WarObservation>,
    pub casualties: Vec<WarCasualtyRun>,
    pub participants: Vec<WarParticipantRun>,
    pub capitulations: Vec<WarCapitulation>,
    /// Shared native membership projection for casualty attribution, not a wire
    /// field or another grouping algorithm.
    #[serde(skip)]
    #[ts(skip)]
    pub(crate) relations: Vec<WarRelationMembership>,
    #[serde(skip)]
    #[ts(skip)]
    open_participants: HashMap<(String, WarSide), usize>,
    #[serde(skip)]
    #[ts(skip)]
    open_relations: HashMap<WarRelationKey, usize>,
}
#[derive(Clone, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub(crate) struct WarRelationKey {
    pub first: (u32, u32),
    pub second: (u32, u32),
    pub start: i32,
}
impl WarRelationKey {
    pub(crate) fn new(a: (u32, u32), b: (u32, u32), start: i32) -> Self {
        let (first, second) = if a < b { (a, b) } else { (b, a) };
        Self {
            first,
            second,
            start,
        }
    }
}
#[derive(Clone, Debug)]
pub(crate) struct WarRelationMembership {
    pub key: WarRelationKey,
    pub first_hour: i32,
    pub last_hour: i32,
}
#[derive(Clone, Debug, Deserialize, Serialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct WarsReply {
    pub branch_id: String,
    /// At least one overview operand table was recorded, even if incomplete.
    pub available: bool,
    /// The overview viewpoint over consecutive readable hours, including peace.
    pub viewpoints: Vec<MembershipRun>,
    /// All resolved historical names of participants and viewpoints, for label measurement only.
    pub country_names: Vec<String>,
    pub wars: Vec<WarTimeline>,
    pub unobserved: Vec<HourSpan>,
    pub capitulations_unobserved: Vec<HourSpan>,
    pub warnings: Vec<String>,
}
/// Replies kept for the most recently requested viewpoints.
#[cfg(test)]
const CACHED_REPLIES: usize = 4;
#[cfg(test)]
struct Cache {
    branch: u64,
    view: Option<String>,
    identities: Option<HashSet<(u32, u32)>>,
    catalogue: Option<Arc<Catalogue>>,
    reply: Arc<WarsReply>,
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
            WarCasualtyRun::decl(),
            WarTimeline::decl(),
            WarsReply::decl(),
        ]
    }
    fn validate(p: &Self::Params) -> Result<(), &'static str> {
        p.branch_id
            .parse::<u64>()
            .map_err(|_| "Invalid wars branch")?;
        if let Some(tag) = &p.viewpoint_tag {
            validate_tag(tag)?;
        }
        Ok(())
    }
    #[cfg(test)]
    fn load(
        source: &Source,
        p: &Self::Params,
        context: &LoadContext,
        cancelled: &AtomicBool,
    ) -> io::Result<Self::Reply> {
        Self::shared(source, p, None, context, cancelled).map(|reply| (*reply).clone())
    }
}
impl Wars {
    #[cfg(test)]
    pub(crate) fn shared(
        source: &Source,
        p: &WarsParams,
        identities: Option<&HashSet<(u32, u32)>>,
        context: &LoadContext,
        cancelled: &AtomicBool,
    ) -> io::Result<Arc<WarsReply>> {
        let branch = p.branch_id.parse().expect("validated");
        let cat = context
            .content
            .as_ref()
            .map(|options| catalogue::catalogue(source, options))
            .transpose()?;
        let state = source.state::<Vec<Cache>>();
        let mut state = state.lock().unwrap_or_else(|e| e.into_inner());
        if let Some(hit) = state.iter().position(|c| {
            c.branch == branch
                && (identities.is_some() || c.view == p.viewpoint_tag)
                && c.identities.as_ref() == identities
                && match (&c.catalogue, &cat) {
                    (Some(a), Some(b)) => Arc::ptr_eq(a, b),
                    (None, None) => true,
                    _ => false,
                }
        }) {
            // The most recently requested first.
            state[..=hit].rotate_right(1);
            return Ok(state[0].reply.clone());
        }
        let reply = Arc::new(if let Some(identities) = identities {
            scan_selected(
                &source.replay(),
                branch,
                p.viewpoint_tag.as_deref(),
                Some(identities),
                cat.as_deref(),
                cancelled,
            )?
        } else {
            scan(
                &source.replay(),
                branch,
                p.viewpoint_tag.as_deref(),
                cat.as_deref(),
                cancelled,
            )?
        });
        state.retain(|c| {
            c.branch != branch
                || c.identities.as_ref() != identities
                || identities.is_none() && c.view != p.viewpoint_tag
        });
        state.insert(
            0,
            Cache {
                branch,
                view: p.viewpoint_tag.clone(),
                identities: identities.cloned(),
                catalogue: cat,
                reply: reply.clone(),
            },
        );
        state.truncate(CACHED_REPLIES);
        Ok(reply)
    }
}
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
struct RelationRun {
    first: String,
    second: String,
    start: i32,
    identity: WarRelationKey,
}
#[derive(Clone)]
struct Observed {
    data: WarObservation,
    relations: Vec<RelationRun>,
    losses: [String; 4],
    loss_pairs: Vec<(Pair, bool)>,
}
struct Hour {
    world: World,
    tags: HashMap<u32, String>,
    identities: HashMap<u32, (u32, u32)>,
    raw_to_slot: HashMap<u32, u32>,
    view: u32,
    recorded_view: Option<u32>,
    goal_owned: HashMap<Pair, rows::WarOwnedRelationRow>,
}
struct HeldHour {
    /// Identities, not views: holding a view copies it when the next hour changes it.
    tables: Vec<ViewIdentity>,
    observed: Result<(Hour, Vec<Observed>), String>,
}

fn append_viewpoint(reply: &mut WarsReply, inputs: &Hour, hour: i32) {
    let Some(tag) = inputs.tags.get(&inputs.view) else {
        return;
    };
    if let Some(run) = reply.viewpoints.last_mut()
        && &run.tag == tag
        && run.last_hour.checked_add(1) == Some(hour)
    {
        run.last_hour = hour;
    } else {
        reply.viewpoints.push(MembershipRun {
            tag: tag.clone(),
            first_hour: hour,
            last_hour: hour,
        });
    }
}
impl Hour {
    fn read(frame: &DecodedFrame, view: Option<&str>) -> Result<Self, String> {
        let countries = frame
            .complete::<tables::Countries>()
            .map_err(|e| e.to_string())?;
        let links = frame
            .complete::<tables::CountryLinks>()
            .map_err(|e| e.to_string())?;
        let tags: HashMap<_, _> = countries
            .iter()
            .map(|c| {
                std::str::from_utf8(&c.tag)
                    .map(|tag| (c.canonical_slot, tag.to_owned()))
                    .map_err(|_| "country tag is not UTF-8".to_owned())
            })
            .collect::<Result<_, _>>()?;
        let raw_to_slot: HashMap<_, _> = links.iter().map(|r| (r.id, r.canonical_slot)).collect();
        let identities = countries
            .iter()
            .map(|r| (r.canonical_slot, (r.canonical_slot, r.id)))
            .collect();
        let slot = |raw: u32| -> Result<u32, String> {
            raw_to_slot
                .get(&raw)
                .copied()
                .filter(|s| tags.contains_key(s))
                .ok_or_else(|| format!("Not recorded: canonical country link {raw}"))
        };
        let recorded_view = frame
            .complete::<tables::LocalPlayer>()
            .ok()
            .and_then(|r| r.first())
            .and_then(|r| {
                slot(if r.country as i32 > 0 {
                    r.country
                } else {
                    r.observer_country
                })
                .ok()
            });
        let viewpoint = match view {
            Some(tag) => tags
                .iter()
                .find_map(|(&s, t)| (t == tag).then_some(s))
                .ok_or_else(|| format!("Not in this recording: viewpoint {tag}"))?,
            None => recorded_view.ok_or("Not recorded: local playing/observer country")?,
        };
        let mut world = World {
            owned: HashMap::new(),
            relations: HashMap::new(),
            adjacent: HashMap::new(),
            factions: HashMap::new(),
            scores: HashMap::new(),
        };
        let owned = frame
            .complete::<tables::WarOwnedRelations>()
            .map_err(|e| e.to_string())?;
        // Keys preserve the complete vector's ordinals, not a tag sort.
        let mut goal_owned = HashMap::new();
        for r in owned {
            let (owner, a, b) = (
                slot(r.owner)?,
                slot(r.first_country)?,
                slot(r.second_country)?,
            );
            if a == b || (owner != a && owner != b) {
                return Err("Not recorded: owned war endpoints disagree with owner".into());
            }
            world
                .owned
                .entry(owner)
                .or_default()
                .push(if owner == a { b } else { a });
            if owner == a && r.retired == 0 && goal_owned.insert(pair(a, b), r.clone()).is_some() {
                return Err(
                    "Not recorded: multiple live owned war occurrences for one pair".into(),
                );
            }
        }
        for r in frame
            .complete::<tables::WarRelations>()
            .map_err(|e| e.to_string())?
        {
            let mut r = r.clone();
            r.first_country = slot(r.first_country)?;
            r.second_country = slot(r.second_country)?;
            let p = pair(r.first_country, r.second_country);
            if p.0 == p.1 || world.relations.insert(p, r).is_some() {
                return Err("Not recorded: duplicate canonical active war pair".into());
            }
            world.adjacent.entry(p.0).or_default().push(p.1);
            world.adjacent.entry(p.1).or_default().push(p.0);
        }
        // Factions are equality operands, never side rosters.
        frame
            .complete::<tables::Factions>()
            .map_err(|e| e.to_string())?;
        for r in frame
            .complete::<tables::FactionMembers>()
            .map_err(|e| e.to_string())?
        {
            let country = slot(r.country)?;
            if world.factions.insert(country, r.faction).is_some() {
                return Err("Not recorded: a country has multiple faction memberships".into());
            }
        }
        let rules = frame
            .complete::<tables::WarNameRules>()
            .map_err(|_| "Not recorded: war overview viewpoint")?;
        if rules.len() != 1 || rules[0].index != 0 {
            return Err("Not recorded: war overview viewpoint row absent or repeated".into());
        }
        if rules[0].viewpoint == 0
            || Some(
                slot(rules[0].viewpoint)
                    .map_err(|_| "Not recorded: war overview viewpoint cannot be resolved")?,
            ) != recorded_view
        {
            return Err("Not recorded: war overview viewpoint differs from LocalPlayer".into());
        }
        if let Ok(scores) = frame.complete::<tables::WarOverviewCountries>() {
            for r in scores {
                world.scores.insert(slot(r.country)?, r.clone());
            }
        }
        Ok(Self {
            world,
            tags,
            identities,
            raw_to_slot,
            view: viewpoint,
            recorded_view,
            goal_owned,
        })
    }
    fn refresh_losses(&mut self, frame: &DecodedFrame) -> bool {
        let Ok(rows) = frame.complete::<tables::WarRelations>() else {
            return false;
        };
        if rows.len() != self.world.relations.len() {
            return false;
        }
        let mut seen = HashSet::new();
        for row in rows {
            let (Some(&a), Some(&b)) = (
                self.raw_to_slot.get(&row.first_country),
                self.raw_to_slot.get(&row.second_country),
            ) else {
                return false;
            };
            if !seen.insert(pair(a, b)) {
                return false;
            }
            let Some(old) = self.world.relations.get(&pair(a, b)) else {
                return false;
            };
            if (
                old.first_country,
                old.second_country,
                old.start_hour,
                old.end_hour,
                old.cancel,
                old.first_was_instigator,
            ) != (
                a,
                b,
                row.start_hour,
                row.end_hour,
                row.cancel,
                row.first_was_instigator,
            ) {
                return false;
            }
        }
        for row in rows {
            let (a, b) = (
                self.raw_to_slot[&row.first_country],
                self.raw_to_slot[&row.second_country],
            );
            let old = self
                .world
                .relations
                .get_mut(&pair(a, b))
                .expect("checked relation");
            old.first_casualties = row.first_casualties;
            old.second_casualties = row.second_casualties;
            old.first_unknown_casualties = row.first_unknown_casualties;
            old.second_unknown_casualties = row.second_unknown_casualties;
        }
        true
    }
    fn losses(&self, pairs: &[(Pair, bool)]) -> [String; 4] {
        let mut sums = [0_i128; 4];
        for &(p, first) in pairs {
            let r = &self.world.relations[&p];
            let (a, b) = if first { (0, 2) } else { (2, 0) };
            sums[a] += i128::from(r.first_casualties);
            sums[a + 1] += i128::from(r.first_unknown_casualties);
            sums[b] += i128::from(r.second_casualties);
            sums[b + 1] += i128::from(r.second_unknown_casualties);
        }
        sums.map(|n| n.to_string())
    }
    fn observe(&self, frame: &DecodedFrame, cat: Option<&Catalogue>) -> Vec<Observed> {
        let naming = naming::Inputs::read(frame);
        self.world
            .blobs(self.view)
            .into_iter()
            .map(|blob| {
                let first: HashSet<_> = blob.first.iter().copied().collect();
                let inside = self.world.inside(&blob);
                let loss_pairs = inside
                    .iter()
                    .map(|&p| (p, first.contains(&self.world.relations[&p].first_country)))
                    .collect::<Vec<_>>();
                let losses = self.losses(&loss_pairs);
                let relations = inside
                    .into_iter()
                    .map(|p| {
                        let r = &self.world.relations[&p];
                        let (a, b) = (&self.tags[&p.0], &self.tags[&p.1]);
                        let (first, second) = if a < b { (a, b) } else { (b, a) };
                        RelationRun {
                            first: first.clone(),
                            second: second.clone(),
                            start: r.start_hour,
                            identity: WarRelationKey::new(
                                self.identities[&p.0],
                                self.identities[&p.1],
                                r.start_hour,
                            ),
                        }
                    })
                    .collect();
                let representative = self.world.representative(&blob);
                let (orientation, represented, name) = match representative {
                    Ok(p) => {
                        let r = &self.world.relations[&p];
                        let instigator = if r.first_was_instigator != 0 {
                            r.first_country
                        } else {
                            r.second_country
                        };
                        let name = naming::name(frame, self, p, cat, &naming);
                        (
                            Some(first.contains(&instigator)),
                            Some([
                                self.tags[&r.first_country].clone(),
                                self.tags[&r.second_country].clone(),
                            ]),
                            name,
                        )
                    }
                    Err(reason) => (None, None, naming::unknown(reason)),
                };
                Observed {
                    relations,
                    losses,
                    loss_pairs,
                    data: WarObservation {
                        first_hour: frame.hour,
                        last_hour: frame.hour,
                        viewpoint_tag: self.tags[&self.view].clone(),
                        viewpoint_side: blob.first.iter().map(|s| self.tags[s].clone()).collect(),
                        opposing_side: blob.second.iter().map(|s| self.tags[s].clone()).collect(),
                        attackers_are_viewpoint: orientation,
                        representative: represented,
                        name,
                    },
                }
            })
            .collect()
    }
}
fn span(spans: &mut Vec<HourSpan>, first: i32, last: i32) {
    if first > last {
        return;
    }
    if let Some(previous) = spans.last_mut()
        && previous.last_hour.checked_add(1) == Some(first)
    {
        previous.last_hour = last;
    } else {
        spans.push(HourSpan {
            first_hour: first,
            last_hour: last,
        });
    }
}
fn sides(data: &WarObservation) -> impl Iterator<Item = (&String, WarSide)> {
    let (a, b) = match data.attackers_are_viewpoint {
        Some(true) => (WarSide::Attacker, WarSide::Defender),
        Some(false) => (WarSide::Defender, WarSide::Attacker),
        None => (WarSide::Viewpoint, WarSide::Opponent),
    };
    data.viewpoint_side
        .iter()
        .map(move |tag| (tag, a))
        .chain(data.opposing_side.iter().map(move |tag| (tag, b)))
}
fn add_observation(war: &mut WarTimeline, mut data: WarObservation, previous: Option<i32>) {
    let hour = data.first_hour;
    if let Some(old) = war.observations.last_mut() {
        data.first_hour = old.first_hour;
        data.last_hour = old.last_hour;
        if old == &data && Some(old.last_hour) == previous {
            old.last_hour = hour;
            war.last_hour = hour;
            for &index in war.open_participants.values() {
                war.participants[index].last_hour = hour;
            }
            return;
        }
        data.first_hour = hour;
        data.last_hour = hour;
    }
    let existing = std::mem::take(&mut war.open_participants);
    let current: HashSet<_> = sides(&data)
        .map(|(tag, side)| (tag.clone(), side))
        .collect();
    for (key, index) in &existing {
        if !current.contains(key) {
            war.participants[*index].left_by = false;
        }
    }
    for (tag, side) in current {
        if let Some(&index) = existing.get(&(tag.clone(), side)) {
            war.participants[index].last_hour = hour;
            war.open_participants.insert((tag, side), index);
        } else {
            war.open_participants
                .insert((tag.clone(), side), war.participants.len());
            war.participants.push(WarParticipantRun {
                tag,
                side,
                first_hour: hour,
                last_hour: hour,
                joined_by: previous.is_none(),
                left_by: true,
            });
        }
    }
    war.last_hour = hour;
    if let Some(old) = war.observations.last_mut() {
        let first = old.first_hour;
        let last = old.last_hour;
        data.first_hour = first;
        data.last_hour = last;
        if old == &data && Some(last) == previous {
            old.last_hour = hour;
            return;
        }
        data.first_hour = hour;
        data.last_hour = hour;
    }
    war.observations.push(data);
}
fn append_losses(war: &mut WarTimeline, hour: i32, losses: [String; 4], previous: Option<i32>) {
    let [a, b, c, d] = losses;
    if let Some(old) = war.casualties.last_mut()
        && Some(old.1) == previous
        && [&old.2, &old.3, &old.4, &old.5] == [&a, &b, &c, &d]
    {
        old.1 = hour;
    } else {
        war.casualties.push(WarCasualtyRun(hour, hour, a, b, c, d));
    }
}
fn append_relations(war: &mut WarTimeline, hour: i32, relations: &[RelationRun]) {
    // Grouping and representative selection decide membership.
    // One indexed tail per lifetime keeps long, wide wars linear in their edges.
    for relation in relations {
        if let Some(&index) = war.open_relations.get(&relation.identity)
            && war.relations[index].last_hour.checked_add(1) == Some(hour)
        {
            war.relations[index].last_hour = hour;
        } else {
            war.open_relations
                .insert(relation.identity.clone(), war.relations.len());
            war.relations.push(WarRelationMembership {
                key: relation.identity.clone(),
                first_hour: hour,
                last_hour: hour,
            });
        }
    }
}
fn continue_wars(
    wars: &mut Vec<WarTimeline>,
    open: &[(usize, Vec<RelationRun>)],
    current: Vec<Observed>,
    previous: Option<i32>,
) -> Vec<(usize, Vec<RelationRun>)> {
    // App rule: greedy one-to-one in native blob order. Choose the earlier
    // group's most continuing bilateral relations; tie: lowest app id.
    let mut inverted: HashMap<&RelationRun, Vec<usize>> = HashMap::new();
    for (id, relations) in open {
        for relation in relations {
            inverted.entry(relation).or_default().push(*id);
        }
    }
    let mut used = HashSet::new();
    let mut next = Vec::new();
    for observation in current {
        let mut counts = HashMap::<usize, usize>::new();
        for relation in &observation.relations {
            for &id in inverted.get(relation).into_iter().flatten() {
                if !used.contains(&id) {
                    *counts.entry(id).or_default() += 1;
                }
            }
        }
        let id = counts
            .into_iter()
            .max_by(|(a, ac), (b, bc)| ac.cmp(bc).then(b.cmp(a)))
            .map(|(id, _)| id)
            .unwrap_or_else(|| {
                let id = wars.len();
                wars.push(WarTimeline {
                    id: id.to_string(),
                    first_hour: observation.data.first_hour,
                    last_hour: observation.data.first_hour,
                    started_by: previous.is_none(),
                    ended_by: true,
                    observations: Vec::new(),
                    casualties: Vec::new(),
                    participants: Vec::new(),
                    capitulations: Vec::new(),
                    relations: Vec::new(),
                    open_relations: HashMap::new(),
                    open_participants: HashMap::new(),
                });
                id
            });
        used.insert(id);
        append_relations(
            &mut wars[id],
            observation.data.first_hour,
            &observation.relations,
        );
        append_losses(
            &mut wars[id],
            observation.data.first_hour,
            observation.losses,
            previous,
        );
        add_observation(&mut wars[id], observation.data, previous);
        next.push((id, observation.relations));
    }
    if previous.is_some() {
        for (id, _) in open {
            if !used.contains(id) {
                wars[*id].ended_by = false;
                let indices = std::mem::take(&mut wars[*id].open_participants);
                for index in indices.into_values() {
                    wars[*id].participants[index].left_by = false;
                }
            }
        }
    }
    next
}
// Resolve the selected native holder's actual dated tag, never a later tag
// replacement or the recording's local player when that holder is missing.
fn selected_view(frame: &DecodedFrame, identities: &HashSet<(u32, u32)>) -> Result<String, String> {
    let countries = frame
        .complete::<tables::Countries>()
        .map_err(|e| e.to_string())?;
    let mut found = countries
        .iter()
        .filter(|row| identities.contains(&(row.canonical_slot, row.id)));
    let row = found
        .next()
        .ok_or("Not recorded: selected native war viewpoint holder")?;
    if found.next().is_some() {
        return Err("Not recorded: multiple selected native war viewpoint holders".into());
    }
    std::str::from_utf8(&row.tag)
        .map(str::to_owned)
        .map_err(|_| "country tag is not UTF-8".into())
}
#[cfg(test)]
fn scan(
    replay: &Replay,
    branch: u64,
    view: Option<&str>,
    cat: Option<&Catalogue>,
    cancelled: &AtomicBool,
) -> io::Result<WarsReply> {
    scan_selected(replay, branch, view, None, cat, cancelled)
}
const TABLES: &[TableId] = &[
    TableId::Clock,
    TableId::Campaign,
    TableId::Countries,
    TableId::CountryLinks,
    TableId::WarRelations,
    TableId::WarOwnedRelations,
    TableId::WarRelationGoals,
    TableId::WarOverviewCountries,
    TableId::WarNameRules,
    TableId::Factions,
    TableId::FactionMembers,
    TableId::LocalPlayer,
    TableId::Capitulation,
    TableId::LexicalTokens,
    TableId::CountryAppearance,
    TableId::ReservedCountryNaming,
    TableId::Politics,
    TableId::PoliticalParties,
    TableId::Autonomy,
    TableId::States,
];

#[cfg(test)]
fn scan_selected(
    replay: &Replay,
    branch: u64,
    view: Option<&str>,
    identities: Option<&HashSet<(u32, u32)>>,
    cat: Option<&Catalogue>,
    cancelled: &AtomicBool,
) -> io::Result<WarsReply> {
    let start = replay
        .branches
        .iter()
        .find(|b| b.id == branch)
        .ok_or_else(|| invalid("Wars branch is not in this recording"))?;
    let (first, last) = branch_bounds(replay, start);
    let mut walk = Walk::new(branch, first);
    crate::blocks::scan_blocks(
        replay,
        replay.index.iter().filter(|b| b.branch == branch),
        TABLES,
        cancelled,
        |blocks| {
            for (_, frames) in blocks {
                if cancelled.load(Ordering::Acquire) {
                    return Err(invalid("Wars request cancelled"));
                }
                let mut decoder = FrameDecoder::for_tables(TABLES);
                let frames = match frames {
                    Ok(frames) => frames,
                    Err(e) => {
                        walk.warnings.insert(format!("Wars block unreadable: {e}"));
                        continue;
                    }
                };
                for frame in frames {
                    if cancelled.load(Ordering::Acquire) {
                        return Err(invalid("Wars request cancelled"));
                    }
                    let decoded = match decoder.advance(&frame) {
                        Ok(frame) => frame,
                        Err(e) => {
                            walk.warnings.insert(format!("Wars frame unreadable: {e}"));
                            break;
                        }
                    };
                    validate_campaign(decoded, replay)?;
                    walk.observe(decoded, view, identities, cat);
                }
            }
            Ok(())
        },
    )?;
    walk.finish(last, cat)
}

struct Walk {
    reply: WarsReply,
    previous: Option<i32>,
    previous_view: Option<String>,
    open: Vec<(usize, Vec<RelationRun>)>,
    seen_caps: HashSet<(String, i32)>,
    dated_caps: HashMap<String, Vec<i32>>,
    identity_since: HashMap<u32, (String, i32)>,
    warnings: BTreeSet<String>,
    last_frame: Option<i32>,
    held: Option<HeldHour>,
    country_labels: labels::CountryLabels,
    compared: Vec<TableId>,
}
impl Walk {
    fn new(branch: u64, first: i32) -> Self {
        let reply=WarsReply{branch_id:branch.to_string(),available:false,viewpoints:Vec::new(),country_names:Vec::new(),wars:Vec::new(),unobserved:Vec::new(),capitulations_unobserved:Vec::new(),warnings:vec!["War continuity and participant runs are app observations; the game stores no grouped war identity, start/end or join/leave hours.".into(),"Attackers and defenders are oriented by the representative relation's instigator.".into(),"Native groups can overlap; casualties belong to each group's distinct bilateral relations and must not be summed across groups.".into()]};
        let compared: Vec<_> = TABLES
            .iter()
            .copied()
            .filter(|&t| {
                t != TableId::Clock
                    && t != TableId::Campaign
                    && t != TableId::Capitulation
                    && t != TableId::WarRelations
            })
            .collect();
        Self {
            reply,
            previous: None,
            previous_view: None,
            open: Vec::new(),
            seen_caps: HashSet::new(),
            dated_caps: HashMap::new(),
            identity_since: HashMap::new(),
            warnings: BTreeSet::new(),
            last_frame: first.checked_sub(1),
            held: None,
            country_labels: Default::default(),
            compared,
        }
    }
    fn observe(
        &mut self,
        decoded: &DecodedFrame,
        view: Option<&str>,
        identities: Option<&HashSet<(u32, u32)>>,
        cat: Option<&Catalogue>,
    ) {
        self.country_labels.observe(decoded, cat);
        self.reply.available |= [
            TableId::WarOwnedRelations,
            TableId::WarRelationGoals,
            TableId::WarOverviewCountries,
            TableId::WarNameRules,
        ]
        .iter()
        .any(|table| {
            !matches!(
                &*decoded.tables[table.index()],
                TableView::Unavailable { .. }
            )
        });
        let hour = decoded.hour;
        if let Some(before) = self.last_frame
            && before.checked_add(1) != Some(hour)
        {
            span(&mut self.reply.unobserved, before + 1, hour - 1);
            span(
                &mut self.reply.capitulations_unobserved,
                before + 1,
                hour - 1,
            );
            for (id, _) in &self.open {
                let war: &mut WarTimeline = &mut self.reply.wars[*id];
                for &index in war.open_participants.values() {
                    war.participants[index].last_hour = war.last_hour;
                }
            }
            self.previous = None;
            self.open.clear();
            self.identity_since.clear();
        }
        self.last_frame = Some(hour);
        let same = self.held.as_mut().is_some_and(|old| {
            old.tables
                .iter()
                .zip(&self.compared)
                .all(|(seen, table)| seen.is(&decoded.tables[table.index()]))
                && old
                    .observed
                    .as_mut()
                    .is_ok_and(|(inputs, _)| inputs.refresh_losses(decoded))
        });
        if !same {
            self.held = Some(HeldHour {
                tables: self
                    .compared
                    .iter()
                    .map(|table| decoded.tables[table.index()].identity())
                    .collect(),
                observed: identities
                    .map_or_else(
                        || Ok(view.map(str::to_owned)),
                        |selected| selected_view(decoded, selected).map(Some),
                    )
                    .and_then(|view| Hour::read(decoded, view.as_deref()))
                    .map(|inputs| {
                        let observations = inputs.observe(decoded, cat);
                        (inputs, observations)
                    }),
            });
        }
        if let Ok((inputs, observed)) = &self.held.as_ref().expect("held hour").observed {
            if !same || self.previous.is_none() {
                self.country_labels.include(inputs, observed);
            }
            append_viewpoint(&mut self.reply, inputs, hour);
            if self.previous_view.as_ref() != inputs.tags.get(&inputs.view) {
                for (id, _) in &self.open {
                    let war = &mut self.reply.wars[*id];
                    for &index in war.open_participants.values() {
                        war.participants[index].last_hour = war.last_hour;
                    }
                }
                self.previous = None;
                self.open.clear();
            }
            self.previous_view = inputs.tags.get(&inputs.view).cloned();
            if same && self.previous.is_some() && self.open.len() == observed.len() {
                // Loss-only hours retain native groups and continuity. No
                // side lists, membership sets or relation runs are rebuilt.
                for ((id, _), observed) in self.open.iter().zip(observed) {
                    let war = &mut self.reply.wars[*id];
                    append_losses(
                        war,
                        hour,
                        inputs.losses(&observed.loss_pairs),
                        self.previous,
                    );
                    append_relations(war, hour, &observed.relations);
                    war.last_hour = hour;
                    war.observations
                        .last_mut()
                        .expect("open war observation")
                        .last_hour = hour;
                }
            } else {
                // A structural change settles the held participant bounds
                // before splitting, merging or ending the previous groups.
                if let Some(last) = self.previous {
                    for (id, _) in &self.open {
                        let war = &mut self.reply.wars[*id];
                        for &index in war.open_participants.values() {
                            war.participants[index].last_hour = last;
                        }
                    }
                }
                let observations = observed
                    .iter()
                    .map(|o| Observed {
                        data: WarObservation {
                            first_hour: hour,
                            last_hour: hour,
                            ..o.data.clone()
                        },
                        relations: o.relations.clone(),
                        losses: inputs.losses(&o.loss_pairs),
                        loss_pairs: o.loss_pairs.clone(),
                    })
                    .collect();
                self.open = continue_wars(
                    &mut self.reply.wars,
                    &self.open,
                    observations,
                    self.previous,
                );
            }
            self.identity_since.retain(|raw, _| {
                inputs
                    .raw_to_slot
                    .get(raw)
                    .is_some_and(|slot| inputs.tags.contains_key(slot))
            });
            for (&raw, &slot) in &inputs.raw_to_slot {
                let Some(tag) = inputs.tags.get(&slot) else {
                    continue;
                };
                let entry = self
                    .identity_since
                    .entry(raw)
                    .or_insert_with(|| (tag.clone(), hour));
                if &entry.0 != tag || self.previous.is_none() {
                    *entry = (tag.clone(), hour);
                }
            }
            match decoded.complete::<tables::Capitulation>() {
                Ok(rows) => {
                    for r in rows.iter().filter(|r| r.capitulated != 0) {
                        let Some((tag, since)) = self.identity_since.get(&r.country) else {
                            continue;
                        };
                        if r.capitulated_hour < *since || r.capitulated_hour > hour {
                            self.warnings.insert("Capitulations whose country identity at the event hour is unobserved are omitted.".into());
                            continue;
                        }
                        if !self.seen_caps.insert((tag.clone(), r.capitulated_hour)) {
                            continue;
                        }
                        self.dated_caps
                            .entry(tag.clone())
                            .or_default()
                            .push(r.capitulated_hour);
                    }
                }
                Err(_) => span(&mut self.reply.capitulations_unobserved, hour, hour),
            }
            self.previous = Some(hour);
        } else {
            self.warnings.insert(
                self.held
                    .as_ref()
                    .expect("held hour")
                    .observed
                    .as_ref()
                    .err()
                    .expect("failed hour")
                    .clone(),
            );
            span(&mut self.reply.unobserved, hour, hour);
            span(&mut self.reply.capitulations_unobserved, hour, hour);
            for (id, _) in &self.open {
                let war: &mut WarTimeline = &mut self.reply.wars[*id];
                for &index in war.open_participants.values() {
                    war.participants[index].last_hour = war.last_hour;
                }
            }
            self.previous = None;
            self.open.clear();
            self.identity_since.clear();
            self.held = None;
        }
    }
    /// Extend unchanged inputs across accepted hours without walking those hours.
    fn extend(&mut self, last: i32) {
        let Some(first) = self.last_frame else { return };
        if last <= first {
            return;
        }
        if self.previous == Some(first) {
            if let Some(viewpoint) = self.reply.viewpoints.last_mut() {
                viewpoint.last_hour = last;
            }
            for (id, _) in &self.open {
                let war = &mut self.reply.wars[*id];
                war.last_hour = last;
                if let Some(observation) = war.observations.last_mut() {
                    observation.last_hour = last;
                }
                if let Some(losses) = war.casualties.last_mut() {
                    losses.1 = last;
                }
                for &index in war.open_relations.values() {
                    war.relations[index].last_hour = last;
                }
            }
            if self
                .reply
                .capitulations_unobserved
                .last()
                .is_some_and(|span| span.last_hour == first)
            {
                span(&mut self.reply.capitulations_unobserved, first + 1, last);
            }
            self.previous = Some(last);
        } else {
            span(&mut self.reply.unobserved, first + 1, last);
            span(&mut self.reply.capitulations_unobserved, first + 1, last);
        }
        self.last_frame = Some(last);
    }
    fn finish(mut self, last: i32, cat: Option<&Catalogue>) -> io::Result<WarsReply> {
        if let Some(before) = self.last_frame {
            span(&mut self.reply.unobserved, before.saturating_add(1), last);
            span(
                &mut self.reply.capitulations_unobserved,
                before.saturating_add(1),
                last,
            );
        }
        self.reply.warnings.extend(self.warnings);
        self.reply.country_names = self.country_labels.finish();
        if cat.is_none() {
            self.reply.warnings.push(
                "War names are Not recorded until the recorded content stack is resolved.".into(),
            );
        }
        for hours in self.dated_caps.values_mut() {
            hours.sort_unstable();
        }
        for war in &mut self.reply.wars {
            for &index in war.open_participants.values() {
                war.participants[index].last_hour = war.last_hour;
            }
            let mut attached = HashSet::new();
            for participant in &war.participants {
                let Some(hours) = self.dated_caps.get(&participant.tag) else {
                    continue;
                };
                // Sorted below once per tag; search only this run's event range.
                let first = hours.partition_point(|h| *h < participant.first_hour);
                for &hour in hours[first..]
                    .iter()
                    .take_while(|h| **h <= participant.last_hour)
                {
                    if attached.insert((participant.tag.clone(), hour)) {
                        war.capitulations.push(WarCapitulation {
                            tag: participant.tag.clone(),
                            hour,
                        });
                    }
                }
            }
            war.open_participants.clear();
            war.open_relations.clear();
            war.participants
                .sort_by(|a, b| a.first_hour.cmp(&b.first_hour).then(a.tag.cmp(&b.tag)));
            war.capitulations.sort_by_key(|c| c.hour);
        }
        check_reply(&self.reply)?;
        Ok(self.reply)
    }
}

fn check_reply(reply: &WarsReply) -> io::Result<()> {
    if crate::transport::json_len(&reply, MAX_PRODUCT_REPLY_BYTES).is_none() {
        return Err(invalid(format!(
            "Wars reply exceeds the shared product reply budget ({} MiB)",
            MAX_PRODUCT_REPLY_BYTES / (1024 * 1024)
        )));
    }
    Ok(())
}
