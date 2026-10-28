//! Backward native-date pages composed from the owning families' event ranges.
use super::*;
use crate::prepared::series::{self, DatedRow, LateIndex, Reader};
use crate::products::special_projects::prepared::completion_stream;
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
    /// Also list the recording's own observations: decision roster rebuilds and retained script state.
    #[serde(default)]
    pub observations: bool,
}
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct Page {
    pub reply: EventFeedReply,
    pub next_cursor: Option<String>,
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
    #[serde(default)]
    observations: bool,
    before: Position,
}

const FAMILIES: [&str; 7] = [
    "country-group-changes",
    "subject-changes",
    "idea-changes",
    "focus-completions",
    "event-feed-facts",
    country_nukes::prepared::INCREASES,
    completion_stream::FAMILY,
];
/// The feed facts' catalogue: diagnostics, observed tables and late
/// references. Each opener's continuing facts are entries apart.
pub(in crate::products::event_feed) type FactsCatalogue = (Builder, BTreeSet<String>, LateIndex);
/// Whether any country's focus was observed, the completions and countries
/// without an event date by reason, and the late references.
type FocusMetadata = (bool, BTreeMap<String, u64>, LateIndex);
struct Metadata {
    groups: country_groups::changes::Metadata,
    subjects: (bool, Vec<String>, LateIndex),
    ideas: idea_history::ChangeMetadata,
    definitions: Option<BTreeMap<String, replay_data::rows::ModifierDefinitionRow>>,
    focus: FocusMetadata,
    facts: FactsCatalogue,
    nukes: country_nukes::prepared::IncreaseCatalogue,
    projects: completion_stream::Metadata,
}
impl Metadata {
    fn late(&self, source: usize) -> Option<&LateIndex> {
        match source {
            0 => Some(&self.groups.late),
            1 => Some(&self.subjects.2),
            2 => Some(&self.ideas.late),
            3 => Some(&self.focus.2),
            4 => Some(&self.facts.2),
            6 => Some(&self.projects.late),
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
    for family in FAMILIES {
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
    let groups = read(store, key, FAMILIES[0], branch)?;
    let subjects = read(store, key, FAMILIES[1], branch)?;
    let ideas: idea_history::ChangeMetadata = read(store, key, FAMILIES[2], branch)?;
    let definitions = ideas.definition_index()?;
    let value = Arc::new(Metadata {
        groups,
        subjects,
        ideas,
        definitions,
        focus: read(store, key, FAMILIES[3], branch)?,
        facts: read(store, key, FAMILIES[4], branch)?,
        nukes: read(store, key, FAMILIES[5], branch)?,
        projects: read(store, key, FAMILIES[6], branch)?,
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
    Increase(country_nukes::prepared::StockpileIncrease),
    Project(completion_stream::Change),
}
fn decode(source: usize, row: DatedRow) -> io::Result<(Position, SourceRow)> {
    let value = match source {
        0 => SourceRow::Group(serde_json::from_value(row.row)?),
        1 => SourceRow::Subject(serde_json::from_value(row.row)?),
        2 => SourceRow::Idea(serde_json::from_value(row.row)?),
        3 => SourceRow::Focus(serde_json::from_value(row.row)?),
        4 => SourceRow::Fact(Box::new(serde_json::from_value(row.row)?)),
        5 => SourceRow::Increase(serde_json::from_value(row.row)?),
        6 => SourceRow::Project(serde_json::from_value(row.row)?),
        _ => return Err(crate::invalid("Unknown prepared feed source")),
    };
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

fn event(reader: &mut Reader<'_>, key: u64, ordinal: u64) -> io::Result<DatedRow> {
    let mut page = reader.events(&[key], i32::MIN, i32::MAX, ordinal, 1)?;
    #[cfg(test)]
    EVENT_ROWS_READ.with(|count| count.set(count.get() + page.rows.len()));
    page.rows
        .pop()
        .ok_or_else(|| crate::invalid("Prepared feed event reference is missing"))
}

fn keys(source: usize, categories: &BTreeSet<EventFeedCategory>, observations: bool) -> Vec<u64> {
    let pairs: Vec<(u64, EventFeedCategory)> = match source {
        0 => vec![
            (0, EventFeedCategory::Diplomacy),
            (1, EventFeedCategory::Diplomacy),
            (2, EventFeedCategory::WarsPeace),
            (3, EventFeedCategory::WarsPeace),
            (4, EventFeedCategory::WarsPeace),
        ],
        1 => vec![(0, EventFeedCategory::Diplomacy)],
        2 => vec![
            (0, EventFeedCategory::Politics),
            (1, EventFeedCategory::Politics),
        ],
        3 => vec![(0, EventFeedCategory::Milestones)],
        6 => vec![(completion_stream::KEY, EventFeedCategory::Milestones)],
        5 => vec![(
            country_nukes::prepared::INCREASE_KEY,
            EventFeedCategory::Milestones,
        )],
        _ => FACT_KEYS
            .iter()
            .copied()
            .filter(|key| key & (1 << 63) == 0)
            // Roster rebuilds and retained script state are recording inputs, not moments in the game.
            .filter(|&key| {
                observations
                    || !matches!(
                        EventFeedKind::from_key(key),
                        Some(EventFeedKind::DecisionRosterRebuilt | EventFeedKind::ScriptStateObserved)
                    )
            })
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
    readers: &mut [Reader<'_>],
    native_ends: &mut Reader<'_>,
    metadata: &Metadata,
    bounds: (i32, i32),
    before: Option<&Position>,
    categories: &BTreeSet<EventFeedCategory>,
    observations: bool,
    limit: usize,
    cancelled: &AtomicBool,
) -> io::Result<(BTreeMap<Position, SourceRow>, BTreeMap<i32, WarEnds>, bool)> {
    let (first, live) = bounds;
    let native_to = before.map_or(live, |position| position.native);
    let mut held: BTreeMap<Position, SourceRow> = BTreeMap::new();
    let mut peace = BTreeMap::new();
    let mut has_older = false;
    let mut order = |source, row: DatedRow| -> io::Result<(Position, SourceRow)> {
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
        let reader = &mut readers[source];
        let late = metadata.late(source);
        for key in keys(source, categories, observations) {
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
                let hour = event(reader, key, last)?.hour;
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
                        let row = event(reader, key, ordinary)?;
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
                    #[cfg(test)]
                    EVENT_ROWS_READ.with(|count| count.set(count.get() + page.rows.len()));
                    if page.rows.is_empty() {
                        return Err(crate::invalid("Prepared feed source page made no progress"));
                    }
                    let next = begin
                        .checked_add(page.rows.len() as u64)
                        .filter(|next| *next <= end)
                        .ok_or_else(|| crate::invalid("Prepared feed source cursor differs"))?;
                    for row in page.rows {
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
        let ordinary = keys(source, categories, observations);
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
                let row = event(reader, key, ordinal)?;
                let (position, row) = order(source, row)?;
                retain(&mut held, position, row);
            }
        }
    }
    Ok((held, peace, has_older))
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
            add_war_end(&mut result, event(reader, key, ordinal)?, live)?;
        }
    }
    Ok(result)
}
fn add_war_end(
    ends: &mut BTreeSet<(i32, String, String)>,
    row: DatedRow,
    live: i32,
) -> io::Result<()> {
    let value: Fact = serde_json::from_value(row.row)?;
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
    metadata: &Metadata,
    politics: Option<&politics_content::PoliticsContent>,
    catalogue: Option<&Catalogue>,
    nuclear_art: &Result<Option<String>, String>,
    focus_art: Option<&artwork::ResolvedArt>,
    texts: &ScopedTexts<'_>,
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
            let built = change
                .span
                .character
                .as_ref()
                .and_then(|holder| metadata.ideas.templates.get(&change.tag)?.get(&holder.id));
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
            let kind = change
                .type_ordinal
                .and_then(|ordinal| change.types.iter().find(|kind| kind.ordinal == ordinal));
            let item = nuclear_item(builder, kind, catalogue, icon)?;
            builder.row(
                EventFeedKind::NukesIncreased,
                RowDate {
                    hour: change.hour,
                    read_hour: change.read_hour,
                    name_hour: change.hour,
                    exact: change.exact,
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
                if let Some(catalogue) = catalogue {
                    super::super::script_presentation(
                        builder,
                        catalogue,
                        fact.row.kind,
                        &mut item,
                        fact.script.as_ref(),
                        Some((texts, fact.row.name_hour)),
                    )?;
                }
                fact.row.item = Some(builder.item(item)?);
            }
            if let Some(info) = fact.nuke_type.and_then(|kind| {
                metadata
                    .nukes
                    .types
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

/// A cold page has no events or cursor; its requested extent is still pending.
pub(crate) fn empty(source: &Source, params: &Params) -> io::Result<Page> {
    let branch: u64 = params
        .branch_id
        .parse()
        .map_err(|_| crate::invalid("Invalid prepared feed branch"))?;
    let start = source
        .replay_ref()
        .branches
        .iter()
        .find(|start| start.id == branch)
        .ok_or_else(|| crate::invalid("Unknown prepared feed branch"))?;
    let (first, last) = branch_bounds(source.replay_ref(), start);
    let first = params.first_hour.unwrap_or(first).max(first);
    let last = params.last_hour.min(last);
    if first > last
        || params.page_size == 0
        || params.page_size == u32::MAX
        || params.cursor.is_some()
    {
        return Err(crate::invalid("Invalid prepared feed page"));
    }
    Ok(Page {
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
    })
}

pub(crate) fn query(
    source: &Source,
    mut params: Params,
    context: &LoadContext,
    store: &store::Store,
    key: &store::SourceKey,
    cancelled: &AtomicBool,
) -> io::Result<Option<Page>> {
    check_cancel(cancelled)?;
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
    let (branch_first, branch_last) = branch_bounds(source.replay_ref(), start);
    let first = params.first_hour.unwrap_or(branch_first).max(branch_first);
    let live = params.last_hour.min(branch_last);
    if first > live || params.page_size == 0 || params.page_size == u32::MAX {
        return Err(crate::invalid("Invalid prepared feed page"));
    }
    let cursor: Option<Cursor> = params
        .cursor
        .as_deref()
        .map(serde_json::from_str)
        .transpose()?;
    if cursor.as_ref().is_some_and(|cursor| {
        cursor.live != live
            || cursor.first != first
            || cursor.categories != params.categories
            || cursor.observations != params.observations
            || cursor.before.native < first
            || cursor.before.native > live
            || cursor.before.read > live
    }) {
        return Err(crate::invalid(
            "Prepared feed cursor belongs to another view",
        ));
    }
    let Some(metadata) = metadata(source, store, key, branch)? else {
        return Ok(None);
    };
    let mut readers: Vec<_> = FAMILIES
        .iter()
        .map(|family| Reader::new(store, key, format!("{family}-{branch}"), cancelled))
        .collect::<io::Result<_>>()?;
    let mut native_ends = Reader::new(store, key, format!("event-feed-facts-{branch}"), cancelled)?;
    let (mut rows, peace, has_older) = candidates(
        &mut readers,
        &mut native_ends,
        &metadata,
        (first, live),
        cursor.as_ref().map(|cursor| &cursor.before),
        &params.categories,
        params.observations,
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
            observations: params.observations,
            before: rows.first_key_value().unwrap().0.clone(),
        })?)
    } else {
        None
    };
    // Revisions remain raw paging operands even when they no longer project a
    // row at Live. Validate the bounded page together, preserving its cursor.
    let projects: Vec<_> = rows
        .iter()
        .filter_map(|(position, row)| match row {
            SourceRow::Project(change) => {
                Some(completion_stream::episode_key(change.id).map(|key| (key, position.ordinal)))
            }
            _ => None,
        })
        .collect::<io::Result<_>>()?;
    let current = readers[6].values_at(
        &projects.iter().map(|(key, _)| *key).collect::<Vec<_>>(),
        live,
    )?;
    let current: BTreeSet<_> = projects
        .into_iter()
        .zip(current)
        .filter_map(|((_, ordinal), current)| {
            (current == Some(i128::from(ordinal))).then_some(ordinal)
        })
        .collect();
    rows.retain(|position, row| match row {
        SourceRow::Project(change) => {
            change.completion.is_some() && current.contains(&position.ordinal)
        }
        _ => true,
    });
    // A retained state observation's parts follow its own notes.
    let mut sections = BTreeMap::<u64, Vec<String>>::new();
    for row in rows.values_mut() {
        let SourceRow::Fact(fact) = row else {
            continue;
        };
        for ordinal in std::mem::take(&mut fact.sections) {
            check_cancel(cancelled)?;
            if let std::collections::btree_map::Entry::Vacant(entry) = sections.entry(ordinal) {
                entry.insert(serde_json::from_value(
                    event(&mut readers[4], super::SECTIONS, ordinal)?.row,
                )?);
            }
            fact.row
                .cause
                .as_mut()
                .ok_or_else(|| crate::invalid("Retained state observation has no notes"))?
                .notes
                .extend(sections[&ordinal].iter().cloned());
        }
    }
    let mut builder = Builder::new(first, live);
    let texts = ScopedTexts::rising(source, branch, cancelled);
    diagnostics(&mut builder, &metadata, live)?;
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
    if catalogue.is_some() {
        texts.prefetch(rows.values().filter_map(|row| match row {
            SourceRow::Fact(fact)
                if fact.row.item.is_some()
                    && fact.script.is_some()
                    && super::super::names_decision(fact.row.kind) =>
            {
                Some(fact.row.name_hour)
            }
            _ => None,
        }));
    }
    let politics = match context
        .content()
        .and_then(|content| politics_content::politics_content(source, content))
    {
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
    // Each completion's art at its own hour, evaluated now: preparation
    // keeps completions as plain events.
    let completions: Vec<(i32, &str, &str)> = rows
        .values()
        .filter_map(|row| match row {
            SourceRow::Focus(change) => {
                Some((change.hour, change.tag.as_str(), change.token.as_str()))
            }
            _ => None,
        })
        .collect();
    let focus_art: Vec<Option<artwork::ResolvedArt>> = match context.content() {
        Ok(content) => match crate::prepared_focus::art::event_art(
            source,
            content,
            branch,
            &completions,
            cancelled,
        )? {
            Ok((art, warnings)) => {
                builder.feed.warnings.extend(warnings);
                art.into_iter().map(Some).collect()
            }
            Err(reason) => {
                builder.unavailable("focusPresentation", "Focus presentation", reason, Some(1));
                vec![None; completions.len()]
            }
        },
        Err(_) => vec![None; completions.len()],
    };
    let mut focus_art = focus_art.into_iter();
    // Completed projects' icons, kept as their choice's operands, drawn as this request shows them.
    // Their content is loaded only for a page holding completions; content that does not load
    // leaves each icon unavailable with why and the rest of the page as it is.
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
            &metadata,
            politics.as_deref(),
            catalogue.as_deref(),
            &nuclear_art,
            art.as_ref(),
            &texts,
            cancelled,
        )?;
    }
    if let Some(content) = politics {
        let (keys, texts) = idea_history::effect_keys(
            builder
                .feed
                .items
                .iter()
                .filter_map(|item| item.idea.as_ref()),
        );
        let (formats, warnings) = idea_format::modifier_formats_from_recorded(
            metadata.definitions.as_ref(),
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
    builder.feed.rows.sort_by(row_order);
    builder.feed.warnings.sort();
    builder.feed.warnings.dedup();
    let mut reply = reply(&builder.feed, &params.branch_id, first, live, true);
    reply.unavailable = builder.feed.unavailable;
    reply.warnings = builder.feed.warnings;
    let page = Page { reply, next_cursor };
    if let Err(error) = crate::products::check_reply_budget(&page) {
        if params.page_size == 1 {
            return Err(error);
        }
        // A page is an I/O unit. Shrinking it changes no source selection or cursor bound.
        params.page_size /= 2;
        return query(source, params, context, store, key, cancelled);
    }
    Ok(Some(page))
}

fn diagnostics(builder: &mut Builder, metadata: &Metadata, live: i32) -> io::Result<()> {
    builder.feed.unavailable = metadata.facts.0.feed.unavailable.clone();
    builder.unavailable_keys = metadata.facts.0.unavailable_keys.clone();
    builder
        .feed
        .warnings
        .extend(metadata.groups.warnings.iter().cloned());
    builder
        .feed
        .warnings
        .extend(metadata.subjects.1.iter().cloned());
    builder
        .feed
        .warnings
        .extend(metadata.projects.warnings.iter().cloned());
    builder.unavailable("warDeclarationOrJoin", "War declaration or join", "WarRelations supplies the pair becoming at war. Declarations and joins require the native event history; refused intervals cannot distinguish those actions.", None);
    let counted = |count: usize| {
        u32::try_from(count).map_err(|_| crate::invalid("Feed refusal count exceeds u32"))
    };
    if !metadata
        .nukes
        .read_spans
        .iter()
        .any(|span| span.first_hour <= live)
    {
        builder.unavailable(
            "nukesIncreased",
            "Nuclear stockpile",
            "No complete Nukes read was recorded.",
            None,
        );
    }
    for (ordinal, count) in &metadata.nukes.refused_types {
        builder.unavailable(
            "nuclearType",
            "Nuclear type",
            format!("Nuclear type {ordinal} has no native content-name mapping."),
            Some(counted(*count)?),
        );
    }
    for (reason, spans) in &metadata.nukes.refusals {
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
    if !metadata.projects.available {
        builder.unavailable(
            "projectFinished",
            "Special projects",
            "Projects was unavailable, incomplete, or never read for any country.",
            None,
        );
    }
    if let Some(reason) = &metadata.projects.content_error {
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
    for (reason, count) in &metadata.ideas.refusals {
        builder.unavailable(
            "ideaObservation",
            "Laws and advisors",
            reason,
            Some(counted(*count)?),
        );
    }
    for (reason, count) in &metadata.focus.1 {
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
    if !metadata.subjects.0 {
        builder.unavailable(
            "becameSubject",
            "Subjects",
            "No complete Subjects read with resolved countries was recorded.",
            None,
        );
    }
    if !metadata.ideas.observed {
        builder.unavailable("lawChanged", "Laws and advisors", "ActiveIdeas and HiredAdvisors were not recorded complete together with resolved countries.", None);
    }
    if !metadata.focus.0 {
        builder.unavailable(
            "focusCompleted",
            "Focus completions",
            "Focus and CompletedFocuses were not recorded complete with resolved countries.",
            None,
        );
    }
    if metadata.ideas.gap_additions > 0 {
        builder.unavailable("ideaAdditionAfterGap", "Law and advisor changes", "Changed holdings after an unrecorded stretch cannot establish whether a law changed or an advisor was hired.", Some(counted(metadata.ideas.gap_additions)?));
    }
    for (token, count) in &metadata.ideas.unclassified {
        builder.unavailable("lawChanged", "Law classification", format!("Idea {token}: the recorded content cannot establish whether this addition is a law."), Some(*count));
    }
    Ok(())
}
