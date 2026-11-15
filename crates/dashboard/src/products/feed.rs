//! The event feed (family 30), read from the declared `feed-events` log (the recorder's typed
//! occurrences and the feed writer's derived rows), every title, body, option, effect and cause
//! word built at the reply from the recording's facts. `event-feed` pages its rows;
//! `event-choices` and `decisions` page cards, an opener with the rows continuing it;
//! `event-feed-span` gives what the feed shows over a span besides its pages: each kind's code,
//! the reads that were not complete, and the rows other families date. Pages have `log-page`'s
//! shape, counted by `log-counts` over the same facets.

mod cards;
mod cause;
mod families;
mod nuclear;
mod row;
pub(crate) mod scope;
mod text;
#[cfg(test)]
mod tests;

pub use cards::{Decisions, EventChoices};
pub(crate) use row::FeedEvent;

use super::{LoadContext, Product, country_ids::Names, logs};
use crate::{
    Source,
    catalogue::{self, Catalogue},
};
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
pub(crate) use text::warm_naming;
use ts_rs::TS;

/// The declared log the feed reads.
pub(crate) const LOG: &str = "feed-events";

#[derive(Clone, Copy, Debug, Deserialize, Serialize, PartialEq, Eq, PartialOrd, Ord, Hash, TS)]
#[serde(rename_all = "camelCase")]
pub enum EventFeedCategory {
    WarsPeace,
    Diplomacy,
    Politics,
    Milestones,
}

#[derive(Clone, Copy, Debug, Deserialize, Serialize, PartialEq, Eq, PartialOrd, Ord, Hash, TS)]
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

/// Each kind the log holds, by its row's kind code: the recorder's occurrence source codes, then
/// the feed writer's derived kinds.
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

    /// The kind a feed row's code holds; none for a code no row of the feed shows (an incomplete
    /// read, the registry's retained state).
    pub fn of_code(code: u8) -> Option<Self> {
        CODES.iter().find(|(held, _)| *held == code).map(|(_, kind)| *kind)
    }

    pub fn code(self) -> Option<u8> {
        CODES.iter().find(|(_, kind)| *kind == self).map(|(code, _)| *code)
    }

    /// A script occurrence: an event dispatch, a choice, a decision's step.
    pub fn scripted(self) -> bool {
        self >= EventFeedKind::ScriptEventFired
    }
}

/// One row of the feed.
#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, TS)]
#[serde(rename_all = "camelCase")]
pub struct EventFeedRow {
    pub id: String,
    pub hour: i32,
    /// The read supplying this fact; the head never shows it before this hour.
    pub read_hour: i32,
    /// Country naming and flags use this hour: the event's, or a vanished country's last seen.
    pub name_hour: i32,
    pub exact: bool,
    pub category: EventFeedCategory,
    pub kind: EventFeedKind,
    pub tags: Vec<String>,
    /// Native action roles, actor then target. Null keeps a role that names no country or that
    /// no link and holder witnessed.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub participants: Option<[Option<String>; 2]>,
    pub item: Option<EventFeedItem>,
    pub amount: Option<u32>,
    /// A native event's recorded cause; absent when the recording holds none.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub cause: Option<EventFeedCause>,
    /// A nuclear strike's target and outcome; absent for every other kind.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub strike: Option<EventFeedStrike>,
}

/// A nuclear strike's target and what the impact did before its authored `on_nuke_drop` effects;
/// a part with a refusal has no total, and the refusal is named.
#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, TS)]
#[serde(rename_all = "camelCase")]
pub struct EventFeedStrike {
    pub province: u32,
    /// None for a province without a state.
    pub state: Option<u32>,
    pub building_levels: Option<f64>,
    /// The owner's war support change, in percent.
    pub war_support: Option<f64>,
    pub units: Option<u32>,
    pub manpower_lost: Option<f64>,
    pub equipment_lost: Option<f64>,
    pub not_recorded: Vec<String>,
}

