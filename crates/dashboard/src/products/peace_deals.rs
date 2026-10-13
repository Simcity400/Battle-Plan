//! Captured finalizer actions with a fallback to dated derived peace outcomes.
//!
//! An owner change is evidence only when its former and new owners had an
//! ongoing WarRelations pair before the change and that pair records an end
//! or disappears from the complete after read. Unsampled conferences require
//! consecutive hours and an end at or absence by the new read. Connected pairs
//! group an outcome, not a proven conference identity. The fallback cannot recover
//! negotiators or action order;
//! successful finalizer observations provide them in `exact`.
//!
//! A state handed to a country the same settlement released joins the transfer
//! graphs marked `released`, from its own former owner at the before read.

use super::{
    LoadContext, MAX_PRODUCT_REPLY_BYTES, Product,
    country_groups::{HourSpan, war_ongoing},
};
use crate::{
    Source, catalogue, content,
    metadata::{CountryKeys, branch_bounds, country_tables, slot_read, validate_campaign},
};
use replay::{
    container::Replay,
    frame::{DecodedFrame, FrameDecoder, ViewIdentity},
    rows::{CountryLinkRow, CountryRow, TableId, tables},
};
use serde::{Deserialize, Serialize};
use std::{
    collections::{HashMap, HashSet, VecDeque},
    io,
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
};
use ts_rs::TS;

pub struct PeaceDeals;
mod exact;
pub use exact::{PeaceCaptured, PeaceCapturedAction, PeaceCapturedSubject, PeaceCapturedTransfer};

#[derive(Clone, Debug, Deserialize, Serialize, TS)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct PeaceDealsParams {
    pub branch_id: String,
}

/// The reads bounding a net change; neither a bid time nor an exact action time.
#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq, TS)]
#[serde(rename_all = "camelCase")]
pub struct PeaceDates {
    pub before_hour: i32,
    pub after_hour: i32,
}

#[derive(Clone, Copy, Debug, Deserialize, Serialize, PartialEq, Eq, Hash, TS)]
#[serde(rename_all = "camelCase")]
pub enum PeaceActionKind {
    StateTransfer,
    NewSubject,
    GovernmentChange,
    CountryReleased,
}

/// A content-proved action name, else a neutral kind the widget can word.
#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq, TS)]
#[serde(rename_all = "camelCase")]
pub struct PeaceActionLabel {
    pub kind: PeaceActionKind,
    pub key: Option<String>,
    pub name: Option<String>,
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq, TS)]
#[serde(rename_all = "camelCase")]
pub struct PeaceGovernment {
    pub ruling_party: Option<u32>,
    pub party_token: Option<String>,
    pub leader_ideology: Option<String>,
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq, TS)]
#[serde(
    tag = "kind",
    rename_all = "camelCase",
    rename_all_fields = "camelCase"
)]
pub enum PeaceChange {
    StateTransfer {
        state: u32,
        from: String,
        to: String,
        from_country: PeaceTransferCountry,
        to_country: PeaceTransferCountry,
        /// Exact signed sum, null if Provinces cannot supply the state.
        victory_points: Option<String>,
        victory_points_read_hour: Option<i32>,
    },
    NewSubject {
        overlord: String,
        subject: String,
        autonomy_state: String,
        start_hour: i32,
    },
    GovernmentChange {
        country: String,
        before: PeaceGovernment,
        after: PeaceGovernment,
    },
    CountryReleased {
        country: String,
        /// Owners of the transferred states at the before read.
        from: Vec<String>,
        states: Vec<u32>,
        /// The released country's native holder at the after read.
        to_country: PeaceTransferCountry,
        /// Each state with its own former owner, in state order.
        transfers: Vec<PeaceReleasedState>,
    },
}

/// A state handed to the released country, with the holder that owned it before.
#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq, TS)]
#[serde(rename_all = "camelCase")]
pub struct PeaceReleasedState {
    pub state: u32,
    pub from_country: PeaceTransferCountry,
    /// Exact signed sum, null if Provinces cannot supply the state.
    pub victory_points: Option<String>,
    pub victory_points_read_hour: Option<i32>,
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq, TS)]
#[serde(rename_all = "camelCase")]
pub struct PeaceItem {
    pub derived: bool,
    pub dates: PeaceDates,
    /// The taker/overlord, null where no enforcer or liberator is recorded.
    pub winner: Option<String>,
    pub action: PeaceActionLabel,
    pub change: PeaceChange,
}

#[derive(Clone, Copy, Debug, Deserialize, Serialize, PartialEq, Eq, TS)]
#[serde(rename_all = "camelCase")]
pub enum PeaceObservation {
    RecordedConference,
    ConferenceNotRecorded,
    /// Outcomes cannot be assigned among sampled conferences or missing rosters.
    ConferenceUnknown,
}

/// An endpoint's native holder and the read that supplies its tag and appearance.
#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq, TS)]
#[serde(rename_all = "camelCase")]
pub struct PeaceTransferCountry {
    pub slot: u32,
    pub country_id: u32,
    pub tag: String,
    pub hour: i32,
}
impl PeaceTransferCountry {
    fn identity(&self) -> (u32, u32) {
        (self.slot, self.country_id)
    }
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq, TS)]
#[serde(rename_all = "camelCase")]
pub struct PeaceTransferNode {
    pub id: String,
    pub country: PeaceTransferCountry,
    /// Separate source/target nodes even when a country occurs on both sides.
    pub side: PeaceTransferSide,
}

#[derive(Clone, Copy, Debug, Deserialize, Serialize, PartialEq, Eq, TS)]
#[serde(rename_all = "camelCase")]
pub enum PeaceTransferSide {
    Source,
    Target,
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq, TS)]
#[serde(rename_all = "camelCase")]
pub struct PeaceTransferLink {
    pub id: String,
    pub source: String,
    pub target: String,
    pub state_count: u32,
    /// Null if any contributing state's VPs are not recorded; never a partial sum.
    pub victory_points: Option<String>,
    /// Dated occurrences, kept on both the card and the campaign graph.
    pub states: Vec<PeaceTransferredState>,
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq, TS)]
#[serde(rename_all = "camelCase")]
pub struct PeaceTransferredState {
    pub state: u32,
    pub from_country: PeaceTransferCountry,
    pub to_country: PeaceTransferCountry,
    pub dates: PeaceDates,
    pub victory_points: Option<String>,
    pub victory_points_read_hour: Option<i32>,
    /// The same settlement released the taker; the state came from its former owner.
    pub released: bool,
}

#[derive(Clone, Debug, Default, Deserialize, Serialize, PartialEq, Eq, TS)]
#[serde(rename_all = "camelCase")]
pub struct PeaceTransferGraph {
    pub nodes: Vec<PeaceTransferNode>,
    pub links: Vec<PeaceTransferLink>,
}

