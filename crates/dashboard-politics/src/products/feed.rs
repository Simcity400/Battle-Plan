
mod cards;
mod cause;
pub(crate) mod decision_state;
mod families;
mod nuclear;
mod row;
pub mod scope;
mod text;
#[cfg(test)]
mod tests;
mod window;

pub use cards::{
    Decisions, DecisionsTail, EventChoices, EventChoicesTail, ScriptOccurrenceTailReply,
    decisions_dated_panel, event_choices_dated_panel,
};
pub use decision_state::{
    COUNTRY_DECISION_STATE_REFUSED, DECISION_CONTAINER_NOT_RECORDED, DECISION_INSTANCE_MISMATCH,
    DECISION_NO_MISSION_LIFECYCLE, DECISION_OCCURRENCE_NOT_FOUND, DecisionLifecycleState,
    EVENT_FEED_HISTORY_INCOMPLETE, EVENT_FEED_HISTORY_NOT_RECORDED,
};
pub(crate) use row::FeedEvent;

use dashboard_base::products::log_page as logs;
use dashboard_base::{products::{LoadContext, Product}, queries::country_ids::Names};
use dashboard_base::{Source, catalogue::{self, Catalogue}};
use dashboard_follow::panel::Panel;
use dashboard_query::{
    Query,
    log::{self, FacetFilter},
    series::MissingSpan,
};
use replay_data::rows::FeedEventRow;
use row::{Columns, Slot};
use serde::{Deserialize, Serialize};
use std::{io, sync::{Arc, Mutex, atomic::AtomicBool}};
use text::{KeptNaming, Notices, Presenter, kept_naming};
pub use text::warm_naming;
#[cfg(any(test, feature = "export-types"))]
use ts_rs::TS;

pub(crate) const LOG: &str = "feed-events";

#[derive(Clone, Copy, Debug, Deserialize, Serialize, PartialEq, Eq, PartialOrd, Ord, Hash)]
#[cfg_attr(any(test, feature = "export-types"), derive(TS))]
#[serde(rename_all = "camelCase")]
pub enum EventFeedCategory {
    WarsPeace,
    Diplomacy,
    Politics,
    Milestones,
}

#[derive(Clone, Copy, Debug, Deserialize, Serialize, PartialEq, Eq, PartialOrd, Ord, Hash)]
#[cfg_attr(any(test, feature = "export-types"), derive(TS))]
#[serde(rename_all = "camelCase")]
pub enum EventFeedKind {
    WarStarted,
    WarDeclared,
    WarJoined,
    CivilWarWarTransferred,
    Annexed,
    WarEnded,
    Released,
    StateTransferred,
    PeaceConferenceStarted,
    Peace,
    Capitulated,
    CeasedToExist,
    FactionJoined,
    FactionLeft,
    BecameSubject,
    GuaranteeGranted,
    RulingPartyChanged,
    LawChanged,
    AdvisorHired,
    FocusCompleted,
    ProjectFinished,
    NukesIncreased,
    NuclearStrike,
    ScriptEventFired,
    NewsEventFired,
    EventOptionSelected,
    DecisionSelected,
    DecisionRemoved,
    DecisionCancelled,
    DecisionTimedOut,
    DecisionActivated,
    DecisionCompleted,
    DecisionReenabled,
    DecisionRemovalEffect,
    DecisionCancellationEffect,
    DecisionCompletionEffect,
    DecisionTimeoutEffect,
    DecisionDurationChanged,
    EventWindowPresented,
}

const CODES: [(u8, EventFeedKind); 26] = {
    use EventFeedKind::*;
    [
        (0, WarDeclared),
        (1, WarJoined),
        (2, CivilWarWarTransferred),
        (3, Annexed),
        (4, WarEnded),
        (5, Released),
        (6, StateTransferred),
        (7, PeaceConferenceStarted),
        (8, ScriptEventFired),
        (9, NewsEventFired),
        (10, EventOptionSelected),
        (11, DecisionSelected),
        (12, DecisionRemoved),
        (13, DecisionCancelled),
        (14, DecisionTimedOut),
        (15, DecisionActivated),
        (16, DecisionCompleted),
        (18, DecisionReenabled),
        (19, DecisionRemovalEffect),
        (20, DecisionCancellationEffect),
        (21, DecisionCompletionEffect),
        (22, DecisionTimeoutEffect),
        (23, DecisionDurationChanged),
        (FeedEventRow::CEASED_TO_EXIST, CeasedToExist),
        (FeedEventRow::GUARANTEE_GRANTED, GuaranteeGranted),
        (FeedEventRow::WINDOW_PRESENTED, EventWindowPresented),
    ]
};