/// What caused a native event, in words, from the operations the collector observed running when
/// it happened. Each absent fact was not observed.
#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq, TS)]
#[serde(rename_all = "camelCase")]
pub struct EventFeedCause {
    /// The native operation the event happened in.
    pub how: Option<String>,
    /// The operation that one ran inside.
    pub within: Option<String>,
    /// The peace conference starting or applying, as its reference in decimal.
    pub conference: Option<String>,
    /// A state whose owner changed.
    pub state: Option<u32>,
    /// A join's ally.
    pub ally: Option<String>,
    /// A join's hostility reason.
    pub reason: Option<String>,
    /// The native operands that change what the event means.
    pub notes: Vec<String>,
}

#[derive(Clone, Debug, Default, Deserialize, Serialize, PartialEq, TS)]
#[serde(rename_all = "camelCase")]
pub struct EventFeedItem {
    pub token: Option<String>,
    pub name: Option<String>,
    pub icon: Option<String>,
    pub description: Option<String>,
    /// A scripted event's authored options in the game's order; absent for other kinds and when
    /// the event's content does not resolve.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub options: Option<Vec<EventFeedOption>>,
    /// Why `name` is null, where a script item's is.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub name_missing: Option<String>,
    /// Why the name, description or options hold U+FFFC where scripted text could not be
    /// expanded: the UI shows each as a "not recorded" token.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub gaps: Option<String>,
}

/// One authored event option: its native ordinal and in-game name, null when the name does not
/// resolve.
#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq, TS)]
#[serde(rename_all = "camelCase")]
pub struct EventFeedOption {
    pub ordinal: u32,
    pub name: Option<String>,
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq, TS)]
#[serde(rename_all = "camelCase")]
pub struct EventFeedUnavailable {
    pub kind: String,
    pub label: String,
    pub reason: String,
    pub count: Option<u32>,
}

/// A page of a declared log of the feed: `log-page`'s parameters with the log fixed.
#[derive(Clone, Debug, Deserialize, Serialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct FeedPageParams {
    pub branch_id: String,
    #[serde(default)]
    pub facets: Vec<FacetFilter>,
    /// The timeline's first hour when none.
    pub from: Option<f64>,
    pub to: f64,
    /// The ordinal the page starts at, counted from the timeline's first hour.
    pub cursor: Option<String>,
    pub page_size: u32,
    /// The countries a row must name (actor, target or an ally joining with it), each tag over the
    /// hours it counts; every row when none. Ordinals and totals count the rows it keeps.
    #[serde(default)]
    pub scope: Option<Vec<super::groups::MembershipRun>>,
}

/// A row of a feed page: `log-page`'s row (no cells) with the feed's row; null for a row the feed
/// does not show (an incomplete read's).
#[derive(Clone, Debug, Serialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct EventFeedPageRow {
    pub hour: i32,
    pub key: String,
    pub ordinal: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub tag: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub own_ordinal: Option<String>,
    pub cells: Vec<logs::LogCell>,
    pub row: Option<EventFeedRow>,
}

#[derive(Clone, Debug, Serialize, TS)]
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
    /// What the page's rows could not show, each reason once with its count.
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

/// The recorded content stack, or a notice naming why it is not there.
fn content(source: &Source, context: &LoadContext, notices: &mut Notices) -> Option<Arc<Catalogue>> {
    match context.content().and_then(|options| catalogue::catalogue(source, options)) {
        Ok(catalogue) => Some(catalogue),
        Err(reason) => {
            notices.unavailable("contentStack", "Recorded content stack", reason.to_string(), Some(1));
            None
        }
    }
}

/// The refusals the content stack noted while the reply was built.
fn content_refusals(presenter: &Presenter<'_>, notices: &mut Notices) -> io::Result<()> {
    if let Some(catalogue) = presenter.catalogue() {
        for (reason, count) in catalogue.refusal_entries() {
            let count = count.map(u32::try_from).transpose().map_err(|_| invalid("Content refusal count exceeds u32"))?;
            notices.unavailable("contentPresentation", "Content presentation", reason, count);
        }
    }
    Ok(())
}