#[derive(Clone, Debug, Deserialize, Serialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct PeaceDeal {
    pub id: String,
    pub derived: bool,
    pub observation: PeaceObservation,
    pub conference_id: Option<String>,
    /// No native conference name or unambiguous grouped-war identity is recorded.
    pub war_name: Option<String>,
    pub first_observed_hour: i32,
    pub last_observed_hour: i32,
    pub current_turn: Option<i32>,
    pub finalized: Option<bool>,
    pub first_finalized_hour: Option<i32>,
    /// Last seen when followed by a consecutive absence, else first absent hour.
    pub end_hour: Option<i32>,
    pub end_is_by: bool,
    pub first_absent_hour: Option<i32>,
    pub participants_derived: bool,
    pub winners: Vec<String>,
    pub losers: Vec<String>,
    pub items: Vec<PeaceItem>,
    /// Ordered finalizer operands and outcomes, absent for the derived fallback.
    pub captured: Option<PeaceCaptured>,
    /// Reason the finalizer capture cannot certify this deal's actions.
    pub capture_unavailable: Option<String>,
    /// Supplying manifest hour when no sampled conference supplied this card.
    #[serde(skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub capture_observed_hour: Option<i32>,
    /// A failed manifest's reason is not evidence at its earlier event hour.
    #[serde(skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub capture_unavailable_hour: Option<i32>,
    pub transfers: PeaceTransferGraph,
    /// Missing inputs or an attribution limitation, not evidence of no actions.
    pub limitations: Vec<String>,
}

#[derive(Clone, Debug, Default, Serialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct PeaceUnobserved {
    pub settlements: Vec<HourSpan>,
    pub conferences: Vec<HourSpan>,
    pub ownership: Vec<HourSpan>,
    pub wars: Vec<HourSpan>,
    pub subjects: Vec<HourSpan>,
    pub governments: Vec<HourSpan>,
    pub existence: Vec<HourSpan>,
    pub victory_points: Vec<HourSpan>,
}

#[derive(Clone, Debug, Serialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct PeaceDealsReply {
    pub branch_id: String,
    pub derived: bool,
    /// Newest card first; no widget needs to scan the replay on playback.
    pub deals: Vec<PeaceDeal>,
    /// Counts transfer occurrences, including a state transferred more than once.
    pub transfers: PeaceTransferGraph,
    pub unobserved: PeaceUnobserved,
    pub warnings: Vec<String>,
}

#[derive(Default)]
struct PeaceCache(HashMap<u64, Arc<PeaceDealsReply>>);

#[derive(Default)]
struct PeaceLabelCache(HashMap<(u64, content::LocalisationKey), Arc<PeaceDealsReply>>);

impl Product for PeaceDeals {
    const KEY: &'static str = "peace-deals";
    const CONTENT_DEPENDENT: bool = true;
    type Params = PeaceDealsParams;
    type Reply = PeaceDealsReply;

    fn types() -> Vec<String> {
        vec![
            PeaceDealsParams::decl(),
            PeaceDealsReply::decl(),
            PeaceDeal::decl(),
            PeaceObservation::decl(),
            PeaceDates::decl(),
            PeaceItem::decl(),
            PeaceChange::decl(),
            PeaceGovernment::decl(),
            PeaceActionKind::decl(),
            PeaceActionLabel::decl(),
            PeaceTransferGraph::decl(),
            PeaceTransferNode::decl(),
            PeaceTransferCountry::decl(),
            PeaceTransferSide::decl(),
            PeaceTransferLink::decl(),
            PeaceTransferredState::decl(),
            PeaceReleasedState::decl(),
            PeaceUnobserved::decl(),
            PeaceCaptured::decl(),
            PeaceCapturedAction::decl(),
            PeaceCapturedTransfer::decl(),
            PeaceCapturedSubject::decl(),
        ]
    }

    fn validate(params: &Self::Params) -> Result<(), &'static str> {
        params
            .branch_id
            .parse::<u64>()
            .map(|_| ())
            .map_err(|_| "Invalid peace-deals branch")
    }

    fn load(
        source: &Source,
        params: &Self::Params,
        context: &LoadContext,
        cancelled: &AtomicBool,
    ) -> io::Result<Self::Reply> {
        check_cancel(cancelled)?;
        let branch = params.branch_id.parse().expect("validated branch");
        let state = source.state::<PeaceCache>();
        let mut state = state.lock().unwrap_or_else(|error| error.into_inner());
        let scan = match state.0.get(&branch) {
            Some(reply) => Arc::clone(reply),
            None => {
                let reply = Arc::new(scan_branch(&source.replay(), branch, cancelled)?);
                state.0.insert(branch, Arc::clone(&reply));
                reply
            }
        };
        drop(state);
        let Some(options) = context.content.as_ref() else {
            let mut reply = (*scan).clone();
            reply.warnings.push(
                "Peace action names are unavailable until the game and user folders are chosen."
                    .into(),
            );
            return admitted(reply);
        };
        let key = match content::localisation_key(options) {
            Ok(key) => key,
            Err(error) => {
                let mut reply = (*scan).clone();
                reply
                    .warnings
                    .push(format!("Peace action names are unavailable: {error}"));
                return admitted(reply);
            }
        };
        let named = source.state::<PeaceLabelCache>();
        let mut named = named.lock().unwrap_or_else(|error| error.into_inner());
        if let Some(reply) = named.0.get(&(branch, key.clone())) {
            return admitted((**reply).clone());
        }
        let mut reply = (*scan).clone();
        match catalogue::catalogue(source, options) {
            Ok(catalogue) => {
                reply
                    .warnings
                    .extend(catalogue.verification.iter().cloned());
                reply.warnings.extend(catalogue.warnings.iter().cloned());
                name_actions(
                    &mut reply,
                    &catalogue.localisation,
                    &catalogue.recorded_language,
                );
            }
            Err(error) => {
                reply
                    .warnings
                    .push(format!("Peace action names are unavailable: {error}"));
                return admitted(reply);
            }
        }
        check_cancel(cancelled)?;
        named
            .0
            .retain(|(cached_branch, _), _| *cached_branch != branch);
        named.0.insert((branch, key), Arc::new(reply.clone()));
        admitted(reply)
    }
}

fn admitted(reply: PeaceDealsReply) -> io::Result<PeaceDealsReply> {
    if crate::transport::json_len(&reply, MAX_PRODUCT_REPLY_BYTES).is_none() {
        return Err(crate::invalid(format!(
            "Peace deals complete reply exceeds the {} MiB transport budget; no entries were dropped.",
            MAX_PRODUCT_REPLY_BYTES / (1024 * 1024),
        )));
    }
    Ok(reply)
}

fn neutral(kind: PeaceActionKind) -> PeaceActionLabel {
    PeaceActionLabel {
        kind,
        key: None,
        name: None,
    }
}

/// Shared tag text: hourly maps clone a reference, not the string.
type Tag = Arc<str>;

/// Raw references are resolved once per frame, with aliases joined by slot,
/// beside each tag's native (canonical slot, country id).
struct CountryTags {
    tags: HashMap<u32, Tag>,
    identities: HashMap<Tag, (u32, u32)>,
}

