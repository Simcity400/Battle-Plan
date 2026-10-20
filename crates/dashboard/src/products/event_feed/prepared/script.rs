//! Event choices and decisions: backward pages of opened occurrences, each
//! with the facts continuing it, read from the feed's own facts alone.
//!
//! An event card opens at a dispatch and continues with the option executed
//! for the same world, native event, instance and receiving country. A
//! decision card opens at a selection (or an activation with no open
//! selection) and continues with the activation, duration writes, effect
//! executions and terminal stores of the same world, country, decision and
//! target. The observer joins them as it publishes the facts
//! ([`super::Links`]); nothing here parses notes or guesses a join. A
//! continuing fact the observer could not join is its own card.
//!
//! Pages step back through the selected facts by their own dates, as the
//! Event feed orders them: a fact read after its date is placed at its date
//! through the facts' late index, never by when it was read. Everything is
//! bound to the Live read hour: a fact or edge is shown only once its own
//! read is reached. A Date range keeps occurrences opened inside it.

use super::query::FactsCatalogue;
use super::*;
use crate::prepared::series::{self, DatedRow, Reader};

/// Which adapter a page belongs to.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) enum Adapter {
    EventChoices,
    Decisions,
}

impl Adapter {
    /// The kinds that open a card.
    fn openers(self) -> &'static [EventFeedKind] {
        use EventFeedKind::*;
        match self {
            Self::EventChoices => &[ScriptEventFired, NewsEventFired],
            Self::Decisions => &[DecisionSelected, DecisionActivated],
        }
    }
    /// The kinds that continue a card; unjoined, each is its own card.
    fn continuing(self) -> &'static [EventFeedKind] {
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

#[derive(Clone, Debug, Deserialize, Serialize, TS)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ScriptPageParams {
    pub branch_id: String,
    pub first_hour: Option<i32>,
    pub last_hour: i32,
    pub cursor: Option<String>,
    pub page_size: u32,
    /// The opening kinds shown; every kind when empty.
    pub kinds: BTreeSet<EventFeedKind>,
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

/// One card: its opener first when recorded, then the facts continuing it
/// in stream order, each through the Live read.
#[derive(Clone, Debug, Serialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct ScriptOccurrence {
    pub id: String,
    /// Whether the first edge opened the card; false for a continuing fact
    /// whose opening was not recorded or joined.
    pub opened: bool,
    pub edges: Vec<ScriptEdge>,
}

#[derive(Clone, Debug, Default, Serialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct ScriptOccurrenceReply {
    pub branch_id: String,
    pub first_hour: i32,
    pub last_hour: i32,
    pub occurrences: Vec<ScriptOccurrence>,
    pub items: Vec<EventFeedItem>,
    pub unavailable: Vec<EventFeedUnavailable>,
    pub warnings: Vec<String>,
}

#[derive(Clone, Debug, Default, Serialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct ScriptOccurrencePage {
    pub reply: ScriptOccurrenceReply,
    pub next_cursor: Option<String>,
}

impl crate::products::ReplyWarnings for ScriptOccurrencePage {
    fn warnings_mut(&mut self) -> &mut Vec<String> {
        &mut self.reply.warnings
    }
}

#[derive(Serialize, Deserialize)]
struct Cursor {
    adapter: Adapter,
    live: i32,
    first: i32,
    kinds: BTreeSet<EventFeedKind>,
    /// The fact the next page ends before.
    before: Placed,
}

/// A fact's place in a page: its date, then its read, then its stream order.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
struct Placed {
    native: i32,
    read: i32,
    key: u64,
    ordinal: u64,
}

/// The live hour a continuation's first page was answered through.
pub(crate) fn cursor_live(cursor: &str) -> Option<i32> {
    serde_json::from_str::<Cursor>(cursor)
        .ok()
        .map(|cursor| cursor.live)
}

#[derive(Default)]
struct Cache(BTreeMap<u64, (store::SourceKey, Vec<store::RangeId>, Arc<FactsCatalogue>)>);

/// The facts' catalogue, decoded once per published text.
fn catalogue_of(
    source: &Source,
    store: &store::Store,
    key: &store::SourceKey,
    branch: u64,
) -> io::Result<Option<Arc<FactsCatalogue>>> {
    let Some(stamps) = series::catalogue_ranges(store, key, "event-feed-facts", branch)? else {
        return Ok(None);
    };
    let state = source.state::<Cache>();
    let mut cache = state.lock().unwrap_or_else(|error| error.into_inner());
    if let Some((held_key, held_stamps, held)) = cache.0.get(&branch)
        && held_key == key
        && *held_stamps == stamps
    {
        return Ok(Some(Arc::clone(held)));
    }
    let Some((_, value)) =
        series::read_catalogue::<FactsCatalogue>(store, key, "event-feed-facts", branch)?
    else {
        return Ok(None);
    };
    let value = Arc::new(value);
    cache
        .0
        .insert(branch, (key.clone(), stamps, Arc::clone(&value)));
    Ok(Some(value))
}

