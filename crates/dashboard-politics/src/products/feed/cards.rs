
use super::{
    EventFeedKind, EventFeedRow, EventFeedUnavailable, FeedEvent, FeedPageParams, LOG, Names, Notices, Presenter, branch, content, content_refusals, invalid, logs, page,
    present_all, kept_naming,
};
use dashboard_base::Source;
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
#[cfg(any(test, feature = "export-types"))]
use ts_rs::TS;

use dashboard_base::products::{LoadContext, Product};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum List {
    EventChoices,
    Decisions,
}

impl List {
    pub fn openers(self) -> &'static [EventFeedKind] {
        use EventFeedKind::*;
        match self {
            Self::EventChoices => &[ScriptEventFired, NewsEventFired],
            Self::Decisions => &[DecisionSelected, DecisionActivated],
        }
    }

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

#[derive(Clone, Copy, Debug, Deserialize, Serialize, PartialEq, Eq, PartialOrd, Ord)]
#[cfg_attr(any(test, feature = "export-types"), derive(TS))]
#[serde(rename_all = "camelCase")]
pub enum ScriptCurrency {
    PoliticalPower,
    CommandPower,
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[cfg_attr(any(test, feature = "export-types"), derive(TS))]
#[serde(rename_all = "camelCase")]
pub struct ScriptCost {
    pub currency: ScriptCurrency,
    #[doc = " The exact fixed5 balance change as a decimal string; negative when spent."]
    pub amount: String,
}

#[derive(Clone, Debug, PartialEq, Serialize)]
#[cfg_attr(any(test, feature = "export-types"), derive(TS))]
#[serde(rename_all = "camelCase")]
pub struct ScriptEdge {
    pub row: EventFeedRow,
    #[doc = " The executed option's signed native ordinal."]
    pub option: Option<i32>,
    #[doc = " The writer's lifecycle result, where its boundary supplies one."]
    pub lifecycle: Option<i32>,
    #[doc = " The writer's duration result in days."]
    pub days: Option<i32>,
    #[doc = " The power the execution was proven to charge; null when not proven."]
    pub costs: Option<Vec<ScriptCost>>,
}

#[derive(Clone, Debug, PartialEq, Serialize)]
#[cfg_attr(any(test, feature = "export-types"), derive(TS))]
#[serde(rename_all = "camelCase")]
pub struct ScriptOccurrence {
    pub id: String,
    #[doc = " Whether the first edge opened the card; false for a continuing row whose opening was not"]
    #[doc = " recorded or joined."]
    pub opened: bool,
    pub edges: Vec<ScriptEdge>,
}

#[derive(Clone, Debug, PartialEq, Serialize)]
#[cfg_attr(any(test, feature = "export-types"), derive(TS))]
#[serde(rename_all = "camelCase")]
pub struct ScriptPageRow {
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
    pub occurrence: Option<ScriptOccurrence>,
}

#[derive(Clone, Debug, PartialEq, Serialize)]
#[cfg_attr(any(test, feature = "export-types"), derive(TS))]
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

pub(crate) fn continuing(query: &Query<'_>, list: List, openers: &[(u64, u64, i32)], to: i32, cancelled: &AtomicBool) -> io::Result<BTreeMap<(u64, u64), Vec<FeedEvent>>> {
    let mut out: BTreeMap<(u64, u64), Vec<FeedEvent>> = BTreeMap::new();
    let hours: BTreeSet<i32> = openers.iter().map(|(.., hour)| *hour).filter(|hour| *hour <= to).collect();
    let Some(&from) = hours.first() else { return Ok(out) };
    let wanted: BTreeSet<(u64, u64)> = openers.iter().map(|(generation, sequence, _)| (*generation, *sequence)).collect();
    let declared = dashboard_declare::log(LOG).ok_or_else(|| invalid("The feed's log is not declared"))?;
    let reader = Reader::new(query, declared)?;
    let filters = [
        FacetFilter { name: "kind".into(), values: list.continuing().iter().filter_map(|kind| kind.code()).map(i64::from).collect(), countries: Vec::new() },
        FacetFilter { name: "opener".into(), values: hours.iter().map(|hour| i64::from(*hour)).collect(), countries: Vec::new() },
    ];
    let selection = reader.select(&filters, query.timeline.first, to)?;
    let columns = super::row::Columns::new()?;
    let mut cursor = 0;
    loop {
        dashboard_text::metadata::check_cancel(cancelled)?;
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

fn cards(list: List, source: &Source, params: &FeedPageParams, context: &LoadContext, cancelled: &AtomicBool) -> io::Result<ScriptOccurrencePage> {
    dashboard_text::metadata::check_cancel(cancelled)?;
    let query = dashboard_base::queries::query(source, params.branch_id.parse().map_err(|_| invalid("Invalid branch"))?)?;
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
    let tagged = params.facets.iter().any(|filter| !filter.countries.is_empty());
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
    let mut presented = present_all(&query, &events, &names, &presenter, &mut notices, cancelled)?.into_iter();
    let mut out = Vec::with_capacity(rows.len());
    for (paged, (shown, continued)) in rows.into_iter().zip(cards) {
        let own = presented.next().ok_or_else(|| invalid("A card row was not presented"))?;
        let mut card_rows: Vec<Option<EventFeedRow>> = std::iter::once(own).chain(presented.by_ref().take(continued.len())).collect();
        if list == List::Decisions {
            let card_events: Vec<&FeedEvent> = std::iter::once(&paged.event).chain(continued.iter()).collect();
            super::decision_state::enforce_same_incarnation(&card_events, &mut card_rows);
        }
        let mut card_rows = card_rows.into_iter();
        let own = card_rows.next().flatten();
        let rows_continuing: Vec<Option<EventFeedRow>> = card_rows.collect();
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
            country: paged.tag,
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
    const FIELDS: dashboard_base::products::Fields = dashboard_base::fields![branchId, facets, from: Start, to: End, cursor, pageSize, "scope[].firstHour": Start, "scope[].lastHour": End, "facets[].countries[]", "facets[].values[]"];
    type Reply = ScriptOccurrencePage;

    #[cfg(any(test, feature = "export-types"))]
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
    const FIELDS: dashboard_base::products::Fields = dashboard_base::fields![branchId, facets, from: Start, to: End, cursor, pageSize, "scope[].firstHour": Start, "scope[].lastHour": End, "facets[].countries[]", "facets[].values[]"];
    type Reply = ScriptOccurrencePage;

    #[cfg(any(test, feature = "export-types"))]
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