impl CountryTags {
    fn resolve(countries: &[CountryRow], links: &[CountryLinkRow]) -> Option<Self> {
        let mut slots = HashMap::with_capacity(countries.len());
        let mut tags = HashMap::with_capacity(links.len());
        let mut identities = HashMap::with_capacity(countries.len());
        for row in countries {
            let tag = Tag::from(std::str::from_utf8(&row.tag).ok()?);
            if tag.is_empty()
                || identities
                    .insert(tag.clone(), (row.canonical_slot, row.id))
                    .is_some()
                || slots.insert(row.canonical_slot, tag.clone()).is_some()
            {
                return None;
            }
            tags.insert(row.id, tag);
        }
        for row in links {
            if let Some(tag) = slots.get(&row.canonical_slot) {
                if tags.get(&row.id).is_some_and(|prior| prior != tag) {
                    return None;
                }
                tags.insert(row.id, tag.clone());
            }
        }
        Some(Self { tags, identities })
    }
}

/// The last `CountryTags`, kept while the only rows they read are unchanged:
/// the Countries keys and CountryLinks.
#[derive(Default)]
struct CountryResolution {
    countries: CountryKeys,
    links: Vec<CountryLinkRow>,
    resolved: Option<Arc<CountryTags>>,
}

impl CountryResolution {
    fn read(&mut self, frame: &DecodedFrame) -> Option<Arc<CountryTags>> {
        let (countries, links) = country_tables(frame).ok()?;
        if self.resolved.is_none() || !self.countries.same(countries) || self.links != links {
            self.resolved = CountryTags::resolve(countries, links).map(Arc::new);
            self.countries = CountryKeys::read(countries);
            self.links = links.to_vec();
        }
        self.resolved.clone()
    }
}

/// The last victory points summed by state, kept while every province's state
/// and victory points compare equal. Provinces changes every hour.
#[derive(Default)]
struct VictoryPoints {
    fields: Vec<(u32, i32)>,
    totals: Option<Arc<HashMap<u32, i64>>>,
}

impl VictoryPoints {
    fn read(&mut self, frame: &DecodedFrame) -> Option<Arc<HashMap<u32, i64>>> {
        let rows = frame.complete::<tables::Provinces>().ok()?;
        let fields = rows.iter().map(|row| (row.state, row.victory_points));
        if self.totals.is_none() || !fields.clone().eq(self.fields.iter().copied()) {
            let mut totals = HashMap::new();
            for (state, victory_points) in fields.clone() {
                *totals.entry(state).or_insert(0_i64) += i64::from(victory_points);
            }
            self.totals = Some(Arc::new(totals));
            self.fields = fields.collect();
        }
        self.totals.clone()
    }
}

struct Snapshot {
    hour: i32,
    roster_complete: bool,
    countries: Arc<CountryTags>,
    owners: HashMap<u32, Tag>,
    states_by_owner: HashMap<Tag, Vec<u32>>,
    unresolved_owners: Vec<(u32, u32)>,
    wars: HashMap<(String, String), (i32, i32)>,
    unresolved_wars: Vec<(u32, u32)>,
    vps: Option<Arc<HashMap<u32, i64>>>,
    subjects: Option<HashMap<(String, String), (i32, String)>>,
    subject_targets: HashMap<String, Vec<String>>,
    governments: Option<HashMap<Tag, PeaceGovernment>>,
    exists: Option<Arc<HashMap<Tag, bool>>>,
    state_action_key: Option<String>,
}

struct OpenConference {
    index: usize,
    before: Option<Arc<Snapshot>>,
}

fn pair(a: &str, b: &str) -> (String, String) {
    if a < b {
        (a.into(), b.into())
    } else {
        (b.into(), a.into())
    }
}

fn snapshot(
    frame: &DecodedFrame,
    countries: Arc<CountryTags>,
    vps: Option<Arc<HashMap<u32, i64>>>,
    exists: Option<Arc<HashMap<Tag, bool>>>,
    state_action_key: Option<String>,
) -> Option<Snapshot> {
    let states = frame.complete::<tables::States>().ok()?;
    let relations = frame.complete::<tables::WarRelations>().ok()?;
    let tags = &countries.tags;
    let mut unresolved_owners = Vec::new();
    let owners = states
        .iter()
        .filter_map(|row| {
            if let Some(tag) = tags.get(&row.owner) {
                Some((row.id, tag.clone()))
            } else {
                unresolved_owners.push((row.id, row.owner));
                None
            }
        })
        .collect::<HashMap<_, _>>();
    let mut states_by_owner: HashMap<Tag, Vec<u32>> = HashMap::new();
    for (&state, tag) in &owners {
        states_by_owner.entry(tag.clone()).or_default().push(state);
    }
    let mut wars = HashMap::new();
    let mut unresolved_wars = Vec::new();
    let mut ambiguous = HashSet::new();
    for row in relations {
        let (Some(first), Some(second)) =
            (tags.get(&row.first_country), tags.get(&row.second_country))
        else {
            unresolved_wars.push((row.first_country, row.second_country));
            continue;
        };
        let key = pair(first, second);
        let value = (row.start_hour, row.end_hour);
        if wars.get(&key).is_some_and(|prior| *prior != value) {
            wars.remove(&key);
            ambiguous.insert(key.clone());
            unresolved_wars.push((row.first_country, row.second_country));
        }
        if !ambiguous.contains(&key) {
            wars.insert(key, value);
        }
    }
    let subjects: Option<HashMap<(String, String), (i32, String)>> =
        frame.complete::<tables::Subjects>().ok().and_then(|rows| {
            rows.iter()
                .filter(|row| super::subjects::active(row, frame.hour))
                .map(|row| {
                    Some((
                        (
                            tags.get(&row.overlord)?.to_string(),
                            tags.get(&row.subject)?.to_string(),
                        ),
                        (
                            row.start_hour,
                            std::str::from_utf8(&row.state).ok()?.to_owned(),
                        ),
                    ))
                })
                .collect()
        });
    let mut subject_targets: HashMap<String, Vec<String>> = HashMap::new();
    if let Some(subjects) = &subjects {
        for (overlord, subject) in subjects.keys() {
            subject_targets
                .entry(overlord.clone())
                .or_default()
                .push(subject.clone());
        }
    }
    let governments = government_snapshot(frame, tags);
    Some(Snapshot {
        hour: frame.hour,
        roster_complete: frame.complete::<tables::PeaceConferences>().is_ok(),
        countries,
        owners,
        states_by_owner,
        unresolved_owners,
        wars,
        unresolved_wars,
        vps,
        subjects,
        subject_targets,
        governments,
        exists,
        state_action_key,
    })
}

fn existence_snapshot(
    frame: &DecodedFrame,
    tags: &HashMap<u32, Tag>,
) -> Option<HashMap<Tag, bool>> {
    frame
        .complete::<tables::CountryCounterInputs>()
        .ok()?
        .iter()
        .map(|row| Some((tags.get(&row.country)?.clone(), row.owned_state_count > 0)))
        .collect()
}