impl EventFeedKind {
    pub fn category(self) -> EventFeedCategory {
        use EventFeedKind::*;
        match self {
            WarStarted | WarDeclared | WarJoined | CivilWarWarTransferred | Annexed | WarEnded | Released | StateTransferred | PeaceConferenceStarted | Peace | Capitulated
            | CeasedToExist | NuclearStrike => EventFeedCategory::WarsPeace,
            FactionJoined | FactionLeft | BecameSubject | GuaranteeGranted => EventFeedCategory::Diplomacy,
            FocusCompleted | ProjectFinished | NukesIncreased => EventFeedCategory::Milestones,
            _ => EventFeedCategory::Politics,
        }
    }

    pub fn of_code(code: u8) -> Option<Self> {
        CODES.iter().find(|(held, _)| *held == code).map(|(_, kind)| *kind)
    }

    pub fn code(self) -> Option<u8> {
        CODES.iter().find(|(_, kind)| *kind == self).map(|(code, _)| *code)
    }

    pub fn scripted(self) -> bool {
        self >= EventFeedKind::ScriptEventFired
    }
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq)]
#[cfg_attr(any(test, feature = "export-types"), derive(TS))]
#[serde(rename_all = "camelCase")]
pub struct EventFeedRow {
    pub id: String,
    pub hour: i32,
    #[doc = " The read supplying this fact; the head never shows it before this hour."]
    pub read_hour: i32,
    #[doc = " Country naming and flags use this hour: the event's, or a vanished country's last seen."]
    pub name_hour: i32,
    pub exact: bool,
    pub category: EventFeedCategory,
    pub kind: EventFeedKind,
    pub tags: Vec<String>,
    #[doc = " Native action roles, actor then target. Null keeps a role that names no country or that"]
    #[doc = " no link and holder witnessed."]
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[cfg_attr(any(test, feature = "export-types"), ts(optional))]
    pub participants: Option<[Option<String>; 2]>,
    pub item: Option<EventFeedItem>,
    pub amount: Option<u32>,
    #[doc = " A native event's recorded cause; absent when the recording holds none."]
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[cfg_attr(any(test, feature = "export-types"), ts(optional))]
    pub cause: Option<EventFeedCause>,
    #[doc = " A nuclear strike's target and outcome; absent for every other kind."]
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[cfg_attr(any(test, feature = "export-types"), ts(optional))]
    pub strike: Option<EventFeedStrike>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[cfg_attr(any(test, feature = "export-types"), ts(optional))]
    pub decision_state: Option<DecisionLifecycleState>,
    #[doc = " A fired event's window decision, or a presented window's recorded content; absent for"]
    #[doc = " every other kind."]
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[cfg_attr(any(test, feature = "export-types"), ts(optional))]
    pub window: Option<EventFeedWindow>,
}

#[derive(Clone, Copy, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[cfg_attr(any(test, feature = "export-types"), derive(TS))]
#[serde(rename_all = "camelCase")]
pub enum EventWindowDecision {
    Requested,
    AiControlled,
    HiddenOrAiControlled,
    NotViewingCountry,
    SenderExcluded,
    PlayerRestrictedNotMajor,
    PlayerRestrictedOptionTrigger,
    ExistingInstance,
    AlreadyOpen,
}

