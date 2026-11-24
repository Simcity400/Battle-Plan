
use dashboard_base::products::{LoadContext, Product};
use dashboard_world::products::holder_tags::{HolderTags, Identity};
use dashboard_base::{Source, catalogue, span::HourSpan};
use dashboard_query::{Query, identity::Holder, log, read};
use replay_data::rows::{PeaceConferenceSpanRow, PeaceOutcomeRow, PeaceSettlementRow, SettledPeaceAction, TableId, tables};
use serde::{Deserialize, Serialize};
use std::{
    collections::{BTreeMap, BTreeSet, HashMap, HashSet},
    io,
    sync::atomic::AtomicBool,
};
#[cfg(any(test, feature = "export-types"))]
use ts_rs::TS;

#[cfg(test)]
mod tests;

fn invalid(what: impl Into<String>) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, what.into())
}

pub struct PeaceDeals;

#[derive(Clone, Debug, PartialEq, Deserialize, Serialize)]
#[cfg_attr(any(test, feature = "export-types"), derive(TS))]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct PeaceDealsParams {
    pub branch_id: String,
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[cfg_attr(any(test, feature = "export-types"), derive(TS))]
#[serde(rename_all = "camelCase")]
pub struct PeaceDates {
    pub before_hour: i32,
    pub after_hour: i32,
}

