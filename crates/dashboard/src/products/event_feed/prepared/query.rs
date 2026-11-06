//! Backward native-date pages composed from the owning families' event ranges.
use super::*;
use crate::prepared::series::{self, DatedRow, LateIndex, Reader};
use crate::prepared_focus::query::{CompletionArts, Feed as FocusFeed};
use crate::products::country_nukes::{
    NukeTypeInfo,
    query::{Feed, Increase},
};
use crate::products::idea_history::{Addition, Feed as Ideas};
use crate::products::special_projects::{
    emit::Change,
    query::{FEED_KEY as PROJECTS_KEY, Feed as Projects},
};
use crate::products::subjects::Feed as SubjectFeed;
#[cfg(test)]
thread_local! {
    pub(super) static EVENT_ROWS_READ: std::cell::Cell<usize> = const { std::cell::Cell::new(0) };
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(crate) struct Params {
    pub branch_id: String,
    pub first_hour: Option<i32>,
    pub last_hour: i32,
    pub cursor: Option<String>,
    pub page_size: u32,
    pub categories: BTreeSet<EventFeedCategory>,
    /// Only the rows read after this hour, which a feed answered through it prepends: every one
    /// dated after it, at most `page_size`; else the page is answered whole.
    pub after: Option<i32>,
}
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct Page {
    pub reply: EventFeedReply,
    pub next_cursor: Option<String>,
    /// The hour the rows were read after, where the page holds only those ([`Params::after`]).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub after: Option<i32>,
}

// The ordering operands include the original native ID and a source ordinal for
// duplicate ledger rows. Non-native IDs address their source event, not a page.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
struct Position {
    native: i32,
    read: i32,
    kind: EventFeedKind,
    hidden: bool,
    tags: Vec<String>,
    id: String,
    source: usize,
    key: u64,
    ordinal: u64,
}
impl Ord for Position {
    fn cmp(&self, other: &Self) -> std::cmp::Ordering {
        self.native.cmp(&other.native).then_with(|| {
            (other.read, other.kind, other.hidden, &other.tags)
                .cmp(&(self.read, self.kind, self.hidden, &self.tags))
                .then_with(|| row_tie_order(self.kind, &other.id, &self.id))
                .then_with(|| {
                    (other.source, other.key, other.ordinal).cmp(&(
                        self.source,
                        self.key,
                        self.ordinal,
                    ))
                })
        })
    }
}
impl PartialOrd for Position {
    fn partial_cmp(&self, other: &Self) -> Option<std::cmp::Ordering> {
        Some(self.cmp(other))
    }
}
#[derive(Serialize, Deserialize)]
struct Cursor {
    live: i32,
    first: i32,
    categories: BTreeSet<EventFeedCategory>,
    before: Position,
}

/// Where each source's events are read, by source: the pages the hourly walk publishes, or,
/// where none, the events a family's publication gives the feed whole ([`Held`]).
const PAGES: [Option<&str>; 7] = [
    Some("country-group-changes"),
    None,
    None,
    None,
    Some("event-feed-facts"),
    None,
    None,
];
/// The sources given whole: the subjects family's relation starts, the ideas family's
/// additions, the focus family's completions, the nukes family's stockpile increases and the
/// projects family's completion changes.
const STARTS: usize = 1;
const IDEAS: usize = 2;
const COMPLETIONS: usize = 3;
const INCREASES: usize = 5;
const PROJECTS: usize = 6;
/// The key every event of an unkeyed source is read under.
const GIVEN_KEY: u64 = 0;
/// The keys the idea additions are read under: the laws changed, then the advisors appointed.
const LAW_KEY: u64 = 0;
const ADVISOR_KEY: u64 = 1;

/// What the feed reads of the families the segment driver prepares, each from its own
/// publication.
#[derive(Clone, Copy)]
pub(crate) struct Families<'a> {
    pub nukes: &'a Feed,
    pub subjects: &'a SubjectFeed,
    pub focus: &'a FocusFeed,
    /// Where the art of the focus completions a page shows is read.
    pub focus_art: &'a dyn CompletionArts,
    pub ideas: &'a Arc<Ideas>,
    pub projects: &'a Projects,
}

/// An event a family gives the feed, dated by the hour that read it.
trait Dated {
    fn read_hour(&self) -> i32;
    /// The event as the feed reads it.
    fn source_row(&self) -> SourceRow;
}

impl Dated for Increase {
    fn read_hour(&self) -> i32 {
        self.hour
    }
    fn source_row(&self) -> SourceRow {
        SourceRow::Increase(self.clone())
    }
}

impl Dated for subjects::SubjectStart {
    fn read_hour(&self) -> i32 {
        self.read_hour
    }
    fn source_row(&self) -> SourceRow {
        SourceRow::Subject(self.clone())
    }
}

impl Dated for Addition {
    fn read_hour(&self) -> i32 {
        self.span.first_hour
    }
    fn source_row(&self) -> SourceRow {
        SourceRow::Idea(self.clone())
    }
}

impl Dated for national_focus::FocusCompletion {
    fn read_hour(&self) -> i32 {
        self.hour
    }
    fn source_row(&self) -> SourceRow {
        SourceRow::Focus(self.clone())
    }
}

impl Dated for Change {
    fn read_hour(&self) -> i32 {
        self.observed_hour
    }
    fn source_row(&self) -> SourceRow {
        SourceRow::Project(self.clone())
    }
}

/// A family's events under the key and ordinal the feed pages them by, each key's in the order
/// of the hours that read them.
trait Keyed {
    type Event: Dated;
    fn given(&self, key: u64) -> &[Self::Event];
    /// The references of the events read after their dates.
    fn late(&self) -> Option<&LateIndex> {
        None
    }
}

/// An unkeyed source's events: every one under [`GIVEN_KEY`].
fn unkeyed<T>(key: u64, events: &[T]) -> &[T] {
    if key == GIVEN_KEY {
        events
    } else {
        &events[..0]
    }
}

impl Keyed for Feed {
    type Event = Increase;
    fn given(&self, key: u64) -> &[Increase] {
        unkeyed(key, &self.increases)
    }
}

impl Keyed for SubjectFeed {
    type Event = subjects::SubjectStart;
    fn given(&self, key: u64) -> &[subjects::SubjectStart] {
        self.keyed(key)
    }
    fn late(&self) -> Option<&LateIndex> {
        Some(&self.late)
    }
}

impl Keyed for FocusFeed {
    type Event = national_focus::FocusCompletion;
    fn given(&self, key: u64) -> &[national_focus::FocusCompletion] {
        unkeyed(key, &self.completions)
    }
}

impl Keyed for Projects {
    type Event = Change;
    fn given(&self, key: u64) -> &[Change] {
        let changes = self.changes.as_slice();
        if key == PROJECTS_KEY {
            changes
        } else {
            &changes[..0]
        }
    }
    fn late(&self) -> Option<&LateIndex> {
        Some(&self.late)
    }
}