/// The state-action keyword selects the semantic binding; the recording
/// supplies its spelling, and content supplies its display name.
/// See engine::peace_conference.
fn state_action_key(frame: &DecodedFrame) -> Result<String, String> {
    let rows = frame
        .complete::<tables::LexicalTokens>()
        .map_err(|error| format!("LexicalTokens: {error}"))?;
    let mut found = rows.iter().filter(|row| {
        installed_content::parser::native::Token {
            kind: installed_content::parser::native::Kind::Bare,
            text: row.literal.clone(),
            line: 0,
            quoted: false,
            byte_end: 0,
        }
        .is_keyword("take_states")
    });
    let row = found
        .next()
        .ok_or("LexicalTokens does not record the native state-taking keyword")?;
    if found.next().is_some() {
        return Err("LexicalTokens records conflicting state-taking keywords".into());
    }
    std::str::from_utf8(&row.literal)
        .map(str::to_owned)
        .map_err(|error| format!("State-taking keyword is not UTF-8: {error}"))
}

fn name_actions(
    reply: &mut PeaceDealsReply,
    localisation: &installed_content::localisation::Localisation,
    recorded_language: &str,
) {
    let mut names: HashMap<String, Option<String>> = HashMap::new();
    let mut refused = catalogue::Refusals::default();
    for deal in &mut reply.deals {
        if let Some(captured) = &mut deal.captured {
            for action in &mut captured.actions {
                if let Some(bytes) = action.cached_name.as_ref() {
                    action.name = crate::scripted_text::stored_text(
                        localisation,
                        recorded_language,
                        bytes,
                        "cached peace-action type name",
                    )
                    .and_then(|text| catalogue::plain_text(&text))
                    .inspect_err(|reason| refused.note(reason.clone(), action.token.to_string()))
                    .ok();
                } else if let Some(key) = action.name_key.as_ref() {
                    action.name = names
                        .entry(key.clone())
                        .or_insert_with(|| {
                            if localisation.get(key).is_none() {
                                refused
                                    .note("localisation key is not in the recorded content", key);
                                return None;
                            }
                            catalogue::localised(localisation, key, catalogue::View::Scripted)
                                .inspect_err(|reason| refused.note(reason.clone(), key))
                                .ok()
                        })
                        .clone();
                } else {
                    reply.warnings.push(format!("Captured peace-action token {} has no proven native naming operand; its name is not recorded.", action.token));
                }
            }
        }
        for item in &mut deal.items {
            let Some(key) = item.action.key.as_ref() else {
                continue;
            };
            item.action.name = names
                .entry(key.clone())
                .or_insert_with(|| {
                    if localisation.get(key).is_none() {
                        refused.note("localisation key is not in the recorded content", key);
                        return None;
                    }
                    catalogue::localised(localisation, key, catalogue::View::Scripted)
                        .inspect_err(|reason| refused.note(reason.clone(), key))
                        .ok()
                })
                .clone();
        }
    }
    reply.warnings.extend(refused.lines("peace action names"));
}

fn government_snapshot(
    frame: &DecodedFrame,
    tags: &HashMap<u32, Tag>,
) -> Option<HashMap<Tag, PeaceGovernment>> {
    let politics = frame.complete::<tables::Politics>().ok()?;
    let parties = frame.complete::<tables::PoliticalParties>().ok()?;
    let appearances = frame.complete::<tables::CountryAppearance>().ok()?;
    let tokens = parties
        .iter()
        .map(|row| {
            Some((
                (tags.get(&row.country)?.clone(), row.id),
                std::str::from_utf8(&row.token).ok()?,
            ))
        })
        .collect::<Option<HashMap<_, _>>>()?;
    let ideologies = appearances
        .iter()
        .map(|row| {
            Some((
                tags.get(&row.country)?.clone(),
                std::str::from_utf8(&row.leader_ideology).ok()?,
            ))
        })
        .collect::<Option<HashMap<_, _>>>()?;
    politics
        .iter()
        .map(|row| {
            let tag = tags.get(&row.country)?;
            Some((
                tag.clone(),
                PeaceGovernment {
                    ruling_party: row.ruling_party,
                    party_token: row
                        .ruling_party
                        .and_then(|id| tokens.get(&(tag.clone(), id)).copied())
                        .map(str::to_owned),
                    leader_ideology: ideologies.get(tag).copied().map(str::to_owned),
                },
            ))
        })
        .collect()
}

fn mark(spans: &mut Vec<HourSpan>, first_hour: i32, last_hour: i32) {
    if first_hour > last_hour {
        return;
    }
    if let Some(last) = spans.last_mut()
        && last
            .last_hour
            .checked_add(1)
            .is_some_and(|next| next >= first_hour)
    {
        last.last_hour = last.last_hour.max(last_hour);
    } else {
        spans.push(HourSpan {
            first_hour,
            last_hour,
        });
    }
}

impl PeaceUnobserved {
    fn all(&mut self, first: i32, last: i32) {
        for spans in [
            &mut self.conferences,
            &mut self.ownership,
            &mut self.wars,
            &mut self.subjects,
            &mut self.governments,
            &mut self.existence,
            &mut self.victory_points,
        ] {
            mark(spans, first, last);
        }
    }
}

fn card(id: String, observation: PeaceObservation, hour: i32) -> PeaceDeal {
    PeaceDeal {
        id, derived: true, observation, conference_id: None, war_name: None,
        first_observed_hour: hour, last_observed_hour: hour, current_turn: None,
        finalized: None, first_finalized_hour: None, end_hour: None, end_is_by: false,
        first_absent_hour: None, participants_derived: true, winners: Vec::new(),
        losers: Vec::new(), items: Vec::new(), captured: None, capture_unavailable: None, capture_observed_hour: None, capture_unavailable_hour: None, transfers: PeaceTransferGraph::default(),
        limitations: vec!["Conference participants and war name are not recorded; sides are inferred only from qualifying transfers.".into()],
    }
}

