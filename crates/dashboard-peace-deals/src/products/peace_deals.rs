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

use dashboard_base::prepared::frame_changes::FrameChanges;
use dashboard_base::{Source, catalogue, content};
use dashboard_base::{
    products::LoadContext,
    products::MAX_PRODUCT_REPLY_BYTES,
    products::Product,
    products::country_groups::{HourSpan, war_ongoing},
};
use dashboard_text::metadata::{CountryKeys, branch_bounds, country_tables, slot_read};
use replay::{
    container::Replay,
    frame::{DecodedFrame, ViewIdentity},
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

#[cfg(test)]
use dashboard_text::metadata::validate_campaign;
#[cfg(test)]
use replay::frame::FrameDecoder;

pub struct PeaceDeals;
mod exact;
pub mod prepared;
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
    /// What still needs later observations, in a reply answered from a
    /// prepared prefix: `outcome` while the deal's conference is open or its
    /// settlement unread at the prefix's last hour. Never set once
    /// preparation has reached the branch's end.
    #[serde(default)]
    pub preparing: Vec<String>,
}

#[derive(Clone, Debug, Default, Deserialize, Serialize, TS)]
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

#[derive(Clone, Debug, Deserialize, Serialize, TS)]
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

#[cfg(test)]
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

    #[cfg(test)]
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
        present(source, branch, context, cancelled, scan)
    }
}