/// A source's events the feed is given whole, read as its pages would be.
trait Held {
    /// How many of `key`'s events were read by `hour`.
    fn through(&self, key: u64, hour: i32) -> usize;
    /// `key`'s event at `ordinal`: the hour that read it, and the event.
    fn at(&self, key: u64, ordinal: usize) -> Option<(i32, SourceRow)>;
    /// The hour that read `key`'s event at `ordinal`.
    fn hour(&self, key: u64, ordinal: usize) -> Option<i32>;
    fn late(&self) -> Option<&LateIndex>;
}

impl<K: Keyed> Held for K {
    fn through(&self, key: u64, hour: i32) -> usize {
        self.given(key)
            .partition_point(|event| event.read_hour() <= hour)
    }
    fn at(&self, key: u64, ordinal: usize) -> Option<(i32, SourceRow)> {
        self.given(key)
            .get(ordinal)
            .map(|event| (event.read_hour(), event.source_row()))
    }
    fn hour(&self, key: u64, ordinal: usize) -> Option<i32> {
        self.given(key).get(ordinal).map(Dated::read_hour)
    }
    fn late(&self) -> Option<&LateIndex> {
        Keyed::late(self)
    }
}

/// The idea additions the feed lists, by key ([`LAW_KEY`], [`ADVISOR_KEY`]), as their places in
/// the ideas' additions, and each idea the recorded content cannot classify, with its additions.
/// An addition of an idea content proves no law is no change the feed lists.
struct Classified {
    listed: [Vec<usize>; 2],
    unclassified: BTreeMap<String, u32>,
}

fn classified(
    ideas: &Ideas,
    politics: Option<&politics_content::PoliticsContent>,
) -> io::Result<Classified> {
    let mut listed: [Vec<usize>; 2] = [Vec::new(), Vec::new()];
    let mut unclassified = BTreeMap::<String, u32>::new();
    for (at, addition) in ideas.additions.iter().enumerate() {
        let key = if addition.span.character.is_some() {
            ADVISOR_KEY
        } else {
            match politics.and_then(|content| {
                content
                    .idea_is_law(&addition.span.token)
                    .ok()
                    .and_then(|law| *law)
            }) {
                Some(true) => LAW_KEY,
                Some(false) => continue,
                None => {
                    let count = unclassified
                        .entry(addition.span.token.clone())
                        .or_default();
                    *count = count
                        .checked_add(1)
                        .ok_or_else(|| crate::invalid("Idea classification count exceeds u32"))?;
                    continue;
                }
            }
        };
        listed[key as usize].push(at);
    }
    Ok(Classified {
        listed,
        unclassified,
    })
}

/// Each branch's [`classified`] additions of one publication of the ideas family under one
/// politics content, both held so they are told apart from their successors.
#[derive(Default)]
struct KeptClassified(
    BTreeMap<
        u64,
        (
            Arc<Ideas>,
            Option<Arc<politics_content::PoliticsContent>>,
            Arc<Classified>,
        ),
    >,
);

/// [`classified`], once per publication of the ideas and politics content of `branch`.
fn kept_classified(
    source: &Source,
    branch: u64,
    ideas: &Arc<Ideas>,
    politics: Option<&Arc<politics_content::PoliticsContent>>,
) -> io::Result<Arc<Classified>> {
    let state = source.state::<KeptClassified>();
    let mut kept = state.lock().unwrap_or_else(|error| error.into_inner());
    if let Some((held_ideas, held_politics, held)) = kept.0.get(&branch)
        && Arc::ptr_eq(held_ideas, ideas)
        && match (held_politics, politics) {
            (Some(held), Some(politics)) => Arc::ptr_eq(held, politics),
            (None, None) => true,
            _ => false,
        }
    {
        return Ok(Arc::clone(held));
    }
    let held = Arc::new(classified(ideas, politics.map(|politics| &**politics))?);
    kept.0.insert(
        branch,
        (Arc::clone(ideas), politics.cloned(), Arc::clone(&held)),
    );
    Ok(held)
}

/// The additions [`Classified`] lists, read from the ideas it indexes.
struct ClassifiedIdeas<'a> {
    ideas: &'a Ideas,
    classified: &'a Classified,
}

impl ClassifiedIdeas<'_> {
    fn listed(&self, key: u64) -> &[usize] {
        usize::try_from(key)
            .ok()
            .and_then(|key| self.classified.listed.get(key))
            .map_or(&[], Vec::as_slice)
    }
    fn addition(&self, key: u64, ordinal: usize) -> Option<&Addition> {
        self.ideas.additions.get(*self.listed(key).get(ordinal)?)
    }
}

impl Held for ClassifiedIdeas<'_> {
    fn through(&self, key: u64, hour: i32) -> usize {
        self.listed(key)
            .partition_point(|&at| self.ideas.additions[at].read_hour() <= hour)
    }
    fn at(&self, key: u64, ordinal: usize) -> Option<(i32, SourceRow)> {
        self.addition(key, ordinal)
            .map(|addition| (addition.read_hour(), addition.source_row()))
    }
    fn hour(&self, key: u64, ordinal: usize) -> Option<i32> {
        self.addition(key, ordinal).map(Dated::read_hour)
    }
    fn late(&self) -> Option<&LateIndex> {
        None
    }
}

/// A source's event as the feed reads it: its key, the hour that read it, its ordinal and the
/// event.
struct Read {
    key: u64,
    hour: i32,
    ordinal: u64,
    row: SourceRow,
}