/// Only these tables are decoded; nothing reads live game state or changes format.
pub fn scan_branch(
    replay: &Replay,
    branch: u64,
    cancelled: &AtomicBool,
) -> io::Result<PeaceDealsReply> {
    let start = replay
        .branches
        .iter()
        .find(|start| start.id == branch)
        .ok_or_else(|| crate::invalid("Peace-deals branch is not in this recording"))?;
    let (first, last) = branch_bounds(replay, start);
    let selected = [
        TableId::Campaign,
        TableId::Countries,
        TableId::CountryLinks,
        TableId::States,
        TableId::Provinces,
        TableId::WarRelations,
        TableId::PeaceConferences,
        TableId::PeaceSettlements,
        TableId::Subjects,
        TableId::Politics,
        TableId::PoliticalParties,
        TableId::CountryAppearance,
        TableId::CountryCounterInputs,
        TableId::LexicalTokens,
    ];
    // Refuse a future cadence change instead of accidentally dating held rows as new reads.
    for table in selected.into_iter().filter(|table| {
        !matches!(
            table,
            TableId::Campaign | TableId::PeaceConferences | TableId::LexicalTokens
        )
    }) {
        if replay::cadence::cadence(table) != replay::cadence::Cadence::Hourly {
            return Err(crate::invalid(format!(
                "Peace deals does not support the {:?} cadence of {table:?}",
                replay::cadence::cadence(table)
            )));
        }
    }
    let mut reply = PeaceDealsReply { branch_id: branch.to_string(), derived: true, deals: Vec::new(), transfers: PeaceTransferGraph::default(), unobserved: PeaceUnobserved::default(), warnings: vec!["Derived net outcomes do not recover bids, negotiators, action order or causation. War names and conference participants are not recorded.".into()] };
    let mut open: HashMap<u64, OpenConference> = HashMap::new();
    let mut pending: HashMap<usize, Option<Arc<Snapshot>>> = HashMap::new();
    let mut processed: HashSet<usize> = HashSet::new();
    let mut unassigned: HashSet<usize> = HashSet::new();
    let mut has_active = false;
    let mut has_ambiguous = false;
    let mut previous: Option<Arc<Snapshot>> = None;
    let mut previous_hour = first.checked_sub(1);
    let mut first_existing: HashMap<Tag, i32> = HashMap::new();
    let mut graph = GraphBuilder::default();
    let mut exact = exact::Observed::default();
    let mut lexical_view: Option<ViewIdentity> = None;
    let mut action_key = None;
    let mut action_refusals = catalogue::Refusals::default();
    let mut country_tags = CountryResolution::default();
    let mut victory_points = VictoryPoints::default();
    crate::blocks::scan_blocks(
        replay,
        replay.index.iter().filter(|block| block.branch == branch),
        &selected,
        cancelled,
        |blocks| {
            for (block, frames) in blocks {
                check_cancel(cancelled)?;
                let mut decoder = FrameDecoder::for_tables(&selected);
                let frames = match frames {
                    Ok(frames) => frames,
                    Err(error) => {
                        reply.warnings.push(format!(
                            "Peace deals: unreadable block {}: {error}",
                            block.first_hour
                        ));
                        continue;
                    }
                };
                for frame in frames {
                    check_cancel(cancelled)?;
                    let decoded = match decoder.advance(&frame).and_then(|decoded| {
                        validate_campaign(decoded, replay)?;
                        Ok(decoded)
                    }) {
                        Ok(decoded) => decoded,
                        Err(error) => {
                            reply.warnings.push(format!(
                                "Peace deals: stopped block at {}: {error}",
                                frame.hour
                            ));
                            break;
                        }
                    };
                    let hour = decoded.hour;
                    if let Some(prev) = previous_hour
                        && prev.checked_add(1).is_some_and(|next| next < hour)
                    {
                        reply.unobserved.all(prev + 1, hour - 1);
                    }
                    previous_hour = Some(hour);
                    // A weak identity: holding the view would copy it at its next delta.
                    let view = &decoded.tables[TableId::LexicalTokens.index()];
                    if lexical_view.as_ref().is_none_or(|prior| !prior.is(view)) {
                        action_key = state_action_key(decoded)
                            .inspect_err(|reason| {
                                action_refusals.note(reason.clone(), "state transfer")
                            })
                            .ok();
                        lexical_view = Some(view.identity());
                    }
                    let countries = country_tags.read(decoded);
                    let vps = victory_points.read(decoded);
                    let exists = countries
                        .as_ref()
                        .and_then(|countries| existence_snapshot(decoded, &countries.tags))
                        .map(Arc::new);
                    let current = countries
                        .as_ref()
                        .and_then(|countries| {
                            snapshot(
                                decoded,
                                Arc::clone(countries),
                                vps.clone(),
                                exists.clone(),
                                action_key.clone(),
                            )
                        })
                        .map(Arc::new);
                    exact.observe(
                        decoded,
                        countries.as_deref(),
                        vps.as_deref(),
                        action_key.as_deref(),
                    );
                    let had_open = !open.is_empty();
                    let had_active = has_active;
                    let roster_read = hour == first
                        || slot_read(&decoded.metrics, TableId::PeaceConferences, hour);
                    let roster = decoded.complete::<tables::PeaceConferences>();
                    let roster_complete = roster.is_ok();
                    if roster_read && let Ok(rows) = roster {
                        let active: Vec<_> = open
                            .values()
                            .filter(|entry| !processed.contains(&entry.index))
                            .map(|entry| entry.index)
                            .collect();
                        if active.len() > 1 {
                            unassigned.extend(active);
                        }
                        let present: HashSet<_> = rows.iter().map(|row| row.id).collect();
                        open.retain(|id, conference| {
                            if present.contains(id) {
                                return true;
                            }
                            let deal = &mut reply.deals[conference.index];
                            deal.first_absent_hour = Some(hour);
                            deal.end_is_by = deal.last_observed_hour.checked_add(1) != Some(hour);
                            deal.end_hour = Some(if deal.end_is_by {
                                hour
                            } else {
                                deal.last_observed_hour
                            });
                            if !processed.contains(&conference.index) {
                                pending.insert(conference.index, conference.before.clone());
                            }
                            false
                        });
                        for row in rows {
                            let conference = open.entry(row.id).or_insert_with(|| {
                                let mut deal = card(
                                    format!("conference:{}:{}", row.id, hour),
                                    PeaceObservation::RecordedConference,
                                    hour,
                                );
                                deal.conference_id = Some(row.id.to_string());
                                reply.deals.push(deal);
                                OpenConference {
                                    index: reply.deals.len() - 1,
                                    before: previous.clone(),
                                }
                            });
                            let index = conference.index;
                            let deal = &mut reply.deals[index];
                            deal.last_observed_hour = hour;
                            deal.current_turn = Some(row.current_turn);
                            deal.finalized = Some(row.finalized != 0);
                            if row.finalized != 0 && deal.first_finalized_hour.is_none() {
                                deal.first_finalized_hour = Some(hour);
                                deal.end_hour = Some(hour);
                                deal.end_is_by = true;
                                if !processed.contains(&index) {
                                    pending.insert(index, conference.before.clone());
                                }
                            }
                            if row.finalized == 0 && current.is_some() {
                                conference.before = current.clone();
                            }
                        }
                        let comparisons: HashSet<_> = open
                            .values()
                            .filter(|entry| !processed.contains(&entry.index))
                            .map(|entry| entry.index)
                            .chain(pending.keys().copied())
                            .collect();
                        if comparisons.len() > 1 {
                            unassigned.extend(comparisons);
                        }
                        has_active = open.values().any(|entry| !processed.contains(&entry.index));
                        has_ambiguous = open.values().any(|entry| {
                            !processed.contains(&entry.index) && unassigned.contains(&entry.index)
                        });
                    } else {
                        mark(&mut reply.unobserved.conferences, hour, hour);
                    }
                    if current
                        .as_ref()
                        .is_none_or(|s| !s.unresolved_owners.is_empty())
                    {
                        mark(&mut reply.unobserved.ownership, hour, hour);
                    }
                    if current
                        .as_ref()
                        .is_none_or(|s| !s.unresolved_wars.is_empty())
                    {
                        mark(&mut reply.unobserved.wars, hour, hour);
                    }
                    if decoded.complete::<tables::Provinces>().is_err() {
                        mark(&mut reply.unobserved.victory_points, hour, hour);
                    }
                    if decoded.complete::<tables::Subjects>().is_err()
                        || current.as_ref().is_none_or(|s| s.subjects.is_none())
                    {
                        mark(&mut reply.unobserved.subjects, hour, hour);
                    }
                    if current.as_ref().is_none_or(|s| s.governments.is_none()) {
                        mark(&mut reply.unobserved.governments, hour, hour);
                    }
                    if exists.is_none() {
                        mark(&mut reply.unobserved.existence, hour, hour);
                    }
                    if let Some(current) = current {
                        let candidates: Vec<_> = pending.keys().copied().collect();
                        let assigned =
                            candidates.len() == 1 && !unassigned.contains(&candidates[0]);
                        if assigned {
                            let index = candidates[0];
                            if let Some(before) = &pending[&index] {
                                let transfers = transfers(before, &current, true);
                                populate(
                                    &mut reply.deals[index],
                                    before,
                                    &current,
                                    transfers,
                                    &first_existing,
                                );
                                for item in &reply.deals[index].items {
                                    graph.add(item);
                                }
                            } else {
                                reply.deals[index].limitations.push("No complete ownership and war read before settlement; outcomes are not recorded.".into());
                            }
                        }
                        if let Some(before) = previous.as_ref() {
                            let consecutive = before.hour.checked_add(1) == Some(hour);
                            // A single resident conference reserves its net change for its
                            // next actual roster read. Several cannot be distinguished.
                            let unmatched = !assigned
                                && (!had_active && !has_active
                                    || has_ambiguous
                                    || candidates.iter().any(|index| unassigned.contains(index)));
                            let transfers = if unmatched && consecutive {
                                transfers(before, &current, false)
                            } else {
                                Vec::new()
                            };
                            if !transfers.is_empty() {
                                let observation = if !had_open
                                    && open.is_empty()
                                    && roster_complete
                                    && before.roster_complete
                                {
                                    PeaceObservation::ConferenceNotRecorded
                                } else {
                                    PeaceObservation::ConferenceUnknown
                                };
                                for (group, transfers) in
                                    components(transfers).into_iter().enumerate()
                                {
                                    let mut deal =
                                        card(format!("outcome:{hour}:{group}"), observation, hour);
                                    deal.end_hour = Some(hour);
                                    deal.end_is_by = true;
                                    populate(
                                        &mut deal,
                                        before,
                                        &current,
                                        transfers,
                                        &first_existing,
                                    );
                                    deal.limitations.push("Connected war-end transfer pairs group this outcome; the data cannot prove how many conferences produced it.".into());
                                    for item in &deal.items {
                                        graph.add(item);
                                    }
                                    reply.deals.push(deal);
                                }
                            }
                        }
                        for &index in &candidates {
                            if unassigned.contains(&index) {
                                reply.deals[index].limitations.push("Sampled conferences overlap this comparison; outcomes cannot be assigned to one.".into());
                            }
                        }
                        for (index, _) in pending.drain() {
                            processed.insert(index);
                        }
                        if !candidates.is_empty() {
                            has_active =
                                open.values().any(|entry| !processed.contains(&entry.index));
                            has_ambiguous = open.values().any(|entry| {
                                !processed.contains(&entry.index)
                                    && unassigned.contains(&entry.index)
                            });
                        }
                        previous = Some(current);
                    }
                    if let Some(exists) = &exists {
                        for (tag, exists) in exists.iter() {
                            if *exists {
                                first_existing.entry(tag.clone()).or_insert(hour);
                            }
                        }
                    }
                }
            }
            Ok(())
        },
    )?;
    if let Some(prev) = previous_hour {
        if let Some(next) = prev.checked_add(1) {
            reply.unobserved.all(next, last);
        }
    } else {
        reply.unobserved.all(first, last);
    }
    for (index, _) in pending {
        reply.deals[index].limitations.push(
            "No complete ownership and war read after settlement; outcomes are not recorded."
                .into(),
        );
    }
    for conference in open.values() {
        if !processed.contains(&conference.index) {
            reply.deals[conference.index]
                .limitations
                .push("No end was observed; the conference outcome is not recorded.".into());
        }
    }
    reply.transfers = graph.finish();
    exact.apply(&mut reply);
    reply
        .warnings
        .extend(action_refusals.lines("peace action bindings"));
    reply.deals.sort_by(|a, b| {
        b.end_hour
            .unwrap_or(b.last_observed_hour)
            .cmp(&a.end_hour.unwrap_or(a.last_observed_hour))
            .then_with(|| a.id.cmp(&b.id))
    });
    Ok(reply)
}