/// Both stream keys of `kind`: read with its native hour, and read later.
fn stream_keys(kind: EventFeedKind) -> [u64; 2] {
    [kind as u64, kind as u64 | (1 << 63)]
}

type Operands = (Option<i32>, Option<i32>, Option<i32>, Option<Vec<ScriptCost>>);

fn edge(fact: &Fact) -> Operands {
    fact.occurrence
        .as_ref()
        .map_or((None, None, None, None), |occurrence| {
            (
                occurrence.option,
                occurrence.lifecycle,
                occurrence.days,
                occurrence.costs.clone(),
            )
        })
}

fn decode(row: DatedRow) -> io::Result<Fact> {
    Ok(serde_json::from_value(row.row)?)
}

fn fact_at(reader: &mut Reader<'_>, (key, ordinal): FactRef) -> io::Result<Fact> {
    let mut page = reader.events(&[key], i32::MIN, i32::MAX, ordinal, 1)?;
    decode(
        page.rows
            .pop()
            .ok_or_else(|| crate::invalid("Prepared feed fact reference is missing"))?,
    )
}

/// The `limit` newest facts of `kinds` dated within `first..=live` and read
/// by `live`, before `before`. A kind's facts read on their date are in date
/// order in its first stream; those read later are reached through `late`.
fn newest(
    reader: &mut Reader<'_>,
    late: &series::LateIndex,
    kinds: &[EventFeedKind],
    (first, live): (i32, i32),
    before: Option<Placed>,
    limit: u32,
    cancelled: &AtomicBool,
) -> io::Result<BTreeMap<Placed, Fact>> {
    let limit = limit as usize;
    let native_to = before.map_or(live, |before| before.native);
    let mut held = BTreeMap::<Placed, Fact>::new();
    let keep = |held: &mut BTreeMap<Placed, Fact>, row: DatedRow| -> io::Result<()> {
        let (key, ordinal, read) = (row.key, row.ordinal, row.hour);
        let fact = decode(row)?;
        let placed = Placed {
            native: fact.row.hour,
            read,
            key,
            ordinal,
        };
        if placed.native >= first
            && placed.read <= live
            && before.is_none_or(|before| placed < before)
        {
            held.insert(placed, fact);
            if held.len() > limit {
                held.pop_first();
            }
        }
        Ok(())
    };
    for &kind in kinds {
        let [ordinary, later] = stream_keys(kind);
        let floor = match first {
            i32::MIN => 0,
            first => reader.event_count_through(ordinary, first - 1)?,
        };
        let mut end = reader.event_count_through(ordinary, native_to)?;
        while end > floor {
            check_cancel(cancelled)?;
            // A full page older than everything held cannot change it.
            if held.len() == limit
                && let Some(oldest) = held.keys().next()
                && reader.events(&[ordinary], i32::MIN, i32::MAX, end - 1, 1)?.rows[0].hour < oldest.native
            {
                break;
            }
            let begin = end.saturating_sub(limit as u64).max(floor);
            let count = u32::try_from(end - begin).expect("page size bound");
            for row in reader.events(&[ordinary], i32::MIN, i32::MAX, begin, count)?.rows {
                keep(&mut held, row)?;
            }
            end = begin;
        }
        for &(_, _, key, ordinal) in late.between(first, native_to, live) {
            if key != later {
                continue;
            }
            check_cancel(cancelled)?;
            let row = reader
                .events(&[key], i32::MIN, i32::MAX, ordinal, 1)?
                .rows
                .pop()
                .ok_or_else(|| crate::invalid("Prepared feed late fact is missing"))?;
            keep(&mut held, row)?;
        }
    }
    Ok(held)
}

/// A cold page: no occurrences or cursor; its extent is still pending.
pub(crate) fn empty(params: &ScriptPageParams, first: i32, last: i32) -> ScriptOccurrencePage {
    ScriptOccurrencePage {
        reply: ScriptOccurrenceReply {
            branch_id: params.branch_id.clone(),
            first_hour: first,
            last_hour: last,
            ..Default::default()
        },
        next_cursor: None,
    }
}

