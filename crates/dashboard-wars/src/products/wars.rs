//! Native, viewpoint-dependent war blobs with explicitly app-owned continuity.
//! The game stores no group identity or participant join/leave dates.
pub mod emit;
pub mod facts;
mod inputs;
mod labels;
mod layers;
mod naming;
mod native;
mod participation;
pub use participation::{WarContributionCountry, WarContributionGroup};
pub mod query;
pub mod served;
#[cfg(any(test, feature = "test-support"))]
pub mod test_fixtures;
#[cfg(test)]
mod columnar_tests;
#[cfg(test)]
pub(crate) mod tests;

use dashboard_base::{
    Source,
    catalogue::{self, Catalogue},
};
use dashboard_base::{
    products::LoadContext,
    products::MAX_PRODUCT_REPLY_BYTES,
    products::Product,
    products::country_groups::{HourSpan, MembershipRun},
    products::validate_tag,
};
use dashboard_core::invalid;
use dashboard_text::metadata::branch_bounds;
#[cfg(any(test, feature = "test-support"))]
use dashboard_text::metadata::validate_campaign;
use native::{Blob, Pair, World, pair};
#[cfg(any(test, feature = "test-support"))]
use replay::container::Replay;
use replay_data::{rows, rows::TableId, rows::tables};
#[cfg(any(test, feature = "test-support"))]
use replay_frame::frame::FrameDecoder;
use replay_frame::{frame::DecodedFrame, frame::TableView};
use serde::{Deserialize, Serialize};
#[cfg(any(test, feature = "test-support"))]
use std::sync::atomic::Ordering;
use std::{
    collections::{BTreeSet, HashMap, HashSet},
    io,
    sync::{Arc, atomic::AtomicBool},
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
#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq, Hash, TS)]
#[serde(rename_all = "camelCase")]
#[derive(replay_spool::Spooled)]
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
    open_relations: HashMap<(WarRelationKey, bool), usize>,
}
#[derive(Clone, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[derive(replay_spool::Spooled)]
pub struct WarRelationKey {
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
    /// Whether the key's first country is on the war's first side, so its casualty counters
    /// count for that side.
    pub oriented: bool,
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
/// The native war groups and contributions one viewpoint sees at a single
/// requested hour, read independently of preparation from that hour's frame
/// alone. Sides, names and casualty counters are the hour's operands; app
/// continuity, start, join and capitulation history need earlier hours, so ids are transient
/// (`@<hour>:<index>`), every bound is a "by" bound and no capitulation is
/// listed.
#[derive(Clone, Debug, Deserialize, Serialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct WarsAtHour {
    pub hour: i32,
    pub contributions: Vec<WarContributionGroup>,
    /// At least one overview operand table was recorded at the hour; false
    /// at an unrecorded hour.
    pub available: bool,
    pub wars: Vec<WarTimeline>,
    pub warnings: Vec<String>,
}
/// One war's casualty runs at the hours a read at hours asks.
#[derive(Clone, Debug, Deserialize, Serialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct WarCasualties {
    /// The war's id in the reply without hours of the same publication.
    pub id: String,
    pub runs: Vec<WarCasualtyRun>,
}
/// A read at hours: what depends on them alone. The wars themselves are the reply without
/// hours ([`WarsReply`]).
#[derive(Clone, Debug, Deserialize, Serialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct WarsHourly {
    pub branch_id: String,
    /// Each war observed at a `reads` hour, in the order of the wars, with its runs there.
    pub casualties: Vec<WarCasualties>,
    /// The groups and contributions at each `current` hour the publication accepted.
    pub current: Vec<WarsAtHour>,
}
/// A walk's only warning that is not an hour it refused.
const CAPITULATIONS_UNOBSERVED: &str =
    "Capitulations whose country identity at the event hour is unobserved are omitted.";