fn check_cancel(cancelled: &AtomicBool) -> io::Result<()> {
    if cancelled.load(Ordering::Acquire) {
        return Err(io::Error::new(
            io::ErrorKind::Interrupted,
            "Peace-deals request cancelled",
        ));
    }
    Ok(())
}

fn transfers(before: &Snapshot, after: &Snapshot, recorded: bool) -> Vec<PeaceItem> {
    let mut result = Vec::new();
    let mut unresolved_pairs = HashSet::new();
    let mut unresolved_countries = HashSet::new();
    let mut unidentified_pair = false;
    for (first, second) in &after.unresolved_wars {
        match (
            after.countries.tags.get(first),
            after.countries.tags.get(second),
        ) {
            (Some(first), Some(second)) => {
                unresolved_pairs.insert(pair(first, second));
            }
            (Some(country), None) | (None, Some(country)) => {
                unresolved_countries.insert(country);
            }
            (None, None) => unidentified_pair = true,
        }
    }
    // Built at the first owner change; most compared hours have none.
    let mut changed_tags: Option<HashSet<&Tag>> = None;
    for (&state, from) in &before.owners {
        let Some(to) = after.owners.get(&state) else {
            continue;
        };
        if from == to {
            continue;
        }
        // Tag changes of a raw country must never masquerade as a territorial deal.
        let changed_tags = changed_tags.get_or_insert_with(|| {
            before
                .countries
                .tags
                .iter()
                .filter(|(raw, tag)| after.countries.tags.get(raw).is_some_and(|now| now != *tag))
                .flat_map(|(raw, tag)| [tag, &after.countries.tags[raw]])
                .collect()
        });
        if changed_tags.contains(from) || changed_tags.contains(to) {
            continue;
        }
        if [from, to].into_iter().any(|tag| {
            !before.countries.identities.contains_key(tag)
                || before.countries.identities.get(tag) != after.countries.identities.get(tag)
        }) {
            continue;
        }
        let key = pair(from, to);
        let Some(&(start, end)) = before.wars.get(&key) else {
            continue;
        };
        if start > before.hour || !war_ongoing(start, end, before.hour) {
            continue;
        }
        match after.wars.get(&key) {
            Some(&(new_start, new_end)) => {
                if new_start != start
                    || new_end <= start
                    || new_end <= before.hour
                    || new_end > after.hour
                    || (!recorded && new_end != after.hour)
                {
                    continue;
                }
            }
            None => {
                // Peace removes both endpoint pointers and destroys the owned relation.
                // An unresolved/conflicting row is not evidence of its removal.
                if unidentified_pair
                    || unresolved_pairs.contains(&key)
                    || unresolved_countries.contains(from)
                    || unresolved_countries.contains(to)
                {
                    continue;
                }
            }
        }
        let vp = after.vps.as_ref().and_then(|vps| vps.get(&state)).copied();
        result.push(PeaceItem {
            derived: true,
            dates: PeaceDates {
                before_hour: before.hour,
                after_hour: after.hour,
            },
            winner: Some(to.to_string()),
            action: PeaceActionLabel {
                kind: PeaceActionKind::StateTransfer,
                key: after.state_action_key.clone(),
                name: None,
            },
            change: PeaceChange::StateTransfer {
                state,
                from: from.to_string(),
                to: to.to_string(),
                from_country: PeaceTransferCountry {
                    slot: before.countries.identities[from].0,
                    country_id: before.countries.identities[from].1,
                    tag: from.to_string(),
                    hour: before.hour,
                },
                to_country: PeaceTransferCountry {
                    slot: after.countries.identities[to].0,
                    country_id: after.countries.identities[to].1,
                    tag: to.to_string(),
                    hour: after.hour,
                },
                victory_points: vp.map(|vp| vp.to_string()),
                victory_points_read_hour: vp.map(|_| after.hour),
            },
        });
    }
    result.sort_by_key(|item| match item.change {
        PeaceChange::StateTransfer { state, .. } => state,
        _ => 0,
    });
    result
}