/// Where a source's events are read: its pages, decoded as their family stored them, or the
/// events a family gives the feed whole.
enum Events<'a> {
    Pages(Reader<'a>, usize),
    Given(&'a dyn Held),
}

/// A paged source's stored row.
fn paged(source: usize, row: &[u8]) -> io::Result<SourceRow> {
    Ok(match source {
        0 => SourceRow::Group(serde_json::from_slice(row)?),
        4 => SourceRow::Fact(Box::new(decode_fact(row)?)),
        _ => return Err(crate::invalid("Unknown paged feed source")),
    })
}

impl<'a> Events<'a> {
    /// The late references of the events the feed is given.
    fn late(&self) -> Option<&'a LateIndex> {
        match self {
            Self::Pages(..) => None,
            Self::Given(held) => {
                let held: &'a dyn Held = *held;
                held.late()
            }
        }
    }

    fn event_count_through(&mut self, key: u64, hour: i32) -> io::Result<u64> {
        match self {
            Self::Pages(reader, _) => reader.event_count_through(key, hour),
            Self::Given(held) => Ok(held.through(key, hour) as u64),
        }
    }

    /// The hour that read the event of `key` at `ordinal`.
    fn event_hour(&mut self, key: u64, ordinal: u64) -> io::Result<i32> {
        match self {
            Self::Pages(reader, _) => reader.event_date(key, ordinal),
            Self::Given(held) => usize::try_from(ordinal)
                .ok()
                .and_then(|at| held.hour(key, at))
                .ok_or_else(|| crate::invalid("Prepared feed event reference is missing")),
        }
    }

    /// The event of `key` at `ordinal`.
    fn event(&mut self, key: u64, ordinal: u64) -> io::Result<Read> {
        #[cfg(test)]
        EVENT_ROWS_READ.with(|count| count.set(count.get() + 1));
        match self {
            Self::Pages(reader, source) => {
                let (hour, row) = reader.event_raw(key, ordinal)?;
                Ok(Read {
                    key,
                    hour,
                    ordinal,
                    row: paged(*source, &row)?,
                })
            }
            Self::Given(held) => {
                let (hour, row) = usize::try_from(ordinal)
                    .ok()
                    .and_then(|at| held.at(key, at))
                    .ok_or_else(|| crate::invalid("Prepared feed event reference is missing"))?;
                Ok(Read {
                    key,
                    hour,
                    ordinal,
                    row,
                })
            }
        }
    }

    /// The events of `keys` dated `from..=to` from ordinal `cursor`, `size` at most, as a
    /// source's pages give them.
    fn events(
        &mut self,
        keys: &[u64],
        from: i32,
        to: i32,
        cursor: u64,
        size: u32,
    ) -> io::Result<Vec<Read>> {
        let held = match self {
            Self::Pages(reader, source) => {
                let page = reader.events(keys, from, to, cursor, size)?;
                #[cfg(test)]
                EVENT_ROWS_READ.with(|count| count.set(count.get() + page.rows.len()));
                return page
                    .rows
                    .into_iter()
                    .map(|row| {
                        Ok(Read {
                            key: row.key,
                            hour: row.hour,
                            ordinal: row.ordinal,
                            row: paged(*source, &row.row)?,
                        })
                    })
                    .collect();
            }
            Self::Given(held) => *held,
        };
        let &[key] = keys else {
            return Err(crate::invalid("Given feed events are read one key at a time"));
        };
        if from > to || size == 0 {
            return Err(crate::invalid("invalid event page"));
        }
        let base = if from == i32::MIN {
            0
        } else {
            held.through(key, from - 1)
        };
        let last = held.through(key, to);
        let first = usize::try_from(cursor).unwrap_or(usize::MAX).max(base);
        let end = first.saturating_add(size as usize).min(last);
        let mut rows = Vec::with_capacity(end.saturating_sub(first));
        for ordinal in first..end {
            let (hour, row) = held
                .at(key, ordinal)
                .ok_or_else(|| crate::invalid("Prepared feed event reference is missing"))?;
            rows.push(Read {
                key,
                hour,
                ordinal: ordinal as u64,
                row,
            });
        }
        #[cfg(test)]
        EVENT_ROWS_READ.with(|count| count.set(count.get() + rows.len()));
        Ok(rows)
    }
}

/// The feed facts' catalogue: diagnostics, observed tables and late
/// references. Each opener's continuing facts are entries apart.
pub(in crate::products::event_feed) type FactsCatalogue = (Builder, BTreeSet<String>, LateIndex);
struct Metadata {
    groups: country_groups::changes::Metadata,
    facts: FactsCatalogue,
}
impl Metadata {
    fn late(&self, source: usize) -> Option<&LateIndex> {
        match source {
            0 => Some(&self.groups.late),
            4 => Some(&self.facts.2),
            _ => None,
        }
    }
}
type Stamps = Vec<store::RangeId>;
#[derive(Default)]
struct Cache(BTreeMap<u64, (store::SourceKey, Stamps, Arc<Metadata>)>);

// Large change catalogues are decoded once per opened source. Published range
// identities and the store key invalidate this cache on append, replacement or Clear.
fn metadata(
    source: &Source,
    store: &store::Store,
    key: &store::SourceKey,
    branch: u64,
) -> io::Result<Option<Arc<Metadata>>> {
    let mut stamps = Vec::new();
    for family in PAGES.into_iter().flatten() {
        let Some(ranges) = series::catalogue_ranges(store, key, family, branch)? else {
            return Ok(None);
        };
        stamps.extend(ranges);
    }
    let state = source.state::<Cache>();
    let mut cache = state.lock().unwrap_or_else(|error| error.into_inner());
    if let Some((held_key, held_stamps, held)) = cache.0.get(&branch)
        && held_key == key
        && *held_stamps == stamps
    {
        return Ok(Some(Arc::clone(held)));
    }
    fn read<T: serde::de::DeserializeOwned>(
        store: &store::Store,
        key: &store::SourceKey,
        family: &str,
        branch: u64,
    ) -> io::Result<T> {
        series::read_catalogue(store, key, family, branch)?
            .map(|(_, value)| value)
            .ok_or_else(|| crate::invalid("Prepared feed catalogue disappeared"))
    }
    let family = |source: usize| PAGES[source].expect("a source's pages");
    let value = Arc::new(Metadata {
        groups: read(store, key, family(0), branch)?,
        facts: read(store, key, family(4), branch)?,
    });
    cache
        .0
        .insert(branch, (key.clone(), stamps, Arc::clone(&value)));
    Ok(Some(value))
}

// Retain the decoded source operand alongside its display position. Projection
// never decodes a reached event again, including nuclear metadata and revisions.
enum SourceRow {
    Group(country_groups::changes::Change),
    Subject(subjects::SubjectStart),
    Idea(idea_history::Addition),
    Focus(national_focus::FocusCompletion),
    Fact(Box<Fact>),
    Increase(Increase),
    Project(Change),
}
fn decode(source: usize, row: Read) -> io::Result<(Position, SourceRow)> {
    let value = row.row;
    let (native, kind, tags, native_id) = match &value {
        SourceRow::Group(change) => (
            change.hour,
            group_kind(&change.kind)?,
            change.tags.clone(),
            None,
        ),
        SourceRow::Subject(change) => (
            change.hour,
            EventFeedKind::BecameSubject,
            vec![change.subject.clone(), change.overlord.clone()],
            None,
        ),
        SourceRow::Idea(change) => (
            change.span.first_hour,
            if change.span.character.is_some() {
                EventFeedKind::AdvisorHired
            } else {
                EventFeedKind::LawChanged
            },
            vec![change.tag.clone()],
            None,
        ),
        SourceRow::Focus(change) => (
            change.hour,
            EventFeedKind::FocusCompleted,
            vec![change.tag.clone()],
            None,
        ),
        SourceRow::Fact(fact) => (
            fact.row.hour,
            fact.row.kind,
            fact.row.tags.clone(),
            (!fact.row.id.is_empty()).then(|| fact.row.id.clone()),
        ),
        SourceRow::Increase(change) => (
            change.hour,
            EventFeedKind::NukesIncreased,
            vec![change.tag.clone()],
            None,
        ),
        SourceRow::Project(change) => (
            change.date.hour,
            EventFeedKind::ProjectFinished,
            change
                .completion
                .as_ref()
                .map(|row| vec![row.tag.clone()])
                .unwrap_or_default(),
            None,
        ),
    };
    Ok((
        Position {
            native,
            read: row.hour,
            kind,
            hidden: matches!(&value, SourceRow::Project(change) if change.completion.is_none()),
            tags,
            id: native_id
                .unwrap_or_else(|| format!("prepared:{source}:{}:{:020}", row.key, row.ordinal)),
            source,
            key: row.key,
            ordinal: row.ordinal,
        },
        value,
    ))
}


