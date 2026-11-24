
use super::{
    EventFeedCategory, EventFeedKind, EventFeedRow, EventFeedUnavailable, Names, Notices, Presenter, branch, content_from, content_refusals, families, invalid,
    cards::{List, ScriptEdge, ScriptOccurrence, edge},
    index::{self, Feed, Item, Locator, ScriptStatus, Scanned, kind_code},
    present_all,
    row::FeedEvent,
    text::{KeptNaming, kept_naming},
};
use dashboard_base::{
    Source,
    catalogue::Catalogue,
    products::{
        LoadContext, Product,
        occurrences::{self, OccurrenceCount, OccurrenceReply, OccurrenceSort},
    },
};
use dashboard_follow::panel::Panel;
use dashboard_query::{
    Query,
    identity::Holders,
    log::occurrence::{Answer, Ask, Chip, Index, PlacedGap, Selection},
    timeline::Timeline,
};
use replay::campaign::Replay;
use serde::{Deserialize, Serialize};
use std::{
    io,
    sync::{Arc, Mutex, atomic::AtomicBool},
};
#[cfg(any(test, feature = "export-types"))]
use ts_rs::TS;

#[derive(Clone, Debug, PartialEq, Deserialize, Serialize)]
#[cfg_attr(any(test, feature = "export-types"), derive(TS))]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct EventFeedTailParams {
    pub branch_id: String,
    #[doc = " The observation cutoff: nothing observed later exists. The branch's last hour when none."]
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[cfg_attr(any(test, feature = "export-types"), ts(optional))]
    pub hour: Option<i32>,
    #[doc = " The first and last occurrence hour listed."]
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[cfg_attr(any(test, feature = "export-types"), ts(optional))]
    pub from: Option<i32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[cfg_attr(any(test, feature = "export-types"), ts(optional))]
    pub to: Option<i32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[cfg_attr(any(test, feature = "export-types"), ts(optional))]
    pub selection: Option<Selection>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[cfg_attr(any(test, feature = "export-types"), ts(optional))]
    pub search: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[cfg_attr(any(test, feature = "export-types"), ts(optional))]
    pub sort: Option<OccurrenceSort>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[cfg_attr(any(test, feature = "export-types"), ts(optional))]
    pub offset: Option<String>,
    pub limit: u32,
    pub categories: Vec<EventFeedCategory>,
}

#[derive(Clone, Debug, PartialEq, Deserialize, Serialize)]
#[cfg_attr(any(test, feature = "export-types"), derive(TS))]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ScriptTailParams {
    pub branch_id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[cfg_attr(any(test, feature = "export-types"), ts(optional))]
    pub hour: Option<i32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[cfg_attr(any(test, feature = "export-types"), ts(optional))]
    pub from: Option<i32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[cfg_attr(any(test, feature = "export-types"), ts(optional))]
    pub to: Option<i32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[cfg_attr(any(test, feature = "export-types"), ts(optional))]
    pub selection: Option<Selection>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[cfg_attr(any(test, feature = "export-types"), ts(optional))]
    pub search: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[cfg_attr(any(test, feature = "export-types"), ts(optional))]
    pub sort: Option<OccurrenceSort>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[cfg_attr(any(test, feature = "export-types"), ts(optional))]
    pub offset: Option<String>,
    pub limit: u32,
    #[doc = " The opening kinds listed; every one when none."]
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[cfg_attr(any(test, feature = "export-types"), ts(optional))]
    pub kinds: Option<Vec<EventFeedKind>>,
    #[doc = " The statuses listed, each at the hour asked; every one when none."]
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[cfg_attr(any(test, feature = "export-types"), ts(optional))]
    pub statuses: Option<Vec<ScriptStatus>>,
}

pub(crate) const TAIL_FIELDS: dashboard_base::products::Fields = dashboard_base::fields![
    branchId,
    hour: Read,
    from: Start,
    to: End,
    selection,
    search,
    sort,
    offset,
    limit,
    categories,
    kinds,
    statuses,
    "selection.membership[].country",
    "selection.membership[].firstHour": Start,
    "selection.membership[].lastHour": End,
    "selection.unobserved[].firstHour": Start,
    "selection.unobserved[].lastHour": End
];