#[allow(clippy::too_many_arguments)]
pub(crate) fn query(
    adapter: Adapter,
    source: &Source,
    mut params: ScriptPageParams,
    context: &LoadContext,
    store: &store::Store,
    key: &store::SourceKey,
    cancelled: &AtomicBool,
) -> io::Result<Option<ScriptOccurrencePage>> {
    check_cancel(cancelled)?;
    let branch = params
        .branch_id
        .parse::<u64>()
        .map_err(|_| crate::invalid("Invalid prepared occurrence branch"))?;
    let start = source
        .replay_ref()
        .branches
        .iter()
        .find(|start| start.id == branch)
        .ok_or_else(|| crate::invalid("Unknown prepared occurrence branch"))?;
    let (branch_first, branch_last) = branch_bounds(source.replay_ref(), start);
    let first = params.first_hour.unwrap_or(branch_first).max(branch_first);
    let live = params.last_hour.min(branch_last);
    if first > live || params.page_size == 0 || params.page_size == u32::MAX {
        return Err(crate::invalid("Invalid prepared occurrence page"));
    }
    if params
        .kinds
        .iter()
        .any(|kind| !adapter.openers().contains(kind))
    {
        return Err(crate::invalid("Unknown opening kind for this list"));
    }
    let openers: Vec<EventFeedKind> = adapter
        .openers()
        .iter()
        .copied()
        .filter(|kind| params.kinds.is_empty() || params.kinds.contains(kind))
        .collect();
    let cursor: Option<Cursor> = params
        .cursor
        .as_deref()
        .map(serde_json::from_str)
        .transpose()?;
    if cursor.as_ref().is_some_and(|cursor| {
        cursor.adapter != adapter
            || cursor.live != live
            || cursor.first != first
            || cursor.kinds != params.kinds
    }) {
        return Err(crate::invalid(
            "Prepared occurrence cursor belongs to another view",
        ));
    }
    let Some(catalogue_facts) = catalogue_of(source, store, key, branch)? else {
        return Ok(None);
    };
    let (diagnostics, _, late, links) = &*catalogue_facts;
    let mut reader = Reader::new(store, key, format!("event-feed-facts-{branch}"), cancelled)?;
    let kinds: Vec<EventFeedKind> = openers
        .iter()
        .chain(adapter.continuing())
        .copied()
        .collect();
    let before = cursor.as_ref().map(|cursor| cursor.before);
    let held = newest(&mut reader, late, &kinds, (first, live), before, params.page_size, cancelled)?;
    let next_cursor = (held.len() == params.page_size as usize)
        .then(|| {
            serde_json::to_string(&Cursor {
                adapter,
                live,
                first,
                kinds: params.kinds.clone(),
                before: *held.keys().next().expect("a full page"),
            })
        })
        .transpose()?;
    let mut builder = Builder::new(first, live);
    builder.feed.unavailable = diagnostics.feed.unavailable.clone();
    builder.unavailable_keys = diagnostics.unavailable_keys.clone();
    let catalogue = match context
        .content()
        .and_then(|content| catalogue::catalogue(source, content))
    {
        Ok(content) => Some(content),
        Err(reason) => {
            builder.unavailable(
                "contentStack",
                "Recorded content stack",
                reason.to_string(),
                Some(1),
            );
            None
        }
    };
    let present = |builder: &mut Builder, fact: Fact| -> io::Result<ScriptEdge> {
        let (option, lifecycle, days, costs) = edge(&fact);
        let mut row = fact.row;
        if let Some(index) = row.item {
            let mut item = diagnostics
                .feed
                .items
                .get(index as usize)
                .ok_or_else(|| crate::invalid("Event feed script item identity differs"))?
                .clone();
            if let Some(catalogue) = catalogue.as_deref() {
                super::super::script_presentation(
                    builder,
                    catalogue,
                    row.kind,
                    &mut item,
                    fact.script.as_ref(),
                )?;
            }
            row.item = Some(builder.item(item)?);
        }
        Ok(ScriptEdge {
            row,
            option,
            lifecycle,
            days,
            costs,
        })
    };
    let mut occurrences = Vec::new();
    for (placed, fact) in held.into_iter().rev() {
        check_cancel(cancelled)?;
        let at = (placed.key, placed.ordinal);
        let kind = fact.row.kind;
        // A joined edge is shown inside its opener's card.
        if fact
            .occurrence
            .as_ref()
            .is_some_and(|occurrence| occurrence.opener.is_some())
        {
            continue;
        }
        let opens = openers.contains(&kind);
        if !opens && !adapter.continuing().contains(&kind) {
            continue;
        }
        let id = fact.row.id.clone();
        let mut edges = vec![present(&mut builder, fact)?];
        if opens {
            for &next in links.0.get(&at).into_iter().flatten() {
                let fact = fact_at(&mut reader, next)?;
                if fact.row.read_hour <= live {
                    edges.push(present(&mut builder, fact)?);
                }
            }
        }
        occurrences.push(ScriptOccurrence {
            id,
            opened: opens,
            edges,
        });
    }
    if let Some(catalogue) = catalogue {
        for (reason, count) in catalogue.refusal_entries() {
            let count = count
                .map(u32::try_from)
                .transpose()
                .map_err(|_| crate::invalid("Content refusal count exceeds u32"))?;
            builder.unavailable("contentPresentation", "Content presentation", reason, count);
        }
    }
    builder.feed.warnings.sort();
    builder.feed.warnings.dedup();
    let page = ScriptOccurrencePage {
        reply: ScriptOccurrenceReply {
            branch_id: params.branch_id.clone(),
            first_hour: first,
            last_hour: live,
            occurrences,
            items: builder.feed.items,
            unavailable: builder.feed.unavailable,
            warnings: builder.feed.warnings,
        },
        next_cursor,
    };
    if let Err(error) = crate::products::check_reply_budget(&page) {
        if params.page_size == 1 {
            return Err(error);
        }
        // A page is an I/O unit; a smaller one keeps the same cursor bounds.
        params.page_size /= 2;
        return query(adapter, source, params, context, store, key, cancelled);
    }
    Ok(Some(page))
}