fn keys(source: usize, categories: &BTreeSet<EventFeedCategory>) -> Vec<u64> {
    let pairs: Vec<(u64, EventFeedCategory)> = match source {
        0 => vec![
            (0, EventFeedCategory::Diplomacy),
            (1, EventFeedCategory::Diplomacy),
            (2, EventFeedCategory::WarsPeace),
            (3, EventFeedCategory::WarsPeace),
            (4, EventFeedCategory::WarsPeace),
        ],
        STARTS => vec![(0, EventFeedCategory::Diplomacy)],
        IDEAS => vec![
            (LAW_KEY, EventFeedCategory::Politics),
            (ADVISOR_KEY, EventFeedCategory::Politics),
        ],
        COMPLETIONS | INCREASES => vec![(GIVEN_KEY, EventFeedCategory::Milestones)],
        PROJECTS => vec![(PROJECTS_KEY, EventFeedCategory::Milestones)],
        _ => FACT_KEYS
            .iter()
            .copied()
            .filter(|key| key & (1 << 63) == 0)
            // Each fact stream filters by its own kind's category.
            .map(|key| {
                (
                    key,
                    EventFeedKind::from_key(key)
                        .map_or(EventFeedCategory::WarsPeace, EventFeedKind::category),
                )
            })
            .collect(),
    };
    pairs
        .into_iter()
        .filter(|(_, category)| categories.contains(category))
        .map(|(key, _)| key)
        .collect()
}

type WarEnds = BTreeSet<(i32, String, String)>;

