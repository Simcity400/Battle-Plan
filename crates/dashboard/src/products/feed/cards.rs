//! Event choices and decisions: pages of cards, each an opening occurrence with the rows
//! continuing it. An event card opens at a dispatch and continues with the option chosen for it;
//! a decision card opens at a selection (or an activation with none open) and continues with its
//! activation, duration writes, effect executions and terminal steps. The feed writer joined them
//! as it wrote them (a row's `opener` and `opener_hour`): nothing here guesses a join. A page's
//! rows are the log's rows of the card kinds that continue nothing (the `opener` facet without a
//! value); a continuing row whose opening was not recorded is a card of its own. A card's edges
//! are found through the `opener` facet at its opener's read hour, read only in the buckets that
//! hold them, through the page's last hour.

use super::{
    EventFeedKind, EventFeedRow, EventFeedUnavailable, FeedEvent, FeedPageParams, LOG, Names, Notices, Presenter, branch, content, content_refusals, invalid, logs, page,
    present_all, kept_naming,
};
use crate::Source;
use dashboard_query::{
    Query,
    log::{FacetFilter, Reader},
    series::{MissingSpan, PAGE_LIMIT, hour_of},
};
use serde::{Deserialize, Serialize};
use std::{
    collections::{BTreeMap, BTreeSet},
    io,
    sync::atomic::AtomicBool,
};
use ts_rs::TS;

use crate::products::{LoadContext, Product};

/// Which list a page belongs to.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum List {
    EventChoices,
    Decisions,
}

impl List {
    /// The kinds that open a card.
    pub fn openers(self) -> &'static [EventFeedKind] {
        use EventFeedKind::*;
        match self {
            Self::EventChoices => &[ScriptEventFired, NewsEventFired],
            Self::Decisions => &[DecisionSelected, DecisionActivated],
        }
    }

    /// The kinds that continue a card; unjoined, each is a card of its own.
    pub fn continuing(self) -> &'static [EventFeedKind] {
        use EventFeedKind::*;
        match self {
            Self::EventChoices => &[EventOptionSelected],
            Self::Decisions => &[
                DecisionActivated,
                DecisionDurationChanged,
                DecisionRemovalEffect,
                DecisionCancellationEffect,
                DecisionCompletionEffect,
                DecisionTimeoutEffect,
                DecisionCompleted,
                DecisionCancelled,
                DecisionTimedOut,
                DecisionRemoved,
            ],
        }
    }
}

#[derive(Clone, Copy, Debug, Deserialize, Serialize, PartialEq, Eq, PartialOrd, Ord, TS)]
#[serde(rename_all = "camelCase")]
pub enum ScriptCurrency {
    PoliticalPower,
    CommandPower,
}

/// One currency a decision execution was proven to charge.
#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq, TS)]
#[serde(rename_all = "camelCase")]
pub struct ScriptCost {
    pub currency: ScriptCurrency,
    /// The exact fixed5 balance change as a decimal string; negative when spent.
    pub amount: String,
}

/// One fact of a card with the native operands its wording needs.
#[derive(Clone, Debug, Serialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct ScriptEdge {
    pub row: EventFeedRow,
    /// The executed option's signed native ordinal.
    pub option: Option<i32>,
    /// The writer's lifecycle result, where its boundary supplies one.
    pub lifecycle: Option<i32>,
    /// The writer's duration result in days.
    pub days: Option<i32>,
    /// The power the execution was proven to charge; null when not proven.
    pub costs: Option<Vec<ScriptCost>>,
}

/// One card: its opener first when recorded, then the rows continuing it in stream order, each
/// through the page's last hour.
#[derive(Clone, Debug, Serialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct ScriptOccurrence {
    pub id: String,
    /// Whether the first edge opened the card; false for a continuing row whose opening was not
    /// recorded or joined.
    pub opened: bool,
    pub edges: Vec<ScriptEdge>,
}

/// A row of a card page: `log-page`'s row (no cells) with its card; null for a row the list does
/// not show.
#[derive(Clone, Debug, Serialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct ScriptPageRow {
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
    pub occurrence: Option<ScriptOccurrence>,
}

