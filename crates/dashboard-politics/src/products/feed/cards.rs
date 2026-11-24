
use super::{
    EventFeedKind, EventFeedRow, EventFeedUnavailable, FeedEvent, FeedTailParams, LOG, Names, Notices, Presenter, TAIL_FIELDS, branch, content_from,
    content_refusals, invalid, kept_naming, logs, present_all, resolve_content, scope, tail_rows,
    text::KeptNaming,
};
use dashboard_base::{Source, catalogue::Catalogue};
use dashboard_follow::panel::Panel;
use dashboard_query::{
    Query,
    log::{self, FacetFilter, Reader},
    series::{MissingSpan, PAGE_LIMIT},
};
use serde::{Deserialize, Serialize};
use std::{
    collections::{BTreeMap, BTreeSet},
    io,
    sync::{Arc, Mutex, atomic::AtomicBool},
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
pub struct ScriptOccurrenceTailReply {
    pub branch_id: String,
    pub log: String,
    pub columns: Vec<String>,
    pub base_ordinal: String,
    pub first_ordinal: String,
    pub offset: String,
    pub total: String,
    pub rows: Vec<ScriptPageRow>,
    pub facet_totals: Vec<logs::LogFacetTotal>,
    pub missing: Vec<MissingSpan>,
    pub unavailable: Vec<EventFeedUnavailable>,
    pub warnings: Vec<String>,
}

dashboard_base::reply_warnings!(ScriptOccurrenceTailReply);

impl logs::TailPanelReply for ScriptOccurrenceTailReply {
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

pub(crate) fn continuing(
    query: &Query<'_>,
    list: List,
    openers: &[(u64, u64, i32)],
    to: i32,
    caches: Option<&logs::LogCaches>,
    cancelled: &AtomicBool,
) -> io::Result<BTreeMap<(u64, u64), Vec<FeedEvent>>> {
    let mut out: BTreeMap<(u64, u64), Vec<FeedEvent>> = BTreeMap::new();
    let hours: BTreeSet<i32> = openers.iter().map(|(.., hour)| *hour).filter(|hour| *hour <= to).collect();
    let Some(&from) = hours.first() else { return Ok(out) };
    let wanted: BTreeSet<(u64, u64)> = openers.iter().map(|(generation, sequence, _)| (*generation, *sequence)).collect();
    let declared = dashboard_declare::log(LOG).ok_or_else(|| invalid("The feed's log is not declared"))?;
    let reader = match caches {
        Some(caches) => Reader::with_cache(query, declared, Arc::clone(&caches.buckets))?,
        None => Reader::new(query, declared)?,
    };
    let filters = [
        FacetFilter { name: "kind".into(), values: list.continuing().iter().filter_map(|kind| kind.code()).map(i64::from).collect(), ..Default::default() },
        FacetFilter { name: "opener".into(), values: hours.iter().map(|hour| i64::from(*hour)).collect(), ..Default::default() },
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

fn load_cards_tail_at(
    list: List,
    scope_index: &scope::ScopeIndex,
    query: &Query<'_>,
    content_result: &Result<Arc<Catalogue>, String>,
    kept: &Arc<Mutex<KeptNaming>>,
    caches: Option<&logs::LogCaches>,
    params: &FeedTailParams,
    hour: i32,
    cancelled: &AtomicBool,
) -> io::Result<ScriptOccurrenceTailReply> {
    dashboard_text::metadata::check_cancel(cancelled)?;
    let (page, rows, facet_totals, missing) = tail_rows(scope_index, query, params, hour, caches)?;
    let openers: Vec<(u64, u64, i32)> = rows
        .iter()
        .filter(|paged| EventFeedKind::of_code(paged.event.code).is_some_and(|kind| list.openers().contains(&kind)))
        .map(|paged| (paged.event.generation, paged.event.sequence, paged.event.read_hour))
        .collect();
    let mut joined = continuing(query, list, &openers, hour, caches, cancelled)?;
    let names = Names::new(&query.holders);
    let mut notices = Notices::default();
    let presenter = Presenter::new(query, content_from(content_result, &mut notices), Arc::clone(kept));
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
    let mut presented = present_all(query, &events, &names, &presenter, &mut notices, cancelled)?.into_iter();
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
    let first_ordinal = out.first().map_or_else(|| page.base.to_string(), |row| row.ordinal.clone());
    let reply = ScriptOccurrenceTailReply {
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

fn cards_tail_load(list: List, source: &Source, params: &FeedTailParams, context: &LoadContext, cancelled: &AtomicBool) -> io::Result<ScriptOccurrenceTailReply> {
    let query = dashboard_base::queries::query(source, params.branch_id.parse().map_err(|_| invalid("Invalid branch"))?)?;
    let hour = params.hour.unwrap_or(query.timeline.last);
    let content_result = resolve_content(source, context);
    let kept = kept_naming(source);
    let scope_index = scope::index(source);
    let caches = dashboard_base::queries::log_caches(source);
    load_cards_tail_at(list, &scope_index, &query, &content_result, &kept, Some(&caches), params, hour, cancelled)
}

fn cards_dated_panel(list: List, name: &'static str, source: &Source, load: &LoadContext, asked: &serde_json::Value) -> Result<Box<dyn Panel>, String> {
    let mut request = asked.clone();
    request.as_object_mut().ok_or_else(|| format!("Invalid {name} panel"))?.remove("panel");
    let params: FeedTailParams = dashboard_query::named::named(&request).map_err(|error| format!("Invalid {name} panel: {error}"))?;
    branch(&params.branch_id)?;
    let branch_id = params.branch_id.parse().map_err(|_| "Invalid branch")?;
    let query = dashboard_base::queries::query(source, branch_id).map_err(|error| error.to_string())?;
    let declared = dashboard_declare::log(LOG).ok_or_else(|| format!("No declared log {LOG}"))?;
    let log_caches = dashboard_base::queries::log_caches(source);
    let pre_reader = Reader::with_cache(&query, declared, Arc::clone(&log_caches.buckets)).map_err(|error| error.to_string())?;
    let selection = Arc::new(pre_reader.select(&params.facets, query.timeline.first, query.timeline.last).map_err(|error| error.to_string())?);
    let mut watch_selections: Vec<log::Selection> = params
        .count_facets
        .iter()
        .map(|ask| pre_reader.select(&ask.facets, query.timeline.first, query.timeline.last))
        .collect::<io::Result<Vec<_>>>()
        .map_err(|error| error.to_string())?;
    let continuing_filter = [FacetFilter {
        name: "kind".into(),
        values: list.continuing().iter().filter_map(|kind| kind.code()).map(i64::from).collect(),
        ..Default::default()
    }];
    watch_selections.push(pre_reader.select(&continuing_filter, query.timeline.first, query.timeline.last).map_err(|error| error.to_string())?);
    let watch_selections = Arc::new(watch_selections);
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
            logs::quiet_tail_missing(&query, LOG, &q_caches, &selection, &watch_selections, &q_params.facets, q_params.from, prev_hour, hour, prev_reply)
        },
        move |hour, cancelled| {
            let query = Query { replay: &replay, timeline: timeline.clone(), holders: holders.clone() };
            load_cards_tail_at(list, &scope_index, &query, &content_result, &kept, Some(&log_caches), &params, hour, cancelled)
        },
    ))
}

pub struct EventChoicesTail;

impl Product for EventChoicesTail {
    const KEY: &'static str = "event-choices-tail";
    const CONTENT_DEPENDENT: bool = true;
    const REPLAYS: bool = true;
    type Params = FeedTailParams;
    const FIELDS: dashboard_base::products::Fields = TAIL_FIELDS;
    type Reply = ScriptOccurrenceTailReply;

    #[cfg(any(test, feature = "export-types"))]
    fn types() -> Vec<String> {
        vec![
            FeedTailParams::decl(),
            ScriptCurrency::decl(),
            ScriptCost::decl(),
            ScriptEdge::decl(),
            ScriptOccurrence::decl(),
            ScriptPageRow::decl(),
            ScriptOccurrenceTailReply::decl(),
        ]
    }

    fn validate(params: &Self::Params) -> Result<(), &'static str> {
        branch(&params.branch_id)
    }

    fn load(source: &Source, params: &Self::Params, context: &LoadContext, cancelled: &AtomicBool) -> io::Result<Self::Reply> {
        cards_tail_load(List::EventChoices, source, params, context, cancelled)
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
    type Params = FeedTailParams;
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
        cards_tail_load(List::Decisions, source, params, context, cancelled)
    }
}

pub fn decisions_dated_panel(source: &Source, load: &LoadContext, asked: &serde_json::Value) -> Result<Box<dyn Panel>, String> {
    cards_dated_panel(List::Decisions, DecisionsTail::KEY, source, load, asked)
}