#[derive(Clone, Debug, PartialEq, Serialize)]
#[cfg_attr(any(test, feature = "export-types"), derive(TS))]
#[serde(rename_all = "camelCase")]
pub struct EventFeedPageRow {
    #[doc = " The occurrence's stable id: the row key and what a click hands to the detail."]
    pub key: String,
    #[doc = " The occurrence hour, by which rows are ranged and ordered."]
    pub hour: i32,
    #[doc = " The hour the recording first witnessed it."]
    pub observed: i32,
    pub ordinal: String,
    pub live: bool,
    pub row: Option<EventFeedRow>,
    #[doc = " A fired event's chosen option, where one was observed by the hour asked."]
    pub choice: Option<ScriptEdge>,
    #[serde(skip_serializing_if = "Option::is_none")]
    #[cfg_attr(any(test, feature = "export-types"), ts(optional))]
    pub status: Option<ScriptStatus>,
}

#[derive(Clone, Debug, PartialEq, Serialize)]
#[cfg_attr(any(test, feature = "export-types"), derive(TS))]
#[serde(rename_all = "camelCase")]
pub struct EventFeedTailReply {
    pub branch_id: String,
    pub total: String,
    pub offset: String,
    pub live: String,
    pub counts: Vec<OccurrenceCount>,
    pub rows: Vec<EventFeedPageRow>,
    pub missing: Vec<PlacedGap>,
    pub next_change: Option<i32>,
    pub unavailable: Vec<EventFeedUnavailable>,
    pub warnings: Vec<String>,
}

dashboard_base::reply_warnings!(EventFeedTailReply);

impl OccurrenceReply for EventFeedTailReply {
    fn next_change(&self) -> Option<i32> {
        self.next_change
    }

    fn row_count(&self) -> usize {
        self.rows.len()
    }
}

#[derive(Clone, Debug, PartialEq, Serialize)]
#[cfg_attr(any(test, feature = "export-types"), derive(TS))]
#[serde(rename_all = "camelCase")]
pub struct ScriptPageRow {
    pub key: String,
    pub hour: i32,
    pub observed: i32,
    pub ordinal: String,
    pub live: bool,
    #[doc = " The card's status at the hour asked."]
    pub status: Option<ScriptStatus>,
    pub occurrence: Option<ScriptOccurrence>,
}

#[derive(Clone, Debug, PartialEq, Serialize)]
#[cfg_attr(any(test, feature = "export-types"), derive(TS))]
#[serde(rename_all = "camelCase")]
pub struct ScriptOccurrenceTailReply {
    pub branch_id: String,
    pub total: String,
    pub offset: String,
    pub live: String,
    pub counts: Vec<OccurrenceCount>,
    pub rows: Vec<ScriptPageRow>,
    pub missing: Vec<PlacedGap>,
    pub next_change: Option<i32>,
    pub unavailable: Vec<EventFeedUnavailable>,
    pub warnings: Vec<String>,
}

dashboard_base::reply_warnings!(ScriptOccurrenceTailReply);

impl OccurrenceReply for ScriptOccurrenceTailReply {
    fn next_change(&self) -> Option<i32> {
        self.next_change
    }

    fn row_count(&self) -> usize {
        self.rows.len()
    }
}

struct Owned {
    replay: Arc<Replay>,
    timeline: Arc<Timeline>,
    holders: Arc<Holders>,
}

impl Owned {
    fn of(query: &Query<'_>, replay: Arc<Replay>) -> Self {
        Self { replay, timeline: Arc::clone(&query.timeline), holders: Arc::clone(&query.holders) }
    }

    fn query(&self) -> Query<'_> {
        Query { replay: &self.replay, timeline: Arc::clone(&self.timeline), holders: Arc::clone(&self.holders) }
    }
}

struct Prepared {
    index: Arc<Index<Item>>,
    feed: Option<Arc<Feed>>,
    scanned: Arc<Scanned>,
    losses: Vec<(i32, i32)>,
    content: Arc<Result<Arc<Catalogue>, String>>,
    kept: Arc<Mutex<KeptNaming>>,
}

fn branch_id(text: &str) -> io::Result<u64> {
    text.parse().map_err(|_| invalid("Invalid branch"))
}

fn prepare(source: &Source, query: &Query<'_>, branch: u64, context: &LoadContext, list: Option<List>, cancelled: &AtomicBool) -> io::Result<Prepared> {
    let content = Arc::new(super::resolve_content(source, context));
    let scanned = index::scanned_for(source, query, branch, cancelled)?;
    let (index, feed) = match list {
        Some(List::Decisions) => (index::decisions_for(source, query, branch, cancelled)?, None),
        Some(List::EventChoices) => (index::choices_for(source, query, branch, cancelled)?, None),
        None => {
            let feed = index::feed_for(source, query, branch, context, content.as_ref().as_ref().ok(), cancelled)?;
            (Arc::clone(&feed.index), Some(feed))
        }
    };
    let losses = source.replay_ref().event_losses(branch).collect();
    Ok(Prepared { index, feed, scanned, losses, content, kept: kept_naming(source) })
}

