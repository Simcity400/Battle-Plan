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
    decode_fact(&row.row)
}

fn fact_at(reader: &mut Reader<'_>, (key, ordinal): FactRef) -> io::Result<Fact> {
    decode_fact(&reader.event_raw(key, ordinal)?.1)
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
        // Once the page is full, older facts cannot change it and need not be read.
        for &(native, read, key, ordinal) in late.between(first, native_to, live).rev() {
            if key != later {
                continue;
            }
            check_cancel(cancelled)?;
            let placed = Placed {
                native,
                read,
                key,
                ordinal,
            };
            if before.is_some_and(|before| placed >= before) {
                continue;
            }
            if held.len() == limit
                && let Some(oldest) = held.keys().next()
            {
                if native < oldest.native {
                    break;
                }
                if placed <= *oldest {
                    continue;
                }
            }
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

/// A page of no occurrences or cursor over `first` through `last`: a cold one, whose extent is
/// still pending, or one outside its branch's hours.
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
    let asked = params.first_hour.unwrap_or(branch_first);
    if asked > params.last_hour || params.page_size == 0 || params.page_size == u32::MAX {
        return Err(crate::invalid("Invalid prepared occurrence page"));
    }
    if params
        .kinds
        .iter()
        .any(|kind| !adapter.openers().contains(kind))
    {
        return Err(crate::invalid("Unknown opening kind for this list"));
    }
    let first = asked.max(branch_first);
    let live = params.last_hour.min(branch_last);
    // Hours wholly outside the branch hold no occurrence.
    if first > live {
        if params.cursor.is_some() {
            return Err(crate::invalid("Invalid prepared occurrence page"));
        }
        return Ok(Some(empty(&params, asked, params.last_hour)));
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
    let kinds: Vec<EventFeedKind> = openers
        .iter()
        .chain(adapter.continuing())
        .copied()
        .collect();
    let before = cursor.as_ref().map(|cursor| cursor.before);
    let facts_section = format!("event-feed-facts-{branch}");
    let edges_section = super::entries_section(branch);
    // The page and every card's joined facts, from one publication of the
    // feed's facts and entries.
    let phase = replay_frame::cpu::scope_phase("sc-q:compose");
    let composed = entries::compose(store, key, &edges_section, cancelled, |links| {
        let Some(catalogue_facts) = catalogue_of(source, store, key, branch)? else {
            return Ok(None);
        };
        let mut reader = Reader::new(store, key, facts_section.clone(), cancelled)?;
        let held = newest(
            &mut reader,
            &catalogue_facts.2,
            &kinds,
            (first, live),
            before,
            params.page_size,
            cancelled,
        )?;
        let mut joined = BTreeMap::new();
        for (placed, fact) in &held {
            let joins = fact
                .occurrence
                .as_ref()
                .is_some_and(|occurrence| occurrence.opener.is_some());
            if !openers.contains(&fact.row.kind) || joins {
                continue;
            }
            let at = (placed.key, placed.ordinal);
            let facts = links
                .prefix(&super::edges_prefix(at))?
                .into_iter()
                .map(|(_, next)| fact_at(&mut reader, serde_json::from_slice(&next)?))
                .collect::<io::Result<Vec<_>>>()?;
            joined.insert(at, facts);
        }
        links.agrees(&facts_section)?;
        Ok(Some((catalogue_facts, held, joined)))
    })?;
    drop(phase);
    let Some((catalogue_facts, held, mut joined)) = composed.flatten() else {
        return Ok(None);
    };
    let (diagnostics, _, _) = &*catalogue_facts;
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
    // Whether a held fact is shown as an occurrence's first edge, and whether it opens one: a
    // joined edge is shown inside its opener's card, with the others its opener read by `live`.
    let shown = |fact: &Fact| {
        let joins = fact
            .occurrence
            .as_ref()
            .is_some_and(|occurrence| occurrence.opener.is_some());
        let opens = openers.contains(&fact.row.kind);
        (!joins && (opens || adapter.continuing().contains(&fact.row.kind))).then_some(opens)
    };
    let present = |builder: &mut Builder, mut fact: Fact| -> io::Result<ScriptEdge> {
        let (option, lifecycle, days, costs) = edge(&fact);
        if fact.row.item.is_some() {
            let mut item = fact
                .item
                .take()
                .ok_or_else(|| crate::invalid("Event feed script item identity differs"))?;
            std::mem::take(&mut fact.shown).show(builder, catalogue.as_deref(), &mut item);
            fact.row.item = Some(builder.item(item)?);
        }
        Ok(ScriptEdge {
            row: fact.row,
            option,
            lifecycle,
            days,
            costs,
        })
    };
    let phase = replay_frame::cpu::scope_phase("sc-q:present");
    let mut occurrences = Vec::new();
    for (placed, fact) in held.into_iter().rev() {
        check_cancel(cancelled)?;
        let at = (placed.key, placed.ordinal);
        let Some(opens) = shown(&fact) else {
            continue;
        };
        let id = fact.row.id.clone();
        let mut edges = vec![present(&mut builder, fact)?];
        if opens {
            for fact in joined.remove(&at).unwrap_or_default() {
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
    drop(phase);
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
    const SERVED: Option<&'static dashboard_base::prepared::served::Plan> =
        Some(&crate::products::event_feed::served::SCRIPT);
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
    const SERVED: Option<&'static dashboard_base::prepared::served::Plan> =
        Some(&crate::products::event_feed::served::SCRIPT);
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