/// Replies kept for the most recently requested viewpoints.
#[cfg(any(test, feature = "test-support"))]
const CACHED_REPLIES: usize = 4;
#[cfg(any(test, feature = "test-support"))]
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
            WarsAtHour::decl(),
            WarCasualties::decl(),
            WarsHourly::decl(),
            WarContributionGroup::decl(),
            WarContributionCountry::decl(),
        ]
    }
    const SERVED: Option<&'static dashboard_base::prepared::served::Plan> = Some(&served::PLAN);
    fn validate(p: &Self::Params) -> Result<(), &'static str> {
        p.branch_id
            .parse::<u64>()
            .map_err(|_| "Invalid wars branch")?;
        if let Some(tag) = &p.viewpoint_tag {
            validate_tag(tag)?;
        }
        Ok(())
    }
    #[cfg(any(test, feature = "test-support"))]
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
    #[cfg(any(test, feature = "test-support"))]
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
#[derive(Clone, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
struct RelationRun {
    first: String,
    second: String,
    start: i32,
    identity: WarRelationKey,
    /// Whether `identity.first` is on the group's first side.
    oriented: bool,
}
#[derive(Clone)]
struct Observed {
    data: WarObservation,
    relations: Vec<RelationRun>,
    losses: [String; 4],
    loss_pairs: Vec<(Pair, bool)>,
    /// The relation the game names the group by.
    representative: Option<WarRelationKey>,
}
/// A war continued at the walk's last hour, with every native group it holds.
#[derive(Clone, Debug, Serialize, Deserialize)]
struct OpenWar {
    id: usize,
    groups: usize,
    relations: Vec<RelationRun>,
    /// Each distinct relation once, so overlapping groups never count twice.
    loss_pairs: Vec<(Pair, bool)>,
}
/// The countries an hour reads: each canonical slot's tag and native identity, and each raw
/// link's slot.
#[derive(Clone, Default, Debug, PartialEq, Eq)]
pub(crate) struct Countries {
    tags: foldhash::HashMap<u32, String>,
    identities: foldhash::HashMap<u32, (u32, u32)>,
    raw_to_slot: foldhash::HashMap<u32, u32>,
}
impl Countries {
    /// `frame`'s countries, or why it holds none.
    fn read(frame: &DecodedFrame) -> Result<Self, String> {
        let countries = frame
            .complete::<tables::Countries>()
            .map_err(|e| e.to_string())?;
        let links = frame
            .complete::<tables::CountryLinks>()
            .map_err(|e| e.to_string())?;
        Self::of(
            countries
                .iter()
                .map(|c| (c.canonical_slot, c.id, c.tag.as_slice())),
            links.iter().map(|r| (r.id, r.canonical_slot)),
        )
    }
    /// The countries of complete Countries rows (slot, native id, tag) and links (raw id, slot).
    fn of<'t>(
        countries: impl Iterator<Item = (u32, u32, &'t [u8])>,
        links: impl Iterator<Item = (u32, u32)>,
    ) -> Result<Self, String> {
        let rows: Vec<_> = countries.collect();
        let tags = rows
            .iter()
            .map(|&(slot, _, tag)| {
                std::str::from_utf8(tag)
                    .map(|tag| (slot, tag.to_owned()))
                    .map_err(|_| "country tag is not UTF-8".to_owned())
            })
            .collect::<Result<_, _>>()?;
        Ok(Self {
            tags,
            identities: rows
                .iter()
                .map(|&(slot, id, _)| (slot, (slot, id)))
                .collect(),
            raw_to_slot: links.collect(),
        })
    }
    fn slot(&self, raw: u32) -> Result<u32, String> {
        linked(&self.tags, &self.raw_to_slot, raw)
    }
    /// The slot tagged `tag`; the lowest when several are.
    fn holding(&self, tag: &str) -> Option<u32> {
        self.tags
            .iter()
            .filter(|(_, held)| held.as_str() == tag)
            .map(|(&slot, _)| slot)
            .min()
    }
}
/// The canonical slot of raw country link `raw`, among the countries tagged.
fn linked(
    tags: &foldhash::HashMap<u32, String>,
    raw_to_slot: &foldhash::HashMap<u32, u32>,
    raw: u32,
) -> Result<u32, String> {
    raw_to_slot
        .get(&raw)
        .copied()
        .filter(|s| tags.contains_key(s))
        .ok_or_else(|| format!("Not recorded: canonical country link {raw}"))
}
/// The recording client's playing or observer country at `frame`'s hour.
fn recorded_view(frame: &DecodedFrame, countries: &Countries) -> Option<u32> {
    frame
        .complete::<tables::LocalPlayer>()
        .ok()
        .and_then(|r| r.first())
        .and_then(|r| {
            countries
                .slot(if r.country as i32 > 0 {
                    r.country
                } else {
                    r.observer_country
                })
                .ok()
        })
}
/// Each owner's owned war occurrences, and each live occurrence by its pair.
type Owned = (
    foldhash::HashMap<u32, Vec<u32>>,
    foldhash::HashMap<Pair, rows::WarOwnedRelationRow>,
);
/// `frame`'s owned war occurrences, or why they cannot be read: an hour's reads past its
/// viewpoint fail here first.
fn owned(frame: &DecodedFrame, countries: &Countries) -> Result<Owned, String> {
    let rows = frame
        .complete::<tables::WarOwnedRelations>()
        .map_err(|e| e.to_string())?;
    let mut owned = foldhash::HashMap::<u32, Vec<u32>>::default();
    // Keys preserve the complete vector's ordinals, not a tag sort.
    let mut goal_owned = foldhash::HashMap::default();
    for r in rows {
        let (owner, a, b) = (
            countries.slot(r.owner)?,
            countries.slot(r.first_country)?,
            countries.slot(r.second_country)?,
        );
        if a == b || (owner != a && owner != b) {
            return Err("Not recorded: owned war endpoints disagree with owner".into());
        }
        owned
            .entry(owner)
            .or_default()
            .push(if owner == a { b } else { a });
        if owner == a && r.retired == 0 && goal_owned.insert(pair(a, b), r.clone()).is_some() {
            return Err("Not recorded: multiple live owned war occurrences for one pair".into());
        }
    }
    Ok((owned, goal_owned))
}
/// The war overview's scores by slot; none where the overview was not read whole.
fn scores(
    frame: &DecodedFrame,
    tags: &foldhash::HashMap<u32, String>,
    raw_to_slot: &foldhash::HashMap<u32, u32>,
) -> Result<foldhash::HashMap<u32, rows::WarOverviewCountryRow>, String> {
    let mut held = foldhash::HashMap::default();
    if let Ok(scores) = frame.complete::<tables::WarOverviewCountries>() {
        for r in scores {
            held.insert(linked(tags, raw_to_slot, r.country)?, r.clone());
        }
    }
    Ok(held)
}
/// The war world past the owned occurrences: the active relations and their row order, the
/// faction memberships, the naming rule's viewpoint check and the scores, or why not.
fn world(
    frame: &DecodedFrame,
    countries: &Countries,
    recorded_view: Option<u32>,
    owned: foldhash::HashMap<u32, Vec<u32>>,
) -> Result<(World, Vec<Pair>), String> {
    let mut world = World {
        owned,
        relations: Default::default(),
        adjacent: Default::default(),
        factions: Default::default(),
        scores: Default::default(),
    };
    let mut relation_order = Vec::new();
    for r in frame
        .complete::<tables::WarRelations>()
        .map_err(|e| e.to_string())?
    {
        let mut r = r.clone();
        r.first_country = countries.slot(r.first_country)?;
        r.second_country = countries.slot(r.second_country)?;
        let p = pair(r.first_country, r.second_country);
        if p.0 == p.1 || world.relations.insert(p, r).is_some() {
            return Err("Not recorded: duplicate canonical active war pair".into());
        }
        relation_order.push(p);
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
        let country = countries.slot(r.country)?;
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
            countries
                .slot(rules[0].viewpoint)
                .map_err(|_| "Not recorded: war overview viewpoint cannot be resolved")?,
        ) != recorded_view
    {
        return Err("Not recorded: war overview viewpoint differs from LocalPlayer".into());
    }
    world.scores = scores(frame, &countries.tags, &countries.raw_to_slot)?;
    Ok((world, relation_order))
}
#[derive(Clone)]
struct Hour {
    world: World,
    /// `world.relations` keys in recorded WarRelations row order.
    relation_order: Vec<Pair>,
    tags: foldhash::HashMap<u32, String>,
    identities: foldhash::HashMap<u32, (u32, u32)>,
    raw_to_slot: foldhash::HashMap<u32, u32>,
    view: u32,
    recorded_view: Option<u32>,
    goal_owned: foldhash::HashMap<Pair, rows::WarOwnedRelationRow>,
}