/// A page of the feed's rows, each with its row from the log, its ordinal, its own ordinal and its
/// tag group.
pub(crate) struct Paged {
    pub event: FeedEvent,
    pub hour: i32,
    pub key: u128,
    pub ordinal: u64,
    pub own: u64,
    pub tag: Option<String>,
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
    let (page, rows, missing) = logs::page(query, &asked, |paged, tag| (paged.row.clone(), paged.ordinal, paged.own, tag.map(str::to_owned)))?;
    let rows = rows
        .into_iter()
        .map(|(row, ordinal, own, tag)| Ok(Paged { event: columns.read(&row)?, hour: row.hour, key: row.key, ordinal, own, tag }))
        .collect::<io::Result<_>>()?;
    Ok((page, rows, missing))
}

/// A role's tag at `hour`; none for no country or one no link and holder witnessed.
fn tag(names: &Names, slot: Slot, hour: i32) -> Option<String> {
    match slot {
        Slot::Of(slot) => names.slot_tag(slot, hour).map(str::to_owned),
        Slot::None | Slot::NotRecorded => None,
    }
}

/// Each of `events` presented ([`present`]), in the order given; worded in hour order, so the
/// presenter's naming frames move forward through the reply's hours.
pub(crate) fn present_all(events: &[&FeedEvent], names: &Names, presenter: &Presenter<'_>, notices: &mut Notices, cancelled: &AtomicBool) -> io::Result<Vec<Option<EventFeedRow>>> {
    let mut order: Vec<usize> = (0..events.len()).collect();
    order.sort_by_key(|&at| events[at].hour);
    let mut out: Vec<Option<EventFeedRow>> = (0..events.len()).map(|_| None).collect();
    for at in order {
        crate::metadata::check_cancel(cancelled)?;
        out[at] = present(events[at], names, presenter, notices)?;
    }
    Ok(out)
}

/// The feed's row of a log row, named and worded; none for a row the feed does not show.
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
    }))
}

/// A row the feed writer derived by comparing reads: a country that ceased to exist (named at the
/// last read it owned states), a guarantee granted (dated at its start where both countries hold
/// their tags from then to the read).
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

/// The feed's rows: a page of `feed-events` (its facets as the request filters them), each row
/// named and worded.
pub struct EventFeed;

impl Product for EventFeed {
    const KEY: &'static str = "event-feed";
    const CONTENT_DEPENDENT: bool = true;
    const REPLAYS: bool = true;
    type Params = FeedPageParams;
    type Reply = EventFeedPage;

    fn types() -> Vec<String> {
        vec![
            FeedPageParams::decl(),
            EventFeedCategory::decl(),
            EventFeedKind::decl(),
            EventFeedOption::decl(),
            EventFeedItem::decl(),
            EventFeedCause::decl(),
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
        crate::metadata::check_cancel(cancelled)?;
        let query = crate::answer::query(source, params.branch_id.parse().map_err(|_| invalid("Invalid branch"))?)?;
        let (page, rows, missing) = page(source, &query, params)?;
        let names = Names::new(&query.holders);
        let mut notices = Notices::default();
        let presenter = Presenter::new(&query, content(source, context, &mut notices), kept_naming(source));
        let tagged = params.facets.iter().any(|filter| !filter.tags.is_empty());
        let events: Vec<&FeedEvent> = rows.iter().map(|paged| &paged.event).collect();
        let presented = present_all(&events, &names, &presenter, &mut notices, cancelled)?;
        let out: Vec<EventFeedPageRow> = rows
            .into_iter()
            .zip(presented)
            .map(|(paged, row)| EventFeedPageRow {
                hour: paged.hour,
                key: paged.key.to_string(),
                ordinal: paged.ordinal.to_string(),
                tag: paged.tag,
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

/// Every annexation the feed recorded on `query`'s timeline, its countries named at its hour, and
/// whether it ran within a civil war's resolution: the civil-war list's outcome evidence.
pub(crate) fn annexations(query: &Query<'_>) -> io::Result<Vec<super::politics::civil_wars::Annexation>> {
    let declared = dashboard_declare::log(LOG).ok_or_else(|| invalid("The feed's log is not declared"))?;
    let reader = log::Reader::new(query, declared)?;
    let annexed = EventFeedKind::Annexed.code().map(i64::from).ok_or_else(|| invalid("An annexation has no feed code"))?;
    let selection = reader.select(&[FacetFilter { name: "kind".into(), values: vec![annexed], tags: Vec::new() }], query.timeline.first, query.timeline.last)?;
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