#[derive(Clone, Debug, Serialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct ScriptOccurrencePage {
    pub branch_id: String,
    pub log: String,
    pub columns: Vec<String>,
    pub first_ordinal: String,
    pub total: String,
    pub cursor: Option<String>,
    pub rows: Vec<ScriptPageRow>,
    pub missing: Vec<MissingSpan>,
    pub unavailable: Vec<EventFeedUnavailable>,
    pub warnings: Vec<String>,
}

dashboard_base::reply_warnings!(ScriptOccurrencePage);

/// What a card's edge says beyond its row.
pub(crate) fn edge(event: &FeedEvent, row: EventFeedRow) -> ScriptEdge {
    let costs = event.costs.and_then(|[political, command]| {
        (political.is_some() || command.is_some()).then(|| {
            [(ScriptCurrency::PoliticalPower, political), (ScriptCurrency::CommandPower, command)]
                .into_iter()
                .filter_map(|(currency, amount)| amount.filter(|amount| *amount != 0).map(|amount| ScriptCost { currency, amount: amount.to_string() }))
                .collect()
        })
    });
    ScriptEdge { row, option: event.option, lifecycle: event.lifecycle, days: event.days, costs }
}

/// The rows continuing `openers` (by generation and sequence, each with its read hour) read
/// through `to`, in stream order: found through the `opener` facet at each opener's read hour.
pub(crate) fn continuing(query: &Query<'_>, list: List, openers: &[(u64, u64, i32)], to: i32, cancelled: &AtomicBool) -> io::Result<BTreeMap<(u64, u64), Vec<FeedEvent>>> {
    let mut out: BTreeMap<(u64, u64), Vec<FeedEvent>> = BTreeMap::new();
    let hours: BTreeSet<i32> = openers.iter().map(|(.., hour)| *hour).filter(|hour| *hour <= to).collect();
    let Some(&from) = hours.first() else { return Ok(out) };
    let wanted: BTreeSet<(u64, u64)> = openers.iter().map(|(generation, sequence, _)| (*generation, *sequence)).collect();
    let declared = dashboard_declare::log(LOG).ok_or_else(|| invalid("The feed's log is not declared"))?;
    let reader = Reader::new(query, declared)?;
    let filters = [
        FacetFilter { name: "kind".into(), values: list.continuing().iter().filter_map(|kind| kind.code()).map(i64::from).collect(), tags: Vec::new() },
        FacetFilter { name: "opener".into(), values: hours.iter().map(|hour| i64::from(*hour)).collect(), tags: Vec::new() },
    ];
    let selection = reader.select(&filters, query.timeline.first, to)?;
    let columns = super::row::Columns::new()?;
    let mut cursor = 0;
    loop {
        crate::metadata::check_cancel(cancelled)?;
        let page = reader.page(&selection.patterns, selection.groups.len(), from, to, cursor, PAGE_LIMIT)?;
        for paged in &page.rows {
            let event = columns.read(&paged.row)?;
            if let Some(opener) = event.opener
                && wanted.contains(&(event.generation, opener))
            {
                out.entry((event.generation, opener)).or_default().push(event);
            }
        }
        match page.cursor() {
            Some(next) => cursor = next,
            None => break,
        }
    }
    Ok(out)
}

