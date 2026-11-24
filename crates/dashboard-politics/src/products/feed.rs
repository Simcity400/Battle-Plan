
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

pub use cards::{Decisions, EventChoices};
pub use decision_state::{DecisionLifecycleState, MISSION_FAILURE_NOT_RECORDED};
pub(crate) use row::FeedEvent;

use dashboard_base::products::log_page as logs;
use dashboard_base::{products::{LoadContext, Product}, queries::country_ids::Names};
use dashboard_base::{Source, catalogue::{self, Catalogue}};
use dashboard_query::{
    Query,
    log::{self, FacetFilter},
    series::MissingSpan,
};
use replay_data::rows::FeedEventRow;
use row::{Columns, Slot};
use serde::{Deserialize, Serialize};
use std::{io, sync::{Arc, atomic::AtomicBool}};
use text::{Notices, Presenter, kept_naming};
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
}

const CODES: [(u8, EventFeedKind); 25] = {
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

fn invalid(what: impl Into<String>) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, what.into())
}

pub(crate) fn branch(text: &str) -> Result<(), &'static str> {
    text.parse::<u64>().map(|_| ()).map_err(|_| "Invalid branch")
}

fn content(source: &Source, context: &LoadContext, notices: &mut Notices) -> Option<Arc<Catalogue>> {
    match context.content().and_then(|options| catalogue::catalogue(source, options)) {
        Ok(catalogue) => Some(catalogue),
        Err(reason) => {
            notices.unavailable("contentStack", "Recorded content stack", reason.to_string(), Some(1));
            None
        }
    }
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
    let Some(kind) = EventFeedKind::of_code(event.code) else { return Ok(None) };
    let id = event.id();
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
    const FIELDS: dashboard_base::products::Fields = dashboard_base::fields![branchId, facets, from: Start, to: End, cursor, pageSize, "scope[].firstHour": Start, "scope[].lastHour": End, "facets[].countries[]", "facets[].values[]"];
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

pub use families::{EventFeedSpan, EventFeedSpanParams, EventFeedSpanReply, FeedKindCode};

pub(crate) fn annexations(query: &Query<'_>) -> io::Result<Vec<super::politics::civil_wars::Annexation>> {
    let declared = dashboard_declare::log(LOG).ok_or_else(|| invalid("The feed's log is not declared"))?;
    let reader = log::Reader::new(query, declared)?;
    let annexed = EventFeedKind::Annexed.code().map(i64::from).ok_or_else(|| invalid("An annexation has no feed code"))?;
    let selection = reader.select(&[FacetFilter { name: "kind".into(), values: vec![annexed], countries: Vec::new() }], query.timeline.first, query.timeline.last)?;
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