/// Connected components in linear work; no country cross-product or pair census.
fn components(items: Vec<PeaceItem>) -> Vec<Vec<PeaceItem>> {
    let mut adjacency: HashMap<String, Vec<String>> = HashMap::new();
    for item in &items {
        if let PeaceChange::StateTransfer { from, to, .. } = &item.change {
            adjacency.entry(from.clone()).or_default().push(to.clone());
            adjacency.entry(to.clone()).or_default().push(from.clone());
        }
    }
    let mut group = HashMap::new();
    let mut count = 0;
    let mut roots: Vec<_> = adjacency.keys().cloned().collect();
    roots.sort();
    for root in roots {
        if group.contains_key(&root) {
            continue;
        }
        let mut queue = VecDeque::from([root.clone()]);
        group.insert(root, count);
        while let Some(country) = queue.pop_front() {
            for other in &adjacency[&country] {
                if !group.contains_key(other) {
                    group.insert(other.clone(), count);
                    queue.push_back(other.clone());
                }
            }
        }
        count += 1;
    }
    let mut groups = vec![Vec::new(); count];
    for item in items {
        if let PeaceChange::StateTransfer { from, .. } = &item.change {
            groups[group[from]].push(item);
        }
    }
    groups
}

/// A released country's losers, and each state it received with its former owner.
type ReleaseSources = (HashSet<String>, Vec<(u32, String)>);

fn populate(
    deal: &mut PeaceDeal,
    before: &Snapshot,
    after: &Snapshot,
    mut items: Vec<PeaceItem>,
    first_existing: &HashMap<Tag, i32>,
) {
    for sample in [before, after] {
        if !sample.unresolved_owners.is_empty() {
            deal.limitations.push(format!("State owners are unresolved at hour {} (state, raw owner): {:?}; the outcome list is incomplete.", sample.hour, sample.unresolved_owners));
        }
        if !sample.unresolved_wars.is_empty() {
            deal.limitations.push(format!("WarRelations pairs are unresolved or conflicting at hour {} (raw countries): {:?}; those pairs cannot attribute transfers.", sample.hour, sample.unresolved_wars));
        }
    }
    let mut winners = HashSet::new();
    let mut losers = HashSet::new();
    let mut removed_war = false;
    for item in &items {
        if let PeaceChange::StateTransfer { from, to, .. } = &item.change {
            winners.insert(to.clone());
            losers.insert(from.clone());
            removed_war |= !after.wars.contains_key(&pair(from, to));
        }
    }
    if removed_war {
        if !deal.end_is_by {
            deal.end_hour = Some(after.hour);
            deal.end_is_by = true;
        }
        deal.limitations.push("The ongoing war pair is absent from the complete after read; its end and the net ownership change are bounded by the two reads, not an exact settlement time.".into());
    }
    let dates = PeaceDates {
        before_hour: before.hour,
        after_hour: after.hour,
    };
    let mut releases = HashSet::new();
    if let (Some(old), Some(new)) = (&before.exists, &after.exists) {
        // Each state keeps its own former owner: one release can draw on several losers.
        let mut released: HashMap<String, ReleaseSources> = HashMap::new();
        for from in &losers {
            for &state in before
                .states_by_owner
                .get(from.as_str())
                .into_iter()
                .flatten()
            {
                let Some(country) = after.owners.get(&state) else {
                    continue;
                };
                if !losers.contains(from)
                    || **country == **from
                    || old.get(country) != Some(&false)
                    || new.get(country) != Some(&true)
                    || !before.countries.identities.contains_key(country)
                    || before.countries.identities.get(country)
                        != after.countries.identities.get(country)
                    || first_existing
                        .get(country)
                        .is_some_and(|first| *first <= before.hour)
                {
                    continue;
                }
                let entry = released.entry(country.to_string()).or_default();
                entry.0.insert(from.clone());
                entry.1.push((state, from.clone()));
            }
        }
        for (country, (from, mut owned)) in released {
            releases.insert(country.clone());
            let mut from: Vec<_> = from.into_iter().collect();
            from.sort();
            owned.sort_unstable();
            // Both holders were checked above: the same native identity at both reads.
            let holder = |snapshot: &Snapshot, tag: &str| {
                let (slot, country_id) = snapshot.countries.identities[tag];
                PeaceTransferCountry {
                    slot,
                    country_id,
                    tag: tag.to_owned(),
                    hour: snapshot.hour,
                }
            };
            let transfers = owned
                .iter()
                .map(|(state, owner)| {
                    let vp = after.vps.as_ref().and_then(|vps| vps.get(state)).copied();
                    PeaceReleasedState {
                        state: *state,
                        from_country: holder(before, owner.as_str()),
                        victory_points: vp.map(|vp| vp.to_string()),
                        victory_points_read_hour: vp.map(|_| after.hour),
                    }
                })
                .collect();
            items.push(PeaceItem {
                derived: true,
                dates: dates.clone(),
                winner: None,
                action: neutral(PeaceActionKind::CountryReleased),
                change: PeaceChange::CountryReleased {
                    to_country: holder(after, country.as_str()),
                    states: owned.iter().map(|(state, _)| *state).collect(),
                    country,
                    from,
                    transfers,
                },
            });
        }
    } else {
        deal.limitations.push(
            "Country existence is not recorded at both reads; releases cannot be derived.".into(),
        );
    }
    if let (Some(old), Some(new)) = (&before.subjects, &after.subjects) {
        for overlord in &winners {
            for subject in after.subject_targets.get(overlord).into_iter().flatten() {
                let (start, state) = &new[&(overlord.clone(), subject.clone())];
                if old.contains_key(&(overlord.clone(), subject.clone()))
                    || !winners.contains(overlord)
                    || !(losers.contains(subject) || releases.contains(subject))
                    || *start <= before.hour
                    || *start > after.hour
                {
                    continue;
                }
                items.push(PeaceItem {
                    derived: true,
                    dates: dates.clone(),
                    winner: Some(overlord.clone()),
                    action: neutral(PeaceActionKind::NewSubject),
                    change: PeaceChange::NewSubject {
                        overlord: overlord.clone(),
                        subject: subject.clone(),
                        autonomy_state: state.clone(),
                        start_hour: *start,
                    },
                });
            }
        }
    } else {
        deal.limitations
            .push("Subjects is not recorded at both reads; new subjects cannot be derived.".into());
    }
    if let (Some(old), Some(new)) = (&before.governments, &after.governments) {
        for country in &losers {
            let (Some(old), Some(new)) = (old.get(country.as_str()), new.get(country.as_str()))
            else {
                deal.limitations.push(format!(
                    "Government of {country} is not recorded at both reads."
                ));
                continue;
            };
            let party_changed =
                matches!((old.ruling_party, new.ruling_party), (Some(a), Some(b)) if a != b);
            let ideology_changed = matches!((&old.leader_ideology, &new.leader_ideology), (Some(a), Some(b)) if !a.is_empty() && !b.is_empty() && a != b);
            let group_changed =
                matches!((&old.party_token, &new.party_token), (Some(a), Some(b)) if a != b);
            if !party_changed && !ideology_changed && !group_changed {
                continue;
            }
            items.push(PeaceItem {
                derived: true,
                dates: dates.clone(),
                winner: None,
                action: neutral(PeaceActionKind::GovernmentChange),
                change: PeaceChange::GovernmentChange {
                    country: country.clone(),
                    before: old.clone(),
                    after: new.clone(),
                },
            });
        }
    } else {
        deal.limitations.push("Government tables are not recorded at both reads; government changes cannot be derived.".into());
    }
    if items.is_empty() {
        deal.limitations.push("No attributable transfers were observed; an empty list does not prove a white peace or no settlement actions.".into());
    }
    if winners.is_empty() {
        deal.limitations.push("Without qualifying transfers, participant sides and subject/government-only settlements cannot be attributed.".into());
    }
    if before.hour.checked_add(1) != Some(after.hour) {
        deal.limitations.push("The outcome reads span several hours; only their net difference is known, not individual action times.".into());
    }
    deal.winners = winners.into_iter().collect();
    deal.winners.sort();
    deal.losers = losers.into_iter().collect();
    deal.losers.sort();
    items.sort_by(|a, b| item_order(a).cmp(&item_order(b)));
    let mut graph = GraphBuilder::default();
    for item in &items {
        graph.add(item);
    }
    deal.transfers = graph.finish();
    deal.items = items;
}