#[derive(Clone, Copy, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[cfg_attr(any(test, feature = "export-types"), derive(TS))]
#[serde(rename_all = "camelCase")]
pub enum EventWindowKind {
    Normal,
    News,
    Leader,
    Operative,
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[cfg_attr(any(test, feature = "export-types"), derive(TS))]
#[serde(rename_all = "camelCase")]
pub struct EventWindowOption {
    #[doc = " The option's place in the live event's own list; null for the fallback OK button."]
    pub ordinal: Option<u32>,
    pub label: String,
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[cfg_attr(any(test, feature = "export-types"), derive(TS))]
#[serde(rename_all = "camelCase")]
pub struct EventWindowPresented {
    pub kind: EventWindowKind,
    #[doc = " False when the user's popup settings hid the built window; null when not recorded."]
    pub shown: Option<bool>,
    #[doc = " Built again by an interface reload, with the same scope."]
    pub rebuilt: bool,
    pub title: Option<String>,
    pub description: Option<String>,
    #[doc = " The resolved picture key; null with `pictureHidden` when the window showed none."]
    pub picture: Option<String>,
    pub picture_hidden: bool,
    #[doc = " The offered options in button order."]
    pub options: Option<Vec<EventWindowOption>>,
    #[doc = " The event's options the window did not offer, from the recorded content; null when"]
    #[doc = " the offered options or the content's option list could not prove it."]
    pub not_offered: Option<Vec<EventFeedOption>>,
    #[doc = " Each field the recording does not hold, with why."]
    pub not_recorded: Vec<String>,
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[cfg_attr(any(test, feature = "export-types"), derive(TS))]
#[serde(rename_all = "camelCase")]
pub struct EventFeedWindow {
    #[doc = " The native pending instance joining a firing to the window built for it."]
    pub instance: Option<u32>,
    #[doc = " On a fired event: whether this client asked for a window, or the gate that refused it."]
    pub decision: Option<EventWindowDecision>,
    #[doc = " On a presented window: what it showed."]
    pub presented: Option<EventWindowPresented>,
    #[doc = " Why `decision` or `presented` is null where the kind carries one."]
    pub missing: Option<String>,
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq)]
#[cfg_attr(any(test, feature = "export-types"), derive(TS))]
#[serde(rename_all = "camelCase")]
pub struct EventFeedStrike {
    pub province: u32,
    #[doc = " None for a province without a state."]
    pub state: Option<u32>,
    pub building_levels: Option<f64>,
    #[doc = " The owner's war support change, in percent."]
    pub war_support: Option<f64>,
    pub units: Option<u32>,
    pub manpower_lost: Option<f64>,
    pub equipment_lost: Option<f64>,
    pub not_recorded: Vec<String>,
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[cfg_attr(any(test, feature = "export-types"), derive(TS))]
#[serde(rename_all = "camelCase")]
pub struct EventFeedCause {
    #[doc = " The native operation the event happened in."]
    pub how: Option<String>,
    #[doc = " The operation that one ran inside."]
    pub within: Option<String>,
    #[doc = " The peace conference starting or applying, as its reference in decimal."]
    pub conference: Option<String>,
    #[doc = " A state whose owner changed."]
    pub state: Option<u32>,
    #[doc = " A join's ally."]
    pub ally: Option<String>,
    #[doc = " A join's hostility reason."]
    pub reason: Option<String>,
    #[doc = " The native operands that change what the event means."]
    pub notes: Vec<String>,
}

#[derive(Clone, Debug, Default, Deserialize, Serialize, PartialEq)]
#[cfg_attr(any(test, feature = "export-types"), derive(TS))]
#[serde(rename_all = "camelCase")]
pub struct EventFeedItem {
    pub token: Option<String>,
    pub name: Option<String>,
    pub icon: Option<String>,
    pub description: Option<String>,
    #[doc = " A scripted event's authored options in the game's order; absent for other kinds and when"]
    #[doc = " the event's content does not resolve."]
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[cfg_attr(any(test, feature = "export-types"), ts(optional))]
    pub options: Option<Vec<EventFeedOption>>,
    #[doc = " Why `name` is null, where a script item's is."]
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[cfg_attr(any(test, feature = "export-types"), ts(optional))]
    pub name_missing: Option<String>,
    #[doc = " Why the name, description or options hold U+FFFC where scripted text could not be"]
    #[doc = r#" expanded: the UI shows each as a "not recorded" token."#]
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[cfg_attr(any(test, feature = "export-types"), ts(optional))]
    pub gaps: Option<String>,
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[cfg_attr(any(test, feature = "export-types"), derive(TS))]
#[serde(rename_all = "camelCase")]
pub struct EventFeedOption {
    pub ordinal: u32,
    pub name: Option<String>,
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[cfg_attr(any(test, feature = "export-types"), derive(TS))]
#[serde(rename_all = "camelCase")]
pub struct EventFeedUnavailable {
    pub kind: String,
    pub label: String,
    pub reason: String,
    pub count: Option<u32>,
}

#[derive(Clone, Debug, PartialEq, Deserialize, Serialize)]
#[cfg_attr(any(test, feature = "export-types"), derive(TS))]
#[serde(rename_all = "camelCase")]
pub struct FeedPageParams {
    pub branch_id: String,
    #[serde(default)]
    pub facets: Vec<FacetFilter>,
    #[doc = " The timeline's first hour when none."]
    pub from: Option<f64>,
    pub to: f64,
    #[doc = " The ordinal the page starts at, counted from the timeline's first hour."]
    pub cursor: Option<String>,
    pub page_size: u32,
    #[doc = " The countries a row must name (actor, target or an ally joining with it), each tag over the"]
    #[doc = " hours it counts; every row when none. Ordinals and totals count the rows it keeps."]
    #[serde(default)]
    pub scope: Option<Vec<dashboard_world::products::groups::MembershipRun>>,
}

#[derive(Clone, Debug, PartialEq, Serialize)]
#[cfg_attr(any(test, feature = "export-types"), derive(TS))]
#[serde(rename_all = "camelCase")]
pub struct EventFeedPageRow {
    pub hour: i32,
    pub key: String,
    pub ordinal: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    #[cfg_attr(any(test, feature = "export-types"), ts(optional))]
    pub country: Option<dashboard_base::products::Country>,
    #[serde(skip_serializing_if = "Option::is_none")]
    #[cfg_attr(any(test, feature = "export-types"), ts(optional))]
    pub own_ordinal: Option<String>,
    pub cells: Vec<logs::LogCell>,
    pub row: Option<EventFeedRow>,
}

#[derive(Clone, Debug, PartialEq, Serialize)]
#[cfg_attr(any(test, feature = "export-types"), derive(TS))]
#[serde(rename_all = "camelCase")]
pub struct EventFeedPage {
    pub branch_id: String,
    pub log: String,
    pub columns: Vec<String>,
    pub first_ordinal: String,
    pub total: String,
    pub cursor: Option<String>,
    pub rows: Vec<EventFeedPageRow>,
    pub missing: Vec<MissingSpan>,
    #[doc = " What the page's rows could not show, each reason once with its count."]
    pub unavailable: Vec<EventFeedUnavailable>,
    pub warnings: Vec<String>,
}

dashboard_base::reply_warnings!(EventFeedPage);

#[derive(Clone, Debug, PartialEq, Deserialize, Serialize)]
#[cfg_attr(any(test, feature = "export-types"), derive(TS))]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct FeedTailParams {
    pub branch_id: String,
    #[serde(default)]
    pub facets: Vec<FacetFilter>,
    #[serde(default)]
    pub from: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[cfg_attr(any(test, feature = "export-types"), ts(optional))]
    pub hour: Option<i32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[cfg_attr(any(test, feature = "export-types"), ts(optional))]
    pub offset: Option<String>,
    pub limit: u32,
    #[serde(default)]
    pub count_facets: Vec<logs::LogFacetCountAsk>,
    #[serde(default)]
    pub scope: Option<Vec<dashboard_world::products::groups::MembershipRun>>,
}

#[derive(Clone, Debug, PartialEq, Serialize)]
#[cfg_attr(any(test, feature = "export-types"), derive(TS))]
#[serde(rename_all = "camelCase")]
pub struct EventFeedTailReply {
    pub branch_id: String,
    pub log: String,
    pub columns: Vec<String>,
    pub base_ordinal: String,
    pub first_ordinal: String,
    pub offset: String,
    pub total: String,
    pub rows: Vec<EventFeedPageRow>,
    pub facet_totals: Vec<logs::LogFacetTotal>,
    pub missing: Vec<MissingSpan>,
    pub unavailable: Vec<EventFeedUnavailable>,
    pub warnings: Vec<String>,
}

dashboard_base::reply_warnings!(EventFeedTailReply);

impl logs::TailPanelReply for EventFeedTailReply {
    fn row_count(&self) -> usize {
        self.rows.len()
    }

