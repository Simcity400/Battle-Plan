
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
    identity::{Country, Holders},
    log::occurrence::{Answer, Ask, Chip, Index, PlacedGap, Selection, Shown},
    series::ranked::GroupSelection,
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
    #[doc = " Rows naming one of these countries at the row's date; every row when neither this nor `group`."]
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[cfg_attr(any(test, feature = "export-types"), ts(optional))]
    pub countries: Option<Vec<Country>>,
    #[doc = " Rows naming a member of the group at the row's date, membership resolved in Rust."]
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[cfg_attr(any(test, feature = "export-types"), ts(optional))]
    pub group: Option<GroupSelection>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[cfg_attr(any(test, feature = "export-types"), ts(optional))]
    pub search: Option<String>,
    #[doc = " One row by its key, with the detail text list rows leave out; no row when it is not in the log at the hour."]
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[cfg_attr(any(test, feature = "export-types"), ts(optional))]
    pub row: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[cfg_attr(any(test, feature = "export-types"), ts(optional))]
    pub sort: Option<OccurrenceSort>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[cfg_attr(any(test, feature = "export-types"), ts(optional))]
    pub offset: Option<String>,
    pub limit: u32,
    pub categories: Vec<EventFeedCategory>,
    #[doc = " Kinds left out of the categories asked, by name; an unknown name is refused."]
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[cfg_attr(any(test, feature = "export-types"), ts(optional))]
    pub exclude: Option<Vec<String>>,
}

#[derive(Clone, Debug, PartialEq, Deserialize, Serialize)]
#[cfg_attr(any(test, feature = "export-types"), derive(TS))]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct EventFeedActivityParams {
    pub branch_id: String,
    #[doc = " The observation cutoff and the last hour binned."]
    pub hour: i32,
    #[doc = " The first hour binned; rows dated before it are counted in `before`."]
    pub from: i32,
    #[cfg_attr(any(test, feature = "export-types"), ts(type = "number"))]
    pub bin_hours: i64,
    pub categories: Vec<EventFeedCategory>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[cfg_attr(any(test, feature = "export-types"), ts(optional))]
    pub exclude: Option<Vec<String>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[cfg_attr(any(test, feature = "export-types"), ts(optional))]
    pub countries: Option<Vec<Country>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[cfg_attr(any(test, feature = "export-types"), ts(optional))]
    pub group: Option<GroupSelection>,
}

#[derive(Clone, Debug, PartialEq, Serialize)]
#[cfg_attr(any(test, feature = "export-types"), derive(TS))]
#[serde(rename_all = "camelCase")]
pub struct EventFeedActivityLine {
    pub category: EventFeedCategory,
    pub before: String,
    #[doc = " Per bin with rows: its index from `firstHour` and its count."]
    #[cfg_attr(any(test, feature = "export-types"), ts(type = "Array<[number, number]>"))]
    pub bins: Vec<(u32, u64)>,
}

#[derive(Clone, Debug, PartialEq, Serialize)]
#[cfg_attr(any(test, feature = "export-types"), derive(TS))]
#[serde(rename_all = "camelCase")]
pub struct EventFeedActivityReply {
    pub branch_id: String,
    #[doc = " The first hour of bin 0."]
    pub first_hour: i32,
    #[cfg_attr(any(test, feature = "export-types"), ts(type = "number"))]
    pub bin_hours: i64,
    pub lines: Vec<EventFeedActivityLine>,
    #[doc = " Hours of the span the feed does not hold, cut at the hour asked."]
    pub missing: Vec<dashboard_query::series::MissingSpan>,
    pub warnings: Vec<String>,
}

dashboard_base::reply_warnings!(EventFeedActivityReply);

#[derive(Clone, Debug, PartialEq, Serialize)]
#[cfg_attr(any(test, feature = "export-types"), derive(TS))]
#[serde(rename_all = "camelCase")]
pub struct EventFeedKindInfo {
    pub kind: EventFeedKind,
    pub category: EventFeedCategory,
}