#[allow(clippy::too_many_arguments, clippy::type_complexity)]
fn candidates(
    readers: &mut [Events<'_>],
    native_ends: &mut Reader<'_>,
    metadata: &Metadata,
    bounds: (i32, i32),
    before: Option<&Position>,
    categories: &BTreeSet<EventFeedCategory>,
    limit: usize,
    cancelled: &AtomicBool,
) -> io::Result<(BTreeMap<Position, SourceRow>, BTreeMap<i32, WarEnds>, bool)> {
    let (first, live) = bounds;
    let native_to = before.map_or(live, |position| position.native);
    let mut held: BTreeMap<Position, SourceRow> = BTreeMap::new();
    let mut peace = BTreeMap::new();
    let mut has_older = false;
    let mut order = |source, row: Read| -> io::Result<(Position, SourceRow)> {
        let (mut position, row) = decode(source, row)?;
        if position.kind == EventFeedKind::Peace {
            if let std::collections::btree_map::Entry::Vacant(entry) = peace.entry(position.native)
            {
                entry.insert(war_ends(native_ends, metadata, position.native, live)?);
            }
            // Use the existing pair suppression before ordering and page selection.
            position.tags =
                peace_fallback(&peace[&position.native], position.native, position.tags);
        }
        Ok((position, row))
    };
    let retain = |held: &mut BTreeMap<Position, SourceRow>, position: Position, row: SourceRow| {
        if position.native >= first
            && position.native <= native_to
            && position.read <= live
            && before.is_none_or(|before| &position < before)
            && (position.kind != EventFeedKind::Peace || position.tags.len() >= 2)
        {
            held.insert(position, row);
            if held.len() > limit {
                held.pop_first();
            }
            true
        } else {
            false
        }
    };
    // Native facts precede derived groups, bounding suppressed peace reads.
    // Each source's ordinary rows establish the raw page's native range before
    // its late references are reached; obsolete project pages never reread a
    // whole older late-completion tie.
    for source in (0..readers.len()).rev() {
        let late = readers[source].late().or_else(|| metadata.late(source));
        let reader = &mut readers[source];
        for key in keys(source, categories) {
            check_cancel(cancelled)?;
            let mut to = native_to;
            let mut remaining = limit;
            while remaining > 0 && to >= first {
                let lower = if has_older || held.len() == limit {
                    first.max(
                        held.keys()
                            .nth(usize::from(has_older && held.len() == limit))
                            .unwrap()
                            .native,
                    )
                } else {
                    first
                };
                let through = reader.event_count_through(key, to)?;
                let Some(last) = through.checked_sub(1).and_then(|ordinal| {
                    late.map_or(Some(ordinal), |late| late.previous_ordinary(key, ordinal))
                }) else {
                    break;
                };
                let hour = reader.event_hour(key, last)?;
                if hour < lower {
                    break;
                }
                if hour > to {
                    return Err(crate::invalid(
                        "Prepared feed source made no backward progress",
                    ));
                }
                let end = reader.event_count_through(key, hour)?;
                let mut begin = if hour == i32::MIN {
                    0
                } else {
                    reader.event_count_through(key, hour - 1)?
                };
                // Same-date display order is forward; dates alone page backward.
                // Seek within even a 5,000-country tie without rereading its prefix.
                if let Some(before) = before.filter(|before| before.native == hour) {
                    let (mut low, mut high) = (begin, end);
                    while low < high {
                        let mid = low + (high - low) / 2;
                        let Some(ordinary) = late
                            .map_or(Some(mid), |late| late.next_ordinary(key, mid))
                            .filter(|ordinal| *ordinal < end)
                        else {
                            high = mid;
                            continue;
                        };
                        let row = reader.event(key, ordinary)?;
                        let position = order(source, row)?.0;
                        if position >= *before {
                            low = ordinary + 1;
                        } else {
                            high = mid;
                        }
                    }
                    begin = low;
                }
                while begin < end && remaining > 0 {
                    let Some(ordinary) = late
                        .map_or(Some(begin), |late| late.next_ordinary(key, begin))
                        .filter(|ordinal| *ordinal < end)
                    else {
                        break;
                    };
                    begin = ordinary;
                    let page = reader.events(&[key], hour, hour, begin, remaining as u32)?;
                    if page.is_empty() {
                        return Err(crate::invalid("Prepared feed source page made no progress"));
                    }
                    let next = begin
                        .checked_add(page.len() as u64)
                        .filter(|next| *next <= end)
                        .ok_or_else(|| crate::invalid("Prepared feed source cursor differs"))?;
                    for row in page {
                        let (position, row) = order(source, row)?;
                        if position.native < position.read {
                            continue; // Already reached through its LateIndex.
                        }
                        if retain(&mut held, position, row) {
                            remaining -= 1;
                        }
                    }
                    begin = next;
                }
                let Some(previous) = hour.checked_sub(1) else {
                    break;
                };
                to = previous;
            }
        }
        let ordinary = keys(source, categories);
        if let Some(late) = late {
            for &(native, _, key, ordinal) in late.between(first, native_to, live).rev() {
                check_cancel(cancelled)?;
                if !ordinary.contains(&(key & !(1 << 63))) {
                    continue;
                }
                if held.len() >= limit - 1 {
                    let cutoff = held
                        .keys()
                        .nth(usize::from(held.len() == limit))
                        .unwrap()
                        .native;
                    if native < cutoff {
                        // This reference proves a continuation below the raw
                        // page. Its older tie cannot change the selected rows.
                        has_older = true;
                        break;
                    }
                }
                let row = reader.event(key, ordinal)?;
                let (position, row) = order(source, row)?;
                retain(&mut held, position, row);
            }
        }
    }
    Ok((held, peace, has_older))
}

/// Rows a delta page reads at a time from one source key.
const DELTA_ROWS: u32 = 256;

/// The rows read after `after` through `live` (`bounds.1`), which a page answered through
/// `after` holds none of, for it to prepend, and whether they are more than `limit`: then the
/// newest `limit` of them are the whole first page, as each dates after every row read by
/// `after`. None where they cannot be prepended: one of them, or a war end naming a peace the
/// page holds, dates by `after`, or a project's change revises one read by then.
#[allow(clippy::too_many_arguments, clippy::type_complexity)]
fn delta(
    readers: &mut [Events<'_>],
    native_ends: &mut Reader<'_>,
    metadata: &Metadata,
    projects: &Projects,
    bounds: (i32, i32),
    after: i32,
    categories: &BTreeSet<EventFeedCategory>,
    limit: usize,
    cancelled: &AtomicBool,
) -> io::Result<Option<(BTreeMap<Position, SourceRow>, BTreeMap<i32, WarEnds>, bool)>> {
    let (first, live) = bounds;
    let mut held: BTreeMap<Position, SourceRow> = BTreeMap::new();
    let mut peace = BTreeMap::new();
    if after >= live {
        return Ok(Some((held, peace, false)));
    }
    let mut taken = 0_usize;
    // A peace held keeps the war ends read by then; one read later dated by then revises it.
    let ended = EventFeedKind::WarEnded as u64;
    if metadata
        .facts
        .2
        .between(i32::MIN, after, live)
        .any(|&(_, read, key, _)| read > after && key & !(1 << 63) == ended)
    {
        return Ok(None);
    }
    // Whether the page can take the row: None where it cannot be prepended.
    let mut admit = |peace: &mut BTreeMap<i32, WarEnds>,
                     source: usize,
                     row: Read|
     -> io::Result<Option<()>> {
        let (mut position, row) = decode(source, row)?;
        // A change to an episode read by then can revise a row held, wherever it now falls.
        if let SourceRow::Project(change) = &row
            && position.read <= live
            && projects.read_by(change.id, after)
        {
            return Ok(None);
        }
        if position.native < first || position.native > live || position.read > live {
            return Ok(Some(()));
        }
        if position.native <= after {
            return Ok(None);
        }
        if position.kind == EventFeedKind::Peace {
            if let std::collections::btree_map::Entry::Vacant(entry) = peace.entry(position.native)
            {
                entry.insert(war_ends(native_ends, metadata, position.native, live)?);
            }
            position.tags =
                peace_fallback(&peace[&position.native], position.native, position.tags);
            if position.tags.len() < 2 {
                return Ok(Some(()));
            }
        }
        held.insert(position, row);
        taken += 1;
        if held.len() > limit + 1 {
            held.pop_first();
        }
        Ok(Some(()))
    };
    for (source, reader) in readers.iter_mut().enumerate() {
        let late = reader.late().or_else(|| metadata.late(source));
        let ordinary = keys(source, categories);
        // Each key's events in the order of the hours that read them, its late ones among them.
        for &key in &ordinary {
            check_cancel(cancelled)?;
            let end = reader.event_count_through(key, live)?;
            let mut cursor = reader.event_count_through(key, after)?;
            while cursor < end {
                let page = reader.events(&[key], after + 1, live, cursor, DELTA_ROWS)?;
                if page.is_empty() {
                    return Err(crate::invalid("Prepared feed source page made no progress"));
                }
                cursor += page.len() as u64;
                for row in page {
                    if admit(&mut peace, source, row)?.is_none() {
                        return Ok(None);
                    }
                }
            }
        }
        // Facts read after their date are kept under keys of their own, reached through the index.
        if let Some(late) = late {
            for &(_, read, key, ordinal) in late.between(first, live, live) {
                if read <= after || key & (1 << 63) == 0 || !ordinary.contains(&(key & !(1 << 63)))
                {
                    continue;
                }
                check_cancel(cancelled)?;
                let row = reader.event(key, ordinal)?;
                if admit(&mut peace, source, row)?.is_none() {
                    return Ok(None);
                }
            }
        }
    }
    drop(admit);
    Ok(Some((held, peace, taken > limit)))
}

fn war_ends(
    reader: &mut Reader<'_>,
    metadata: &Metadata,
    hour: i32,
    live: i32,
) -> io::Result<BTreeSet<(i32, String, String)>> {
    let first = hour.saturating_sub(1);
    let key = EventFeedKind::WarEnded as u64;
    let mut result = BTreeSet::new();
    let count = reader.event_count_through(key, hour)?;
    let base = if first == i32::MIN {
        0
    } else {
        reader.event_count_through(key, first - 1)?
    };
    let mut cursor = base;
    while cursor < count {
        let page = reader.events(&[key], first, hour, cursor, 256)?;
        if page.rows.is_empty() {
            return Err(crate::invalid("Prepared war-end page made no progress"));
        }
        cursor += page.rows.len() as u64;
        for row in page.rows {
            add_war_end(&mut result, row, live)?;
        }
    }
    for &(_, _, key, ordinal) in metadata.facts.2.between(first, hour, live) {
        if key & !(1 << 63) == EventFeedKind::WarEnded as u64 {
            let (hour, row) = reader.event_raw(key, ordinal)?;
            add_war_end(
                &mut result,
                DatedRow {
                    key,
                    hour,
                    ordinal,
                    row,
                },
                live,
            )?;
        }
    }
    Ok(result)
}
fn add_war_end(
    ends: &mut BTreeSet<(i32, String, String)>,
    row: DatedRow,
    live: i32,
) -> io::Result<()> {
    let value = decode_fact(&row.row)?;
    if value.row.read_hour <= live && value.row.tags.len() == 2 {
        ends.insert(war_end_key(
            value.row.hour,
            &value.row.tags[0],
            &value.row.tags[1],
        ));
    }
    Ok(())
}

#[allow(clippy::too_many_arguments)]
fn project(
    builder: &mut Builder,
    position: &Position,
    row: SourceRow,
    ideas: &Ideas,
    nuke_types: &[NukeTypeInfo],
    politics: Option<&politics_content::PoliticsContent>,
    catalogue: Option<&Catalogue>,
    nuclear_art: &Result<Option<String>, String>,
    focus_art: Option<&artwork::ResolvedArt>,
    cancelled: &AtomicBool,
) -> io::Result<()> {
    let from = builder.feed.rows.len();
    match row {
        SourceRow::Group(change) => {
            group_row(builder, &change, change.name.clone().and_then(Result::ok))?;
        }
        SourceRow::Subject(change) => {
            subject_row(builder, &change);
        }
        SourceRow::Idea(change) => {
            let _phase = replay_frame::cpu::scope_phase("ef-q:idea-rows");
            let built = change
                .span
                .character
                .as_ref()
                .and_then(|holder| ideas.templates.get(&change.tag)?.get(&holder.id));
            idea_rows(
                builder,
                politics,
                [(
                    change.tag.as_str(),
                    &change.span,
                    change.naming.as_ref(),
                    built,
                )],
                cancelled,
            )?;
        }
        SourceRow::Focus(change) => {
            focus_row(
                builder,
                &change,
                focus_art.map(|(artwork, reason)| (artwork, reason.as_ref())),
            )?;
        }
        SourceRow::Increase(change) => {
            let icon = match nuclear_art {
                Ok(icon) => icon.clone(),
                Err(reason) => {
                    builder.unavailable(
                        "nuclearArtwork",
                        "Nuclear stockpile artwork",
                        reason.to_string(),
                        Some(1),
                    );
                    None
                }
            };
            // A whole-stockpile increase names no type.
            let item = nuclear_item(builder, None, catalogue, icon)?;
            builder.row(
                EventFeedKind::NukesIncreased,
                RowDate {
                    hour: change.hour,
                    read_hour: change.hour,
                    name_hour: change.hour,
                    exact: false,
                },
                vec![change.tag],
                Some(item),
                Some(change.amount),
            );
        }
        SourceRow::Project(change) => {
            if let Some(completion) = &change.completion {
                project_row(builder, completion)?;
            }
        }
        SourceRow::Fact(mut fact) => {
            if fact.row.item.is_some() {
                let mut item = fact
                    .item
                    .take()
                    .ok_or_else(|| crate::invalid("Event feed script item identity differs"))?;
                std::mem::take(&mut fact.shown).show(builder, catalogue, &mut item);
                fact.row.item = Some(builder.item(item)?);
            }
            if let Some(info) = fact.nuke_type.and_then(|kind| {
                nuke_types
                    .iter()
                    .find(|info| info.ordinal == kind && info.supported)
            }) {
                let icon = match nuclear_art {
                    Ok(icon) => icon.clone(),
                    Err(reason) => {
                        builder.unavailable(
                            "nuclearArtwork",
                            "Nuclear stockpile artwork",
                            reason.to_string(),
                            Some(1),
                        );
                        None
                    }
                };
                fact.row.item = Some(nuclear_item(builder, Some(info), catalogue, icon)?);
            }
            builder.feed.rows.push(fact.row);
        }
    }
    for row in &mut builder.feed.rows[from..] {
        row.id = position.id.clone();
    }
    Ok(())
}

/// The live hour a continuation's first page was answered through, which
/// every later page of it keeps; None for a cursor this feed did not write.
pub(crate) fn cursor_live(cursor: &str) -> Option<i32> {
    serde_json::from_str::<Cursor>(cursor)
        .ok()
        .map(|cursor| cursor.live)
}

/// The hours a page reads, `first` through `last`: those it asks within its branch's `bounds`, or
/// None where it asks only hours outside them. Asked hours that end before they start refuse.
fn page_hours(params: &Params, (first, last): (i32, i32)) -> io::Result<Option<(i32, i32)>> {
    let asked = params.first_hour.unwrap_or(first);
    if asked > params.last_hour || params.page_size == 0 || params.page_size == u32::MAX {
        return Err(crate::invalid("Invalid prepared feed page"));
    }
    let (first, last) = (asked.max(first), params.last_hour.min(last));
    Ok((first <= last).then_some((first, last)))
}

/// A page of no events or cursor over `first` through `last`.
fn blank(params: &Params, first: i32, last: i32) -> Page {
    Page {
        reply: EventFeedReply {
            branch_id: params.branch_id.clone(),
            first_hour: first,
            last_hour: last,
            windows: Vec::new(),
            rows: Vec::new(),
            items: Vec::new(),
            modifiers: Vec::new(),
            texts: Vec::new(),
            unavailable: Vec::new(),
            warnings: Vec::new(),
        },
        next_cursor: None,
        after: None,
    }
}

/// The page's branch and that branch's first and last hours.
fn branch_of(source: &Source, params: &Params) -> io::Result<(u64, (i32, i32))> {
    let branch = params
        .branch_id
        .parse::<u64>()
        .map_err(|_| crate::invalid("Invalid prepared feed branch"))?;
    let start = source
        .replay_ref()
        .branches
        .iter()
        .find(|start| start.id == branch)
        .ok_or_else(|| crate::invalid("Unknown prepared feed branch"))?;
    Ok((branch, branch_bounds(source.replay_ref(), start)))
}

/// A page outside its branch's hours, which hold no event, over the hours it asks.
fn outside(params: &Params, bounds: (i32, i32)) -> io::Result<Page> {
    if params.cursor.is_some() {
        return Err(crate::invalid("Invalid prepared feed page"));
    }
    Ok(blank(
        params,
        params.first_hour.unwrap_or(bounds.0),
        params.last_hour,
    ))
}

/// A cold page has no events or cursor; its requested extent within its branch is still pending.
pub(crate) fn empty(source: &Source, params: &Params) -> io::Result<Page> {
    let (_, bounds) = branch_of(source, params)?;
    match page_hours(params, bounds)? {
        Some(_) if params.cursor.is_some() => Err(crate::invalid("Invalid prepared feed page")),
        Some((first, last)) => Ok(blank(params, first, last)),
        None => outside(params, bounds),
    }
}

pub(crate) fn query(
    source: &Source,
    mut params: Params,
    context: &LoadContext,
    store: &store::Store,
    key: &store::SourceKey,
    families: Option<Families<'_>>,
    cancelled: &AtomicBool,
) -> io::Result<Option<Page>> {
    check_cancel(cancelled)?;
    let (branch, bounds) = branch_of(source, &params)?;
    let Some((first, live)) = page_hours(&params, bounds)? else {
        return outside(&params, bounds).map(Some);
    };
    let cursor: Option<Cursor> = params
        .cursor
        .as_deref()
        .map(serde_json::from_str)
        .transpose()?;
    if params.after.is_some() && cursor.is_some() {
        return Err(crate::invalid(
            "A prepared feed page read after an hour has no cursor",
        ));
    }
    if cursor.as_ref().is_some_and(|cursor| {
        cursor.live != live
            || cursor.first != first
            || cursor.categories != params.categories
            || cursor.before.native < first
            || cursor.before.native > live
            || cursor.before.read > live
    }) {
        return Err(crate::invalid(
            "Prepared feed cursor belongs to another view",
        ));
    }
    let phase = replay_frame::cpu::scope_phase("ef-q:metadata");
    let (Some(metadata), Some(families)) = (metadata(source, store, key, branch)?, families) else {
        return Ok(None);
    };
    // Laws are told from other ideas by the recorded content, read as the page is asked.
    let politics = context
        .content()
        .and_then(|content| politics_content::politics_content(source, content));
    drop(phase);
    let phase = replay_frame::cpu::scope_phase("ef-q:classify");
    let classified = kept_classified(source, branch, families.ideas, politics.as_ref().ok())?;
    let ideas = ClassifiedIdeas {
        ideas: families.ideas,
        classified: &classified,
    };
    drop(phase);
    let phase = replay_frame::cpu::scope_phase("ef-q:candidates");
    let given: [Option<&dyn Held>; 7] = [
        None,
        Some(families.subjects),
        Some(&ideas),
        Some(families.focus),
        None,
        Some(families.nukes),
        Some(families.projects),
    ];
    let mut readers: Vec<_> = PAGES
        .iter()
        .zip(given)
        .enumerate()
        .map(|(source, (pages, given))| match (pages, given) {
            (Some(family), _) => Reader::new(store, key, format!("{family}-{branch}"), cancelled)
                .map(|reader| Events::Pages(reader, source)),
            (None, Some(held)) => Ok(Events::Given(held)),
            (None, None) => Err(crate::invalid("A prepared feed source has no events")),
        })
        .collect::<io::Result<_>>()?;
    let mut native_ends = Reader::new(store, key, format!("event-feed-facts-{branch}"), cancelled)?;
    // A page read after an hour at or past the live one is answered whole.
    let since = match params.after.filter(|&after| after <= live) {
        Some(after) => delta(
            &mut readers,
            &mut native_ends,
            &metadata,
            families.projects,
            (first, live),
            after,
            &params.categories,
            params.page_size as usize,
            cancelled,
        )?
        .map(|rows| (rows, after)),
        None => None,
    };
    let (mut rows, peace, next_cursor, after) = match since {
        Some(((rows, peace, false), after)) => (rows, peace, None, Some(after)),
        Some(((mut rows, peace, true), _)) => {
            rows.pop_first();
            let before = rows.first_key_value().unwrap().0.clone();
            let next_cursor = serde_json::to_string(&Cursor {
                live,
                first,
                categories: params.categories.clone(),
                before,
            })?;
            (rows, peace, Some(next_cursor), None)
        }
        None => {
            let (mut rows, peace, has_older) = candidates(
                &mut readers,
                &mut native_ends,
                &metadata,
                (first, live),
                cursor.as_ref().map(|cursor| &cursor.before),
                &params.categories,
                params.page_size as usize + 1,
                cancelled,
            )?;
            let more = has_older || rows.len() > params.page_size as usize;
            if rows.len() > params.page_size as usize {
                rows.pop_first();
            }
            let next_cursor = if more {
                let oldest = rows.first_key_value().unwrap().0;
                if cursor
                    .as_ref()
                    .is_some_and(|cursor| oldest >= &cursor.before)
                {
                    return Err(crate::invalid(
                        "Prepared feed continuation made no progress",
                    ));
                }
                Some(serde_json::to_string(&Cursor {
                    live,
                    first,
                    categories: params.categories.clone(),
                    before: rows.first_key_value().unwrap().0.clone(),
                })?)
            } else {
                None
            };
            (rows, peace, next_cursor, None)
        }
    };
    drop(phase);
    // Revisions remain raw paging operands even when they no longer project a
    // row at Live: only an episode's change current at Live is listed.
    rows.retain(|position, row| match row {
        SourceRow::Project(change) => {
            change.completion.is_some()
                && families
                    .projects
                    .current(change.id, position.ordinal, live)
        }
        _ => true,
    });
    let phase = replay_frame::cpu::scope_phase("ef-q:diagnostics");
    let mut builder = Builder::new(first, live);
    diagnostics(
        &mut builder,
        &metadata,
        &families,
        &classified.unclassified,
        live,
    )?;
    let nuke_types = families.nukes.types();
    let mut project_refusals = catalogue::Refusals::default();
    for row in rows.values() {
        if let SourceRow::Project(change) = row {
            for (reason, count) in &change.warnings {
                builder.unavailable(
                    "projectFinished",
                    "Special project presentation",
                    reason,
                    Some(
                        u32::try_from(*count)
                            .map_err(|_| crate::invalid("Project refusal count exceeds u32"))?,
                    ),
                );
            }
            project_refusals.merge(&change.refusals);
        }
    }
    for (reason, count) in project_refusals.counted_diagnostic_lines("Project texts") {
        builder.unavailable(
            "projectFinished",
            "Special project presentation",
            reason,
            Some(
                u32::try_from(count)
                    .map_err(|_| crate::invalid("Project refusal count exceeds u32"))?,
            ),
        );
    }
    drop(phase);
    let phase = replay_frame::cpu::scope_phase("ef-q:catalogue");
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
    let politics = match politics {
        Ok(content) => Some(content),
        Err(reason) => {
            builder.unavailable(
                "politicsContent",
                "Politics content",
                reason.to_string(),
                Some(1),
            );
            None
        }
    };
    if let Some(content) = &politics {
        builder
            .feed
            .warnings
            .extend(content.warnings.iter().cloned());
    }
    drop(phase);
    let phase = replay_frame::cpu::scope_phase("ef-q:nuclear-art");
    let nuclear_art = if rows.keys().any(|position| {
        matches!(
            position.kind,
            EventFeedKind::NukesIncreased | EventFeedKind::NuclearStrike
        )
    }) {
        catalogue
            .as_deref()
            .map(artwork::nuke_icon)
            .transpose()
            .map(Option::flatten)
            .map_err(|error| error.to_string())
    } else {
        Ok(None)
    };
    // Each completion's art as preparation chose it at its own hour, read for this page alone.
    let (ordinals, hours): (Vec<u64>, Vec<i32>) = rows
        .iter()
        .filter_map(|(position, row)| match row {
            SourceRow::Focus(change) => Some((position.ordinal, change.hour)),
            _ => None,
        })
        .unzip();
    drop(phase);
    let phase = replay_frame::cpu::scope_phase("ef-q:focus-art");
    let focus_art: Vec<Option<artwork::ResolvedArt>> = match context.content() {
        Ok(content) => {
            let stored = families.focus_art.art(&ordinals, cancelled)?;
            let completions: Vec<(i32, Option<crate::prepared_focus::art::CompletionArt>)> =
                hours.into_iter().zip(stored).collect();
            match crate::prepared_focus::art::event_art(source, content, &completions)? {
                Ok((art, warnings)) => {
                    builder.feed.warnings.extend(warnings);
                    art.into_iter().map(Some).collect()
                }
                Err(reason) => {
                    builder.unavailable("focusPresentation", "Focus presentation", reason, Some(1));
                    vec![None; completions.len()]
                }
            }
        }
        Err(_) => vec![None; ordinals.len()],
    };
    let mut focus_art = focus_art.into_iter();
    // Completed projects' icons, kept as their choice's operands, drawn as this request shows them.
    // Their content is loaded only for a page holding completions; content that does not load
    // leaves each icon unavailable with why and the rest of the page as it is.
    drop(phase);
    let phase = replay_frame::cpu::scope_phase("ef-q:project-art");
    let failed = special_projects::present_completions(
        source,
        &special_projects::ArtSource::new(source, context.content.as_ref()),
        rows.values_mut().filter_map(|row| match row {
            SourceRow::Project(change) => change.completion.as_mut(),
            _ => None,
        }),
    )?;
    builder
        .feed
        .warnings
        .extend(special_projects::art_warnings(failed));
    drop(phase);
    let phase = replay_frame::cpu::scope_phase("ef-q:present");
    for (position, row) in rows {
        check_cancel(cancelled)?;
        if position.kind == EventFeedKind::Peace {
            builder.war_ends = peace[&position.native].clone();
        }
        let art = if matches!(row, SourceRow::Focus(_)) {
            focus_art.next().flatten()
        } else {
            None
        };
        project(
            &mut builder,
            &position,
            row,
            families.ideas,
            &nuke_types,
            politics.as_deref(),
            catalogue.as_deref(),
            &nuclear_art,
            art.as_ref(),
            cancelled,
        )?;
    }
    drop(phase);
    let phase = replay_frame::cpu::scope_phase("ef-q:idea-format");
    if let Some(content) = politics {
        let (keys, texts) = idea_history::effect_keys(
            builder
                .feed
                .items
                .iter()
                .filter_map(|item| item.idea.as_ref()),
        );
        let (formats, warnings) = idea_format::modifier_formats_from_recorded(
            families.ideas.definitions.as_ref(),
            &keys,
            &content,
        );
        builder.feed.modifiers = formats;
        builder.feed.texts = idea_format::texts(&content, &texts);
        builder.feed.warnings.extend(warnings);
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
    drop(phase);
    let _phase = replay_frame::cpu::scope_phase("ef-q:reply");
    builder.feed.rows.sort_by(row_order);
    builder.feed.warnings.sort();
    builder.feed.warnings.dedup();
    let mut reply = reply(&builder.feed, &params.branch_id, first, live, true);
    reply.unavailable = builder.feed.unavailable;
    reply.warnings = builder.feed.warnings;
    let page = Page {
        reply,
        next_cursor,
        after,
    };
    if let Err(error) = crate::products::check_reply_budget(&page) {
        if params.page_size == 1 {
            return Err(error);
        }
        // A page is an I/O unit. Shrinking it changes no source selection or cursor bound.
        params.page_size /= 2;
        return query(source, params, context, store, key, Some(families), cancelled);
    }
    Ok(Some(page))
}

fn diagnostics(
    builder: &mut Builder,
    metadata: &Metadata,
    families: &Families<'_>,
    unclassified: &BTreeMap<String, u32>,
    live: i32,
) -> io::Result<()> {
    let Families {
        nukes,
        subjects,
        focus,
        ideas,
        projects,
        focus_art: _,
    } = *families;
    builder.feed.unavailable = metadata.facts.0.feed.unavailable.clone();
    builder.unavailable_keys = metadata.facts.0.unavailable_keys.clone();
    builder
        .feed
        .warnings
        .extend(metadata.groups.warnings.iter().cloned());
    builder
        .feed
        .warnings
        .extend(subjects.warnings.iter().cloned());
    builder
        .feed
        .warnings
        .extend(projects.warnings.iter().cloned());
    builder.unavailable("warDeclarationOrJoin", "War declaration or join", "WarRelations supplies the pair becoming at war. Declarations and joins require the native event history; refused intervals cannot distinguish those actions.", None);
    let counted = |count: usize| {
        u32::try_from(count).map_err(|_| crate::invalid("Feed refusal count exceeds u32"))
    };
    if nukes.first_read.is_none_or(|first| first > live) {
        builder.unavailable(
            "nukesIncreased",
            "Nuclear stockpile",
            "No complete Nukes read was recorded.",
            None,
        );
    }
    for (ordinal, count) in &nukes.refused {
        builder.unavailable(
            "nuclearType",
            "Nuclear type",
            format!("Nuclear type {ordinal} has no native content-name mapping."),
            Some(counted(*count)?),
        );
    }
    for (reason, spans) in &nukes.refusals {
        let count: u64 = spans
            .iter()
            .filter(|span| span.first_hour <= live)
            .map(|span| {
                (i64::from(span.last_hour.min(live)) - i64::from(span.first_hour) + 1) as u64
            })
            .sum();
        if count > 0 {
            builder.unavailable(
                "nukesIncreased",
                "Nuclear stockpile",
                reason,
                Some(
                    u32::try_from(count)
                        .map_err(|_| crate::invalid("Nuclear refusal count exceeds u32"))?,
                ),
            );
        }
    }
    if !projects.available {
        builder.unavailable(
            "projectFinished",
            "Special projects",
            "Projects was unavailable, incomplete, or never read for any country.",
            None,
        );
    }
    if let Some(reason) = &projects.content_error {
        builder.unavailable(
            "projectFinished",
            "Special project presentation",
            reason,
            Some(1),
        );
    }
    for (table, kind, label) in [
        ("WarRelations", "warStarted", "Wars and peace"),
        ("Capitulation", "capitulated", "Capitulations"),
        ("CountryCounterInputs", "ceasedToExist", "Country existence"),
        ("FactionMembers", "factionJoined", "Faction changes"),
        ("RelationTuples", "guaranteeGranted", "Guarantees"),
    ] {
        if !metadata.groups.observed.contains(table) && !metadata.facts.1.contains(table) {
            builder.unavailable(
                kind,
                label,
                format!("{table} was never recorded complete on this branch."),
                None,
            );
        }
    }
    for (kind, label, reason, count) in [
        (
            "warStartIdentity",
            "War starts",
            "Observed exact war starts could not be attributed to country identities at their native start hours.",
            metadata.groups.unattributed_starts,
        ),
        (
            "capitulationIdentity",
            "Capitulations",
            "Observed exact capitulations could not be attributed to country identities at their native event hours.",
            metadata.groups.unattributed_capitulations,
        ),
        (
            "capitulationCoverage",
            "Capitulation read coverage",
            "Recorded hours had unread capitulation tables; this counts unread hours, not unseen events.",
            metadata.groups.unread_capitulations,
        ),
    ] {
        if count > 0 {
            builder.unavailable(kind, label, reason, Some(counted(count)?));
        }
    }
    let mut names = catalogue::Refusals::default();
    for (reason, ids) in &metadata.groups.name_refusals {
        for id in ids {
            names.note(reason, id.to_string());
        }
    }
    for (reason, count) in names.counted_diagnostic_lines("faction names are not shown") {
        builder.unavailable("factionName", "Faction name", reason, Some(counted(count)?));
    }
    for refusal in &metadata.groups.refusals {
        builder.unavailable(
            &refusal.key,
            &refusal.label,
            &refusal.reason,
            Some(
                u32::try_from(refusal.count)
                    .map_err(|_| crate::invalid("Feed refusal count exceeds u32"))?,
            ),
        );
    }
    for (reason, count) in &ideas.refusals {
        builder.unavailable(
            "ideaObservation",
            "Laws and advisors",
            reason,
            Some(counted(*count)?),
        );
    }
    for (reason, count) in &focus.unavailable {
        builder.unavailable(
            "focusCompleted",
            "Focus completions",
            reason,
            Some(
                u32::try_from(*count)
                    .map_err(|_| crate::invalid("Feed refusal count exceeds u32"))?,
            ),
        );
    }
    if !subjects.observed {
        builder.unavailable(
            "becameSubject",
            "Subjects",
            "No complete Subjects read with resolved countries was recorded.",
            None,
        );
    }
    if !ideas.observed {
        builder.unavailable("lawChanged", "Laws and advisors", "ActiveIdeas and HiredAdvisors were not recorded complete together with resolved countries.", None);
    }
    if !focus.observed {
        builder.unavailable(
            "focusCompleted",
            "Focus completions",
            "Focus and CompletedFocuses were not recorded complete with resolved countries.",
            None,
        );
    }
    if ideas.gap_additions > 0 {
        builder.unavailable("ideaAdditionAfterGap", "Law and advisor changes", "Changed holdings after an unrecorded stretch cannot establish whether a law changed or an advisor was hired.", Some(counted(ideas.gap_additions)?));
    }
    for (token, count) in unclassified {
        builder.unavailable("lawChanged", "Law classification", format!("Idea {token}: the recorded content cannot establish whether this addition is a law."), Some(*count));
    }
    Ok(())
}