struct Window<'p> {
    from: Option<i32>,
    to: Option<i32>,
    selection: Option<&'p Selection>,
    search: Option<&'p str>,
    sort: Option<&'p OccurrenceSort>,
    offset: Option<&'p str>,
    limit: u32,
}

fn answer(index: &Index<Item>, query: &Query<'_>, window: &Window<'_>, hour: i32, kinds: Option<&[u16]>, states: Option<&[u8]>, live_first: bool, chips: &[Chip]) -> io::Result<Answer> {
    let (order, newest) = occurrences::order(window.sort, &[])?;
    let (offset, limit) = occurrences::window(window.offset, window.limit)?;
    index.ask(
        &query.holders,
        &Ask { hour, from: window.from, to: window.to, selection: window.selection, kinds, states, search: window.search, order, live_first: live_first && newest, offset, limit, chips },
    )
}

fn locators(index: &Index<Item>, answer: &Answer, hour: i32) -> Vec<Locator> {
    let mut out = Vec::new();
    for shown in &answer.rows {
        if let Item::Log { head, edges } = &index.get(shown.at).payload {
            out.push(*head);
            out.extend(edges.iter().copied().filter(|edge| edge.hour <= hour));
        }
    }
    out
}

fn notices_for(prepared: &Prepared, answer: &Answer, from: i32, hour: i32, notices: &mut Notices) {
    families::incomplete_notices(&prepared.scanned.incomplete, &prepared.losses, from, hour, notices);
    families::window_notices(&prepared.scanned.windows_unrecorded, from, hour, notices);
    if answer.unattributed > 0 {
        notices.unavailable(
            "unattributed",
            "Country not identified",
            "These events name no country the recording identifies at their date, so the country selection cannot place them.",
            Some(u32::try_from(answer.unattributed).unwrap_or(u32::MAX)),
        );
    }
}

fn load_feed_at(prepared: &Prepared, query: &Query<'_>, params: &EventFeedTailParams, hour: i32, cancelled: &AtomicBool) -> io::Result<EventFeedTailReply> {
    dashboard_text::metadata::check_cancel(cancelled)?;
    let feed = prepared.feed.as_ref().ok_or_else(|| invalid("The event feed's index is not prepared"))?;
    let kinds: Vec<u16> = EventFeedKind::ALL.iter().filter(|kind| params.categories.contains(&kind.category())).map(|&kind| kind_code(kind)).collect();
    let window = Window { from: params.from, to: params.to, selection: params.selection.as_ref(), search: params.search.as_deref(), sort: params.sort.as_ref(), offset: params.offset.as_deref(), limit: params.limit };
    let answer = answer(&feed.index, query, &window, hour, Some(&kinds), None, false, &[])?;
    let read = index::read_events(query, &locators(&feed.index, &answer, hour))?;
    let names = Names::new(&query.holders);
    let mut notices = Notices::default();
    let presenter = Presenter::new(query, content_from(&prepared.content, &mut notices), Arc::clone(&prepared.kept));
    let mut events: Vec<&FeedEvent> = Vec::new();
    let mut shapes: Vec<(usize, Option<usize>)> = Vec::new();
    for shown in &answer.rows {
        let Item::Log { head, edges } = &feed.index.get(shown.at).payload else {
            shapes.push((usize::MAX, None));
            continue;
        };
        let head_at = events.len();
        events.push(read.get(&head.key).ok_or_else(|| invalid("An event in the window was not read back"))?);
        let choice = edges.iter().filter(|edge| edge.hour <= hour).filter_map(|edge| read.get(&edge.key)).rfind(|event| event.option.is_none_or(|option| option >= 0)).map(|event| {
            events.push(event);
            events.len() - 1
        });
        shapes.push((head_at, choice));
    }
    let mut presented = present_all(query, &events, &names, &presenter, &mut notices, cancelled)?;
    let rows = answer
        .rows
        .iter()
        .zip(shapes)
        .map(|(shown, (head_at, choice))| {
            let occurrence = feed.index.get(shown.at);
            let (row, choice) = match &occurrence.payload {
                Item::Family(row) => (Some(row.as_ref().clone()), None),
                Item::Log { .. } => {
                    let row = presented[head_at].take();
                    let choice = choice.and_then(|at| presented[at].take().map(|row| edge(events[at], row)));
                    (row, choice)
                }
            };
            EventFeedPageRow {
                key: occurrence.id.to_string(),
                hour: occurrence.date,
                observed: occurrence.observed,
                ordinal: shown.ordinal.to_string(),
                live: shown.live,
                row,
                choice,
                status: shown.state.and_then(ScriptStatus::of_code),
            }
        })
        .collect();
    content_refusals(&presenter, &mut notices)?;
    let from = params.from.unwrap_or(query.timeline.first);
    notices_for(prepared, &answer, from, hour, &mut notices);
    families::unread(&params.categories, &mut notices);
    for (categories, kind, label, reason, count) in &feed.notices {
        if categories.iter().any(|category| params.categories.contains(category)) {
            notices.unavailable(kind, label, reason.clone(), *count);
        }
    }
    if params.categories.contains(&EventFeedCategory::Milestones) {
        let hours: i64 = feed.projects_unrecorded.iter().map(|&(first, last)| (i64::from(last.min(hour)) - i64::from(first.max(from)) + 1).max(0)).sum();
        if hours > 0 {
            notices.unavailable("projectFinished", "Special projects", format!("Project histories were not recorded for {hours} hours of this span; a completion dated then is not listed."), None);
        }
    }
    let reply = EventFeedTailReply {
        branch_id: params.branch_id.clone(),
        total: answer.total.to_string(),
        offset: answer.offset.to_string(),
        live: answer.live.to_string(),
        counts: Vec::new(),
        rows,
        missing: answer.gaps,
        next_change: answer.next_change,
        unavailable: notices.into_list(),
        warnings: feed.warnings.clone(),
    };
    dashboard_base::products::check_reply_budget(&reply)?;
    Ok(reply)
}