#[derive(Clone, Debug, PartialEq, Serialize)]
#[cfg_attr(any(test, feature = "export-types"), derive(TS))]
#[serde(rename_all = "camelCase")]
pub struct EventFeedKindsReply {
    pub kinds: Vec<EventFeedKindInfo>,
    pub warnings: Vec<String>,
}

dashboard_base::reply_warnings!(EventFeedKindsReply);

fn feed_kinds(categories: &[EventFeedCategory], exclude: Option<&[String]>) -> io::Result<Vec<u16>> {
    let mut left_out = Vec::new();
    for name in exclude.unwrap_or_default() {
        let kind: EventFeedKind = serde_json::from_value(serde_json::Value::String(name.clone())).map_err(|_| invalid(format!("The event feed has no kind {name}")))?;
        left_out.push(kind);
    }
    Ok(EventFeedKind::ALL.iter().filter(|kind| categories.contains(&kind.category()) && !left_out.contains(kind)).map(|&kind| kind_code(kind)).collect())
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
    #[doc = " Rows naming one of these countries at the row's date; every row when neither this nor `group`."]
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[cfg_attr(any(test, feature = "export-types"), ts(optional))]
    pub countries: Option<Vec<Country>>,
    #[doc = " Rows naming a member of the group at the row's date, membership resolved in Rust."]
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[cfg_attr(any(test, feature = "export-types"), ts(optional))]
    pub group: Option<GroupSelection>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[cfg_attr(any(test, feature = "export-types"), ts(optional))]
    pub search: Option<String>,
    #[doc = " One row by its key, with the detail text list rows leave out; no row when it is not in the log at the hour."]
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[cfg_attr(any(test, feature = "export-types"), ts(optional))]
    pub row: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[cfg_attr(any(test, feature = "export-types"), ts(optional))]
    pub sort: Option<OccurrenceSort>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[cfg_attr(any(test, feature = "export-types"), ts(optional))]
    pub offset: Option<String>,
    pub limit: u32,
    #[doc = " The events whose choices are listed; every one when none. Decisions take none."]
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[cfg_attr(any(test, feature = "export-types"), ts(optional))]
    pub events: Option<Vec<ScriptEventKind>>,
    #[doc = " The statuses listed, each at the hour asked; every one when none."]
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[cfg_attr(any(test, feature = "export-types"), ts(optional))]
    pub statuses: Option<Vec<ScriptStatus>>,
}

#[derive(Clone, Copy, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[cfg_attr(any(test, feature = "export-types"), derive(TS))]
#[serde(rename_all = "camelCase")]
pub enum ScriptEventKind {
    Country,
    News,
}

impl ScriptEventKind {
    fn kind(self) -> EventFeedKind {
        match self {
            Self::Country => EventFeedKind::ScriptEventFired,
            Self::News => EventFeedKind::NewsEventFired,
        }
    }
}

const FEED_FIELDS: dashboard_base::products::Fields =
    dashboard_base::fields![branchId, hour: Read, from: Start, to: End, countries, "countries[]", group, search, sort, offset, limit, categories, exclude, "exclude[]", row];

const CARD_FIELDS: dashboard_base::products::Fields =
    dashboard_base::fields![branchId, hour: Read, from: Start, to: End, countries, "countries[]", group, search, sort, offset, limit, events, statuses, row];

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
    selection: Option<Selection>,
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