    fn missing(&self) -> &[MissingSpan] {
        &self.missing
    }

    fn set_missing(&mut self, missing: Vec<MissingSpan>) {
        self.missing = missing;
    }
}

pub(crate) const TAIL_FIELDS: dashboard_base::products::Fields = dashboard_base::fields![
    branchId,
    facets,
    from: Start,
    hour: Read,
    offset,
    limit,
    countFacets,
    "scope[].firstHour": Start,
    "scope[].lastHour": End,
    "facets[].countries[]",
    "facets[].values[]",
    "facets[].membership[].country",
    "facets[].membership[].firstHour": Start,
    "facets[].membership[].lastHour": End,
    "facets[].unobserved[].firstHour": Start,
    "facets[].unobserved[].lastHour": End,
    "countFacets[].facets[].countries[]",
    "countFacets[].facets[].values[]",
    "countFacets[].facets[].membership[].country",
    "countFacets[].facets[].membership[].firstHour": Start,
    "countFacets[].facets[].membership[].lastHour": End,
    "countFacets[].facets[].unobserved[].firstHour": Start,
    "countFacets[].facets[].unobserved[].lastHour": End
];

fn invalid(what: impl Into<String>) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, what.into())
}

pub(crate) fn branch(text: &str) -> Result<(), &'static str> {
    text.parse::<u64>().map(|_| ()).map_err(|_| "Invalid branch")
}

pub(crate) fn resolve_content(source: &Source, context: &LoadContext) -> Result<Arc<Catalogue>, String> {
    context.content().and_then(|options| catalogue::catalogue(source, options)).map_err(|reason| reason.to_string())
}

pub(crate) fn content_from(resolved: &Result<Arc<Catalogue>, String>, notices: &mut Notices) -> Option<Arc<Catalogue>> {
    match resolved {
        Ok(catalogue) => Some(Arc::clone(catalogue)),
        Err(reason) => {
            notices.unavailable("contentStack", "Recorded content stack", reason.clone(), Some(1));
            None
        }
    }
}

fn content(source: &Source, context: &LoadContext, notices: &mut Notices) -> Option<Arc<Catalogue>> {
    content_from(&resolve_content(source, context), notices)
}

fn content_refusals(presenter: &Presenter<'_>, notices: &mut Notices) -> io::Result<()> {
    if let Some(catalogue) = presenter.catalogue() {
        for (reason, count) in catalogue.refusal_entries() {
            let count = count.map(u32::try_from).transpose().map_err(|_| invalid("Content refusal count exceeds u32"))?;
            notices.unavailable("contentPresentation", "Content presentation", reason, count);
        }
    }
    Ok(())
}

pub(crate) struct Paged {
    pub event: FeedEvent,
    pub hour: i32,
    pub key: u128,
    pub ordinal: u64,
    pub own: u64,
    pub tag: Option<dashboard_base::products::Country>,
}

pub(crate) fn page(source: &Source, query: &Query<'_>, params: &FeedPageParams) -> io::Result<(log::Page, Vec<Paged>, Vec<MissingSpan>)> {
    if let Some(runs) = &params.scope {
        return scope::page(source, query, params, runs);
    }
    let asked = logs::LogPageParams {
        branch_id: params.branch_id.clone(),
        log: LOG.to_owned(),
        facets: params.facets.clone(),
        from: params.from,
        to: params.to,
        cursor: params.cursor.clone(),
        page_size: params.page_size,
        columns: None,
    };
    let columns = Columns::new()?;
    let (page, rows, missing) = logs::page(query, &asked, |paged, tag| (paged.row.clone(), paged.ordinal, paged.own, tag.cloned()))?;
    let rows = rows
        .into_iter()
        .map(|(row, ordinal, own, tag)| Ok(Paged { event: columns.read(&row)?, hour: row.hour, key: row.key, ordinal, own, tag }))
        .collect::<io::Result<_>>()?;
    Ok((page, rows, missing))
}

pub(crate) fn tail_rows(
    scope_index: &scope::ScopeIndex,
    query: &Query<'_>,
    params: &FeedTailParams,
    hour: i32,
    caches: Option<&logs::LogCaches>,
) -> io::Result<(log::NewestPage, Vec<Paged>, Vec<logs::LogFacetTotal>, Vec<MissingSpan>)> {
    let offset = params.offset.as_deref().unwrap_or("0").parse::<u64>().map_err(|_| invalid("Invalid offset"))?;
    if let Some(runs) = &params.scope {
        return scope::tail_with_index(scope_index, query, &params.facets, params.from, hour, offset, params.limit, &params.count_facets, runs, caches);
    }
    let columns = Columns::new()?;
    let (page, rows, totals, missing) = logs::tail_with(
        query,
        LOG,
        &params.facets,
        params.from,
        hour,
        offset,
        params.limit,
        &params.count_facets,
        caches,
        |paged, tag| (paged.row.clone(), paged.ordinal, paged.own, tag.cloned()),
    )?;
    let rows = rows
        .into_iter()
        .map(|(row, ordinal, own, tag)| Ok(Paged { event: columns.read(&row)?, hour: row.hour, key: row.key, ordinal, own, tag }))
        .collect::<io::Result<_>>()?;
    Ok((page, rows, totals, missing))
}

fn tag(names: &Names, slot: Slot, hour: i32) -> Option<String> {
    match slot {
        Slot::Of(slot) => names.slot_tag(slot, hour).map(str::to_owned),
        Slot::None | Slot::NotRecorded => None,
    }
}

pub(crate) fn present_all(query: &Query<'_>, events: &[&FeedEvent], names: &Names, presenter: &Presenter<'_>, notices: &mut Notices, cancelled: &AtomicBool) -> io::Result<Vec<Option<EventFeedRow>>> {
    let states = decision_state::resolve_all(query, events)?;
    let mut order: Vec<usize> = (0..events.len()).collect();
    order.sort_by_key(|&at| events[at].hour);
    let mut out: Vec<Option<EventFeedRow>> = (0..events.len()).map(|_| None).collect();
    for at in order {
        dashboard_text::metadata::check_cancel(cancelled)?;
        let mut presented = present(events[at], names, presenter, notices)?;
        if let Some(row) = presented.as_mut() {
            row.decision_state = states[at].clone();
        }
        out[at] = presented;
    }
    Ok(out)
}

pub(crate) fn present(event: &FeedEvent, names: &Names, presenter: &Presenter<'_>, notices: &mut Notices) -> io::Result<Option<EventFeedRow>> {
    if event.code == FeedEventRow::WINDOWS_UNRECORDED {
        notices.unavailable("eventWindows", "Event windows", format!("From read {}: {}", event.read_hour, String::from_utf8_lossy(&event.failure)), Some(1));
        return Ok(None);
    }
    let Some(kind) = EventFeedKind::of_code(event.code) else { return Ok(None) };
    let id = event.id();
    if kind == EventFeedKind::EventWindowPresented {
        return window::presented(event, id, names, presenter, notices).map(Some);
    }
    if event.derived() {
        return Ok(derived(event, kind, id, names, notices));
    }
    let participants = [event.actor, event.target].map(|party| tag(names, party.slot, event.hour));
    for ((role, named), party) in ["actor", "target"].into_iter().zip(&participants).zip([event.actor, event.target]) {
        if named.is_none() && !(kind.scripted() && party.slot == Slot::None) {
            notices.sampled("nativeEventCountry", "Native event country", format!("The {role} of event {} at {} has no exact country-reference and published-link witness.", event.sequence, event.hour), 1);
        }
    }
    let ally = event.cause.and_then(|cause| cause.ally).map(|ally| tag(names, ally.slot, event.hour));
    if ally == Some(None) {
        notices.sampled("nativeEventCountry", "Native event country", format!("The ally of event {} at {} has no exact country-reference and published-link witness.", event.sequence, event.hour), 1);
    }
    let item = if kind.scripted() {
        let token = match &event.token {
            None => {
                notices.sampled("scriptEventToken", "Script content identity", format!("Occurrence {} content token was not captured.", event.sequence), 1);
                None
            }
            Some(bytes) if bytes.is_empty() => event.native_event.map(|(_, id)| id.to_string()),
            Some(bytes) => match std::str::from_utf8(bytes) {
                Ok(token) => Some(token.to_owned()),
                Err(_) => {
                    notices.sampled("scriptEventToken", "Script content identity", format!("Occurrence {} has a non-UTF-8 content token.", event.sequence), 1);
                    None
                }
            },
        };
        match &event.scopes {
            None => notices.sampled("scriptEventScope", "Bound script scope", format!("Occurrence {} scope graph was not fully captured.", event.sequence), 1),
            Some(scopes) => {
                for (at, scope) in scopes.iter().enumerate() {
                    for (missing, operand) in [(scope.combat_context_unrecorded, "combat context"), (scope.target_storage_unrecorded, "saved targets"), (scope.tooltip_storage_unrecorded, "tooltip targets")] {
                        if missing {
                            notices.sampled("scriptEventScope", "Bound script scope", format!("Occurrence {} scope {at}: {operand} operands were not captured.", event.sequence), 1);
                        }
                    }
                }
            }
        }
        Some(presenter.item(event, kind, token, notices)?)
    } else {
        None
    };
    Ok(Some(EventFeedRow {
        id,
        hour: event.hour,
        read_hour: event.read_hour,
        name_hour: event.hour,
        exact: true,
        category: kind.category(),
        kind,
        tags: participants.iter().flatten().cloned().collect(),
        participants: Some(participants),
        item,
        amount: None,
        cause: cause::words(event, ally.flatten()),
        strike: None,
        decision_state: None,
        window: window::decided(event, kind),
    }))
}

fn derived(event: &FeedEvent, kind: EventFeedKind, id: String, names: &Names, notices: &mut Notices) -> Option<EventFeedRow> {
    let row = |hour: i32, name_hour: i32, exact: bool, tags: Vec<String>| EventFeedRow {
        id: id.clone(),
        hour,
        read_hour: event.read_hour,
        name_hour,
        exact,
        category: kind.category(),
        kind,
        tags,
        participants: None,
        item: None,
        amount: None,
        cause: None,
        strike: None,
        decision_state: None,
        window: None,
    };
    match kind {
        EventFeedKind::CeasedToExist => {
            let named = event.dated.unwrap_or(event.read_hour);
            match tag(names, event.actor.slot, named) {
                Some(country) => Some(row(event.read_hour, named, false, vec![country])),
                None => {
                    notices.sampled("ceasedToExist", "Country existence", format!("The country that ceased to exist at {} holds no tag at its last read with states.", event.read_hour), 1);
                    None
                }
            }
        }
        _ => {
            let at_read = [event.actor.slot, event.target.slot].map(|slot| tag(names, slot, event.read_hour));
            let [Some(guarantor), Some(guaranteed)] = at_read else {
                notices.sampled("guaranteeGranted", "Guarantees", format!("A guarantee read at {} names a country that holds no tag then.", event.read_hour), 1);
                return None;
            };
            let start = event.dated.unwrap_or(event.read_hour);
            let exact = [event.actor.slot, event.target.slot].map(|slot| tag(names, slot, start)) == [Some(guarantor.clone()), Some(guaranteed.clone())];
            let hour = if exact { start } else { event.read_hour };
            Some(row(hour, hour, exact, vec![guarantor, guaranteed]))
        }
    }
}

pub struct EventFeed;

impl Product for EventFeed {
    const KEY: &'static str = "event-feed";
    const CONTENT_DEPENDENT: bool = true;
    const REPLAYS: bool = true;
    type Params = FeedPageParams;
    const FIELDS: dashboard_base::products::Fields = dashboard_base::fields![
        branchId,
        facets,
        from: Start,
        to: End,
        cursor,
        pageSize,
        "scope[].firstHour": Start,
        "scope[].lastHour": End,
        "facets[].countries[]",
        "facets[].values[]",
        "facets[].membership[].country",
        "facets[].membership[].firstHour": Start,
        "facets[].membership[].lastHour": End,
        "facets[].unobserved[].firstHour": Start,
        "facets[].unobserved[].lastHour": End
    ];
    type Reply = EventFeedPage;