fn chips(list: List) -> (Vec<Chip>, &'static [&'static str]) {
    match list {
        List::Decisions => (vec![Chip { kinds: None, states: Some(vec![ScriptStatus::Active.code()]) }], &["running"]),
        List::EventChoices => (vec![Chip { kinds: None, states: Some(vec![ScriptStatus::Awaiting.code()]) }], &["awaiting"]),
    }
}

fn load_cards_at(list: List, prepared: &Prepared, query: &Query<'_>, params: &ScriptTailParams, hour: i32, cancelled: &AtomicBool) -> io::Result<ScriptOccurrenceTailReply> {
    dashboard_text::metadata::check_cancel(cancelled)?;
    let kinds: Option<Vec<u16>> = params.kinds.as_ref().map(|kinds| {
        let mut codes: Vec<u16> = kinds.iter().filter(|kind| list.openers().contains(kind)).map(|&kind| kind_code(kind)).collect();
        if list == List::EventChoices {
            codes.push(kind_code(EventFeedKind::EventOptionSelected));
        } else {
            codes.extend(list.continuing().iter().map(|&kind| kind_code(kind)));
        }
        codes
    });
    let states: Option<Vec<u8>> = params.statuses.as_ref().map(|statuses| statuses.iter().map(|status| status.code()).collect());
    let (chips, keys) = chips(list);
    let window = Window { from: params.from, to: params.to, selection: params.selection.as_ref(), search: params.search.as_deref(), sort: params.sort.as_ref(), offset: params.offset.as_deref(), limit: params.limit };
    let answer = answer(&prepared.index, query, &window, hour, kinds.as_deref(), states.as_deref(), true, &chips)?;
    let read = index::read_events(query, &locators(&prepared.index, &answer, hour))?;
    let names = Names::new(&query.holders);
    let mut notices = Notices::default();
    let presenter = Presenter::new(query, content_from(&prepared.content, &mut notices), Arc::clone(&prepared.kept));
    let mut events: Vec<&FeedEvent> = Vec::new();
    let mut spans: Vec<(usize, usize, bool)> = Vec::new();
    for shown in &answer.rows {
        let occurrence = prepared.index.get(shown.at);
        let Item::Log { head, edges } = &occurrence.payload else { return Err(invalid("A card holds no feed rows")) };
        let first = events.len();
        for at in edges.iter().filter(|edge| edge.hour <= hour) {
            events.push(read.get(&at.key).ok_or_else(|| invalid("A card's edge was not read back"))?);
        }
        let opened = edges.first().is_some_and(|first| first == head) && EventFeedKind::of_code(events[first].code).is_some_and(|kind| list.openers().contains(&kind)) && events[first].opener.is_none();
        spans.push((first, events.len(), opened));
    }
    let presented = present_all(query, &events, &names, &presenter, &mut notices, cancelled)?;
    let mut rows = Vec::with_capacity(answer.rows.len());
    for (shown, (first, end, opened)) in answer.rows.iter().zip(spans) {
        let occurrence = prepared.index.get(shown.at);
        let mut card: Vec<Option<EventFeedRow>> = presented[first..end].to_vec();
        if list == List::Decisions {
            super::decision_state::enforce_same_incarnation(&events[first..end], &mut card);
        }
        let edges: Vec<ScriptEdge> = events[first..end].iter().zip(card).filter_map(|(event, row)| row.map(|row| edge(event, row))).collect();
        rows.push(ScriptPageRow {
            key: occurrence.id.to_string(),
            hour: occurrence.date,
            observed: occurrence.observed,
            ordinal: shown.ordinal.to_string(),
            live: shown.live,
            status: shown.state.and_then(ScriptStatus::of_code),
            occurrence: (!edges.is_empty()).then(|| ScriptOccurrence { id: occurrence.id.to_string(), opened, edges }),
        });
    }
    content_refusals(&presenter, &mut notices)?;
    notices_for(prepared, &answer, params.from.unwrap_or(query.timeline.first), hour, &mut notices);
    let reply = ScriptOccurrenceTailReply {
        branch_id: params.branch_id.clone(),
        total: answer.total.to_string(),
        offset: answer.offset.to_string(),
        live: answer.live.to_string(),
        counts: occurrences::counts(&answer, keys),
        rows,
        missing: answer.gaps,
        next_change: answer.next_change,
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
    type Params = EventFeedTailParams;
    const FIELDS: dashboard_base::products::Fields = TAIL_FIELDS;
    type Reply = EventFeedTailReply;

    #[cfg(any(test, feature = "export-types"))]
    fn types() -> Vec<String> {
        use dashboard_query::log::{CountrySpan, UnobservedSpan, occurrence::Direction};
        vec![
            CountrySpan::decl(),
            UnobservedSpan::decl(),
            Selection::decl(),
            Direction::decl(),
            OccurrenceSort::decl(),
            OccurrenceCount::decl(),
            PlacedGap::decl(),
            ScriptStatus::decl(),
            EventFeedTailParams::decl(),
            super::EventFeedCategory::decl(),
            EventFeedKind::decl(),
            super::EventFeedOption::decl(),
            super::EventFeedItem::decl(),
            super::EventFeedCause::decl(),
            super::DecisionLifecycleState::decl(),
            super::EventWindowDecision::decl(),
            super::EventWindowKind::decl(),
            super::EventWindowOption::decl(),
            super::EventWindowPresented::decl(),
            super::EventFeedWindow::decl(),
            EventFeedRow::decl(),
            super::EventFeedStrike::decl(),
            EventFeedUnavailable::decl(),
            EventFeedPageRow::decl(),
            EventFeedTailReply::decl(),
        ]
    }

    fn validate(params: &Self::Params) -> Result<(), &'static str> {
        branch(&params.branch_id)
    }

    fn load(source: &Source, params: &Self::Params, context: &LoadContext, cancelled: &AtomicBool) -> io::Result<Self::Reply> {
        let branch = branch_id(&params.branch_id)?;
        let query = dashboard_base::queries::query(source, branch)?;
        let hour = occurrences::cutoff(&query, params.hour)?;
        let prepared = prepare(source, &query, branch, context, None, cancelled)?;
        load_feed_at(&prepared, &query, params, hour, cancelled)
    }
}