/// The occurrence lists' product metadata; their queries are routed by key.
pub struct EventChoices;
impl Product for EventChoices {
    const KEY: &'static str = "event-choices";
    const CONTENT_DEPENDENT: bool = true;
    type Params = ScriptPageParams;
    type Reply = ScriptOccurrencePage;
    fn types() -> Vec<String> {
        vec![
            ScriptPageParams::decl(),
            ScriptCurrency::decl(),
            ScriptCost::decl(),
            ScriptEdge::decl(),
            ScriptOccurrence::decl(),
            ScriptOccurrenceReply::decl(),
            ScriptOccurrencePage::decl(),
        ]
    }
    fn validate(params: &Self::Params) -> Result<(), &'static str> {
        params
            .branch_id
            .parse::<u64>()
            .map(|_| ())
            .map_err(|_| "Invalid branch")
    }
}

pub struct Decisions;
impl Product for Decisions {
    const KEY: &'static str = "decisions";
    const CONTENT_DEPENDENT: bool = true;
    type Params = ScriptPageParams;
    type Reply = ScriptOccurrencePage;
    fn types() -> Vec<String> {
        EventChoices::types()
    }
    fn validate(params: &Self::Params) -> Result<(), &'static str> {
        EventChoices::validate(params)
    }
}

/// Every recorded annexation through `through`, its countries as tagged at
/// its native hour, and whether it ran within a civil war's resolution: the
/// civil-war list's outcome evidence.
pub(crate) fn annexations(
    store: &store::Store,
    key: &store::SourceKey,
    branch: u64,
    through: i32,
    cancelled: &AtomicBool,
) -> io::Result<Vec<crate::products::politics_state::civil_wars::Annexation>> {
    use replay_data::event_feed_rows::EventFeedRoute;
    let civil = EventFeedRoute::CivilWarAnnex as u8;
    let mut reader = Reader::new(store, key, format!("event-feed-facts-{branch}"), cancelled)?;
    let keys = stream_keys(EventFeedKind::Annexed);
    let mut cursor = 0;
    let mut result = Vec::new();
    loop {
        check_cancel(cancelled)?;
        let page = reader.events(&keys, i32::MIN, through, cursor, 256)?;
        for row in page.rows {
            let fact = decode(row)?;
            let [annexer, annexed] = fact.row.participants.clone().unwrap_or([None, None]);
            result.push(crate::products::politics_state::civil_wars::Annexation {
                hour: fact.row.hour,
                read_hour: fact.row.read_hour,
                annexer,
                annexed,
                civil_war: fact.occurrence.as_ref().is_some_and(|occurrence| {
                    occurrence.route == Some(civil) || occurrence.within == Some(civil)
                }),
            });
        }
        match page.cursor {
            Some(next) => cursor = next,
            None => break,
        }
    }
    Ok(result)
}