    #[cfg(any(test, feature = "export-types"))]
    fn types() -> Vec<String> {
        vec![
            FeedPageParams::decl(),
            EventFeedCategory::decl(),
            EventFeedKind::decl(),
            EventFeedOption::decl(),
            EventFeedItem::decl(),
            EventFeedCause::decl(),
            DecisionLifecycleState::decl(),
            EventWindowDecision::decl(),
            EventWindowKind::decl(),
            EventWindowOption::decl(),
            EventWindowPresented::decl(),
            EventFeedWindow::decl(),
            EventFeedRow::decl(),
            EventFeedStrike::decl(),
            EventFeedUnavailable::decl(),
            EventFeedPageRow::decl(),
            EventFeedPage::decl(),
        ]
    }

    fn validate(params: &Self::Params) -> Result<(), &'static str> {
        branch(&params.branch_id)
    }

    fn load(source: &Source, params: &Self::Params, context: &LoadContext, cancelled: &AtomicBool) -> io::Result<Self::Reply> {
        dashboard_text::metadata::check_cancel(cancelled)?;
        let query = dashboard_base::queries::query(source, params.branch_id.parse().map_err(|_| invalid("Invalid branch"))?)?;
        let (page, rows, missing) = page(source, &query, params)?;
        let names = Names::new(&query.holders);
        let mut notices = Notices::default();
        let presenter = Presenter::new(&query, content(source, context, &mut notices), kept_naming(source));
        let tagged = params.facets.iter().any(|filter| !filter.countries.is_empty());
        let events: Vec<&FeedEvent> = rows.iter().map(|paged| &paged.event).collect();
        let presented = present_all(&query, &events, &names, &presenter, &mut notices, cancelled)?;
        let out: Vec<EventFeedPageRow> = rows
            .into_iter()
            .zip(presented)
            .map(|(paged, row)| EventFeedPageRow {
                hour: paged.hour,
                key: paged.key.to_string(),
                ordinal: paged.ordinal.to_string(),
                country: paged.tag,
                own_ordinal: tagged.then(|| paged.own.to_string()),
                cells: Vec::new(),
                row,
            })
            .collect();
        content_refusals(&presenter, &mut notices)?;
        let reply = EventFeedPage {
            branch_id: params.branch_id.clone(),
            log: LOG.to_owned(),
            columns: Vec::new(),
            first_ordinal: page.first.to_string(),
            total: page.total().to_string(),
            cursor: page.cursor().map(|cursor| cursor.to_string()),
            rows: out,
            missing,
            unavailable: notices.into_list(),
            warnings: Vec::new(),
        };
        dashboard_base::products::check_reply_budget(&reply)?;
        Ok(reply)
    }
}