fn item_order(item: &PeaceItem) -> (u8, &str, u32) {
    match &item.change {
        PeaceChange::StateTransfer { state, to, .. } => (0, to, *state),
        PeaceChange::NewSubject { subject, .. } => (1, subject, 0),
        PeaceChange::GovernmentChange { country, .. } => (2, country, 0),
        PeaceChange::CountryReleased { country, .. } => (3, country, 0),
    }
}

/// A country's dated native identity: canonical slot and native country id.
type CountryKey = (u32, u32);

#[derive(Default)]
struct GraphBuilder {
    links: HashMap<(CountryKey, CountryKey), Vec<PeaceTransferredState>>,
}

impl GraphBuilder {
    /// Plain transfers, and each state a released country received.
    fn add(&mut self, item: &PeaceItem) {
        self.add_with(item, false, &|_| true);
    }
    /// `released` marks a plain transfer whose taker the same settlement released;
    /// `keep` selects which of a release's states this graph still draws.
    fn add_with(&mut self, item: &PeaceItem, released: bool, keep: &dyn Fn(u32) -> bool) {
        match &item.change {
            PeaceChange::StateTransfer {
                state,
                from_country,
                to_country,
                victory_points,
                victory_points_read_hour,
                ..
            } => self.push(PeaceTransferredState {
                state: *state,
                from_country: from_country.clone(),
                to_country: to_country.clone(),
                dates: item.dates.clone(),
                victory_points: victory_points.clone(),
                victory_points_read_hour: *victory_points_read_hour,
                released,
            }),
            PeaceChange::CountryReleased {
                to_country,
                transfers,
                ..
            } => {
                for transfer in transfers.iter().filter(|transfer| keep(transfer.state)) {
                    self.push(PeaceTransferredState {
                        state: transfer.state,
                        from_country: transfer.from_country.clone(),
                        to_country: to_country.clone(),
                        dates: item.dates.clone(),
                        victory_points: transfer.victory_points.clone(),
                        victory_points_read_hour: transfer.victory_points_read_hour,
                        released: true,
                    });
                }
            }
            PeaceChange::NewSubject { .. } | PeaceChange::GovernmentChange { .. } => {}
        }
    }
    fn push(&mut self, state: PeaceTransferredState) {
        self.links
            .entry((state.from_country.identity(), state.to_country.identity()))
            .or_default()
            .push(state);
    }
    fn finish(self) -> PeaceTransferGraph {
        let mut graph = PeaceTransferGraph::default();
        let mut nodes = HashMap::<String, PeaceTransferNode>::new();
        let mut links: Vec<_> = self.links.into_iter().collect();
        links.sort_by_key(|link| link.0);
        for ((from, to), states) in links {
            let source = format!("source:{}:{}", from.0, from.1);
            let target = format!("target:{}:{}", to.0, to.1);
            for state in &states {
                for (id, country, side) in [
                    (&source, &state.from_country, PeaceTransferSide::Source),
                    (&target, &state.to_country, PeaceTransferSide::Target),
                ] {
                    let node = nodes
                        .entry(id.clone())
                        .or_insert_with(|| PeaceTransferNode {
                            id: id.clone(),
                            country: country.clone(),
                            side,
                        });
                    if country.hour > node.country.hour {
                        node.country = country.clone();
                    }
                }
            }
            let vp = states.iter().try_fold(0_i64, |total, state| {
                total.checked_add(state.victory_points.as_ref()?.parse::<i64>().ok()?)
            });
            graph.links.push(PeaceTransferLink {
                id: format!("{}:{source}{target}", source.len()),
                source,
                target,
                state_count: states.len() as u32,
                victory_points: vp.map(|vp| vp.to_string()),
                states,
            });
        }
        graph.nodes = nodes.into_values().collect();
        graph.nodes.sort_by(|a, b| a.id.cmp(&b.id));
        graph
    }
}

#[cfg(test)]
mod tests;