fn present(
    source: &Source,
    branch: u64,
    context: &LoadContext,
    cancelled: &AtomicBool,
    scan: Arc<PeaceDealsReply>,
) -> io::Result<PeaceDealsReply> {
    let Some(options) = context.content.as_ref() else {
        let mut reply = (*scan).clone();
        reply.warnings.push(
            "Peace action names are unavailable until the game and user folders are chosen.".into(),
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

fn admitted(reply: PeaceDealsReply) -> io::Result<PeaceDealsReply> {
    if dashboard_core::transport::json_len(&reply, MAX_PRODUCT_REPLY_BYTES).is_none() {
        return Err(dashboard_core::invalid(format!(
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
#[derive(Serialize, Deserialize)]
struct CountryTags {
    #[serde(serialize_with = "dashboard_core::map_pairs::ordered_map")]
    tags: HashMap<u32, Tag>,
    #[serde(serialize_with = "dashboard_core::map_pairs::ordered_map")]
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
    links: Option<ViewIdentity>,
    ready: bool,
    resolved: Option<Arc<CountryTags>>,
}

impl CountryResolution {
    fn read_changed(
        &mut self,
        frame: &DecodedFrame,
        changes: &FrameChanges,
    ) -> Option<Arc<CountryTags>> {
        if self.ready
            && !changes.rows_changed[TableId::Countries.index()]
            && !changes.rows_changed[TableId::CountryLinks.index()]
        {
            return self.resolved.clone();
        }
        let Ok((countries, links)) = country_tables(frame) else {
            self.ready = true;
            self.resolved = None;
            self.countries = CountryKeys::default();
            self.links = None;
            return None;
        };
        if !self.ready
            || !self.countries.same(countries)
            || self
                .links
                .as_ref()
                .is_none_or(|view| !view.is(&frame.tables[TableId::CountryLinks.index()]))
        {
            self.resolved = CountryTags::resolve(countries, links).map(Arc::new);
            self.countries = CountryKeys::read(countries);
            self.links = Some(frame.tables[TableId::CountryLinks.index()].identity());
        }
        self.ready = true;
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
    fn read_changed(
        &mut self,
        frame: &DecodedFrame,
        changes: &FrameChanges,
    ) -> Option<Arc<HashMap<u32, i64>>> {
        if !changes.rows_changed[TableId::Provinces.index()] {
            return self.totals.clone();
        }
        let Ok(rows) = frame.complete::<tables::Provinces>() else {
            self.totals = None;
            return None;
        };
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

#[derive(Clone, Serialize, Deserialize)]
struct Snapshot {
    hour: i32,
    roster_complete: bool,
    countries: Arc<CountryTags>,
    #[serde(serialize_with = "dashboard_core::map_pairs::ordered_map")]
    owners: Arc<HashMap<u32, Tag>>,
    #[serde(serialize_with = "dashboard_core::map_pairs::ordered_map")]
    states_by_owner: Arc<HashMap<Tag, Vec<u32>>>,
    unresolved_owners: Arc<Vec<(u32, u32)>>,
    #[serde(with = "dashboard_core::map_pairs::shared")]
    wars: Arc<HashMap<(String, String), (i32, i32)>>,
    unresolved_wars: Arc<Vec<(u32, u32)>>,
    #[serde(serialize_with = "dashboard_core::map_pairs::ordered_shared_optional_map")]
    vps: Option<Arc<HashMap<u32, i64>>>,
    #[serde(with = "dashboard_core::map_pairs::shared")]
    subjects: Arc<HashMap<(String, String), (i32, String)>>,
    subjects_known: bool,
    #[serde(serialize_with = "dashboard_core::map_pairs::ordered_map")]
    subject_targets: Arc<HashMap<String, Vec<String>>>,
    #[serde(serialize_with = "dashboard_core::map_pairs::ordered_shared_optional_map")]
    governments: Option<Arc<HashMap<Tag, PeaceGovernment>>>,
    #[serde(serialize_with = "dashboard_core::map_pairs::ordered_shared_optional_map")]
    exists: Option<Arc<HashMap<Tag, bool>>>,
    state_action_key: Option<String>,
}

#[derive(Serialize, Deserialize)]
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

/// Current native fields and their resolved joins. No frame or replay row is kept.
#[allow(clippy::type_complexity)]
#[derive(Default)]
struct SnapshotInputs {
    owners: Option<Vec<(u32, u32)>>,
    wars: Option<Vec<(u32, u32, i32, i32)>>,
    subjects: Option<Vec<(u32, u32, i32, i32, u8, Vec<u8>)>>,
    existence: Option<Vec<(u32, bool)>>,
    politics: Option<Vec<(u32, Option<u32>)>>,
    parties: Option<Vec<(u32, u32, Vec<u8>)>>,
    appearances: Option<Vec<(u32, Vec<u8>)>>,
    resolved_owners: Option<(
        Arc<HashMap<u32, Tag>>,
        Arc<HashMap<Tag, Vec<u32>>>,
        Arc<Vec<(u32, u32)>>,
    )>,
    resolved_wars: Option<(
        Arc<HashMap<(String, String), (i32, i32)>>,
        Arc<Vec<(u32, u32)>>,
    )>,
    resolved_subjects: Option<(
        Arc<HashMap<(String, String), (i32, String)>>,
        Arc<HashMap<String, Vec<String>>>,
    )>,
    resolved_governments: Option<Arc<HashMap<Tag, PeaceGovernment>>>,
    resolved_existence: Option<Arc<HashMap<Tag, bool>>>,
    subject_expiry: Option<i32>,
    ready: bool,
    #[cfg(test)]
    row_visits: usize,
}
impl SnapshotInputs {
    fn refresh(
        &mut self,
        frame: &DecodedFrame,
        changes: &FrameChanges,
        countries: Option<&CountryTags>,
        holders_changed: bool,
    ) -> bool {
        let forced = !self.ready;
        macro_rules! fields {
            ($field:ident, $table:ident, $project:expr) => {{
                let mut changed = forced;
                if forced || changes.rows_changed[TableId::$table.index()] {
                    let next = frame.complete::<tables::$table>().ok().map(|rows| {
                        #[cfg(test)]
                        {
                            self.row_visits += rows.len();
                        }
                        rows.iter().map($project).collect::<Vec<_>>()
                    });
                    changed |= self.$field != next;
                    self.$field = next;
                }
                changed
            }};
        }
        let owners_changed = fields!(owners, States, |row: &replay::rows::StateRow| (
            row.id, row.owner
        ));
        let wars_changed = fields!(wars, WarRelations, |row: &replay::rows::WarRelationRow| (
            row.first_country,
            row.second_country,
            row.start_hour,
            row.end_hour
        ));
        let subjects_changed = fields!(subjects, Subjects, |row: &replay::rows::SubjectRow| (
            row.overlord,
            row.subject,
            row.start_hour,
            row.end_hour,
            row.cancel,
            row.state.clone()
        ));
        let exists_changed = fields!(
            existence,
            CountryCounterInputs,
            |row: &replay::rows::CountryCounterInputRow| (row.country, row.owned_state_count > 0)
        );
        let politics_changed = fields!(politics, Politics, |row: &replay::rows::PoliticsRow| (
            row.country,
            row.ruling_party
        ));
        let parties_changed = fields!(
            parties,
            PoliticalParties,
            |row: &replay::rows::PoliticalPartyRow| (row.country, row.id, row.token.clone())
        );
        let appearances_changed = fields!(
            appearances,
            CountryAppearance,
            |row: &replay::rows::CountryAppearanceRow| (row.country, row.leader_ideology.clone())
        );
        if owners_changed || holders_changed {
            self.resolved_owners = self
                .owners
                .as_ref()
                .zip(countries)
                .map(|(rows, countries)| {
                    let mut owners = HashMap::new();
                    let mut unresolved = Vec::new();
                    for &(id, raw) in rows {
                        if let Some(tag) = countries.tags.get(&raw) {
                            owners.insert(id, tag.clone());
                        } else {
                            unresolved.push((id, raw));
                        }
                    }
                    let mut by_owner = HashMap::<Tag, Vec<u32>>::new();
                    for (&id, tag) in &owners {
                        by_owner.entry(tag.clone()).or_default().push(id);
                    }
                    // State order, not hash order: the continuation stores these lists.
                    for states in by_owner.values_mut() {
                        states.sort_unstable();
                    }
                    (Arc::new(owners), Arc::new(by_owner), Arc::new(unresolved))
                });
        }
        if wars_changed || holders_changed {
            self.resolved_wars = self.wars.as_ref().zip(countries).map(|(rows, countries)| {
                let mut wars = HashMap::new();
                let mut unresolved = Vec::new();
                let mut ambiguous = HashSet::new();
                for &(a, b, start, end) in rows {
                    let (Some(a_tag), Some(b_tag)) =
                        (countries.tags.get(&a), countries.tags.get(&b))
                    else {
                        unresolved.push((a, b));
                        continue;
                    };
                    let key = pair(a_tag, b_tag);
                    let value = (start, end);
                    if wars.get(&key).is_some_and(|prior| *prior != value) {
                        wars.remove(&key);
                        ambiguous.insert(key.clone());
                        unresolved.push((a, b));
                    }
                    if !ambiguous.contains(&key) {
                        wars.insert(key, value);
                    }
                }
                (Arc::new(wars), Arc::new(unresolved))
            });
        }
        if subjects_changed
            || holders_changed
            || self.subject_expiry.is_some_and(|due| due <= frame.hour)
        {
            self.subject_expiry = self
                .subjects
                .iter()
                .flatten()
                .filter_map(|&(_, _, start, end, cancel, _)| {
                    (cancel == 0 && end > start && end > frame.hour).then_some(end)
                })
                .min();
            self.resolved_subjects =
                self.subjects
                    .as_ref()
                    .zip(countries)
                    .and_then(|(rows, countries)| {
                        let subjects: HashMap<_, _> = rows
                            .iter()
                            .filter(|(_, _, start, end, cancel, _)| {
                                *cancel == 0 && (*end <= *start || frame.hour < *end)
                            })
                            .map(|(a, b, start, _, _, state)| {
                                Some((
                                    (
                                        countries.tags.get(a)?.to_string(),
                                        countries.tags.get(b)?.to_string(),
                                    ),
                                    (*start, std::str::from_utf8(state).ok()?.to_owned()),
                                ))
                            })
                            .collect::<Option<_>>()?;
                        let mut targets = HashMap::<String, Vec<String>>::new();
                        for (a, b) in subjects.keys() {
                            targets.entry(a.clone()).or_default().push(b.clone());
                        }
                        // Tag order, not hash order: the continuation stores these lists.
                        for subjects in targets.values_mut() {
                            subjects.sort_unstable();
                        }
                        Some((Arc::new(subjects), Arc::new(targets)))
                    });
        }
        if exists_changed || holders_changed {
            self.resolved_existence =
                self.existence
                    .as_ref()
                    .zip(countries)
                    .and_then(|(rows, countries)| {
                        rows.iter()
                            .map(|(raw, exists)| Some((countries.tags.get(raw)?.clone(), *exists)))
                            .collect::<Option<HashMap<_, _>>>()
                            .map(Arc::new)
                    });
        }
        if politics_changed || parties_changed || appearances_changed || holders_changed {
            self.resolved_governments = self
                .politics
                .as_ref()
                .zip(self.parties.as_ref())
                .zip(self.appearances.as_ref())
                .zip(countries)
                .and_then(|(((politics, parties), appearances), countries)| {
                    let tokens = parties
                        .iter()
                        .map(|(raw, id, token)| {
                            Some((
                                (countries.tags.get(raw)?.clone(), *id),
                                std::str::from_utf8(token).ok()?,
                            ))
                        })
                        .collect::<Option<HashMap<_, _>>>()?;
                    let ideologies = appearances
                        .iter()
                        .map(|(raw, token)| {
                            Some((
                                countries.tags.get(raw)?.clone(),
                                std::str::from_utf8(token).ok()?,
                            ))
                        })
                        .collect::<Option<HashMap<_, _>>>()?;
                    politics
                        .iter()
                        .map(|(raw, ruling)| {
                            let tag = countries.tags.get(raw)?;
                            Some((
                                tag.clone(),
                                PeaceGovernment {
                                    ruling_party: *ruling,
                                    party_token: ruling
                                        .and_then(|id| tokens.get(&(tag.clone(), id)).copied())
                                        .map(str::to_owned),
                                    leader_ideology: ideologies
                                        .get(tag)
                                        .copied()
                                        .map(str::to_owned),
                                },
                            ))
                        })
                        .collect::<Option<HashMap<_, _>>>()
                        .map(Arc::new)
                });
        }
        self.ready = true;
        exists_changed || holders_changed
    }
    fn snapshot(
        &self,
        frame: &DecodedFrame,
        countries: Arc<CountryTags>,
        vps: Option<Arc<HashMap<u32, i64>>>,
        state_action_key: Option<String>,
    ) -> Option<Snapshot> {
        let (owners, by_owner, unresolved_owners) = self.resolved_owners.as_ref()?;
        let (wars, unresolved_wars) = self.resolved_wars.as_ref()?;
        let (subjects, subject_targets) = self
            .resolved_subjects
            .clone()
            .unwrap_or_else(|| (Arc::new(HashMap::new()), Arc::new(HashMap::new())));
        Some(Snapshot {
            hour: frame.hour,
            roster_complete: frame.complete::<tables::PeaceConferences>().is_ok(),
            countries,
            owners: Arc::clone(owners),
            states_by_owner: Arc::clone(by_owner),
            unresolved_owners: Arc::clone(unresolved_owners),
            wars: Arc::clone(wars),
            unresolved_wars: Arc::clone(unresolved_wars),
            vps,
            subjects_known: self.resolved_subjects.is_some(),
            subjects,
            subject_targets,
            governments: self.resolved_governments.clone(),
            exists: self.resolved_existence.clone(),
            state_action_key,
        })
    }
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
                    action.name = dashboard_text::scripted_text::stored_text(
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
        preparing: Vec::new(),
    }
}

/// Only these tables are decoded; nothing reads live game state or changes format.
#[cfg(test)]
pub fn scan_branch(
    replay: &Replay,
    branch: u64,
    cancelled: &AtomicBool,
) -> io::Result<PeaceDealsReply> {
    let start = replay
        .branches
        .iter()
        .find(|start| start.id == branch)
        .ok_or_else(|| dashboard_core::invalid("Peace-deals branch is not in this recording"))?;
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
        let cadence = replay::cadence::cadence_in(table, replay.format);
        if cadence != replay::cadence::Cadence::Hourly {
            return Err(dashboard_core::invalid(format!(
                "Peace deals does not support the {cadence:?} cadence of {table:?}"
            )));
        }
    }
    let mut scan = Scan::new(branch, first, last, replay.format.version());
    dashboard_core::blocks::scan_blocks(
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
                        scan.reply.warnings.push(format!(
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
                            scan.reply.warnings.push(format!(
                                "Peace deals: stopped block at {}: {error}",
                                frame.hour
                            ));
                            break;
                        }
                    };
                    scan.observe(decoded)?;
                }
            }
            Ok(())
        },
    )?;
    Ok(scan.reply())
}

#[derive(Serialize, Deserialize)]
struct Scan {
    reply: PeaceDealsReply,
    format: u16,
    first: i32,
    last: i32,
    #[serde(serialize_with = "dashboard_core::map_pairs::ordered_map")]
    open: HashMap<u64, OpenConference>,
    #[serde(serialize_with = "dashboard_core::map_pairs::ordered_map")]
    pending: HashMap<usize, Option<Arc<Snapshot>>>,
    #[serde(serialize_with = "dashboard_core::map_pairs::ordered_set")]
    processed: HashSet<usize>,
    #[serde(serialize_with = "dashboard_core::map_pairs::ordered_set")]
    unassigned: HashSet<usize>,
    has_active: bool,
    has_ambiguous: bool,
    previous: Option<Arc<Snapshot>>,
    previous_hour: Option<i32>,
    unobserved_frontier: Option<(i32, i32, [bool; 7])>,
    #[serde(serialize_with = "dashboard_core::map_pairs::ordered_map")]
    first_existing: HashMap<Tag, i32>,
    exact: exact::Observed,
    #[serde(skip)]
    lexical_view: Option<ViewIdentity>,
    action_key: Option<String>,
    action_reasons: std::collections::BTreeSet<String>,
    #[serde(skip)]
    country_tags: CountryResolution,
    #[serde(skip)]
    victory_points: VictoryPoints,
    #[serde(skip)]
    snapshot_inputs: SnapshotInputs,
    /// The scan ends at a prepared prefix's last hour, not the branch's:
    /// what it has not yet observed is still being prepared.
    #[serde(skip)]
    prefix: bool,
}
impl Scan {
    fn new(branch: u64, first: i32, last: i32, format: u16) -> Self {
        Self {
            reply: PeaceDealsReply { branch_id: branch.to_string(), derived: true, deals: Vec::new(), transfers: PeaceTransferGraph::default(), unobserved: PeaceUnobserved::default(), warnings: vec!["Derived net outcomes do not recover bids, negotiators, action order or causation. War names and conference participants are not recorded.".into()] },
            format, first, last,
            open: HashMap::new(), pending: HashMap::new(), processed: HashSet::new(), unassigned: HashSet::new(),
            has_active: false, has_ambiguous: false, previous: None, previous_hour: first.checked_sub(1), unobserved_frontier: None,
            first_existing: HashMap::new(), exact: Default::default(), lexical_view: None, action_key: None,
            action_reasons: Default::default(), country_tags: Default::default(), victory_points: Default::default(), snapshot_inputs: Default::default(),
            prefix: false,
        }
    }
    fn seal_unobserved(&mut self) {
        if let Some((first, last, flags)) = self.unobserved_frontier
            && first <= last
        {
            for (spans, missing) in [
                &mut self.reply.unobserved.conferences,
                &mut self.reply.unobserved.ownership,
                &mut self.reply.unobserved.wars,
                &mut self.reply.unobserved.victory_points,
                &mut self.reply.unobserved.subjects,
                &mut self.reply.unobserved.governments,
                &mut self.reply.unobserved.existence,
            ]
            .into_iter()
            .zip(flags)
            {
                if missing {
                    mark(spans, first, last);
                }
            }
            self.unobserved_frontier = last.checked_add(1).map(|first| (first, last, flags));
        }
    }
    fn observe_unobserved(&mut self, hour: i32, flags: [bool; 7]) {
        if let Some((_, last, previous)) = &mut self.unobserved_frontier
            && *previous == flags
            && last.checked_add(1) == Some(hour)
        {
            *last = hour;
            return;
        }
        self.seal_unobserved();
        self.unobserved_frontier = Some((hour, hour, flags));
    }
    #[cfg(test)]
    fn observe(&mut self, frame: &DecodedFrame) -> io::Result<()> {
        self.observe_changed(
            frame,
            &FrameChanges {
                rows_changed: [true; replay::frame::TABLE_COUNT],
                table_read: [true; replay::frame::TABLE_COUNT],
                slot_read: [true; replay::frame::TABLE_COUNT],
                updates: None,
            },
        )
    }
    fn observe_changed(&mut self, frame: &DecodedFrame, changes: &FrameChanges) -> io::Result<()> {
        let hour = frame.hour;
        if let Some(prev) = self.previous_hour
            && prev.checked_add(1).is_some_and(|next| next < hour)
        {
            self.seal_unobserved();
            self.reply.unobserved.all(prev + 1, hour - 1);
        }
        self.previous_hour = Some(hour);
        // A weak identity: holding the view would copy it at its next delta.
        let view = &frame.tables[TableId::LexicalTokens.index()];
        if self
            .lexical_view
            .as_ref()
            .is_none_or(|prior| !prior.is(view))
        {
            self.action_key = state_action_key(frame)
                .inspect_err(|reason| {
                    self.action_reasons.insert(reason.clone());
                })
                .ok();
            self.lexical_view = Some(view.identity());
        }
        let prior_countries = self.country_tags.resolved.clone();
        let countries = self.country_tags.read_changed(frame, changes);
        let holders_changed = match (&prior_countries, &countries) {
            (Some(a), Some(b)) => !Arc::ptr_eq(a, b),
            (None, None) => false,
            _ => true,
        };
        let vps = self.victory_points.read_changed(frame, changes);
        let exists_changed =
            self.snapshot_inputs
                .refresh(frame, changes, countries.as_deref(), holders_changed);
        let exists = self.snapshot_inputs.resolved_existence.clone();
        let current = countries
            .as_ref()
            .and_then(|countries| {
                self.snapshot_inputs.snapshot(
                    frame,
                    Arc::clone(countries),
                    vps.clone(),
                    self.action_key.clone(),
                )
            })
            .map(Arc::new);
        self.exact.observe_changed(
            frame,
            countries.as_deref(),
            vps.as_deref(),
            self.action_key.as_deref(),
            holders_changed || !self.snapshot_inputs.ready,
            changes,
        );
        let had_open = !self.open.is_empty();
        let had_active = self.has_active;
        let roster_read = hour == self.first
            || slot_read(
                replay::format::FormatVersion::read(self.format)?,
                &frame.metrics,
                TableId::PeaceConferences,
                hour,
            );
        let roster = frame.complete::<tables::PeaceConferences>();
        let roster_complete = roster.is_ok();
        if roster_read && let Ok(rows) = roster {
            let active: Vec<_> = self
                .open
                .values()
                .filter(|entry| !self.processed.contains(&entry.index))
                .map(|entry| entry.index)
                .collect();
            if active.len() > 1 {
                self.unassigned.extend(active);
            }
            let present: HashSet<_> = rows.iter().map(|row| row.id).collect();
            self.open.retain(|id, conference| {
                if present.contains(id) {
                    return true;
                }
                let deal = &mut self.reply.deals[conference.index];
                deal.first_absent_hour = Some(hour);
                deal.end_is_by = deal.last_observed_hour.checked_add(1) != Some(hour);
                deal.end_hour = Some(if deal.end_is_by {
                    hour
                } else {
                    deal.last_observed_hour
                });
                if !self.processed.contains(&conference.index) {
                    self.pending
                        .insert(conference.index, conference.before.clone());
                }
                false
            });
            for row in rows {
                let conference = self.open.entry(row.id).or_insert_with(|| {
                    let mut deal = card(
                        format!("conference:{}:{}", row.id, hour),
                        PeaceObservation::RecordedConference,
                        hour,
                    );
                    deal.conference_id = Some(row.id.to_string());
                    self.reply.deals.push(deal);
                    OpenConference {
                        index: self.reply.deals.len() - 1,
                        before: self.previous.clone(),
                    }
                });
                let index = conference.index;
                let deal = &mut self.reply.deals[index];
                deal.last_observed_hour = hour;
                deal.current_turn = Some(row.current_turn);
                deal.finalized = Some(row.finalized != 0);
                if row.finalized != 0 && deal.first_finalized_hour.is_none() {
                    deal.first_finalized_hour = Some(hour);
                    deal.end_hour = Some(hour);
                    deal.end_is_by = true;
                    if !self.processed.contains(&index) {
                        self.pending.insert(index, conference.before.clone());
                    }
                }
                if row.finalized == 0 && current.is_some() {
                    conference.before = if self.unassigned.contains(&index) {
                        None
                    } else {
                        current.clone()
                    };
                }
            }
            let comparisons: HashSet<_> = self
                .open
                .values()
                .filter(|entry| !self.processed.contains(&entry.index))
                .map(|entry| entry.index)
                .chain(self.pending.keys().copied())
                .collect();
            if comparisons.len() > 1 {
                self.unassigned.extend(comparisons);
            }
            self.has_active = self
                .open
                .values()
                .any(|entry| !self.processed.contains(&entry.index));
            self.has_ambiguous = self.open.values().any(|entry| {
                !self.processed.contains(&entry.index) && self.unassigned.contains(&entry.index)
            });
        }
        self.observe_unobserved(
            hour,
            [
                !(roster_read && roster_complete),
                current
                    .as_ref()
                    .is_none_or(|s| !s.unresolved_owners.is_empty()),
                current
                    .as_ref()
                    .is_none_or(|s| !s.unresolved_wars.is_empty()),
                frame.complete::<tables::Provinces>().is_err(),
                frame.complete::<tables::Subjects>().is_err()
                    || current.as_ref().is_none_or(|s| !s.subjects_known),
                current.as_ref().is_none_or(|s| s.governments.is_none()),
                exists.is_none(),
            ],
        );
        if let Some(current) = current {
            let candidates: Vec<_> = self.pending.keys().copied().collect();
            let assigned = candidates.len() == 1 && !self.unassigned.contains(&candidates[0]);
            if assigned {
                let index = candidates[0];
                if let Some(before) = &self.pending[&index] {
                    let transfers = transfers(before, &current, true);
                    populate(
                        &mut self.reply.deals[index],
                        before,
                        &current,
                        transfers,
                        &self.first_existing,
                    );
                } else {
                    self.reply.deals[index].limitations.push("No complete ownership and war read before settlement; outcomes are not recorded.".into());
                }
            }
            if let Some(before) = self.previous.as_ref() {
                let consecutive = before.hour.checked_add(1) == Some(hour);
                // A single resident conference reserves its net change for its
                // next actual roster read. Several cannot be distinguished.
                let unmatched = !assigned
                    && (!had_active && !self.has_active
                        || self.has_ambiguous
                        || candidates
                            .iter()
                            .any(|index| self.unassigned.contains(index)));
                let transfers =
                    if unmatched && consecutive && !Arc::ptr_eq(&before.owners, &current.owners) {
                        transfers(before, &current, false)
                    } else {
                        Vec::new()
                    };
                if !transfers.is_empty() {
                    let observation = if !had_open
                        && self.open.is_empty()
                        && roster_complete
                        && before.roster_complete
                    {
                        PeaceObservation::ConferenceNotRecorded
                    } else {
                        PeaceObservation::ConferenceUnknown
                    };
                    for (group, transfers) in components(transfers).into_iter().enumerate() {
                        let mut deal = card(format!("outcome:{hour}:{group}"), observation, hour);
                        deal.end_hour = Some(hour);
                        deal.end_is_by = true;
                        populate(&mut deal, before, &current, transfers, &self.first_existing);
                        deal.limitations.push("Connected war-end transfer pairs group this outcome; the data cannot prove how many conferences produced it.".into());
                        self.reply.deals.push(deal);
                    }
                }
            }
            for &index in &candidates {
                if self.unassigned.contains(&index) {
                    self.reply.deals[index].limitations.push("Sampled conferences overlap this comparison; outcomes cannot be assigned to one.".into());
                }
            }
            for (index, _) in self.pending.drain() {
                self.processed.insert(index);
            }
            if !candidates.is_empty() {
                self.has_active = self
                    .open
                    .values()
                    .any(|entry| !self.processed.contains(&entry.index));
                self.has_ambiguous = self.open.values().any(|entry| {
                    !self.processed.contains(&entry.index) && self.unassigned.contains(&entry.index)
                });
            }
            self.previous = Some(current);
        }
        if exists_changed && let Some(exists) = &exists {
            for (tag, exists) in exists.iter() {
                if *exists {
                    self.first_existing.entry(tag.clone()).or_insert(hour);
                }
            }
        }
        Ok(())
    }
    /// Hours `first..=last` of a block read by its changes, which continue
    /// the last observed hour (`frame`, whose rows they hold) and at which no
    /// field this scan reads changed, with the conference roster's slot read
    /// at `reads`: [`Self::observe_changed`]'s step for each, at once. Every
    /// operand, holder, snapshot and comparison is the hour before's, so an
    /// hour moves only the unobserved frontier (a roster read of a complete,
    /// empty roster with no conference held marks the roster read), the
    /// finalizer frontier and its missing hours, and the hour of the
    /// snapshot it takes. False, with nothing changed, when an hour could do
    /// more: a gap, a subject relation's end, a roster read while a
    /// conference or comparison is held, or a comparison waiting for this
    /// snapshot. The caller then observes the hours on their frames.
    fn advance_quiet(
        &mut self,
        frame: &DecodedFrame,
        first: i32,
        last: i32,
        reads: &[i32],
    ) -> bool {
        let Some((_, _, flags)) = self.unobserved_frontier else {
            return false;
        };
        if last < first
            || self.previous_hour.and_then(|hour| hour.checked_add(1)) != Some(first)
            || !self.exact.continues(first)
            || !self.snapshot_inputs.ready
            || !self.country_tags.ready
            || self
                .snapshot_inputs
                .subject_expiry
                .is_some_and(|due| due <= last)
        {
            return false;
        }
        let roster = frame.complete::<tables::PeaceConferences>();
        let roster_complete = roster.is_ok();
        // The branch's first hour reads the roster too.
        let branch_first = self.first;
        let read = |hour: i32| hour == branch_first || reads.binary_search(&hour).is_ok();
        let any_read = (first..=last).contains(&branch_first) || !reads.is_empty();
        // Each hour takes the hour before's snapshot again, when it took one.
        let current = self
            .previous
            .as_ref()
            .filter(|previous| Some(previous.hour) == self.previous_hour)
            .cloned();
        if (current.is_some() && !self.pending.is_empty())
            || (any_read
                && roster_complete
                && (roster.is_ok_and(|rows| !rows.is_empty())
                    || !self.open.is_empty()
                    || !self.pending.is_empty()))
        {
            return false;
        }
        let settlements = exact::missing_reason(&frame.tables[TableId::PeaceSettlements.index()]);
        for hour in first..=last {
            let roster_read = read(hour);
            if roster_read && roster_complete {
                // The empty roster leaves no conference active or ambiguous.
                self.has_active = false;
                self.has_ambiguous = false;
            }
            let mut flags = flags;
            flags[0] = !(roster_read && roster_complete);
            self.observe_unobserved(hour, flags);
        }
        self.exact.advance_quiet(first, last, settlements);
        self.previous_hour = Some(last);
        if let Some(current) = current {
            self.previous = Some(Arc::new(Snapshot {
                hour: last,
                ..(*current).clone()
            }));
        }
        true
    }
    fn close_continuation(&mut self) {
        // At a prepared prefix's end, a later read may still settle the deal.
        let prefix = self.prefix;
        let unsettled = |deal: &mut PeaceDeal, limitation: &str| {
            if prefix {
                if deal.preparing.is_empty() {
                    deal.preparing.push("outcome".into());
                }
            } else {
                deal.limitations.push(limitation.into());
            }
        };
        for (index, _) in self.pending.drain() {
            unsettled(
                &mut self.reply.deals[index],
                "No complete ownership and war read after settlement; outcomes are not recorded.",
            );
        }
        for conference in self.open.values() {
            if !self.processed.contains(&conference.index) {
                unsettled(
                    &mut self.reply.deals[conference.index],
                    "No end was observed; the conference outcome is not recorded.",
                );
            }
        }
        self.open.clear();
    }

    fn reply(mut self) -> PeaceDealsReply {
        self.seal_unobserved();
        self.close_continuation();
        let mut reply = self.reply;
        if let Some(prev) = self.previous_hour {
            if let Some(next) = prev.checked_add(1) {
                reply.unobserved.all(next, self.last);
            }
        } else {
            reply.unobserved.all(self.first, self.last);
        }
        self.exact.apply(&mut reply);
        let mut refusals = catalogue::Refusals::default();
        for reason in self.action_reasons {
            refusals.note(reason, "state transfer");
        }
        reply
            .warnings
            .extend(refusals.lines("peace action bindings"));
        reply.deals.sort_by(|a, b| {
            b.end_hour
                .unwrap_or(b.last_observed_hour)
                .cmp(&a.end_hour.unwrap_or(a.last_observed_hour))
                .then_with(|| a.id.cmp(&b.id))
        });
        reply
    }
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
    for (first, second) in after.unresolved_wars.iter() {
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
    for (&state, from) in before.owners.iter() {
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
    // Ordered: per-country limitations and items are listed in tag order.
    let mut winners = std::collections::BTreeSet::new();
    let mut losers = std::collections::BTreeSet::new();
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
    if let (Some(old), Some(new)) = (
        before.subjects_known.then_some(&before.subjects),
        after.subjects_known.then_some(&after.subjects),
    ) {
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
    deal.losers = losers.into_iter().collect();
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

#[cfg(any(test, feature = "test-support"))]
pub mod test_fixtures;
#[cfg(test)]
pub(crate) mod tests;