fn parse<P: serde::de::DeserializeOwned>(name: &str, asked: &serde_json::Value) -> Result<P, String> {
    let mut request = asked.clone();
    request.as_object_mut().ok_or_else(|| format!("Invalid {name} panel"))?.remove("panel");
    dashboard_query::named::named(&request).map_err(|error| format!("Invalid {name} panel: {error}"))
}

pub fn event_feed_dated_panel(source: &Source, load: &LoadContext, asked: &serde_json::Value) -> Result<Box<dyn Panel>, String> {
    let params: EventFeedTailParams = parse(EventFeedTail::KEY, asked)?;
    EventFeedTail::validate(&params)?;
    let branch = branch_id(&params.branch_id).map_err(|error| error.to_string())?;
    let query = dashboard_base::queries::query(source, branch).map_err(|error| error.to_string())?;
    let prepared = prepare(source, &query, branch, load, None, &AtomicBool::new(false)).map_err(|error| error.to_string())?;
    let owned = Owned::of(&query, source.replay());
    Ok(occurrences::occurrence_panel(source.tables(), move |hour, cancelled| {
        let query = owned.query();
        let hour = occurrences::cutoff(&query, Some(hour))?;
        load_feed_at(&prepared, &query, &params, hour, cancelled)
    }))
}