#[derive(Clone, Copy, Debug, Deserialize, Serialize, PartialEq, Eq, Hash)]
#[cfg_attr(any(test, feature = "export-types"), derive(TS))]
#[serde(rename_all = "camelCase")]
pub enum PeaceActionKind {
    StateTransfer,
    NewSubject,
    GovernmentChange,
    CountryReleased,
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[cfg_attr(any(test, feature = "export-types"), derive(TS))]
#[serde(rename_all = "camelCase")]
pub struct PeaceActionLabel {
    pub kind: PeaceActionKind,
    pub key: Option<String>,
    pub name: Option<String>,
}

#[derive(Clone, Debug, Default, Deserialize, Serialize, PartialEq, Eq)]
#[cfg_attr(any(test, feature = "export-types"), derive(TS))]
#[serde(rename_all = "camelCase")]
pub struct PeaceGovernment {
    pub ruling_party: Option<u32>,
    pub party_token: Option<String>,
    pub leader_ideology: Option<String>,
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[cfg_attr(any(test, feature = "export-types"), derive(TS))]
#[serde(tag = "kind", rename_all = "camelCase", rename_all_fields = "camelCase")]
pub enum PeaceChange {
    StateTransfer {
        state: u32,
        from: String,
        to: String,
        from_country: PeaceTransferCountry,
        to_country: PeaceTransferCountry,
        #[doc = " Exact signed sum, null if Provinces cannot supply the state."]
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
        #[doc = " Owners of the transferred states at the before read."]
        from: Vec<String>,
        states: Vec<u32>,
        #[doc = " The released country's holder at the after read."]
        to_country: PeaceTransferCountry,
        #[doc = " Each state with its own former owner, in state order."]
        transfers: Vec<PeaceReleasedState>,
    },
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[cfg_attr(any(test, feature = "export-types"), derive(TS))]
#[serde(rename_all = "camelCase")]
pub struct PeaceReleasedState {
    pub state: u32,
    pub from_country: PeaceTransferCountry,
    pub victory_points: Option<String>,
    pub victory_points_read_hour: Option<i32>,
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[cfg_attr(any(test, feature = "export-types"), derive(TS))]
#[serde(rename_all = "camelCase")]
pub struct PeaceItem {
    pub derived: bool,
    pub dates: PeaceDates,
    #[doc = " The taker/overlord, null where no enforcer or liberator is recorded."]
    pub winner: Option<String>,
    pub action: PeaceActionLabel,
    pub change: PeaceChange,
}

#[derive(Clone, Copy, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[cfg_attr(any(test, feature = "export-types"), derive(TS))]
#[serde(rename_all = "camelCase")]
pub enum PeaceObservation {
    RecordedConference,
    ConferenceNotRecorded,
    ConferenceUnknown,
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[cfg_attr(any(test, feature = "export-types"), derive(TS))]
#[serde(rename_all = "camelCase")]
pub struct PeaceTransferCountry {
    pub slot: u32,
    pub country_id: u32,
    pub tag: String,
    pub hour: i32,
}

impl PeaceTransferCountry {
    fn identity(&self) -> Identity {
        (self.slot, self.country_id)
    }
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[cfg_attr(any(test, feature = "export-types"), derive(TS))]
#[serde(rename_all = "camelCase")]
pub struct PeaceTransferNode {
    pub id: String,
    pub country: PeaceTransferCountry,
    #[doc = " Separate source/target nodes even when a country occurs on both sides."]
    pub side: PeaceTransferSide,
}

#[derive(Clone, Copy, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[cfg_attr(any(test, feature = "export-types"), derive(TS))]
#[serde(rename_all = "camelCase")]
pub enum PeaceTransferSide {
    Source,
    Target,
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[cfg_attr(any(test, feature = "export-types"), derive(TS))]
#[serde(rename_all = "camelCase")]
pub struct PeaceTransferLink {
    pub id: String,
    pub source: String,
    pub target: String,
    pub state_count: u32,
    #[doc = " Null if any contributing state's VPs are not recorded; never a partial sum."]
    pub victory_points: Option<String>,
    pub states: Vec<PeaceTransferredState>,
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[cfg_attr(any(test, feature = "export-types"), derive(TS))]
#[serde(rename_all = "camelCase")]
pub struct PeaceTransferredState {
    pub state: u32,
    pub from_country: PeaceTransferCountry,
    pub to_country: PeaceTransferCountry,
    pub dates: PeaceDates,
    pub victory_points: Option<String>,
    pub victory_points_read_hour: Option<i32>,
    #[doc = " The same settlement released the taker; the state came from its former owner."]
    pub released: bool,
}

#[derive(Clone, Debug, Default, Deserialize, Serialize, PartialEq, Eq)]
#[cfg_attr(any(test, feature = "export-types"), derive(TS))]
#[serde(rename_all = "camelCase")]
pub struct PeaceTransferGraph {
    pub nodes: Vec<PeaceTransferNode>,
    pub links: Vec<PeaceTransferLink>,
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[cfg_attr(any(test, feature = "export-types"), derive(TS))]
#[serde(rename_all = "camelCase")]
pub struct PeaceCapturedAction {
    pub derived: bool,
    pub country: u32,
    #[doc = " Event-hour tags; null when that raw reference cannot be resolved then."]
    pub country_tag: Option<String>,
    pub turn: u32,
    pub ordinal: u32,
    pub token: u32,
    pub token_name: Option<String>,
    pub name_key: Option<String>,
    pub name: Option<String>,
    #[doc = " The country the name's [`NAME_COUNTRY`] stands for, as an event-hour tag: the game names a"]
    #[doc = " puppet demand with the country it puppets."]
    pub name_country: Option<String>,
    #[serde(skip)]
    #[cfg_attr(any(test, feature = "export-types"), ts(skip))]
    pub(crate) cached_name: Option<Vec<u8>>,
    pub taker: u32,
    pub giver: u32,
    pub negotiator: u32,
    pub taker_tag: Option<String>,
    pub giver_tag: Option<String>,
    pub negotiator_tag: Option<String>,
    pub status: i32,
    pub cost: i32,
    pub form: u8,
    pub state: Option<i32>,
    pub flags: Option<[u8; 4]>,
    pub country_operand: Option<u32>,
    pub country_operand_tag: Option<String>,
    pub puppet_auxiliary: Option<String>,
    pub ship: Option<String>,
    pub ratio: Option<String>,
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[cfg_attr(any(test, feature = "export-types"), derive(TS))]
#[serde(rename_all = "camelCase")]
pub struct PeaceCapturedTransfer {
    pub derived: bool,
    pub hour: i32,
    pub state: i32,
    pub taker: u32,
    pub to: Option<String>,
    #[doc = " A matching action's giver, absent for automatic/no-demand outcomes."]
    pub giver: Option<u32>,
    pub from: Option<String>,
    pub from_country: Option<PeaceTransferCountry>,
    pub to_country: Option<PeaceTransferCountry>,
    pub automatic: bool,
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[cfg_attr(any(test, feature = "export-types"), derive(TS))]
#[serde(rename_all = "camelCase")]
pub struct PeaceCapturedSubject {
    pub derived: bool,
    pub overlord: u32,
    pub subject: u32,
    pub overlord_tag: Option<String>,
    pub subject_tag: Option<String>,
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[cfg_attr(any(test, feature = "export-types"), derive(TS))]
#[serde(rename_all = "camelCase")]
pub struct PeaceCaptured {
    pub hour: i32,
    pub publication_hour: i32,
    #[doc = " Raw references and native order, independent of country-tag resolution."]
    pub winners: Vec<u32>,
    pub losers: Vec<u32>,
    pub actions: Vec<PeaceCapturedAction>,
    pub transfers: Vec<PeaceCapturedTransfer>,
    pub subjects: Vec<PeaceCapturedSubject>,
}

#[derive(Clone, Debug, PartialEq, Serialize)]
#[cfg_attr(any(test, feature = "export-types"), derive(TS))]
#[serde(rename_all = "camelCase")]
pub struct PeaceDeal {
    pub id: String,
    pub derived: bool,
    pub observation: PeaceObservation,
    pub conference_id: Option<String>,
    #[doc = " No native conference name or unambiguous grouped-war identity is recorded."]
    pub war_name: Option<String>,
    pub first_observed_hour: i32,
    pub last_observed_hour: i32,
    pub current_turn: Option<i32>,
    pub finalized: Option<bool>,
    pub first_finalized_hour: Option<i32>,
    #[doc = " Last seen when followed by a consecutive absence, else first absent hour."]
    pub end_hour: Option<i32>,
    pub end_is_by: bool,
    pub first_absent_hour: Option<i32>,
    pub participants_derived: bool,
    pub winners: Vec<String>,
    pub losers: Vec<String>,
    pub items: Vec<PeaceItem>,
    #[doc = " Ordered finalizer operands and outcomes, absent for the derived fallback."]
    pub captured: Option<PeaceCaptured>,
    #[doc = " Reason the finalizer capture cannot certify this deal's actions."]
    pub capture_unavailable: Option<String>,
    #[doc = " The settlement's hour when no conference span supplied this card."]
    #[serde(skip_serializing_if = "Option::is_none")]
    #[cfg_attr(any(test, feature = "export-types"), ts(optional))]
    pub capture_observed_hour: Option<i32>,
    #[doc = " A failed settlement's hour: its reason is no evidence at an earlier hour."]
    #[serde(skip_serializing_if = "Option::is_none")]
    #[cfg_attr(any(test, feature = "export-types"), ts(optional))]
    pub capture_unavailable_hour: Option<i32>,
    pub transfers: PeaceTransferGraph,
    #[doc = " Missing inputs or an attribution limitation, not evidence of no actions."]
    pub limitations: Vec<String>,
}

#[derive(Clone, Debug, PartialEq, Default, Serialize)]
#[cfg_attr(any(test, feature = "export-types"), derive(TS))]
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

#[derive(Clone, Debug, PartialEq, Serialize)]
#[cfg_attr(any(test, feature = "export-types"), derive(TS))]
#[serde(rename_all = "camelCase")]
pub struct PeaceDealsReply {
    pub branch_id: String,
    pub derived: bool,
    #[doc = " Newest card first."]
    pub deals: Vec<PeaceDeal>,
    #[doc = " Counts transfer occurrences, including a state transferred more than once."]
    pub transfers: PeaceTransferGraph,
    pub unobserved: PeaceUnobserved,
    pub warnings: Vec<String>,
}

dashboard_base::reply_warnings!(PeaceDealsReply);

const DERIVED_WARNING: &str = "Derived net outcomes do not recover bids, negotiators, action order or causation. War names and conference participants are not recorded.";
const TAKE_STATES_LABEL: &str = "PEACE_TAKE_STATES_DEMAND";
const DEMAND_LABELS: [(&str, &str); 4] = [
    ("take_states", TAKE_STATES_LABEL),
    ("liberate", "PEACE_LIBERATE_LABEL"),
    ("force_government", "PEACE_FORCE_GOVERNMENT_LABEL"),
    ("puppet", PUPPET_LABEL),
];
const PUPPET_LABEL: &str = "PEACE_PUPPET_COUNTRY_LABEL";
pub const NAME_COUNTRY: &str = "{COUNTRY}";

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Outcome {
    pub after: i32,
    pub ordinal: u32,
    pub observation: u8,
    pub conference: Option<u64>,
    pub before: i32,
    pub change: u8,
    pub group: u32,
    pub state: u32,
    pub from: Identity,
    pub to: Identity,
    pub victory_points: Option<i64>,
}

#[derive(Clone, Debug)]
pub struct Settled {
    pub publication_hour: i32,
    pub row: PeaceSettlementRow,
}

pub struct Joins {
    pub holders: HolderTags,
    pub raw: HashMap<(u32, i32), (Identity, String)>,
    pub tokens: HashMap<u32, String>,
    pub action_key: Result<String, String>,
    pub subjects: HashMap<(Identity, Identity, i32), (i32, Option<String>)>,
    pub governments: HashMap<(Identity, i32), PeaceGovernment>,
}

impl Default for Joins {
    fn default() -> Self {
        Self {
            holders: HolderTags::default(),
            raw: HashMap::new(),
            tokens: HashMap::new(),
            action_key: Err("no state was taken".into()),
            subjects: HashMap::new(),
            governments: HashMap::new(),
        }
    }
}

impl Joins {
    fn country(&self, identity: Identity, hour: i32) -> Option<PeaceTransferCountry> {
        let tag = self.holders.tag_at(identity, hour)?;
        Some(PeaceTransferCountry { slot: identity.0, country_id: identity.1, tag: tag.to_owned(), hour })
    }

    fn raw_tag(&self, raw: u32, hour: i32) -> Option<String> {
        self.raw.get(&(raw, hour)).map(|(_, tag)| tag.clone())
    }

    fn raw_country(&self, raw: u32, hour: i32) -> Option<PeaceTransferCountry> {
        self.raw.get(&(raw, hour)).map(|((slot, id), tag)| PeaceTransferCountry { slot: *slot, country_id: *id, tag: tag.clone(), hour })
    }