fn load_event_feed_tail_at(
    scope_index: &scope::ScopeIndex,
    query: &Query<'_>,
    content_result: &Result<Arc<Catalogue>, String>,
    kept: &Arc<Mutex<KeptNaming>>,
    caches: Option<&logs::LogCaches>,
    params: &FeedTailParams,
    hour: i32,
    cancelled: &AtomicBool,
) -> io::Result<EventFeedTailReply> {
    dashboard_text::metadata::check_cancel(cancelled)?;
    let (page, rows, facet_totals, missing) = tail_rows(scope_index, query, params, hour, caches)?;
    let names = Names::new(&query.holders);
    let mut notices = Notices::default();
    let presenter = Presenter::new(query, content_from(content_result, &mut notices), Arc::clone(kept));
    let tagged = params.facets.iter().any(|filter| !filter.countries.is_empty());
    let events: Vec<&FeedEvent> = rows.iter().map(|paged| &paged.event).collect();
    let presented = present_all(query, &events, &names, &presenter, &mut notices, cancelled)?;
    let out: Vec<EventFeedPageRow> = rows
        .into_iter()
        .zip(presented)
        .map(|(paged, row)| EventFeedPageRow {
            hour: paged.hour,
            key: paged.key.to_string(),
            ordinal: paged.ordinal.to_string(),
            country: paged.tag,
            own_ordinal: tagged.then(|| paged.own.to_string()),
            cells: Vec::new(),
            row,
        })
        .collect();
    content_refusals(&presenter, &mut notices)?;
    let first_ordinal = out.first().map_or_else(|| page.base.to_string(), |row| row.ordinal.clone());
    let reply = EventFeedTailReply {
        branch_id: params.branch_id.clone(),
        log: LOG.to_owned(),
        columns: Vec::new(),
        base_ordinal: page.base.to_string(),
        first_ordinal,
        offset: page.offset.to_string(),
        total: page.total.to_string(),
        rows: out,
        facet_totals,
        missing,
        unavailable: notices.into_list(),
        warnings: Vec::new(),
    };
    dashboard_base::products::check_reply_budget(&reply)?;
    Ok(reply)
}