fn cards_load(list: List, source: &Source, params: &ScriptTailParams, context: &LoadContext, cancelled: &AtomicBool) -> io::Result<ScriptOccurrenceTailReply> {
    let branch = branch_id(&params.branch_id)?;
    let query = dashboard_base::queries::query(source, branch)?;
    let hour = occurrences::cutoff(&query, params.hour)?;
    let prepared = prepare(source, &query, branch, context, Some(list), cancelled)?;
    load_cards_at(list, &prepared, &query, params, hour, cancelled)
}

fn cards_dated_panel(list: List, name: &str, source: &Source, load: &LoadContext, asked: &serde_json::Value) -> Result<Box<dyn Panel>, String> {
    let params: ScriptTailParams = parse(name, asked)?;
    branch(&params.branch_id)?;
    let branch = branch_id(&params.branch_id).map_err(|error| error.to_string())?;
    let query = dashboard_base::queries::query(source, branch).map_err(|error| error.to_string())?;
    let prepared = prepare(source, &query, branch, load, Some(list), &AtomicBool::new(false)).map_err(|error| error.to_string())?;
    let owned = Owned::of(&query, source.replay());
    Ok(occurrences::occurrence_panel(source.tables(), move |hour, cancelled| {
        let query = owned.query();
        let hour = occurrences::cutoff(&query, Some(hour))?;
        load_cards_at(list, &prepared, &query, &params, hour, cancelled)
    }))
}

pub struct EventChoicesTail;

impl Product for EventChoicesTail {
    const KEY: &'static str = "event-choices-tail";
    const CONTENT_DEPENDENT: bool = true;
    const REPLAYS: bool = true;
    type Params = ScriptTailParams;
    const FIELDS: dashboard_base::products::Fields = TAIL_FIELDS;
    type Reply = ScriptOccurrenceTailReply;

    #[cfg(any(test, feature = "export-types"))]
    fn types() -> Vec<String> {
        use super::cards::{ScriptCost, ScriptCurrency};
        vec![ScriptTailParams::decl(), ScriptCurrency::decl(), ScriptCost::decl(), ScriptEdge::decl(), ScriptOccurrence::decl(), ScriptPageRow::decl(), ScriptOccurrenceTailReply::decl()]
    }

    fn validate(params: &Self::Params) -> Result<(), &'static str> {
        branch(&params.branch_id)
    }

    fn load(source: &Source, params: &Self::Params, context: &LoadContext, cancelled: &AtomicBool) -> io::Result<Self::Reply> {
        cards_load(List::EventChoices, source, params, context, cancelled)
    }
}

pub fn event_choices_dated_panel(source: &Source, load: &LoadContext, asked: &serde_json::Value) -> Result<Box<dyn Panel>, String> {
    cards_dated_panel(List::EventChoices, EventChoicesTail::KEY, source, load, asked)
}

pub struct DecisionsTail;

impl Product for DecisionsTail {
    const KEY: &'static str = "decisions-tail";
    const CONTENT_DEPENDENT: bool = true;
    const REPLAYS: bool = true;
    type Params = ScriptTailParams;
    const FIELDS: dashboard_base::products::Fields = TAIL_FIELDS;
    type Reply = ScriptOccurrenceTailReply;

    #[cfg(any(test, feature = "export-types"))]
    fn types() -> Vec<String> {
        EventChoicesTail::types()
    }

    fn validate(params: &Self::Params) -> Result<(), &'static str> {
        branch(&params.branch_id)
    }

    fn load(source: &Source, params: &Self::Params, context: &LoadContext, cancelled: &AtomicBool) -> io::Result<Self::Reply> {
        cards_load(List::Decisions, source, params, context, cancelled)
    }
}

pub fn decisions_dated_panel(source: &Source, load: &LoadContext, asked: &serde_json::Value) -> Result<Box<dyn Panel>, String> {
    cards_dated_panel(List::Decisions, DecisionsTail::KEY, source, load, asked)
}