    fn key(&self) -> Option<String> {
        self.action_key.as_ref().ok().cloned()
    }
}

fn neutral(kind: PeaceActionKind) -> PeaceActionLabel {
    PeaceActionLabel { kind, key: None, name: None }
}

fn card(id: String, observation: PeaceObservation, hour: i32) -> PeaceDeal {
    PeaceDeal {
        id,
        derived: true,
        observation,
        conference_id: None,
        war_name: None,
        first_observed_hour: hour,
        last_observed_hour: hour,
        current_turn: None,
        finalized: None,
        first_finalized_hour: None,
        end_hour: None,
        end_is_by: false,
        first_absent_hour: None,
        participants_derived: true,
        winners: Vec::new(),
        losers: Vec::new(),
        items: Vec::new(),
        captured: None,
        capture_unavailable: None,
        capture_observed_hour: None,
        capture_unavailable_hour: None,
        transfers: PeaceTransferGraph::default(),
        limitations: vec!["Conference participants and war name are not recorded; sides are inferred only from qualifying transfers.".into()],
    }
}

fn vp_text(vp: Option<i64>) -> Option<String> {
    vp.map(|vp| vp.to_string())
}

fn populate(deal: &mut PeaceDeal, rows: &[&Outcome], joins: &Joins) {
    let Some(first) = rows.first() else { return };
    let (before, after) = (first.before, first.after);
    let dates = PeaceDates { before_hour: before, after_hour: after };
    let mut items = Vec::new();
    let mut winners = BTreeSet::new();
    let mut losers = BTreeSet::new();
    let mut unnamed = BTreeSet::new();
    let mut released: BTreeMap<Identity, Vec<&Outcome>> = BTreeMap::new();
    for row in rows {
        match row.change {
            PeaceOutcomeRow::TRANSFER => {
                let (Some(from), Some(to)) = (joins.country(row.from, before), joins.country(row.to, after)) else {
                    unnamed.insert(row.state);
                    continue;
                };
                winners.insert(to.tag.clone());
                losers.insert(from.tag.clone());
                items.push(PeaceItem {
                    derived: true,
                    dates: dates.clone(),
                    winner: Some(to.tag.clone()),
                    action: PeaceActionLabel { kind: PeaceActionKind::StateTransfer, key: joins.key(), name: None },
                    change: PeaceChange::StateTransfer {
                        state: row.state,
                        from: from.tag.clone(),
                        to: to.tag.clone(),
                        from_country: from,
                        to_country: to,
                        victory_points: vp_text(row.victory_points),
                        victory_points_read_hour: row.victory_points.map(|_| after),
                    },
                });
            }
            PeaceOutcomeRow::NEW_SUBJECT => {
                let (subject, overlord) = (joins.holders.tag_at(row.from, after), joins.holders.tag_at(row.to, after));
                match (overlord, subject, joins.subjects.get(&(row.to, row.from, after))) {
                    (Some(overlord), Some(subject), Some((start, level))) => items.push(PeaceItem {
                        derived: true,
                        dates: dates.clone(),
                        winner: Some(overlord.to_owned()),
                        action: neutral(PeaceActionKind::NewSubject),
                        change: PeaceChange::NewSubject {
                            overlord: overlord.to_owned(),
                            subject: subject.to_owned(),
                            autonomy_state: level.clone().unwrap_or_default(),
                            start_hour: *start,
                        },
                    }),
                    _ => deal.limitations.push(format!("A new subject at hour {after} is not recorded with its countries and relation; it is not shown.")),
                }
            }
            PeaceOutcomeRow::GOVERNMENT => {
                let Some(country) = joins.holders.tag_at(row.from, after) else {
                    deal.limitations.push(format!("A government change at hour {after} names a country that holds no tag; it is not shown."));
                    continue;
                };
                match (joins.governments.get(&(row.from, before)), joins.governments.get(&(row.from, after))) {
                    (Some(old), Some(new)) => items.push(PeaceItem {
                        derived: true,
                        dates: dates.clone(),
                        winner: None,
                        action: neutral(PeaceActionKind::GovernmentChange),
                        change: PeaceChange::GovernmentChange { country: country.to_owned(), before: old.clone(), after: new.clone() },
                    }),
                    _ => deal.limitations.push(format!("Government of {country} is not recorded at both reads.")),
                }
            }
            PeaceOutcomeRow::RELEASED => released.entry(row.to).or_default().push(row),
            _ => {}
        }
    }
    for (country, rows) in released {
        let Some(to_country) = joins.country(country, after) else {
            unnamed.extend(rows.iter().map(|row| row.state));
            continue;
        };
        let mut from = BTreeSet::new();
        let mut transfers = Vec::new();
        for row in &rows {
            let Some(owner) = joins.country(row.from, before) else {
                unnamed.insert(row.state);
                continue;
            };
            from.insert(owner.tag.clone());
            transfers.push(PeaceReleasedState {
                state: row.state,
                from_country: owner,
                victory_points: vp_text(row.victory_points),
                victory_points_read_hour: row.victory_points.map(|_| after),
            });
        }
        if transfers.is_empty() {
            continue;
        }
        transfers.sort_by_key(|transfer| transfer.state);
        items.push(PeaceItem {
            derived: true,
            dates: dates.clone(),
            winner: None,
            action: neutral(PeaceActionKind::CountryReleased),
            change: PeaceChange::CountryReleased {
                country: to_country.tag.clone(),
                from: from.into_iter().collect(),
                states: transfers.iter().map(|transfer| transfer.state).collect(),
                to_country,
                transfers,
            },
        });
    }
    if !unnamed.is_empty() {
        deal.limitations.push(format!("States {unnamed:?} changed hands between countries that hold no tag at hours {before} or {after}; the outcome list is incomplete."));
    }
    if items.is_empty() {
        deal.limitations.push("No attributable transfers were observed; an empty list does not prove a white peace or no settlement actions.".into());
    }
    if winners.is_empty() {
        deal.limitations.push("Without qualifying transfers, participant sides and subject/government-only settlements cannot be attributed.".into());
    }
    if before.checked_add(1) != Some(after) {
        deal.limitations.push("The outcome reads span several hours; only their net difference is known, not individual action times.".into());
    }
    deal.winners = winners.into_iter().collect();
    deal.losers = losers.into_iter().collect();
    items.sort_by(|a, b| item_order(a).cmp(&item_order(b)));
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

fn conference_card(span: &PeaceConferenceSpanRow, rows: &[&Outcome], joins: &Joins) -> PeaceDeal {
    let mut deal = card(format!("conference:{}:{}", span.conference, span.first_observed_hour), PeaceObservation::RecordedConference, span.first_observed_hour);
    deal.conference_id = Some(span.conference.to_string());
    deal.last_observed_hour = span.last_observed_hour;
    deal.current_turn = Some(span.current_turn);
    deal.finalized = Some(span.finalized != 0);
    deal.first_finalized_hour = span.first_finalized_hour;
    deal.first_absent_hour = span.first_absent_hour;
    match (span.first_absent_hour, span.first_finalized_hour) {
        (Some(absent), _) => {
            deal.end_is_by = span.last_observed_hour.checked_add(1) != Some(absent);
            deal.end_hour = Some(if deal.end_is_by { absent } else { span.last_observed_hour });
        }
        (None, Some(finalized)) => {
            deal.end_hour = Some(finalized);
            deal.end_is_by = true;
        }
        (None, None) => {}
    }
    match span.comparison {
        PeaceConferenceSpanRow::COMPARED => populate(&mut deal, rows, joins),
        PeaceConferenceSpanRow::NO_BEFORE => deal.limitations.push("No complete ownership and war read before settlement; outcomes are not recorded.".into()),
        PeaceConferenceSpanRow::OVERLAPPED => deal.limitations.push("Sampled conferences overlap this comparison; outcomes cannot be assigned to one.".into()),
        _ if deal.end_hour.is_some() => deal.limitations.push("No complete ownership and war read after settlement; outcomes are not recorded.".into()),
        _ => deal.limitations.push("No end was observed; the conference outcome is not recorded.".into()),
    }
    deal
}

fn outcome_card(rows: &[&Outcome], joins: &Joins) -> PeaceDeal {
    let first = rows[0];
    let observation = if first.observation == PeaceOutcomeRow::NOT_RECORDED { PeaceObservation::ConferenceNotRecorded } else { PeaceObservation::ConferenceUnknown };
    let mut deal = card(format!("outcome:{}:{}", first.after, first.group), observation, first.after);
    deal.end_hour = Some(first.after);
    deal.end_is_by = true;
    populate(&mut deal, rows, joins);
    deal.limitations.push("Connected war-end transfer pairs group this outcome; the data cannot prove how many conferences produced it.".into());
    deal
}

fn captured_action(action: &SettledPeaceAction, joins: &Joins, hour: i32) -> PeaceCapturedAction {
    let tag = |raw: u32| joins.raw_tag(raw, hour);
    PeaceCapturedAction {
        derived: false,
        country: action.country,
        country_tag: tag(action.country),
        turn: action.turn,
        ordinal: action.ordinal,
        token: action.token,
        token_name: joins.tokens.get(&action.token).cloned(),
        name_key: action.name_key.as_ref().and_then(|key| std::str::from_utf8(key).ok().map(str::to_owned)),
        name: None,
        name_country: None,
        cached_name: action.cached_name.clone(),
        taker: action.taker,
        giver: action.giver,
        negotiator: action.negotiator,
        taker_tag: tag(action.taker),
        giver_tag: tag(action.giver),
        negotiator_tag: tag(action.negotiator),
        status: action.status,
        cost: action.cost,
        form: action.form,
        state: action.state,
        flags: action.flags,
        country_operand: action.country_operand,
        country_operand_tag: action.country_operand.and_then(tag),
        puppet_auxiliary: action.puppet_auxiliary.map(|value| value.to_string()),
        ship: action.ship.map(|value| value.to_string()),
        ratio: action.ratio.map(|value| value.to_string()),
    }
}

#[derive(Default)]
struct GraphBuilder {
    links: HashMap<(Identity, Identity), Vec<PeaceTransferredState>>,
}

impl GraphBuilder {
    fn add_with(&mut self, item: &PeaceItem, released: bool, keep: &dyn Fn(u32) -> bool) {
        match &item.change {
            PeaceChange::StateTransfer { state, from_country, to_country, victory_points, victory_points_read_hour, .. } => self.push(PeaceTransferredState {
                state: *state,
                from_country: from_country.clone(),
                to_country: to_country.clone(),
                dates: item.dates.clone(),
                victory_points: victory_points.clone(),
                victory_points_read_hour: *victory_points_read_hour,
                released,
            }),
            PeaceChange::CountryReleased { to_country, transfers, .. } => {
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
        self.links.entry((state.from_country.identity(), state.to_country.identity())).or_default().push(state);
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
                for (id, country, side) in [(&source, &state.from_country, PeaceTransferSide::Source), (&target, &state.to_country, PeaceTransferSide::Target)] {
                    let node = nodes.entry(id.clone()).or_insert_with(|| PeaceTransferNode { id: id.clone(), country: country.clone(), side });
                    if country.hour > node.country.hour {
                        node.country = country.clone();
                    }
                }
            }
            let vp = states.iter().try_fold(0_i64, |total, state| total.checked_add(state.victory_points.as_ref()?.parse::<i64>().ok()?));
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

pub fn fold(spans: &[PeaceConferenceSpanRow], outcomes: &[Outcome], settled: &[Settled], joins: &Joins) -> (Vec<PeaceDeal>, PeaceTransferGraph, Vec<PeaceWarning>) {
    let mut warnings = Vec::new();
    let mut recorded: HashMap<u64, Vec<&Outcome>> = HashMap::new();
    let mut unsampled: BTreeMap<(i32, u32), Vec<&Outcome>> = BTreeMap::new();
    for row in outcomes {
        match (row.observation, row.conference) {
            (PeaceOutcomeRow::RECORDED, Some(conference)) => recorded.entry(conference).or_default().push(row),
            (PeaceOutcomeRow::RECORDED, None) => warnings.push(PeaceWarning::at(row.after, format!("A compared conference's outcome at hour {} names no conference; it is not shown.", row.after))),
            _ => unsampled.entry((row.after, row.group)).or_default().push(row),
        }
    }
    let mut deals: Vec<PeaceDeal> = Vec::new();
    for span in spans {
        let rows: Vec<&Outcome> = recorded.get(&span.conference).into_iter().flatten().copied().filter(|row| Some(row.after) == span.compared_hour).collect();
        deals.push(conference_card(span, &rows, joins));
    }
    for rows in unsampled.values() {
        deals.push(outcome_card(rows, joins));
    }
    let vps: HashMap<(u32, i32), i64> = outcomes.iter().filter_map(|row| Some(((row.state, row.after), row.victory_points?))).collect();
    let releases: HashMap<(u32, i32), Identity> = deals
        .iter()
        .flat_map(|deal| &deal.items)
        .filter(|item| item.derived)
        .flat_map(|item| match &item.change {
            PeaceChange::CountryReleased { to_country, transfers, .. } => transfers.iter().map(|transfer| ((transfer.state, item.dates.after_hour), to_country.identity())).collect::<Vec<_>>(),
            _ => Vec::new(),
        })
        .collect();
    let mut indices: HashMap<String, Vec<(i32, usize)>> = HashMap::new();
    for (at, deal) in deals.iter().enumerate() {
        if let Some(id) = &deal.conference_id {
            indices.entry(id.clone()).or_default().push((deal.first_observed_hour, at));
        }
    }
    for listings in indices.values_mut() {
        listings.sort_unstable();
    }
    let mut exact_states: HashSet<(u32, i32)> = HashSet::new();
    let mut released_exact: HashSet<(u32, i32, Identity)> = HashSet::new();
    let mut kept_releases: HashMap<usize, Vec<PeaceItem>> = HashMap::new();
    let mut seen = HashSet::new();
    for Settled { publication_hour, row } in settled {
        if !seen.insert(row.sequence) {
            continue;
        }
        let publication_hour = *publication_hour;
        let Some(id) = row.conference else {
            warnings.push(PeaceWarning::at(row.hour.unwrap_or(publication_hour), format!("A finalizer conference identity was unavailable: {}", String::from_utf8_lossy(row.failure.as_deref().unwrap_or_default()))));
            continue;
        };
        let id = id.to_string();
        let listings = indices.entry(id.clone()).or_default();
        let ends = row.hour.unwrap_or(publication_hour);
        let index = match listings.iter().rev().find(|&&(first, _)| first <= ends).or(listings.first()) {
            Some(&(_, at)) => at,
            None => {
                let mut deal = card(format!("captured:{id}:{}", row.sequence), PeaceObservation::RecordedConference, ends);
                deal.conference_id = Some(id.clone());
                deal.capture_observed_hour = Some(publication_hour);
                deals.push(deal);
                listings.push((ends, deals.len() - 1));
                deals.len() - 1
            }
        };
        let deal = &mut deals[index];
        let Some(hour) = row.hour.filter(|_| row.failure.is_none() && row.omitted == 0) else {
            let reason = format!(
                "Finalizer capture unavailable or truncated (at least {} unread operands): {}",
                row.omitted,
                row.failure.as_deref().map(String::from_utf8_lossy).unwrap_or_else(|| if row.hour.is_none() { "finalizer hour unavailable".into() } else { "capture capacity exceeded".into() })
            );
            deal.capture_unavailable = Some(reason.clone());
            deal.capture_unavailable_hour = Some(publication_hour);
            deal.limitations.push(reason);
            continue;
        };
        deal.derived = false;
        deal.capture_unavailable = None;
        deal.capture_unavailable_hour = None;
        deal.participants_derived = false;
        deal.finalized = Some(true);
        deal.first_finalized_hour = Some(hour);
        deal.last_observed_hour = deal.last_observed_hour.max(hour);
        deal.end_hour = Some(hour);
        deal.end_is_by = false;
        deal.limitations = vec!["Finalization destroys ordered actions; a completed save can validate retained outcomes only.".into()];
        deal.winners = row.winners.iter().filter_map(|&raw| joins.raw_tag(raw, hour)).collect();
        deal.losers = row.losers.iter().filter_map(|&raw| joins.raw_tag(raw, hour)).collect();
        if deal.winners.len() != row.winners.len() || deal.losers.len() != row.losers.len() {
            deal.limitations.push("Some participant tags are not recorded at the finalizer hour; their raw references are preserved.".into());
        }
        let actions: Vec<PeaceCapturedAction> = row.actions.iter().map(|action| captured_action(action, joins, hour)).collect();
        let mut givers: HashMap<i32, Option<u32>> = HashMap::new();
        for action in &actions {
            if let Some(state) = action.state {
                givers.entry(state).and_modify(|old| if *old != Some(action.giver) { *old = None }).or_insert(Some(action.giver));
            }
        }
        let transfers: Vec<PeaceCapturedTransfer> = row
            .transfers
            .iter()
            .map(|transfer| {
                let giver = if transfer.automatic { None } else { givers.get(&transfer.state).copied().flatten() };
                PeaceCapturedTransfer {
                    derived: false,
                    hour,
                    state: transfer.state,
                    taker: transfer.taker,
                    to: joins.raw_tag(transfer.taker, hour),
                    giver,
                    from: giver.and_then(|raw| joins.raw_tag(raw, hour)),
                    from_country: giver.and_then(|raw| joins.raw_country(raw, hour)),
                    to_country: joins.raw_country(transfer.taker, hour),
                    automatic: transfer.automatic,
                }
            })
            .collect();
        let fallback = std::mem::take(&mut deal.items);
        let undrawable: HashSet<u32> = transfers.iter().filter(|t| t.from_country.is_none() || t.to_country.is_none()).filter_map(|t| u32::try_from(t.state).ok()).collect();
        for transfer in &transfers {
            let (Some(from_country), Some(to_country), Ok(state)) = (&transfer.from_country, &transfer.to_country, u32::try_from(transfer.state)) else { continue };
            exact_states.insert((state, publication_hour));
            if releases.get(&(state, publication_hour)) == Some(&to_country.identity()) {
                released_exact.insert((state, hour, to_country.identity()));
            }
            let vp = vps.get(&(state, publication_hour)).copied();
            deal.items.push(PeaceItem {
                derived: false,
                dates: PeaceDates { before_hour: hour, after_hour: hour },
                winner: Some(to_country.tag.clone()),
                action: PeaceActionLabel { kind: PeaceActionKind::StateTransfer, key: joins.key(), name: None },
                change: PeaceChange::StateTransfer {
                    state,
                    from: from_country.tag.clone(),
                    to: to_country.tag.clone(),
                    from_country: from_country.clone(),
                    to_country: to_country.clone(),
                    victory_points: vp_text(vp),
                    victory_points_read_hour: vp.map(|_| publication_hour),
                },
            });
        }
        let kept: Vec<PeaceItem> = fallback
            .iter()
            .filter(|item| item.derived && item.dates.after_hour == publication_hour)
            .filter_map(|item| {
                let mut kept = item.clone();
                let PeaceChange::CountryReleased { transfers, .. } = &mut kept.change else { return None };
                transfers.retain(|transfer| undrawable.contains(&transfer.state));
                (!transfers.is_empty()).then_some(kept)
            })
            .collect();
        if !kept.is_empty() {
            kept_releases.entry(index).or_default().extend(kept);
        }
        deal.items.extend(fallback.into_iter().filter(|item| {
            item.derived && item.dates.after_hour == publication_hour && matches!(&item.change, PeaceChange::StateTransfer { state, .. } if undrawable.contains(state))
        }));
        if !undrawable.is_empty() {
            deal.limitations.push(format!(
                "{} transfers with unrecorded event-hour endpoint identities remain in captured transfers; the country graph cannot draw those endpoints.",
                transfers.iter().filter(|t| t.from_country.is_none() || t.to_country.is_none()).count()
            ));
        }
        deal.captured = Some(PeaceCaptured {
            hour,
            publication_hour,
            winners: row.winners.clone(),
            losers: row.losers.clone(),
            actions,
            transfers,
            subjects: row
                .subjects
                .iter()
                .map(|subject| PeaceCapturedSubject {
                    derived: false,
                    overlord: subject.overlord,
                    subject: subject.subject,
                    overlord_tag: joins.raw_tag(subject.overlord, hour),
                    subject_tag: joins.raw_tag(subject.subject, hour),
                })
                .collect(),
        });
    }
    let mut campaign = GraphBuilder::default();
    for (index, deal) in deals.iter_mut().enumerate() {
        deal.items.retain(|item| !item.derived || !matches!(&item.change, PeaceChange::StateTransfer { state, .. } if exact_states.contains(&(*state, item.dates.after_hour))));
        let kept = kept_releases.remove(&index).unwrap_or_default();
        let mut graph = GraphBuilder::default();
        for item in deal.items.iter().chain(&kept) {
            let released = match &item.change {
                PeaceChange::StateTransfer { state, to_country, .. } => !item.derived && released_exact.contains(&(*state, item.dates.after_hour, to_country.identity())),
                _ => false,
            };
            let keep = |state: u32| !exact_states.contains(&(state, item.dates.after_hour));
            graph.add_with(item, released, &keep);
            campaign.add_with(item, released, &keep);
        }
        deal.transfers = graph.finish();
    }
    deals.retain(|deal| !deal.derived || deal.conference_id.is_some() || !deal.items.is_empty());
    deals.sort_by(|a, b| b.end_hour.unwrap_or(b.last_observed_hour).cmp(&a.end_hour.unwrap_or(a.last_observed_hour)).then_with(|| a.id.cmp(&b.id)));
    if let Err(reason) = &joins.action_key {
        warnings.push(PeaceWarning { hour: None, text: format!("State transfers are not named by their recorded keyword: {reason}") });
    }
    (deals, campaign.finish(), warnings)
}

fn is_keyword(spelling: &str, keyword: &str) -> bool {
    content_reader_script::native::Token { kind: content_reader_script::native::Kind::Bare, text: spelling.as_bytes().to_vec(), line: 0, quoted: false, byte_end: 0 }.is_keyword(keyword)
}

fn takes_states(spelling: &str) -> bool {
    is_keyword(spelling, "take_states")
}

fn demand_label(spelling: &str) -> Option<&'static str> {
    DEMAND_LABELS.iter().find(|(keyword, _)| is_keyword(spelling, keyword)).map(|(_, label)| *label)
}

fn name_actions(reply: &mut PeaceDealsReply, localisation: &content_reader_localisation::localisation::Localisation, recorded_language: &str) {
    let mut names: HashMap<String, Option<String>> = HashMap::new();
    let mut refused = catalogue::Refusals::default();
    let mut localise = |key: &str, refused: &mut catalogue::Refusals| {
        names
            .entry(key.to_owned())
            .or_insert_with(|| {
                if localisation.get(key).is_none() {
                    refused.note("localisation key is not in the recorded content", key);
                    return None;
                }
                let country = [("COUNTRY", content_reader_localisation::localisation::Argument::Text(NAME_COUNTRY))];
                let arguments: &[_] = if key == PUPPET_LABEL { &country } else { &[] };
                catalogue::localised_with(localisation, key, arguments, catalogue::View::Scripted).inspect_err(|reason| refused.note(reason.clone(), key)).ok()
            })
            .clone()
    };
    for deal in &mut reply.deals {
        if let Some(captured) = &mut deal.captured {
            for action in &mut captured.actions {
                let spellings = [action.token_name.as_deref(), action.name_key.as_deref()];
                if let Some(label) = spellings.into_iter().flatten().find_map(demand_label) {
                    action.name = match (label, &action.country_operand_tag) {
                        (PUPPET_LABEL, None) => {
                            refused.note("the country it puppets is not recorded at its hour", label);
                            None
                        }
                        (PUPPET_LABEL, Some(tag)) => {
                            action.name_country = Some(tag.clone());
                            localise(label, &mut refused)
                        }
                        _ => localise(label, &mut refused),
                    };
                } else if let Some(bytes) = action.cached_name.as_ref() {
                    action.name = dashboard_text::scripted_text::stored_text(localisation, recorded_language, bytes, "cached peace-action type name")
                        .and_then(|text| catalogue::plain_text(&text))
                        .and_then(|text| match spellings.into_iter().flatten().any(|spelling| is_keyword(spelling, &text)) {
                            true => Err(format!("the cached name of peace-action type {text} is its keyword, which the game never shows as a name")),
                            false => Ok(text),
                        })
                        .inspect_err(|reason| refused.note(reason.clone(), action.token.to_string()))
                        .ok();
                } else if let Some(key) = action.name_key.as_ref() {
                    action.name = localise(key, &mut refused);
                } else {
                    reply.warnings.push(format!("Captured peace-action token {} has no proven native naming operand; its name is not recorded.", action.token));
                }
            }
        }
        for item in &mut deal.items {
            if item.action.key.is_none() {
                continue;
            }
            item.action.name = if item.action.kind == PeaceActionKind::StateTransfer { localise(TAKE_STATES_LABEL, &mut refused) } else { item.action.key.clone().and_then(|key| localise(&key, &mut refused)) };
        }
    }
    reply.warnings.extend(refused.lines("peace action names"));
}

fn complement(reads: &[(i32, i32)], first: i32, last: i32, out: &mut Vec<HourSpan>) {
    let mut at = i64::from(first);
    for &(a, b) in reads {
        if i64::from(a) > at {
            out.push(HourSpan { first_hour: at as i32, last_hour: a - 1 });
        }
        at = at.max(i64::from(b) + 1);
    }
    if at <= i64::from(last) {
        out.push(HourSpan { first_hour: at as i32, last_hour: last });
    }
}

fn number(cells: &[log::Cell], at: usize) -> Option<i64> {
    match cells.get(at) {
        Some(log::Cell::Number(value)) => Some(*value),
        _ => None,
    }
}

fn logged(query: &Query<'_>, name: &str) -> io::Result<Vec<log::Row>> {
    let declared = dashboard_declare::log(name).ok_or_else(|| invalid(format!("No log is declared as {name}")))?;
    let reader = log::Reader::new(query, declared)?;
    let (first, last) = (query.timeline.first, query.timeline.last);
    let selection = reader.select(&[], first, last)?;
    let page = reader.page(&selection.patterns, selection.groups.len(), first, last, 0, u32::MAX)?;
    Ok(page.rows.into_iter().map(|paged| paged.row).collect())
}

fn slot_holders(query: &Query<'_>) -> HashMap<u32, Vec<(u32, String, i32, i32)>> {
    let mut slots: HashMap<u32, Vec<(u32, String, i32, i32)>> = HashMap::new();
    for (tag, runs) in &query.holders.tags {
        for run in runs {
            if let Holder::Found { slot, id } = run.holder {
                slots.entry(slot).or_default().push((id, tag.clone(), run.first, run.last));
            }
        }
    }
    slots
}

impl Product for PeaceDeals {
    const KEY: &'static str = "peace-deals";
    const CONTENT_DEPENDENT: bool = true;
    type Params = PeaceDealsParams;
    const FIELDS: dashboard_base::products::Fields = dashboard_base::fields![branchId];
    type Reply = PeaceDealsReply;

    #[cfg(any(test, feature = "export-types"))]
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
            HourSpan::decl(),
        ]
    }

    fn validate(params: &Self::Params) -> Result<(), &'static str> {
        params.branch_id.parse::<u64>().map(|_| ()).map_err(|_| "Invalid peace-deals branch")
    }

    fn load(source: &Source, params: &Self::Params, context: &LoadContext, cancelled: &AtomicBool) -> io::Result<Self::Reply> {
        let branch = params.branch_id.parse::<u64>().map_err(|_| invalid("Invalid peace-deals branch"))?;
        let assembled = assemble(source, branch, cancelled)?;
        let derived = assembled.deals.iter().any(|deal| deal.derived || deal.items.iter().any(|item| item.derived));
        let warnings = std::iter::once(DERIVED_WARNING.to_owned()).chain(assembled.warnings.into_iter().map(|warning| warning.text)).collect();
        let mut reply = PeaceDealsReply { branch_id: params.branch_id.clone(), derived, deals: assembled.deals, transfers: assembled.transfers, unobserved: assembled.unobserved, warnings };
        match context.content.as_ref() {
            None => reply.warnings.push("Peace action names are unavailable until the game and user folders are chosen.".into()),
            Some(options) => match catalogue::catalogue(source, options) {
                Ok(catalogue) => {
                    reply.warnings.extend(catalogue.verification.iter().cloned());
                    reply.warnings.extend(catalogue.warnings.iter().cloned());
                    name_actions(&mut reply, &catalogue.localisation, &catalogue.recorded_language);
                }
                Err(error) => reply.warnings.push(format!("Peace action names are unavailable: {error}")),
            },
        }
        dashboard_base::products::check_reply_budget(&reply)?;
        Ok(reply)
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PeaceWarning {
    pub hour: Option<i32>,
    pub text: String,
}

impl PeaceWarning {
    fn at(hour: i32, text: String) -> Self {
        Self { hour: Some(hour), text }
    }
}

pub(crate) struct Assembled {
    pub(crate) deals: Vec<PeaceDeal>,
    pub(crate) transfers: PeaceTransferGraph,
    pub(crate) unobserved: PeaceUnobserved,
    pub(crate) warnings: Vec<PeaceWarning>,
}

pub(crate) fn assemble(source: &Source, branch: u64, cancelled: &AtomicBool) -> io::Result<Assembled> {
    let query = dashboard_base::queries::query(source, branch)?;
    let recording = query.replay.recording();
    let mut warnings = Vec::new();
    let mut unobserved = PeaceUnobserved::default();
    let mut rows = Vec::new();
    for stretch in &query.timeline.stretches {
        dashboard_text::metadata::check_cancel(cancelled)?;
        let (first, last, run) = (stretch.first, stretch.last, stretch.run);
        complement(&replay_lanes::lanes::reads(recording, run, TableId::PeaceSettlements, first, last)?, first, last, &mut unobserved.settlements);
        complement(&replay_lanes::lanes::reads(recording, run, TableId::PeaceConferences, first, last)?, first, last, &mut unobserved.conferences);
        complement(&replay_lanes::lanes::reads(recording, run, TableId::PeaceOutcomes, first, last)?, first, last, &mut unobserved.ownership);
    }
    let span_columns = SPAN_COLUMNS.map(|name| dashboard_declare::field(TableId::PeaceConferenceSpans, name).unwrap_or(usize::MAX));
    if span_columns.contains(&usize::MAX) {
        return Err(invalid("peace conference spans does not hold the columns the peace deals read"));
    }
    for row in logged(&query, "peace-conference-spans")? {
        match conference_span(&row, &span_columns) {
            Some(span) => rows.push(span),
            None => warnings.push(PeaceWarning::at(row.hour, format!("A peace conference's span at hour {} is not recorded in full; it is not shown.", row.hour))),
        }
    }
    for gap in &query.timeline.unheld {
        for held in [&mut unobserved.settlements, &mut unobserved.conferences, &mut unobserved.ownership] {
            held.push(HourSpan { first_hour: gap.first, last_hour: gap.last });
        }
    }
    for held in [&mut unobserved.settlements, &mut unobserved.conferences, &mut unobserved.ownership] {
        *held = dashboard_world::products::subjects::settle(std::mem::take(held));
    }
    let ownership = unobserved.ownership.clone();
    for held in [&mut unobserved.wars, &mut unobserved.subjects, &mut unobserved.governments, &mut unobserved.existence, &mut unobserved.victory_points] {
        held.clone_from(&ownership);
    }
    let columns: [usize; 11] = [
        "observation", "conference", "before_hour", "change", "group", "state", "from_slot", "from_id", "to_slot", "to_id", "victory_points",
    ]
    .map(|name| dashboard_declare::field(TableId::PeaceOutcomes, name).unwrap_or(usize::MAX));
    if columns.contains(&usize::MAX) {
        return Err(invalid("peace outcomes does not hold the columns the peace deals read"));
    }
    let mut outcomes = Vec::new();
    for row in logged(&query, "peace-outcomes")? {
        match outcome(&row, &columns) {
            Some(outcome) => outcomes.push(outcome),
            None => warnings.push(PeaceWarning::at(row.hour, format!("A peace outcome at hour {} is not recorded in full; it is not shown.", row.hour))),
        }
    }
    dashboard_text::metadata::check_cancel(cancelled)?;
    let settlement_columns = SETTLEMENT_COLUMNS.map(|name| dashboard_declare::field(TableId::PeaceSettlements, name).unwrap_or(usize::MAX));
    if settlement_columns.contains(&usize::MAX) {
        return Err(invalid("peace settlements does not hold the columns the peace deals read"));
    }
    let mut settled = Vec::new();
    for row in logged(&query, "peace-settlements")? {
        match settlement(&row, &settlement_columns) {
            Some(settlement) => settled.push(Settled { publication_hour: row.hour, row: settlement }),
            None => warnings.push(PeaceWarning::at(row.hour, format!("A peace settlement at hour {} is not recorded in full; it is not shown.", row.hour))),
        }
    }
    dashboard_text::metadata::check_cancel(cancelled)?;
    let joins = joins(source, &query, &outcomes, &settled, &mut warnings)?;
    let (deals, transfers, more) = fold(&listings(rows), &outcomes, &settled, &joins);
    warnings.extend(more);
    Ok(Assembled { deals, transfers, unobserved, warnings })
}

fn listings(rows: Vec<PeaceConferenceSpanRow>) -> Vec<PeaceConferenceSpanRow> {
    let mut spans: BTreeMap<(u64, i32), PeaceConferenceSpanRow> = BTreeMap::new();
    for span in rows {
        let listing = (span.conference, span.first_observed_hour);
        if spans.get(&listing).is_none_or(|held| held.hour <= span.hour) {
            spans.insert(listing, span);
        }
    }
    spans.into_values().collect()
}

const SETTLEMENT_COLUMNS: [&str; 9] = ["conference", "hour", "winners", "losers", "actions", "transfers", "failure", "omitted", "subjects"];

fn settlement(row: &log::Row, columns: &[usize; 9]) -> Option<PeaceSettlementRow> {
    let cell = |index: usize| row.cells.get(columns[index]);
    let optional = |index: usize| match cell(index) {
        Some(log::Cell::Number(value)) => Some(Some(*value)),
        Some(log::Cell::Missing) => Some(None),
        _ => None,
    };
    fn list(cell: Option<&log::Cell>) -> Option<Option<&[u8]>> {
        match cell {
            Some(log::Cell::List(bytes)) if !bytes.is_empty() => Some(Some(&bytes[..])),
            Some(log::Cell::List(_) | log::Cell::Missing) => Some(None),
            _ => None,
        }
    }
    let u32s = |index: usize| -> Option<Vec<u32>> {
        let bytes = list(cell(index))?.unwrap_or_default();
        (bytes.len() % 4 == 0).then(|| bytes.chunks_exact(4).map(|item| u32::from_le_bytes([item[0], item[1], item[2], item[3]])).collect())
    };
    fn decoded<T: replay_data::value::Value>(bytes: Option<&[u8]>) -> Option<T> {
        T::decode(&mut replay_data::codec::Decoder::new(bytes?)).ok()
    }
    fn listed<T: replay_data::value::Value + Default>(bytes: Option<&[u8]>) -> Option<T> {
        match bytes {
            Some(bytes) => T::decode(&mut replay_data::codec::Decoder::new(bytes)).ok(),
            None => Some(T::default()),
        }
    }
    Some(PeaceSettlementRow {
        sequence: row.key as u64,
        conference: optional(0)?.map(|value| value as u64),
        hour: optional(1)?.map(|value| value as i32),
        winners: u32s(2)?,
        losers: u32s(3)?,
        actions: listed(list(cell(4))?)?,
        transfers: listed(list(cell(5))?)?,
        failure: match list(cell(6))? {
            None => None,
            bytes => decoded(bytes)?,
        },
        omitted: number(&row.cells, columns[7])? as u64,
        subjects: listed(list(cell(8))?)?,
    })
}

const SPAN_COLUMNS: [&str; 9] = [
    "first_observed_hour", "last_observed_hour", "current_turn", "finalized", "first_finalized_hour", "first_absent_hour", "comparison", "before_hour", "compared_hour",
];

fn conference_span(row: &log::Row, columns: &[usize; 9]) -> Option<PeaceConferenceSpanRow> {
    let at = |index: usize| number(&row.cells, columns[index]);
    let optional = |index: usize| match row.cells.get(columns[index]) {
        Some(log::Cell::Number(value)) => Some(Some(*value as i32)),
        Some(log::Cell::Missing) => Some(None),
        _ => None,
    };
    Some(PeaceConferenceSpanRow {
        conference: (row.key >> 32) as u64,
        hour: row.key as u32 as i32,
        first_observed_hour: at(0)? as i32,
        last_observed_hour: at(1)? as i32,
        current_turn: at(2)? as i32,
        finalized: at(3)? as u8,
        first_finalized_hour: optional(4)?,
        first_absent_hour: optional(5)?,
        comparison: at(6)? as u8,
        before_hour: optional(7)?,
        compared_hour: optional(8)?,
    })
}

fn outcome(row: &log::Row, columns: &[usize; 11]) -> Option<Outcome> {
    let at = |index: usize| number(&row.cells, columns[index]);
    Some(Outcome {
        after: (row.key >> 32) as u32 as i32,
        ordinal: row.key as u32,
        observation: at(0)? as u8,
        conference: at(1).map(|value| value as u64),
        before: at(2)? as i32,
        change: at(3)? as u8,
        group: at(4)? as u32,
        state: at(5)? as u32,
        from: (at(6)? as u32, at(7)? as u32),
        to: (at(8)? as u32, at(9)? as u32),
        victory_points: at(10),
    })
}

fn joins(source: &Source, query: &Query<'_>, outcomes: &[Outcome], settled: &[Settled], warnings: &mut Vec<PeaceWarning>) -> io::Result<Joins> {
    let recording = query.replay.recording();
    let mut joins = Joins { holders: HolderTags::new(query), ..Joins::default() };
    let slots = slot_holders(query);
    for Settled { publication_hour, row } in settled {
        let hour = row.hour.unwrap_or(*publication_hour);
        let Some(stretch) = query.timeline.at(hour).or_else(|| query.timeline.at(*publication_hour)) else { continue };
        let links = dashboard_base::queries::links(source, stretch.run)?;
        let raws = row
            .winners
            .iter()
            .chain(&row.losers)
            .copied()
            .chain(row.actions.iter().flat_map(|action| [action.country, action.taker, action.giver, action.negotiator]))
            .chain(row.actions.iter().filter_map(|action| action.country_operand))
            .chain(row.transfers.iter().map(|transfer| transfer.taker))
            .chain(row.subjects.iter().flat_map(|subject| [subject.overlord, subject.subject]));
        for raw in raws {
            if joins.raw.contains_key(&(raw, hour)) {
                continue;
            }
            let Some(slot) = links.slot_at(raw, hour) else { continue };
            let holder = slots.get(&slot).and_then(|runs| runs.iter().find(|run| run.2 <= hour && hour <= run.3));
            if let Some((id, tag, ..)) = holder {
                joins.raw.insert((raw, hour), ((slot, *id), tag.clone()));
            }
        }
    }
    let tokens: BTreeSet<u32> = settled.iter().flat_map(|settled| settled.row.actions.iter().map(|action| action.token)).collect();
    let transfers = !settled.is_empty() || outcomes.iter().any(|row| row.change == PeaceOutcomeRow::TRANSFER);
    if transfers && let Some(stretch) = query.timeline.stretches.last() {
        let frame = replay_lanes::at::frame(recording, stretch.run, stretch.last, &[read::Read::all(TableId::LexicalTokens)])?;
        match frame.complete::<tables::LexicalTokens>() {
            Ok(rows) => {
                let mut found: Option<&[u8]> = None;
                let mut conflict = false;
                for row in rows {
                    if tokens.contains(&row.token)
                        && let Ok(text) = std::str::from_utf8(&row.literal)
                    {
                        joins.tokens.insert(row.token, text.to_owned());
                    }
                    if std::str::from_utf8(&row.literal).is_ok_and(takes_states) {
                        conflict |= found.is_some_and(|held| held != row.literal.as_slice());
                        found = Some(&row.literal);
                    }
                }
                joins.action_key = match (found, conflict) {
                    (_, true) => Err("lexical tokens records conflicting state-taking keywords".into()),
                    (Some(literal), false) => Ok(String::from_utf8_lossy(literal).into_owned()),
                    (None, false) => Err("lexical tokens does not record the native state-taking keyword".into()),
                };
            }
            Err(error) => joins.action_key = Err(format!("lexical tokens: {error}")),
        }
    }
    for row in outcomes.iter().filter(|row| row.change == PeaceOutcomeRow::NEW_SUBJECT) {
        let Some(stretch) = query.timeline.at(row.after) else { continue };
        let subjects = dashboard_world::products::subjects::cached_subjects(source, stretch.run)?;
        if let Some(relation) = subjects.relation_at(row.to.1, row.from.1, row.after) {
            joins.subjects.insert((row.to, row.from, row.after), relation);
        }
    }
    let mut wanted: BTreeMap<i32, BTreeSet<Identity>> = BTreeMap::new();
    for row in outcomes.iter().filter(|row| row.change == PeaceOutcomeRow::GOVERNMENT) {
        wanted.entry(row.before).or_default().insert(row.from);
        wanted.entry(row.after).or_default().insert(row.from);
    }
    for (hour, countries) in wanted {
        let Some(stretch) = query.timeline.at(hour) else { continue };
        let keys: Vec<u128> = countries.iter().map(|identity| u128::from(identity.1)).collect();
        let reads = [
            read::Read { table: TableId::Politics, columns: None, keys: read::Keys::These(&keys) },
            read::Read::all(TableId::PoliticalParties),
            read::Read { table: TableId::CountryAppearance, columns: None, keys: read::Keys::These(&keys) },
        ];
        let frame = replay_lanes::at::frame(recording, stretch.run, hour, &reads)?;
        let (politics, parties, appearances) = match (frame.complete::<tables::Politics>(), frame.complete::<tables::PoliticalParties>(), frame.complete::<tables::CountryAppearance>()) {
            (Ok(politics), Ok(parties), Ok(appearances)) => (politics, parties, appearances),
            _ => {
                warnings.push(PeaceWarning::at(hour, format!("Governments are not recorded at hour {hour}; government changes there are not shown.")));
                continue;
            }
        };
        for identity in countries {
            let Some(ruling) = politics.iter().find(|row| row.country == identity.1) else { continue };
            let party_token = ruling.ruling_party.and_then(|party| parties.iter().find(|row| row.country == identity.1 && row.id == party)).and_then(|row| std::str::from_utf8(&row.token).ok()).map(str::to_owned);
            let leader_ideology = appearances.iter().find(|row| row.country == identity.1).and_then(|row| std::str::from_utf8(&row.leader_ideology).ok()).map(str::to_owned);
            joins.governments.insert((identity, hour), PeaceGovernment { ruling_party: ruling.ruling_party, party_token, leader_ideology });
        }
    }
    Ok(joins)
}