/// What `view`, or the viewpoint among `identities`, reads at `decoded` and the groups it
/// observes there, each group's losses summed when `losses` are kept; or why it cannot.
#[cfg(any(test, feature = "test-support"))]
fn read_hour(
    decoded: &DecodedFrame,
    view: Option<&str>,
    identities: Option<&HashSet<(u32, u32)>>,
    cat: Option<&Catalogue>,
    losses: bool,
) -> Result<(Hour, Vec<Observed>), String> {
    identities
        .map_or_else(
            || Ok(view.map(str::to_owned)),
            |selected| selected_view(decoded, selected).map(Some),
        )
        .and_then(|view| Hour::read(decoded, view.as_deref()))
        .map(|inputs| {
            let observations = inputs.observe(decoded, cat, losses);
            (inputs, observations)
        })
}

/// Each group `observed` at `hour`, as a structural change opens it.
fn opening(observed: &[Observed], hour: i32) -> Vec<Observed> {
    observed
        .iter()
        .map(|o| Observed {
            data: WarObservation {
                first_hour: hour,
                last_hour: hour,
                ..o.data.clone()
            },
            ..o.clone()
        })
        .collect()
}

/// Whether `held`, an hour read again, observes exactly the groups `before` held, their losses
/// aside: continuing each war with them then extends it exactly as its losses alone do.
fn same_groups(before: &[Observed], held: &[Observed]) -> bool {
    before.len() == held.len()
        && before.iter().zip(held).all(|(a, b)| {
            let WarObservation {
                first_hour: _,
                last_hour: _,
                viewpoint_tag,
                viewpoint_side,
                opposing_side,
                attackers_are_viewpoint,
                representative,
                name,
            } = &a.data;
            a.relations == b.relations
                && a.loss_pairs == b.loss_pairs
                && a.representative == b.representative
                && *viewpoint_tag == b.data.viewpoint_tag
                && *viewpoint_side == b.data.viewpoint_side
                && *opposing_side == b.data.opposing_side
                && *attackers_are_viewpoint == b.data.attackers_are_viewpoint
                && *representative == b.data.representative
                && *name == b.data.name
        })
}

/// The countries a walk's hour read, and its viewpoint's slot among them.
#[derive(Clone)]
pub(crate) struct Viewed {
    countries: Arc<Countries>,
    view: u32,
}
#[cfg(any(test, feature = "test-support"))]
impl Viewed {
    fn of(hour: &Hour) -> Self {
        Self {
            countries: Arc::new(Countries {
                tags: hour.tags.clone(),
                identities: hour.identities.clone(),
                raw_to_slot: hour.raw_to_slot.clone(),
            }),
            view: hour.view,
        }
    }
}

/// An hour a walk observes: what its viewpoint reads there and the groups it sees, or why it
/// cannot; the capitulations (each capitulated country's raw id and date), or none where they
/// were not read whole; and what sums a group's losses where the walk keeps them.
pub(crate) struct Seen<'a> {
    hour: i32,
    read: Result<(Viewed, Vec<Observed>), String>,
    capitulations: Result<&'a [(u32, i32)], ()>,
    losses: Option<&'a Losses<'a>>,
}