fn prepare(source: &Source, query: &Query<'_>, branch: u64, context: &LoadContext, list: Option<List>, asked: (Option<&[Country]>, Option<&GroupSelection>), cancelled: &AtomicBool) -> io::Result<Prepared> {
    let selection = dashboard_world::products::group_members::dated(source, query, context, asked.0, asked.1, cancelled)?;
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
    Ok(Prepared { selection, index, feed, scanned, losses, content, kept: kept_naming(source) })
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
        &Ask { hour, from: window.from, to: window.to, gap_end: None, selection: window.selection, subject: None, kinds, states, search: window.search, order, live_first: live_first && newest, offset, limit, chips },
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

fn next_change(prepared: &Prepared, answer: &Answer, windows: bool, hour: i32) -> Option<i32> {
    let incomplete = &prepared.scanned.incomplete;
    let read = incomplete.get(incomplete.partition_point(|(at, ..)| *at <= hour)).map(|(at, ..)| *at);
    let reason = windows.then(|| prepared.scanned.windows_unrecorded.iter().map(|(first, ..)| *first).filter(|first| *first > hour).min()).flatten();
    [answer.next_change, read, reason].into_iter().flatten().min()
}

fn feed_next_change(feed: &Feed, categories: &[EventFeedCategory], (from, to): (i32, i32), hour: i32) -> Option<i32> {
    let warning = feed.warnings.iter().map(|(at, _)| *at).filter(|at| *at > hour).min();
    let left_out = categories.contains(&EventFeedCategory::WarsPeace).then(|| feed.unattributed.get(feed.unattributed.partition_point(|&at| at <= hour)).copied()).flatten();
    let unrecorded = feed
        .unrecorded
        .iter()
        .filter(|family| categories.contains(&family.category))
        .flat_map(|family| &family.spans)
        .filter(|&&(first, last)| first <= to && last >= from)
        .map(|&(first, last)| (first.max(from), last.min(to)))
        .filter_map(|(first, last)| if first <= hour && hour < last { Some(hour + 1) } else { (first > hour).then_some(first) })
        .min();
    [warning, left_out, unrecorded].into_iter().flatten().min()
}

fn notices_for(prepared: &Prepared, answer: &Answer, windows: bool, from: i32, hour: i32, notices: &mut Notices) {
    families::incomplete_notices(&prepared.scanned.incomplete, &prepared.losses, from, hour, notices);
    if windows {
        families::window_notices(&prepared.scanned.windows_unrecorded, from, hour, notices);
    }
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
    let kinds = feed_kinds(&params.categories, params.exclude.as_deref())?;
    let window = Window { from: params.from, to: params.to, selection: prepared.selection.as_ref(), search: params.search.as_deref(), sort: params.sort.as_ref(), offset: params.offset.as_deref(), limit: params.limit };
    let mut answer = answer(&feed.index, query, &window, hour, Some(&kinds), None, false, &[])?;
    let opened = only_row(&prepared.scanned, &feed.index, params.row.as_deref(), &mut answer, hour);
    let read = index::read_events(query, &locators(&feed.index, &answer, hour))?;
    let names = Names::new(&query.holders);
    let mut notices = Notices::default();
    let presenter = Presenter::new(query, content_from(&prepared.content, &mut notices), Arc::clone(&prepared.kept));
    let presenter = if opened { presenter } else { presenter.without_effects() };
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
    let mut presented = present_all(query, &prepared.scanned, &events, &names, &presenter, &mut notices, cancelled)?;
    if !opened {
        listed(&mut presented, false, true);
    }
    let rows = answer
        .rows
        .iter()
        .zip(shapes)
        .map(|(shown, (head_at, choice))| {
            let occurrence = feed.index.get(shown.at);
            let (key, mut row, mut choice) = match &occurrence.payload {
                Item::Family(row) => (row.id.clone(), Some(row.as_ref().clone()), None),
                Item::Log { .. } => {
                    let row = presented[head_at].take();
                    let choice = choice.and_then(|at| presented[at].take().map(|row| edge(events[at], row)));
                    (events[head_at].id(), row, choice)
                }
            };
            if !opened {
                let chosen = choice.as_ref().and_then(|choice| choice.option).or_else(|| row.as_ref().and_then(|row| row.cause.as_ref()).and_then(|cause| cause.selected_option));
                if let Some(item) = row.as_mut().and_then(|row| row.item.as_mut()) {
                    chosen_names(item, chosen);
                }
                let listed_options = row.as_ref().and_then(|row| row.item.as_ref()).is_some_and(|item| item.options.as_ref().is_some_and(|options| !options.is_empty()));
                if let Some(choice) = choice.as_mut() {
                    choice.row.cause = None;
                    choice.row.window = None;
                    if listed_options {
                        choice.row.item = None;
                    }
                }
            }
            EventFeedPageRow {
                key,
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
    notices_for(prepared, &answer, true, from, hour, &mut notices);
    families::unread(&params.categories, &mut notices);
    for (categories, kind, label, reason, count) in &feed.notices {
        if categories.iter().any(|category| params.categories.contains(category)) {
            notices.unavailable(kind, label, reason.clone(), *count);
        }
    }
    if params.categories.contains(&EventFeedCategory::WarsPeace) {
        let left_out = feed.unattributed.partition_point(|&at| at <= hour).saturating_sub(feed.unattributed.partition_point(|&at| at < from));
        if left_out > 0 {
            notices.unavailable(
                "warUnattributed",
                "War starts and capitulations",
                "These war starts or capitulations name a country no link resolved at their date, so they are left out.",
                Some(u32::try_from(left_out).unwrap_or(u32::MAX)),
            );
        }
    }
    let to = params.to.map_or(hour, |to| to.min(hour));
    for family in feed.unrecorded.iter().filter(|family| params.categories.contains(&family.category)) {
        let hours: i64 = family.spans.iter().map(|&(first, last)| (i64::from(last.min(to)) - i64::from(first.max(from)) + 1).max(0)).sum();
        if hours > 0 {
            notices.unavailable(family.kind, family.label, format!("{} {hours} hours of this span {}.", family.listed, family.why), None);
        }
    }
    let reply = EventFeedTailReply {
        branch_id: params.branch_id.clone(),
        total: answer.total.to_string(),
        offset: answer.offset.to_string(),
        live: answer.live.to_string(),
        counts: Vec::new(),
        rows,
        next_change: [next_change(prepared, &answer, true, hour), feed_next_change(feed, &params.categories, (from, params.to.unwrap_or(i32::MAX)), hour)].into_iter().flatten().min(),
        missing: answer.gaps,
        unavailable: notices.into_list(),
        warnings: feed.warnings.iter().filter(|(at, _)| *at <= hour).map(|(_, text)| text.clone()).collect(),
    };
    dashboard_base::products::check_reply_budget(&reply)?;
    Ok(reply)
}

const STATUSES_LISTED: [ScriptStatus; 9] = [
    ScriptStatus::Active,
    ScriptStatus::Taken,
    ScriptStatus::Completed,
    ScriptStatus::Failed,
    ScriptStatus::Cancelled,
    ScriptStatus::TimedOut,
    ScriptStatus::Removed,
    ScriptStatus::Unsupported,
    ScriptStatus::NotRecorded,
];

fn chips(list: List) -> (Vec<Chip>, &'static [&'static str]) {
    match list {
        List::Decisions => (vec![Chip { kinds: None, states: Some(vec![ScriptStatus::Active.code()]) }], &["running"]),
        List::EventChoices => (vec![Chip { kinds: None, states: Some(vec![ScriptStatus::Awaiting.code()]) }], &["awaiting"]),
    }
}

fn only_row(scanned: &index::Scanned, index: &Index<Item>, row: Option<&str>, answer: &mut Answer, hour: i32) -> bool {
    let Some(row) = row else { return false };
    let found = scanned.locate(row).and_then(|key| index.find(key)).or_else(|| index::family_at(index, row)).filter(|&at| index.get(at).observed <= hour);
    answer.rows = found
        .map(|at| {
            let state = index.get(at).state_at(hour);
            Shown { at, ordinal: 0, live: state.is_some_and(|state| index::LIVE.contains(&state)), state }
        })
        .into_iter()
        .collect();
    true
}

pub(super) fn listed_choice(edges: &[ScriptEdge]) -> Option<i32> {
    edges.iter().rfind(|edge| edge.row.kind == EventFeedKind::EventOptionSelected && edge.option.is_none_or(|option| option >= 0)).and_then(|edge| edge.option)
}

fn chosen_names(item: &mut super::EventFeedItem, chosen: Option<i32>) {
    for option in item.options.iter_mut().flatten() {
        if chosen.is_none_or(|chosen| i64::from(chosen) != i64::from(option.ordinal)) {
            option.name = None;
        }
    }
}

pub(super) fn listed(rows: &mut [Option<EventFeedRow>], description: bool, window: bool) {
    for row in rows.iter_mut().flatten() {
        if let Some(cause) = row.cause.as_mut() {
            cause.notes.clear();
        }
        if row.cause.as_ref().is_some_and(|cause| {
            cause.how.is_none() && cause.within.is_none() && cause.conference.is_none() && cause.state.is_none() && cause.ally.is_none() && cause.reason.is_none() && cause.selected_option.is_none() && !cause.target_recorded
        }) {
            row.cause = None;
        }
        if let Some(item) = row.item.as_mut() {
            item.gaps = None;
            if !description {
                item.description = None;
            }
        }
        if !window {
            row.window = None;
        } else if let Some(presented) = row.window.as_mut().and_then(|window| window.presented.as_mut()) {
            presented.description = None;
            presented.options = None;
            presented.not_offered = None;
        }
        if matches!(row.decision_state, Some(super::DecisionLifecycleState::NotApplicable { .. })) {
            row.decision_state = None;
        }
    }
}

fn load_cards_at(list: List, prepared: &Prepared, query: &Query<'_>, params: &ScriptTailParams, hour: i32, cancelled: &AtomicBool) -> io::Result<ScriptOccurrenceTailReply> {
    dashboard_text::metadata::check_cancel(cancelled)?;
    if list == List::Decisions && params.events.is_some() {
        return Err(invalid("Decisions are not filtered by event kind"));
    }
    let kinds: Option<Vec<u16>> = params.events.as_ref().map(|events| {
        let mut codes: Vec<u16> = events.iter().map(|event| kind_code(event.kind())).collect();
        codes.push(kind_code(EventFeedKind::EventOptionSelected));
        codes
    });
    let states: Option<Vec<u8>> = match (&params.statuses, list) {
        (Some(statuses), _) => Some(statuses.iter().map(|status| status.code()).collect()),
        (None, List::Decisions) => Some(STATUSES_LISTED.iter().map(|status| status.code()).collect()),
        (None, List::EventChoices) => None,
    };
    let (chips, keys) = chips(list);
    let window = Window { from: params.from, to: params.to, selection: prepared.selection.as_ref(), search: params.search.as_deref(), sort: params.sort.as_ref(), offset: params.offset.as_deref(), limit: params.limit };
    let mut answer = answer(&prepared.index, query, &window, hour, kinds.as_deref(), states.as_deref(), true, &chips)?;
    let opened_row = only_row(&prepared.scanned, &prepared.index, params.row.as_deref(), &mut answer, hour);
    let read = index::read_events(query, &locators(&prepared.index, &answer, hour))?;
    let names = Names::new(&query.holders);
    let mut notices = Notices::default();
    let presenter = Presenter::new(query, content_from(&prepared.content, &mut notices), Arc::clone(&prepared.kept));
    let presenter = if opened_row { presenter } else { presenter.without_effects() };
    let mut events: Vec<&FeedEvent> = Vec::new();
    let mut spans: Vec<(usize, usize, bool)> = Vec::new();
    for shown in &answer.rows {
        let occurrence = prepared.index.get(shown.at);
        let Item::Log { head, edges } = &occurrence.payload else { return Err(invalid("A card holds no feed rows")) };
        let first = events.len();
        for at in edges.iter().filter(|edge| edge.hour <= hour) {
            events.push(read.get(&at.key).ok_or_else(|| invalid("A card's edge was not read back"))?);
        }
        let opened = edges.first().is_some_and(|first| first == head) && events.get(first).is_some_and(|event| EventFeedKind::of_code(event.code).is_some_and(|kind| list.openers().contains(&kind)) && event.opener.is_none());
        spans.push((first, events.len(), opened));
    }
    let presented = present_all(query, &prepared.scanned, &events, &names, &presenter, &mut notices, cancelled)?;
    let mut rows = Vec::with_capacity(answer.rows.len());
    for (shown, (first, end, opened)) in answer.rows.iter().zip(spans) {
        let occurrence = prepared.index.get(shown.at);
        let mut card: Vec<Option<EventFeedRow>> = presented[first..end].to_vec();
        if list == List::Decisions {
            super::decision_state::enforce_same_incarnation(&events[first..end], &mut card);
        }
        if !opened_row {
            listed(&mut card, list == List::Decisions, list == List::Decisions);
        }
        let mut edges: Vec<ScriptEdge> = events[first..end].iter().zip(card).filter_map(|(event, row)| row.map(|row| edge(event, row))).collect();
        if !opened_row {
            let chosen = listed_choice(&edges);
            let mut named = false;
            for edge in &mut edges {
                if named {
                    edge.row.item = None;
                }
                named |= edge.row.item.is_some();
                if let Some(item) = edge.row.item.as_mut() {
                    chosen_names(item, chosen);
                }
            }
        }
        let Item::Log { head, .. } = &occurrence.payload else { return Err(invalid("A card holds no feed rows")) };
        let id = read.get(&head.key).map(FeedEvent::id).ok_or_else(|| invalid("A card's head was not read back"))?;
        rows.push(ScriptPageRow {
            key: id.clone(),
            hour: occurrence.date,
            observed: occurrence.observed,
            ordinal: shown.ordinal.to_string(),
            live: shown.live,
            status: shown.state.and_then(ScriptStatus::of_code),
            occurrence: (!edges.is_empty()).then(|| ScriptOccurrence { id, opened, edges }),
        });
    }
    content_refusals(&presenter, &mut notices)?;
    notices_for(prepared, &answer, list == List::EventChoices, params.from.unwrap_or(query.timeline.first), hour, &mut notices);
    let reply = ScriptOccurrenceTailReply {
        branch_id: params.branch_id.clone(),
        total: answer.total.to_string(),
        offset: answer.offset.to_string(),
        live: answer.live.to_string(),
        counts: occurrences::counts(&answer, keys),
        rows,
        next_change: next_change(prepared, &answer, list == List::EventChoices, hour),
        missing: answer.gaps,
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
    const FIELDS: dashboard_base::products::Fields = FEED_FIELDS;
    type Reply = EventFeedTailReply;

    #[cfg(any(test, feature = "export-types"))]
    fn types() -> Vec<String> {
        use dashboard_query::{log::occurrence::Direction, series::ranked::{GroupKind, GroupMembership}};
        vec![
            Country::decl(),
            GroupKind::decl(),
            GroupMembership::decl(),
            GroupSelection::decl(),
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
        let prepared = prepare(source, &query, branch, context, None, (params.countries.as_deref(), params.group.as_ref()), cancelled)?;
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
    let prepared = prepare(source, &query, branch, load, None, (params.countries.as_deref(), params.group.as_ref()), &AtomicBool::new(false)).map_err(|error| error.to_string())?;
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
    let prepared = prepare(source, &query, branch, context, Some(list), (params.countries.as_deref(), params.group.as_ref()), cancelled)?;
    load_cards_at(list, &prepared, &query, params, hour, cancelled)
}

fn cards_dated_panel(list: List, name: &str, source: &Source, load: &LoadContext, asked: &serde_json::Value) -> Result<Box<dyn Panel>, String> {
    let params: ScriptTailParams = parse(name, asked)?;
    branch(&params.branch_id)?;
    let branch = branch_id(&params.branch_id).map_err(|error| error.to_string())?;
    let query = dashboard_base::queries::query(source, branch).map_err(|error| error.to_string())?;
    let prepared = prepare(source, &query, branch, load, Some(list), (params.countries.as_deref(), params.group.as_ref()), &AtomicBool::new(false)).map_err(|error| error.to_string())?;
    let owned = Owned::of(&query, source.replay());
    Ok(occurrences::occurrence_panel(source.tables(), move |hour, cancelled| {
        let query = owned.query();
        let hour = occurrences::cutoff(&query, Some(hour))?;
        load_cards_at(list, &prepared, &query, &params, hour, cancelled)
    }))
}

pub struct EventFeedActivity;

impl Product for EventFeedActivity {
    const KEY: &'static str = "event-feed-activity";
    const CONTENT_DEPENDENT: bool = true;
    type Params = EventFeedActivityParams;
    const FIELDS: dashboard_base::products::Fields =
        dashboard_base::fields![branchId, hour: Read, from: Start, binHours, categories, exclude, "exclude[]", countries, "countries[]", group];
    type Reply = EventFeedActivityReply;

    #[cfg(any(test, feature = "export-types"))]
    fn types() -> Vec<String> {
        vec![EventFeedActivityParams::decl(), EventFeedActivityLine::decl(), EventFeedActivityReply::decl()]
    }

    fn validate(params: &Self::Params) -> Result<(), &'static str> {
        branch(&params.branch_id)
    }

    fn load(source: &Source, params: &Self::Params, context: &LoadContext, cancelled: &AtomicBool) -> io::Result<Self::Reply> {
        let branch = branch_id(&params.branch_id)?;
        let query = dashboard_base::queries::query(source, branch)?;
        let hour = occurrences::cutoff(&query, Some(params.hour))?;
        if params.from > hour || params.bin_hours < 1 {
            return Err(invalid("Invalid activity span"));
        }
        let prepared = prepare(source, &query, branch, context, None, (params.countries.as_deref(), params.group.as_ref()), cancelled)?;
        let feed = prepared.feed.as_ref().ok_or_else(|| invalid("The event feed's index is not prepared"))?;
        let whole = Ask {
            hour,
            from: None,
            to: None,
            gap_end: None,
            selection: prepared.selection.as_ref(),
            subject: None,
            kinds: None,
            states: None,
            search: None,
            order: dashboard_query::log::occurrence::Order::Date(dashboard_query::log::occurrence::Direction::Descending),
            live_first: false,
            offset: 0,
            limit: 1,
            chips: &[],
        };
        let mut lines = Vec::with_capacity(params.categories.len());
        for &category in &params.categories {
            dashboard_text::metadata::check_cancel(cancelled)?;
            let kinds = feed_kinds(&[category], params.exclude.as_deref())?;
            let (before, bins) = feed.index.bins(&query.holders, &Ask { kinds: Some(&kinds), ..whole }, params.from, hour, params.bin_hours)?;
            lines.push(EventFeedActivityLine { category, before: before.to_string(), bins });
        }
        let missing = feed.index.coverage(&query.holders, &whole, params.from, hour);
        Ok(EventFeedActivityReply {
            branch_id: params.branch_id.clone(),
            first_hour: (i64::from(params.from).div_euclid(params.bin_hours) * params.bin_hours) as i32,
            bin_hours: params.bin_hours,
            lines,
            missing,
            warnings: Vec::new(),
        })
    }
}

pub struct EventFeedKinds;

impl Product for EventFeedKinds {
    const KEY: &'static str = "event-feed-kinds";
    const CONTENT_DEPENDENT: bool = false;
    type Params = dashboard_base::products::BranchParams;
    const FIELDS: dashboard_base::products::Fields = dashboard_base::fields![branchId];
    type Reply = EventFeedKindsReply;

    #[cfg(any(test, feature = "export-types"))]
    fn types() -> Vec<String> {
        vec![EventFeedKindInfo::decl(), EventFeedKindsReply::decl()]
    }

    fn load(_source: &Source, _params: &Self::Params, _context: &LoadContext, _cancelled: &AtomicBool) -> io::Result<Self::Reply> {
        Ok(EventFeedKindsReply {
            kinds: EventFeedKind::ALL.iter().filter(|kind| !super::index::internal(**kind)).map(|&kind| EventFeedKindInfo { kind, category: kind.category() }).collect(),
            warnings: Vec::new(),
        })
    }
}

pub struct EventChoicesTail;

impl Product for EventChoicesTail {
    const KEY: &'static str = "event-choices-tail";
    const CONTENT_DEPENDENT: bool = true;
    const REPLAYS: bool = true;
    type Params = ScriptTailParams;
    const FIELDS: dashboard_base::products::Fields = CARD_FIELDS;
    type Reply = ScriptOccurrenceTailReply;

    #[cfg(any(test, feature = "export-types"))]
    fn types() -> Vec<String> {
        use super::cards::{ScriptCost, ScriptCurrency};
        vec![ScriptEventKind::decl(), ScriptTailParams::decl(), ScriptCurrency::decl(), ScriptCost::decl(), ScriptEdge::decl(), ScriptOccurrence::decl(), ScriptPageRow::decl(), ScriptOccurrenceTailReply::decl()]
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
    const FIELDS: dashboard_base::products::Fields = CARD_FIELDS;
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