/// A page of `list`'s cards.
fn cards(list: List, source: &Source, params: &FeedPageParams, context: &LoadContext, cancelled: &AtomicBool) -> io::Result<ScriptOccurrencePage> {
    crate::metadata::check_cancel(cancelled)?;
    let query = crate::answer::query(source, params.branch_id.parse().map_err(|_| invalid("Invalid branch"))?)?;
    let to = hour_of(Some(params.to), "to")?;
    let (page, rows, missing) = page(source, &query, params)?;
    let openers: Vec<(u64, u64, i32)> = rows
        .iter()
        .filter(|paged| EventFeedKind::of_code(paged.event.code).is_some_and(|kind| list.openers().contains(&kind)))
        .map(|paged| (paged.event.generation, paged.event.sequence, paged.event.read_hour))
        .collect();
    let mut joined = continuing(&query, list, &openers, to, cancelled)?;
    let names = Names::new(&query.holders);
    let mut notices = Notices::default();
    let presenter = Presenter::new(&query, content(source, context, &mut notices), kept_naming(source));
    let tagged = params.facets.iter().any(|filter| !filter.tags.is_empty());
    // Each row's kind as the list shows it, and an opener's continuing rows.
    let cards: Vec<(Option<EventFeedKind>, Vec<FeedEvent>)> = rows
        .iter()
        .map(|paged| {
            let shown = EventFeedKind::of_code(paged.event.code).filter(|kind| list.openers().contains(kind) || list.continuing().contains(kind));
            let continued = match shown {
                Some(kind) if list.openers().contains(&kind) => joined.remove(&(paged.event.generation, paged.event.sequence)).unwrap_or_default(),
                _ => Vec::new(),
            };
            (shown, continued)
        })
        .collect();
    let events: Vec<&FeedEvent> = rows.iter().zip(&cards).flat_map(|(paged, (_, continued))| std::iter::once(&paged.event).chain(continued)).collect();
    let mut presented = present_all(&events, &names, &presenter, &mut notices, cancelled)?.into_iter();
    let mut out = Vec::with_capacity(rows.len());
    for (paged, (shown, continued)) in rows.into_iter().zip(cards) {
        let own = presented.next().ok_or_else(|| invalid("A card row was not presented"))?;
        let rows_continuing: Vec<Option<EventFeedRow>> = presented.by_ref().take(continued.len()).collect();
        let occurrence = match (shown, own) {
            (Some(kind), Some(row)) => {
                let mut edges = vec![edge(&paged.event, row)];
                edges.extend(continued.iter().zip(rows_continuing).filter_map(|(event, row)| row.map(|row| edge(event, row))));
                Some(ScriptOccurrence { id: paged.event.id(), opened: list.openers().contains(&kind), edges })
            }
            _ => None,
        };
        out.push(ScriptPageRow {
            hour: paged.hour,
            key: paged.key.to_string(),
            ordinal: paged.ordinal.to_string(),
            tag: paged.tag,
            own_ordinal: tagged.then(|| paged.own.to_string()),
            cells: Vec::new(),
            occurrence,
        });
    }
    content_refusals(&presenter, &mut notices)?;
    let reply = ScriptOccurrencePage {
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

pub struct EventChoices;

impl Product for EventChoices {
    const KEY: &'static str = "event-choices";
    const CONTENT_DEPENDENT: bool = true;
    const REPLAYS: bool = true;
    type Params = FeedPageParams;
    type Reply = ScriptOccurrencePage;

    fn types() -> Vec<String> {
        vec![ScriptCurrency::decl(), ScriptCost::decl(), ScriptEdge::decl(), ScriptOccurrence::decl(), ScriptPageRow::decl(), ScriptOccurrencePage::decl()]
    }

    fn validate(params: &Self::Params) -> Result<(), &'static str> {
        branch(&params.branch_id)
    }

    fn load(source: &Source, params: &Self::Params, context: &LoadContext, cancelled: &AtomicBool) -> io::Result<Self::Reply> {
        cards(List::EventChoices, source, params, context, cancelled)
    }
}

pub struct Decisions;

impl Product for Decisions {
    const KEY: &'static str = "decisions";
    const CONTENT_DEPENDENT: bool = true;
    const REPLAYS: bool = true;
    type Params = FeedPageParams;
    type Reply = ScriptOccurrencePage;

    fn types() -> Vec<String> {
        EventChoices::types()
    }

    fn validate(params: &Self::Params) -> Result<(), &'static str> {
        branch(&params.branch_id)
    }

    fn load(source: &Source, params: &Self::Params, context: &LoadContext, cancelled: &AtomicBool) -> io::Result<Self::Reply> {
        cards(List::Decisions, source, params, context, cancelled)
    }
}