fn append_viewpoint(reply: &mut WarsReply, tag: Option<&String>, hour: i32) {
    let Some(tag) = tag else {
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
/// The group `blob` that `view` observes at `hour` in `world`, named for its representative or
/// why it has none, its losses summed by `losses` where they are kept.
#[allow(clippy::too_many_arguments)]
fn observed(
    world: &World,
    countries: (
        &foldhash::HashMap<u32, String>,
        &foldhash::HashMap<u32, (u32, u32)>,
    ),
    view: u32,
    blob: Blob,
    named: Result<(Pair, WarName), String>,
    losses: Option<&Losses<'_>>,
    hour: i32,
) -> Observed {
    let (tags, identities) = countries;
    let first: HashSet<_> = blob.first.iter().copied().collect();
    let inside = world.inside(&blob);
    let loss_pairs = inside
        .iter()
        .map(|&p| (p, first.contains(&world.relations[&p].first_country)))
        .collect::<Vec<_>>();
    let losses = losses.map_or_else(Default::default, |losses| losses(&loss_pairs));
    let relations = loss_pairs
        .iter()
        .map(|&(p, first_side)| {
            let r = &world.relations[&p];
            let (a, b) = (&tags[&p.0], &tags[&p.1]);
            let (first, second) = if a < b { (a, b) } else { (b, a) };
            let identity = WarRelationKey::new(identities[&p.0], identities[&p.1], r.start_hour);
            let canonical = identities[&r.first_country] == identity.first;
            RelationRun {
                first: first.clone(),
                second: second.clone(),
                start: r.start_hour,
                oriented: first_side == canonical,
                identity,
            }
        })
        .collect();
    let representative = named.as_ref().ok().map(|(p, _)| {
        WarRelationKey::new(
            identities[&p.0],
            identities[&p.1],
            world.relations[p].start_hour,
        )
    });
    let (orientation, represented, name) = match named {
        Ok((p, name)) => {
            let r = &world.relations[&p];
            let instigator = if r.first_was_instigator != 0 {
                r.first_country
            } else {
                r.second_country
            };
            (
                Some(first.contains(&instigator)),
                Some([
                    tags[&r.first_country].clone(),
                    tags[&r.second_country].clone(),
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
        representative,
        data: WarObservation {
            first_hour: hour,
            last_hour: hour,
            viewpoint_tag: tags[&view].clone(),
            viewpoint_side: blob.first.iter().map(|s| tags[s].clone()).collect(),
            opposing_side: blob.second.iter().map(|s| tags[s].clone()).collect(),
            attackers_are_viewpoint: orientation,
            representative: represented,
            name,
        },
    }
}
impl Hour {
    fn read(frame: &DecodedFrame, view: Option<&str>) -> Result<Self, String> {
        let countries = Countries::read(frame)?;
        let recorded_view = recorded_view(frame, &countries);
        let viewpoint = match view {
            Some(tag) => countries
                .holding(tag)
                .ok_or_else(|| format!("Not in this recording: viewpoint {tag}"))?,
            None => recorded_view.ok_or("Not recorded: local playing/observer country")?,
        };
        let (owned, goal_owned) = owned(frame, &countries)?;
        let (world, relation_order) = world(frame, &countries, recorded_view, owned)?;
        Ok(Self::of(
            countries,
            (world, relation_order, goal_owned),
            viewpoint,
            recorded_view,
        ))
    }
    fn of(
        countries: Countries,
        (world, relation_order, goal_owned): (
            World,
            Vec<Pair>,
            foldhash::HashMap<Pair, rows::WarOwnedRelationRow>,
        ),
        view: u32,
        recorded_view: Option<u32>,
    ) -> Self {
        let Countries {
            tags,
            identities,
            raw_to_slot,
        } = countries;
        Self {
            world,
            relation_order,
            tags,
            identities,
            raw_to_slot,
            view,
            recorded_view,
            goal_owned,
        }
    }
    #[cfg(test)]
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
    #[cfg(any(test, feature = "test-support"))]
    fn losses(&self, pairs: &[(Pair, bool)]) -> [String; 4] {
        losses_of(&self.world, pairs)
    }
    #[cfg(any(test, feature = "test-support"))]
    fn observe(
        &self,
        frame: &DecodedFrame,
        cat: Option<&Catalogue>,
        losses: bool,
    ) -> Vec<Observed> {
        let namer = naming::Namer::new(frame, cat);
        let count = |pairs: &[(Pair, bool)]| self.losses(pairs);
        self.world
            .blobs(self.view)
            .into_iter()
            .map(|blob| {
                let named = self
                    .world
                    .representative(&blob)
                    .map(|p| (p, namer.name(self, p).0));
                observed(
                    &self.world,
                    (&self.tags, &self.identities),
                    self.view,
                    blob,
                    named,
                    losses.then_some(&count as &Losses<'_>),
                    frame.hour,
                )
            })
            .collect()
    }
}
/// A group's losses from the counters of the relations of `world` it holds.
fn losses_of(world: &World, pairs: &[(Pair, bool)]) -> [String; 4] {
    let mut sums = [0_i128; 4];
    // A country's unknown loss is copied into each of its relations: the game
    // adds its largest counter (at least zero) once per country.
    let mut unknown = foldhash::HashMap::<(u32, usize), i64>::with_capacity_and_hasher(
        2 * pairs.len(),
        Default::default(),
    );
    for &(p, first) in pairs {
        let r = &world.relations[&p];
        let (a, b) = if first { (0, 2) } else { (2, 0) };
        sums[a] += i128::from(r.first_casualties);
        sums[b] += i128::from(r.second_casualties);
        for (country, side, value) in [
            (r.first_country, a + 1, r.first_unknown_casualties),
            (r.second_country, b + 1, r.second_unknown_casualties),
        ] {
            let held = unknown.entry((country, side)).or_default();
            *held = (*held).max(value);
        }
    }
    for ((_, side), value) in unknown {
        sums[side] += i128::from(value);
    }
    sums.map(|n| n.to_string())
}
/// Whether an overview operand table was recorded at `frame`'s hour, even if incomplete.
fn available(frame: &DecodedFrame) -> bool {
    [
        TableId::WarOwnedRelations,
        TableId::WarRelationGoals,
        TableId::WarOverviewCountries,
        TableId::WarNameRules,
    ]
    .iter()
    .any(|table| {
        !matches!(
            &*frame.tables[table.index()],
            TableView::Unavailable { .. }
        )
    })
}
/// Each capitulated country's raw id and date at `frame`'s hour, or none where the
/// capitulations were not read whole.
fn capitulations(frame: &DecodedFrame) -> Result<Vec<(u32, i32)>, ()> {
    frame
        .complete::<tables::Capitulation>()
        .map(|rows| {
            rows.iter()
                .filter(|r| r.capitulated != 0)
                .map(|r| (r.country, r.capitulated_hour))
                .collect()
        })
        .map_err(drop)
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
    // New runs follow native side order; the set only removes repeats. Runs
    // sharing an hour and tag keep this order through the final sort.
    let mut current = HashSet::new();
    let ordered: Vec<_> = sides(&data)
        .map(|(tag, side)| (tag.clone(), side))
        .filter(|key| current.insert(key.clone()))
        .collect();
    for (key, index) in &existing {
        if !current.contains(key) {
            war.participants[*index].left_by = false;
        }
    }
    for (tag, side) in ordered {
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
        let open = (relation.identity.clone(), relation.oriented);
        if let Some(&index) = war.open_relations.get(&open)
            && war.relations[index].last_hour.checked_add(1) == Some(hour)
        {
            war.relations[index].last_hour = hour;
        } else {
            war.open_relations.insert(open, war.relations.len());
            war.relations.push(WarRelationMembership {
                key: relation.identity.clone(),
                oriented: relation.oriented,
                first_hour: hour,
                last_hour: hour,
            });
        }
    }
}
/// One war's native groups at an hour as one observation: the first group's
/// name and orientation, every member once, and each relation once.
fn merge_groups(mut groups: Vec<Observed>, losses: Option<&Losses<'_>>) -> Observed {
    let mut merged = groups.remove(0);
    if groups.is_empty() {
        return merged;
    }
    let mut viewpoint: HashSet<String> = merged.data.viewpoint_side.iter().cloned().collect();
    let mut opposing: HashSet<String> = merged.data.opposing_side.iter().cloned().collect();
    let mut relations: HashSet<WarRelationKey> =
        merged.relations.iter().map(|r| r.identity.clone()).collect();
    let mut pairs: HashSet<Pair> = merged.loss_pairs.iter().map(|(p, _)| *p).collect();
    for group in groups {
        merged.data.viewpoint_side.extend(
            group.data.viewpoint_side.into_iter().filter(|tag| viewpoint.insert(tag.clone())),
        );
        merged.data.opposing_side.extend(
            group.data.opposing_side.into_iter().filter(|tag| opposing.insert(tag.clone())),
        );
        merged
            .relations
            .extend(group.relations.into_iter().filter(|r| relations.insert(r.identity.clone())));
        merged
            .loss_pairs
            .extend(group.loss_pairs.into_iter().filter(|(p, _)| pairs.insert(*p)));
    }
    if let Some(losses) = losses {
        merged.losses = losses(&merged.loss_pairs);
    }
    merged
}

/// A group's losses from its relations, for a walk that keeps them.
type Losses<'a> = dyn Fn(&[(Pair, bool)]) -> [String; 4] + 'a;
/// Each of `current`'s groups' war, as [`continue_wars`] assigns them: the open war it continues
/// or joins, or a new one, numbered from `wars` in group order.
fn assignment(wars: usize, open: &[OpenWar], current: &[Observed]) -> Vec<usize> {
    // App rule: greedy one-to-one in native blob order. Choose the earlier
    // group's most continuing bilateral relations; tie: lowest app id.
    // A relation continues by its native identities, so a renamed country keeps its war.
    let mut inverted: HashMap<&WarRelationKey, Vec<usize>> = HashMap::new();
    for war in open {
        for relation in &war.relations {
            inverted.entry(&relation.identity).or_default().push(war.id);
        }
    }
    let mut used = HashSet::new();
    let mut continued: Vec<Option<usize>> = current
        .iter()
        .map(|observation| {
            let mut counts = HashMap::<usize, usize>::new();
            for relation in &observation.relations {
                for &id in inverted.get(&relation.identity).into_iter().flatten() {
                    if !used.contains(&id) {
                        *counts.entry(id).or_default() += 1;
                    }
                }
            }
            let id = counts
                .into_iter()
                .max_by(|(a, ac), (b, bc)| ac.cmp(bc).then(b.cmp(a)))
                .map(|(id, _)| id)?;
            used.insert(id);
            Some(id)
        })
        .collect();
    // The game lists a new declaration as its own group beside the war it
    // joins, named for that war, for a day or more. A group no open war
    // continues joins the war this hour that holds the relation the group is
    // named for, the one sharing most relations; tie: lowest app id.
    let mut held: HashMap<usize, HashSet<&WarRelationKey>> = HashMap::new();
    for (observation, id) in current.iter().zip(&continued) {
        if let Some(id) = id {
            held.entry(*id)
                .or_default()
                .extend(observation.relations.iter().map(|r| &r.identity));
        }
    }
    let mut new = wars;
    for (index, observation) in current.iter().enumerate() {
        if continued[index].is_some() {
            continue;
        }
        let joined = observation.representative.as_ref().and_then(|named| {
            held.iter()
                .filter(|(_, relations)| relations.contains(named))
                .map(|(&id, relations)| {
                    let shared = observation
                        .relations
                        .iter()
                        .filter(|r| relations.contains(&r.identity))
                        .count();
                    (id, shared)
                })
                .max_by(|(a, ac), (b, bc)| ac.cmp(bc).then(b.cmp(a)))
                .map(|(id, _)| id)
        });
        let id = joined.unwrap_or_else(|| {
            new += 1;
            new - 1
        });
        continued[index] = Some(id);
        held.entry(id)
            .or_default()
            .extend(observation.relations.iter().map(|r| &r.identity));
    }
    continued
        .into_iter()
        .map(|id| id.expect("every group continues or starts a war"))
        .collect()
}
fn continue_wars(
    wars: &mut Vec<WarTimeline>,
    open: &[OpenWar],
    current: Vec<Observed>,
    previous: Option<i32>,
    losses: Option<&Losses<'_>>,
) -> Vec<OpenWar> {
    let continued = assignment(wars.len(), open, &current);
    let used: HashSet<usize> = continued.iter().copied().collect();
    // Wars in the native order of their first group. New wars are numbered in group order, so
    // each first appears as the next war.
    let mut order: Vec<usize> = Vec::new();
    let mut groups: HashMap<usize, Vec<Observed>> = HashMap::new();
    for (observation, id) in current.into_iter().zip(continued) {
        if id == wars.len() {
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
        }
        if !groups.contains_key(&id) {
            order.push(id);
        }
        groups.entry(id).or_default().push(observation);
    }
    let mut next = Vec::new();
    for id in order {
        let held = groups.remove(&id).expect("ordered war");
        let count = held.len();
        let observation = merge_groups(held, losses);
        let hour = observation.data.first_hour;
        append_relations(&mut wars[id], hour, &observation.relations);
        if losses.is_some() {
            append_losses(&mut wars[id], hour, observation.losses, previous);
        }
        add_observation(&mut wars[id], observation.data, previous);
        next.push(OpenWar {
            id,
            groups: count,
            relations: observation.relations,
            loss_pairs: observation.loss_pairs,
        });
    }
    if previous.is_some() {
        for OpenWar { id, .. } in open {
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
#[cfg(any(test, feature = "test-support"))]
fn selected_view(frame: &DecodedFrame, identities: &HashSet<(u32, u32)>) -> Result<String, String> {
    let countries = frame
        .complete::<tables::Countries>()
        .map_err(|e| e.to_string())?;
    selected_holder(
        countries
            .iter()
            .map(|row| (row.canonical_slot, row.id, row.tag.as_slice())),
        identities,
    )
}
/// The tag of the one row of `countries` (slot, native id, tag) holding a selected identity.
fn selected_holder<'t>(
    countries: impl Iterator<Item = (u32, u32, &'t [u8])>,
    identities: &HashSet<(u32, u32)>,
) -> Result<String, String> {
    let mut found = countries.filter(|&(slot, id, _)| identities.contains(&(slot, id)));
    let (_, _, tag) = found
        .next()
        .ok_or("Not recorded: selected native war viewpoint holder")?;
    if found.next().is_some() {
        return Err("Not recorded: multiple selected native war viewpoint holders".into());
    }
    std::str::from_utf8(tag)
        .map(str::to_owned)
        .map_err(|_| "country tag is not UTF-8".into())
}
#[cfg(any(test, feature = "test-support"))]
fn scan(
    replay: &Replay,
    branch: u64,
    view: Option<&str>,
    cat: Option<&Catalogue>,
    cancelled: &AtomicBool,
) -> io::Result<WarsReply> {
    scan_selected(replay, branch, view, None, cat, cancelled)
}
#[cfg(any(test, feature = "test-support"))]
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

#[cfg(any(test, feature = "test-support"))]
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
    walk.regroup_unchanged = true;
    dashboard_core::blocks::scan_blocks(
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
                    walk.observe_frame(decoded, view, identities, cat);
                }
            }
            Ok(())
        },
    )?;
    walk.finish(last, cat)
}

/// The groups `view` sees at `frame`'s hour, from that frame alone
/// ([`WarsAtHour`]): the branch walk's observation of one hour.
#[cfg(any(test, feature = "test-support"))]
pub(crate) fn at_hour(
    branch: u64,
    frame: &DecodedFrame,
    view: Option<&str>,
    cat: Option<&Catalogue>,
) -> io::Result<WarsAtHour> {
    let hour = frame.hour;
    // A capitulation is a dated event that the hour alone cannot place.
    let mut frame = frame.clone();
    frame.tables[TableId::Capitulation.index()] = TableView::Unavailable {
        reason: "Capitulations need earlier hours".into(),
    }
    .into();
    let mut walk = Walk::new(branch, hour);
    walk.labels = false;
    walk.observe_frame(&frame, view, None, cat);
    let reply = walk.finish(hour, cat)?;
    Ok(at_hour_reply(
        hour,
        reply,
        participation::read(&frame, view, cat),
    ))
}

/// The reply of one hour's walk, its wars named by that hour.
fn at_hour_reply(
    hour: i32,
    mut reply: WarsReply,
    contributions: Vec<WarContributionGroup>,
) -> WarsAtHour {
    for (index, war) in reply.wars.iter_mut().enumerate() {
        war.id = format!("@{hour}:{index}");
    }
    WarsAtHour {
        hour,
        contributions,
        available: reply.available,
        wars: reply.wars,
        warnings: reply.warnings,
    }
}

#[derive(Clone)]
struct Walk {
    reply: WarsReply,
    previous: Option<i32>,
    /// The viewpoint's native identity, so a rename keeps its wars.
    previous_view: Option<(u32, u32)>,
    open: Vec<OpenWar>,
    seen_caps: HashSet<(String, i32)>,
    dated_caps: HashMap<String, Vec<i32>>,
    identity_since: foldhash::HashMap<u32, (String, i32)>,
    warnings: BTreeSet<String>,
    last_frame: Option<i32>,
    /// What the last hour observed read and the groups it saw, while it read.
    held: Option<(Viewed, Vec<Observed>)>,
    country_labels: labels::CountryLabels,
    /// Whether historical country names are measured: one hour's walk ([`at_hour`]) names
    /// only that hour's countries.
    labels: bool,
    /// Whether an hour observing the very groups held before still continues its wars as a
    /// structural change does: only the reference walk the tests compare with.
    regroup_unchanged: bool,
    /// Whether each war's loss runs are kept: a query reads its casualties from the prepared
    /// counters at the hours it asks for instead. A walk keeping them observes every hour it
    /// walks and never extends.
    losses: bool,
    /// The countries identities and involved countries were last refreshed from: an hour
    /// holding the same ones refreshes neither again.
    identified: Option<Arc<Countries>>,
    /// Each held group's war as the last structural change assigned it ([`assignment`]).
    assigned: Vec<usize>,
    // Continuation reads the previous assignment even when every input table stays unchanged.
    settled: bool,
}
impl Walk {
    fn new(branch: u64, first: i32) -> Self {
        let reply=WarsReply{branch_id:branch.to_string(),available:false,viewpoints:Vec::new(),country_names:Vec::new(),wars:Vec::new(),unobserved:Vec::new(),capitulations_unobserved:Vec::new(),warnings:vec!["War continuity and participant runs are app observations; the game stores no grouped war identity, start/end or join/leave hours.".into(),"Attackers and defenders are oriented by the representative relation's instigator.".into(),"Native groups can overlap. A group the game lists beside the war it is named for, as it does for a new declaration, joins that war; a war's casualties count each of its bilateral relations once.".into()]};
        Self {
            reply,
            previous: None,
            previous_view: None,
            open: Vec::new(),
            seen_caps: HashSet::new(),
            dated_caps: HashMap::new(),
            identity_since: Default::default(),
            warnings: BTreeSet::new(),
            last_frame: first.checked_sub(1),
            held: None,
            country_labels: Default::default(),
            labels: true,
            regroup_unchanged: false,
            losses: true,
            identified: None,
            assigned: Vec::new(),
            settled: false,
        }
    }
    /// Observes `decoded` as a frame read whole.
    #[cfg(any(test, feature = "test-support"))]
    fn observe_frame(
        &mut self,
        decoded: &DecodedFrame,
        view: Option<&str>,
        identities: Option<&HashSet<(u32, u32)>>,
        cat: Option<&Catalogue>,
    ) {
        if self.labels {
            self.country_labels.observe(decoded, cat);
        }
        self.reply.available |= available(decoded);
        let capitulations = capitulations(decoded);
        let capitulations = capitulations.as_deref().map_err(|_| ());
        match read_hour(decoded, view, identities, cat, self.losses) {
            Ok((hour, observed)) => {
                let losses = |pairs: &[(Pair, bool)]| hour.losses(pairs);
                self.observe(Seen {
                    hour: decoded.hour,
                    read: Ok((Viewed::of(&hour), observed)),
                    capitulations,
                    losses: Some(&losses as &Losses<'_>),
                });
            }
            Err(reason) => self.observe(Seen {
                hour: decoded.hour,
                read: Err(reason),
                capitulations,
                losses: None,
            }),
        }
    }
    /// Each open war's open participants end at its last hour.
    fn settle_open(&mut self) {
        for OpenWar { id, .. } in &self.open {
            let war: &mut WarTimeline = &mut self.reply.wars[*id];
            for &index in war.open_participants.values() {
                war.participants[index].last_hour = war.last_hour;
            }
        }
    }
    fn observe(&mut self, seen: Seen<'_>) {
        let hour = seen.hour;
        if let Some(before) = self.last_frame
            && before.checked_add(1) != Some(hour)
        {
            span(&mut self.reply.unobserved, before + 1, hour - 1);
            span(
                &mut self.reply.capitulations_unobserved,
                before + 1,
                hour - 1,
            );
            self.settle_open();
            self.previous = None;
            self.open.clear();
            self.identity_since.clear();
        }
        self.last_frame = Some(hour);
        let (viewed, observed) = match seen.read {
            Ok(read) => read,
            Err(reason) => {
                self.warnings.insert(reason);
                span(&mut self.reply.unobserved, hour, hour);
                span(&mut self.reply.capitulations_unobserved, hour, hour);
                self.settle_open();
                self.previous = None;
                self.open.clear();
                self.identity_since.clear();
                self.held = None;
                return;
            }
        };
        // An hour observing the very groups held before changes only its losses.
        let regrouped = !self.regroup_unchanged
            && self
                .held
                .as_ref()
                .is_some_and(|(_, before)| same_groups(before, &observed));
        let identified = self
            .identified
            .as_ref()
            .is_some_and(|countries| Arc::ptr_eq(countries, &viewed.countries));
        let viewpoint = viewed.countries.identities.get(&viewed.view).copied();
        // The same groups of the same countries, seen by the same viewpoint, involve no
        // country they did not.
        if self.labels
            && (self.previous.is_none()
                || !(identified && regrouped && self.previous_view == viewpoint))
        {
            self.country_labels.include(&viewed, &observed);
        }
        append_viewpoint(
            &mut self.reply,
            viewed.countries.tags.get(&viewed.view),
            hour,
        );
        if self.previous_view != viewpoint {
            self.settle_open();
            self.previous = None;
            self.open.clear();
        }
        self.previous_view = viewpoint;
        let losses = if self.losses { seen.losses } else { None };
        // The very groups held before continue as losses alone only while continuing them
        // would assign each to the war it holds: overlapping groups can trade wars.
        if regrouped
            && self.settled
            && self.previous.is_some()
            && self.open.iter().map(|war| war.groups).sum::<usize>() == observed.len()
        {
            // Loss-only hours retain native groups and continuity. No
            // side lists, membership sets or relation runs are rebuilt.
            for open in &self.open {
                let war = &mut self.reply.wars[open.id];
                if let Some(losses) = losses {
                    append_losses(war, hour, losses(&open.loss_pairs), self.previous);
                }
                append_relations(war, hour, &open.relations);
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
                for OpenWar { id, .. } in &self.open {
                    let war = &mut self.reply.wars[*id];
                    for &index in war.open_participants.values() {
                        war.participants[index].last_hour = last;
                    }
                }
            }
            let observations = opening(&observed, hour);
            self.assigned = assignment(self.reply.wars.len(), &self.open, &observations);
            self.open = continue_wars(
                &mut self.reply.wars,
                &self.open,
                observations,
                self.previous,
                losses,
            );
            self.settled = assignment(self.reply.wars.len(), &self.open, &observed) == self.assigned;
        }
        // The same countries after an observed hour leave every identity as it was.
        if self.previous.is_none() || !identified {
            let countries = &viewed.countries;
            self.identity_since.retain(|raw, _| {
                countries
                    .raw_to_slot
                    .get(raw)
                    .is_some_and(|slot| countries.tags.contains_key(slot))
            });
            for (&raw, &slot) in &countries.raw_to_slot {
                let Some(tag) = countries.tags.get(&slot) else {
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
            self.identified = Some(Arc::clone(countries));
        }
        match seen.capitulations {
            Ok(rows) => {
                for &(country, capitulated) in rows {
                    let Some((tag, since)) = self.identity_since.get(&country) else {
                        continue;
                    };
                    if capitulated < *since || capitulated > hour {
                        self.warnings.insert(CAPITULATIONS_UNOBSERVED.into());
                        continue;
                    }
                    if !self.seen_caps.insert((tag.clone(), capitulated)) {
                        continue;
                    }
                    self.dated_caps
                        .entry(tag.clone())
                        .or_default()
                        .push(capitulated);
                }
            }
            Err(()) => span(&mut self.reply.capitulations_unobserved, hour, hour),
        }
        self.previous = Some(hour);
        self.held = Some((viewed, observed));
    }
    /// Extend unchanged inputs across accepted hours without walking those hours.
    fn extend(&mut self, last: i32) {
        let Some(first) = self.last_frame else { return };
        if last <= first {
            return;
        }
        if self.previous == Some(first) {
            if let Some((_, observed)) = &self.held {
                let mut at = first;
                while at < last && !self.settled {
                    at += 1;
                    // Unchanged groups still read the wars assigned at the previous hour.
                    for OpenWar { id, .. } in &self.open {
                        let war = &mut self.reply.wars[*id];
                        for &index in war.open_participants.values() {
                            war.participants[index].last_hour = war.last_hour;
                        }
                    }
                    let observations = opening(observed, at);
                    self.assigned = assignment(self.reply.wars.len(), &self.open, observed);
                    self.open = continue_wars(
                        &mut self.reply.wars,
                        &self.open,
                        observations,
                        self.previous,
                        None,
                    );
                    self.settled =
                        assignment(self.reply.wars.len(), &self.open, observed) == self.assigned;
                    self.previous = Some(at);
                }
            }
            if let Some(viewpoint) = self.reply.viewpoints.last_mut() {
                viewpoint.last_hour = last;
            }
            for OpenWar { id, relations, .. } in &self.open {
                let war = &mut self.reply.wars[*id];
                war.last_hour = last;
                if let Some(observation) = war.observations.last_mut() {
                    observation.last_hour = last;
                }
                if let Some(losses) = war.casualties.last_mut() {
                    losses.1 = last;
                }
                // Only the relations the war holds now: one that left it keeps its last hour.
                for relation in relations {
                    let open = (relation.identity.clone(), relation.oriented);
                    if let Some(&index) = war.open_relations.get(&open) {
                        war.relations[index].last_hour = last;
                    }
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
            // A finished reply keeps no open index, nor its allocation.
            war.open_participants = HashMap::new();
            war.open_relations = HashMap::new();
            war.participants
                .sort_by(|a, b| a.first_hour.cmp(&b.first_hour).then(a.tag.cmp(&b.tag)));
            war.capitulations.sort_by_key(|c| c.hour);
        }
        check_reply(&self.reply)?;
        Ok(self.reply)
    }
}

fn check_reply(reply: &WarsReply) -> io::Result<()> {
    if dashboard_core::transport::json_len(&reply, MAX_PRODUCT_REPLY_BYTES).is_none() {
        return Err(invalid(format!(
            "Wars reply exceeds the shared product reply budget ({} MiB)",
            MAX_PRODUCT_REPLY_BYTES / (1024 * 1024)
        )));
    }
    Ok(())
}