pub struct EventFeedTail;

impl Product for EventFeedTail {
    const KEY: &'static str = "event-feed-tail";
    const CONTENT_DEPENDENT: bool = true;
    const REPLAYS: bool = true;
    type Params = FeedTailParams;
    const FIELDS: dashboard_base::products::Fields = TAIL_FIELDS;
    type Reply = EventFeedTailReply;

    #[cfg(any(test, feature = "export-types"))]
    fn types() -> Vec<String> {
        vec![FeedTailParams::decl(), EventFeedTailReply::decl()]
    }

    fn validate(params: &Self::Params) -> Result<(), &'static str> {
        branch(&params.branch_id)
    }

    fn load(source: &Source, params: &Self::Params, context: &LoadContext, cancelled: &AtomicBool) -> io::Result<Self::Reply> {
        let query = dashboard_base::queries::query(source, params.branch_id.parse().map_err(|_| invalid("Invalid branch"))?)?;
        let hour = params.hour.unwrap_or(query.timeline.last);
        let content_result = resolve_content(source, context);
        let kept = kept_naming(source);
        let scope_index = scope::index(source);
        let caches = dashboard_base::queries::log_caches(source);
        load_event_feed_tail_at(&scope_index, &query, &content_result, &kept, Some(&caches), params, hour, cancelled)
    }
}

pub fn event_feed_dated_panel(source: &Source, load: &LoadContext, asked: &serde_json::Value) -> Result<Box<dyn Panel>, String> {
    let mut request = asked.clone();
    request.as_object_mut().ok_or("Invalid event-feed-tail panel")?.remove("panel");
    let params: FeedTailParams = dashboard_query::named::named(&request).map_err(|error| format!("Invalid event-feed-tail panel: {error}"))?;
    EventFeedTail::validate(&params)?;
    let branch_id = params.branch_id.parse().map_err(|_| "Invalid branch")?;
    let query = dashboard_base::queries::query(source, branch_id).map_err(|error| error.to_string())?;
    let declared = dashboard_declare::log(LOG).ok_or_else(|| format!("No declared log {LOG}"))?;
    let log_caches = dashboard_base::queries::log_caches(source);
    let pre_reader = log::Reader::with_cache(&query, declared, Arc::clone(&log_caches.buckets)).map_err(|error| error.to_string())?;
    let selection = Arc::new(pre_reader.select(&params.facets, query.timeline.first, query.timeline.last).map_err(|error| error.to_string())?);
    let count_selections: Arc<Vec<log::Selection>> = Arc::new(
        params
            .count_facets
            .iter()
            .map(|ask| pre_reader.select(&ask.facets, query.timeline.first, query.timeline.last))
            .collect::<io::Result<Vec<_>>>()
            .map_err(|error| error.to_string())?,
    );
    let content_result = Arc::new(resolve_content(source, load));
    let kept = kept_naming(source);
    let scope_index = scope::index(source);
    let (replay, timeline, holders, executable) = (source.replay(), query.timeline, query.holders, source.tables());
    let params = Arc::new(params);
    let q_replay = Arc::clone(&replay);
    let q_timeline = timeline.clone();
    let q_holders = Arc::clone(&holders);
    let q_params = Arc::clone(&params);
    let q_caches = log_caches.clone();
    Ok(logs::dated_tail_panel(
        executable,
        move |prev_hour, hour, prev_reply, cancelled| {
            dashboard_text::metadata::check_cancel(cancelled)?;
            let query = Query { replay: &q_replay, timeline: q_timeline.clone(), holders: q_holders.clone() };
            logs::quiet_tail_missing(&query, LOG, &q_caches, &selection, &count_selections, &q_params.facets, q_params.from, prev_hour, hour, prev_reply)
        },
        move |hour, cancelled| {
            let query = Query { replay: &replay, timeline: timeline.clone(), holders: holders.clone() };
            load_event_feed_tail_at(&scope_index, &query, &content_result, &kept, Some(&log_caches), &params, hour, cancelled)
        },
    ))
}

pub use families::{EventFeedSpan, EventFeedSpanParams, EventFeedSpanReply, FeedKindCode};

pub(crate) fn annexations(query: &Query<'_>) -> io::Result<Vec<super::politics::civil_wars::Annexation>> {
    let declared = dashboard_declare::log(LOG).ok_or_else(|| invalid("The feed's log is not declared"))?;
    let reader = log::Reader::new(query, declared)?;
    let annexed = EventFeedKind::Annexed.code().map(i64::from).ok_or_else(|| invalid("An annexation has no feed code"))?;
    let selection = reader.select(&[FacetFilter { name: "kind".into(), values: vec![annexed], ..Default::default() }], query.timeline.first, query.timeline.last)?;
    let (columns, names) = (Columns::new()?, Names::new(&query.holders));
    let mut out = Vec::new();
    let mut cursor = 0;
    loop {
        let page = reader.page(&selection.patterns, selection.groups.len(), query.timeline.first, query.timeline.last, cursor, dashboard_query::series::PAGE_LIMIT)?;
        for paged in &page.rows {
            let event = columns.read(&paged.row)?;
            out.push(super::politics::civil_wars::Annexation {
                hour: event.hour,
                read_hour: event.read_hour,
                annexer: tag(&names, event.actor.slot, event.hour),
                annexed: tag(&names, event.target.slot, event.hour),
                civil_war: cause::civil_war(&event),
            });
        }
        match page.cursor() {
            Some(next) => cursor = next,
            None => return Ok(out),
        }
    }
}
