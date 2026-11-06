//! Shared recorded inputs, independent of a country's reply and content folders.
use super::*;
#[cfg(any(test, feature = "test-support"))]
use dashboard_text::metadata::validate_campaign;
use dashboard_text::metadata::{check_cancel, tagged_countries};
use dashboard_text::triggers::inputs::{self as reuse, ReuseCounts, Since, TableChanges};
#[cfg(any(test, feature = "test-support"))]
use replay::container::Replay;
use replay_data::rows::CountryRow;
#[cfg(any(test, feature = "test-support"))]
use replay_data::{rows::RowCodec, rows::TableRows, rows::TableSchema};
#[cfg(any(test, feature = "test-support"))]
use replay_frame::{frame::FrameDecoder, frame::Metrics, frame::SharedView, frame::TableView};
use std::collections::{BTreeSet, HashMap};

#[cfg(any(test, feature = "test-support"))]
const SELECTED: [TableId; 9] = [
    TableId::Campaign,
    TableId::Countries,
    TableId::CountryLinks,
    TableId::Projects,
    TableId::ProjectPrograms,
    TableId::ProjectHistory,
    TableId::Characters,
    TableId::CharacterPortraits,
    TableId::LexicalTokens,
];
#[cfg(any(test, feature = "test-support"))]
const TABLES: [TableId; 6] = [
    TableId::Projects,
    TableId::ProjectPrograms,
    TableId::ProjectHistory,
    TableId::Characters,
    TableId::CharacterPortraits,
    TableId::LexicalTokens,
];

#[derive(Default, PartialEq, Eq)]
pub(super) struct Actors {
    pub(super) known: bool,
    pub(super) unresolved: usize,
    pub(super) tags: BTreeMap<Vec<u8>, (u32, u32)>,
    pub(super) links: BTreeMap<u32, Vec<CountryLinkRow>>,
}
impl Actors {
    #[cfg(any(test, feature = "test-support"))]
    fn digest(&self) -> [u8; 32] {
        let mut hash = blake3::Hasher::new();
        hash.update(&[u8::from(self.known)]);
        hash.update(&(self.unresolved as u64).to_le_bytes());
        hash.update(&(self.tags.len() as u64).to_le_bytes());
        for (tag, &(id, slot)) in &self.tags {
            hash.update(&(tag.len() as u64).to_le_bytes());
            hash.update(tag);
            hash.update(&id.to_le_bytes());
            hash.update(&slot.to_le_bytes());
        }
        hash.update(&(self.links.len() as u64).to_le_bytes());
        for (&slot, links) in &self.links {
            hash.update(&slot.to_le_bytes());
            hash.update(&(links.len() as u64).to_le_bytes());
            for link in links {
                hash.update(&link.id.to_le_bytes());
            }
        }
        *hash.finalize().as_bytes()
    }
    #[cfg(any(test, feature = "test-support"))]
    pub(super) fn read(frame: &DecodedFrame) -> Self {
        let countries = frame
            .complete::<tables::Countries>()
            .ok()
            .map(dashboard_text::metadata::CountryKeys::read);
        let links = Self::link_operands(frame);
        Self::bind(countries.as_ref(), links.as_deref())
    }
    #[cfg(any(test, feature = "test-support"))]
    pub(super) fn link_operands(frame: &DecodedFrame) -> Option<Vec<(u32, u32)>> {
        Some(
            frame
                .complete::<tables::CountryLinks>()
                .ok()?
                .iter()
                .map(|link| (link.id, link.canonical_slot))
                .collect(),
        )
    }
    pub(super) fn bind(
        countries: Option<&dashboard_text::metadata::CountryKeys>,
        links: Option<&[(u32, u32)]>,
    ) -> Self {
        let mut actors = Self::default();
        if let Some((countries, links)) = countries.zip(links) {
            let countries: Vec<_> = countries
                .iter()
                .map(|(id, canonical_slot, tag)| CountryRow {
                    id,
                    canonical_slot,
                    tag: tag.to_vec(),
                    ..Default::default()
                })
                .collect();
            let links: Vec<_> = links
                .iter()
                .map(|&(id, canonical_slot)| CountryLinkRow { id, canonical_slot })
                .collect();
            actors.known = true;
            for link in &links {
                actors
                    .links
                    .entry(link.canonical_slot)
                    .or_default()
                    .push(link.clone());
            }
            for (tag, found) in tagged_countries(&countries, &links) {
                if let Some((country, slot)) = found.found() {
                    actors.tags.insert(tag.to_vec(), (country.id, slot));
                } else {
                    actors.unresolved += 1;
                }
            }
        }
        actors
    }
}

/// The recording oracle counts structural diagnostics once per input span.
#[cfg(any(test, feature = "test-support"))]
#[derive(Clone, Default)]
pub(super) struct SpanBoundary {
    hour: Option<i32>,
    actors: Option<[u8; 32]>,
    views: Option<[ViewIdentity; TABLES.len()]>,
}
#[cfg(any(test, feature = "test-support"))]
impl SpanBoundary {
    fn restore_views(&mut self, frame: &DecodedFrame) {
        self.views = Some(TABLES.map(|table| frame.tables[table.index()].identity()));
    }
    pub(super) fn observe(&mut self, frame: &DecodedFrame, actors: &Actors) -> bool {
        let actors = actors.digest();
        self.observe_digest(frame, actors)
    }
    fn observe_digest(&mut self, frame: &DecodedFrame, actors: [u8; 32]) -> bool {
        let same = self.hour.and_then(|hour| hour.checked_add(1)) == Some(frame.hour)
            && self.actors == Some(actors)
            && self.views.as_ref().is_some_and(|views| {
                views
                    .iter()
                    .zip(TABLES)
                    .all(|(view, table)| view.is(&frame.tables[table.index()]))
            });
        self.hour = Some(frame.hour);
        self.actors = Some(actors);
        self.restore_views(frame);
        !same
    }
}

/// The complete table's rows at `keys`, in key order; None when it is incomplete.
#[cfg(any(test, feature = "test-support"))]
fn rows_at<T: TableSchema>(
    frame: &DecodedFrame,
    keys: impl IntoIterator<Item = u32>,
) -> Option<Vec<T::Row>> {
    let rows = frame.complete::<T>().ok()?;
    let keys: BTreeSet<_> = keys.into_iter().map(u128::from).collect();
    Some(
        keys.iter()
            .filter_map(|key| rows.binary_search_by_key(key, |row| row.key()).ok())
            .map(|index| rows[index].clone())
            .collect(),
    )
}

/// The projected rows, or Unavailable: the projection reads neither the rows
/// nor the reason of an incomplete table.
#[cfg(any(test, feature = "test-support"))]
fn projected<T: TableSchema>(rows: Option<Vec<T::Row>>) -> SharedView {
    rows.map_or_else(
        || TableView::Unavailable {
            reason: "Not recorded completely".into(),
        },
        |rows| TableView::Complete(T::wrap(rows)),
    )
    .into()
}

#[cfg(any(test, feature = "test-support"))]
pub(super) struct Inputs {
    views: [SharedView; TABLES.len()],
    /// The recorded LexicalTokens read, which renews cached presentations.
    pub lexical: ViewIdentity,
    actors: Arc<Actors>,
}
#[cfg(any(test, feature = "test-support"))]
impl Inputs {
    fn new(frame: &DecodedFrame, actors: Arc<Actors>, previous: Option<&Self>) -> Self {
        // Builder::person reads each program a project references, that program's
        // scientist, the character's portrait holder and script token;
        // Builder::presentation each definition's token. Only those rows are kept,
        // so no span holds a recorded roster or dictionary.
        let projects = frame.complete::<tables::Projects>().unwrap_or_default();
        let programs = rows_at::<tables::ProjectPrograms>(
            frame,
            projects
                .iter()
                .filter(|project| {
                    project.program_reference as u32
                        == engine_projects::nproject_cprogram::REFERENCE_TYPE
                })
                .map(|project| (project.program_reference >> 32) as u32),
        );
        let scientists: Vec<_> = programs
            .iter()
            .flatten()
            .filter(|program| program.scientist as u32 == engine_native::ccharacter::REFERENCE_TYPE)
            .map(|program| (program.scientist >> 32) as u32)
            .collect();
        let characters = rows_at::<tables::Characters>(frame, scientists.iter().copied());
        let portraits = rows_at::<tables::CharacterPortraits>(frame, scientists);
        let definitions = projects.iter().map(|project| project.definition_id);
        let names = characters
            .iter()
            .flatten()
            .filter_map(|character| u32::try_from(character.name_token_id).ok());
        let lexical = rows_at::<tables::LexicalTokens>(frame, definitions.chain(names));
        // In TABLES order.
        let mut views = [
            frame.tables[TableId::Projects.index()].clone(),
            projected::<tables::ProjectPrograms>(programs),
            frame.tables[TableId::ProjectHistory.index()].clone(),
            projected::<tables::Characters>(characters),
            projected::<tables::CharacterPortraits>(portraits),
            projected::<tables::LexicalTokens>(lexical),
        ];
        // Equal rows keep the previous span's allocation, not another copy.
        if let Some(previous) = previous {
            for (view, seen) in views.iter_mut().zip(&previous.views) {
                if *view == *seen {
                    *view = seen.clone();
                }
            }
        }
        Self {
            views,
            lexical: frame.tables[TableId::LexicalTokens.index()].identity(),
            actors,
        }
    }
    pub(super) fn unresolved_actors(&self) -> usize {
        self.actors.unresolved
    }
    fn base_frame(&self, branch: u64) -> DecodedFrame {
        let unselected = SharedView::from(TableView::Unavailable {
            reason: "Not selected for project projection".into(),
        });
        let mut frame = DecodedFrame {
            branch,
            hour: 0,
            tables: std::array::from_fn(|_| unselected.clone()),
            metrics: Metrics::default(),
        };
        for (table, view) in TABLES.iter().zip(&self.views) {
            frame.tables[table.index()] = view.clone();
        }
        frame
    }
    pub(super) fn world_frame(&self, branch: u64) -> DecodedFrame {
        let mut frame = self.base_frame(branch);
        if self.actors.known {
            // Completion admission reads native identity only, as actor_frame does.
            let mut countries: Vec<_> = self
                .actors
                .tags
                .iter()
                .map(|(tag, &(id, slot))| CountryRow {
                    id,
                    canonical_slot: slot,
                    tag: tag.clone(),
                    ..Default::default()
                })
                .collect();
            countries.sort_by_key(|country| country.id);
            let mut links: Vec<_> = self.actors.links.values().flatten().cloned().collect();
            links.sort_by_key(|link| link.id);
            frame.tables[TableId::Countries.index()] =
                TableView::Complete(TableRows::Countries(countries)).into();
            frame.tables[TableId::CountryLinks.index()] =
                TableView::Complete(TableRows::CountryLinks(links)).into();
        }
        frame
    }
    pub(super) fn actor_frame(&self, branch: u64, tag: &str) -> Option<DecodedFrame> {
        let &(id, slot) = self.actors.tags.get(tag.as_bytes())?;
        // The builder reads only native identity from Countries. Scoped text
        // uses the separate full World pass, never these projected defaults.
        let country = CountryRow {
            id,
            canonical_slot: slot,
            tag: tag.as_bytes().to_vec(),
            ..Default::default()
        };
        let mut frame = self.base_frame(branch);
        frame.tables[TableId::Countries.index()] =
            TableView::Complete(TableRows::Countries(vec![country])).into();
        frame.tables[TableId::CountryLinks.index()] = TableView::Complete(TableRows::CountryLinks(
            self.actors.links.get(&slot)?.clone(),
        ))
        .into();
        Some(frame)
    }
}

#[cfg(any(test, feature = "test-support"))]
#[derive(Clone)]
pub(super) struct Span {
    pub first_hour: i32,
    pub last_hour: i32,
    pub inputs: Arc<Inputs>,
    pub reads: Vec<i32>,
}
#[cfg(any(test, feature = "test-support"))]
#[derive(Clone, Default)]
pub(super) struct BranchScan {
    /// The recording's format, which dates its daily tables
    /// ([`dashboard_text::metadata::slot_read`]).
    pub(crate) format: replay_data::format::FormatVersion,
    pub campaign_start: Option<i32>,
    pub spans: Vec<Span>,
    blocks: usize,
    actors: Option<Arc<Actors>>,
    actor_views: Option<[ViewIdentity; 2]>,
    boundary: SpanBoundary,
}
#[cfg(any(test, feature = "test-support"))]
impl BranchScan {
    #[cfg(test)]
    pub(super) fn scan(replay: &Replay, branch: u64, cancelled: &AtomicBool) -> io::Result<Self> {
        let mut scan = Self::default();
        scan.extend(replay, branch, cancelled)?;
        Ok(scan)
    }
    fn observe(&mut self, frame: &DecodedFrame) {
        let actor_tables = [TableId::Countries, TableId::CountryLinks];
        if self.actor_views.as_ref().is_none_or(|views| {
            actor_tables
                .iter()
                .zip(views)
                .any(|(table, view)| !view.is(&frame.tables[table.index()]))
        }) {
            let actors = Actors::read(frame);
            if self.actors.as_ref().is_none_or(|old| **old != actors) {
                self.actors = Some(Arc::new(actors));
            }
            self.actor_views =
                Some(actor_tables.map(|table| frame.tables[table.index()].identity()));
        }
        let actors = Arc::clone(self.actors.as_ref().expect("actor index observed"));
        let read = slot_read(self.format, &frame.metrics, TableId::Projects, frame.hour);
        let changed = self.boundary.observe(frame, &actors);
        if let Some(span) = self.spans.last_mut()
            && !changed
        {
            span.last_hour = frame.hour;
            if read {
                span.reads.push(frame.hour);
            }
            return;
        }
        let previous = self.spans.last().map(|span| &*span.inputs);
        let inputs = Arc::new(Inputs::new(frame, actors, previous));
        self.spans.push(Span {
            first_hour: frame.hour,
            last_hour: frame.hour,
            inputs,
            reads: if read { vec![frame.hour] } else { Vec::new() },
        });
    }
    fn extend(
        &mut self,
        replay: &Replay,
        branch: u64,
        cancelled: &AtomicBool,
    ) -> io::Result<usize> {
        self.format = replay.format;
        self.campaign_start = replay
            .index
            .iter()
            .filter(|block| block.branch == branch)
            .map(|block| block.first_hour)
            .min();
        let mut decoded_blocks = 0;
        dashboard_core::blocks::scan_blocks(
            replay,
            replay
                .index
                .iter()
                .skip(self.blocks)
                .filter(|block| block.branch == branch),
            &SELECTED,
            cancelled,
            |blocks| {
                for (_, frames) in blocks {
                    check_cancel(cancelled)?;
                    decoded_blocks += 1;
                    let mut decoder = FrameDecoder::for_tables(&SELECTED);
                    let Ok(frames) = frames else {
                        continue;
                    };
                    for frame in frames {
                        check_cancel(cancelled)?;
                        let Ok(frame) = decoder.advance(&frame) else {
                            break;
                        };
                        if validate_campaign(frame, replay).is_err() {
                            break;
                        }
                        self.observe(frame);
                    }
                }
                Ok(())
            },
        )?;
        self.blocks = replay.index.len();
        Ok(decoded_blocks)
    }
}

#[cfg(any(test, feature = "test-support"))]
#[derive(Clone, Default)]
pub(super) struct Scans {
    branches: Vec<(u64, Arc<BranchScan>)>,
    #[cfg(any(test, feature = "test-support"))]
    pub decoded_blocks: usize,
}
#[cfg(any(test, feature = "test-support"))]
pub(super) fn cached(
    source: &Source,
    branch: u64,
    cancelled: &AtomicBool,
) -> io::Result<Arc<BranchScan>> {
    check_cancel(cancelled)?;
    // Verified appends share immutable inputs; only new blocks are decoded in
    // the new revision. A rewritten prefix inherits no state (Source contract).
    let state = source.append_state::<Scans>();
    let mut state = state.lock().unwrap_or_else(|error| error.into_inner());
    let index = state.branches.iter().position(|(id, _)| *id == branch);
    if let Some(index) = index
        && state.branches[index].1.blocks == source.replay_ref().index.len()
    {
        return Ok(Arc::clone(&state.branches[index].1));
    }
    let mut scan = index
        .map(|index| (*state.branches[index].1).clone())
        .unwrap_or_default();
    let decoded = scan.extend(source.replay_ref(), branch, cancelled)?;
    #[cfg(any(test, feature = "test-support"))]
    {
        state.decoded_blocks += decoded;
    }
    #[cfg(not(test))]
    let _ = decoded;
    let scan = Arc::new(scan);
    if let Some(index) = index {
        state.branches.remove(index);
    }
    state.branches.push((branch, Arc::clone(&scan)));
    // Branch retention only. Every requested actor remains projectable.
    if state.branches.len() > 4 {
        state.branches.remove(0);
    }
    Ok(scan)
}

/// A scoped project name as one hour evaluated it: its text, and what it refused.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[derive(replay_spool::Spooled)]
pub(super) struct Name {
    pub(super) text: Option<String>,
    pub(super) warnings: Vec<String>,
}

/// A scoped name's owner tag, canonical slot and definition.
pub(super) type NameKey = (String, u32, u32);
struct NameRun {
    first: i32,
    last: i32,
    value: Name,
}
#[derive(Default, Serialize, Deserialize)]
pub(super) struct Names {
    #[serde(skip)]
    runs: BTreeMap<(String, u32, u32), Vec<NameRun>>,
    counts: BTreeMap<String, BTreeMap<String, usize>>,
    /// The last hour each name's refusals were counted.
    #[serde(skip)]
    counted: BTreeMap<NameKey, i32>,
    #[cfg(any(test, feature = "test-support"))]
    pub decoded_blocks: usize,
}
impl Names {
    pub(super) fn at(&self, tag: &str, slot: u32, definition: u32, hour: i32) -> Option<String> {
        let runs = self.runs.get(&(tag.to_owned(), slot, definition))?;
        let index = runs
            .partition_point(|run| run.first <= hour)
            .checked_sub(1)?;
        (hour <= runs[index].last)
            .then(|| runs[index].value.text.clone())
            .flatten()
    }
    #[cfg(any(test, feature = "test-support"))]
    pub(super) fn refusals_at(
        &self,
        tag: &str,
        slot: u32,
        definition: u32,
        hour: i32,
    ) -> &[String] {
        self.runs
            .get(&(tag.to_owned(), slot, definition))
            .and_then(|runs| {
                runs.iter()
                    .rfind(|run| run.first <= hour && hour <= run.last)
            })
            .map_or(&[], |run| run.value.warnings.as_slice())
    }
    #[cfg(any(test, feature = "test-support"))]
    pub(super) fn warnings(&self, tag: &str) -> Vec<String> {
        self.counts
            .get(tag)
            .into_iter()
            .flatten()
            .map(|(reason, count)| format!("{reason} ({count} project-name observations)."))
            .collect()
    }
    /// The name observations with refusals counted since last taken, by owner and reason.
    pub(super) fn take_counts(&mut self) -> BTreeMap<String, BTreeMap<String, usize>> {
        std::mem::take(&mut self.counts)
    }

    /// What these names, an hour's, changed of `sent`, the names last sent: each key's new name,
    /// none where it names nothing now, or only a project of `except`'s (provisional names,
    /// sent apart). `sent` then holds them.
    pub(super) fn changes(
        &self,
        sent: &mut HashMap<NameKey, Name>,
        except: &BTreeSet<NameKey>,
    ) -> Vec<(NameKey, Option<Name>)> {
        let mut changes = Vec::new();
        let mut held = 0;
        for (key, runs) in &self.runs {
            let Some(run) = runs.last().filter(|_| !except.contains(key)) else {
                continue;
            };
            held += 1;
            if sent.get(key) != Some(&run.value) {
                sent.insert(key.clone(), run.value.clone());
                changes.push((key.clone(), Some(run.value.clone())));
            }
        }
        if sent.len() > held {
            let gone: Vec<NameKey> = sent
                .keys()
                .filter(|key| !self.runs.contains_key(*key) || except.contains(*key))
                .cloned()
                .collect();
            for key in gone {
                sent.remove(&key);
                changes.push((key, None));
            }
        }
        changes
    }
    /// Whether `key`'s refusals were counted at `hour`.
    fn counted_at(&self, key: &NameKey, hour: i32) -> bool {
        self.counted.get(key) == Some(&hour)
    }

    /// `key`'s name at `hour`, its refusals counted once an hour where `counted`.
    fn observe(&mut self, key: (String, u32, u32), hour: i32, value: Name, counted: bool) {
        if counted && !self.counted_at(&key, hour) {
            self.counted.insert(key.clone(), hour);
            for reason in &value.warnings {
                *self
                    .counts
                    .entry(key.0.clone())
                    .or_default()
                    .entry(reason.clone())
                    .or_default() += 1;
            }
        }
        let runs = self.runs.entry(key).or_default();
        // Several projects may use the same definition and owner at this hour.
        if runs.last().is_some_and(|last| last.last == hour) {
            return;
        }
        if let Some(last) = runs.last_mut()
            && last.last.checked_add(1) == Some(hour)
            && last.value == value
        {
            last.last = hour;
        } else {
            runs.push(NameRun {
                first: hour,
                last: hour,
                value,
            });
        }
    }
}

#[derive(Default, Serialize, Deserialize)]
struct Actor {
    last: Option<((u32, u32), i32)>,
    identity: Option<(u32, u32)>,
    admitted: bool,
    open: BTreeSet<(u32, u32)>,
    /// Its projects not visible of themselves that may be open episodes the scan did not see
    /// begin: named, their refusals counted apart ([`NameScan::uncertain`]).
    #[serde(default)]
    provisional: BTreeSet<(u32, u32)>,
    /// Whether its first projects read are to be taken as possibly open.
    #[serde(default)]
    seeding: bool,
}

/// A scoped name's last evaluation: the key it localised, the name it gave
/// and every input it read ([`dashboard_text::triggers::inputs`]).
struct Evaluation {
    key: String,
    name: Name,
    inputs: reuse::Inputs,
}

#[derive(Default, Serialize, Deserialize)]
pub(super) struct NameScan {
    #[serde(with = "dashboard_core::map_pairs")]
    actors: BTreeMap<Vec<u8>, Actor>,
    #[serde(skip)]
    keys: BTreeMap<(u32, String), Option<String>>,
    #[serde(skip)]
    lexical_view: Option<ViewIdentity>,
    #[serde(skip)]
    active: BTreeSet<(u32, String)>,
    pub(super) names: Names,
    /// Each owner, slot and definition's last evaluation: an hour at which
    /// every input it read reads the same gives the same name, without
    /// evaluating it ([`reused_name`]).
    #[serde(skip)]
    evaluations: BTreeMap<(String, u32, u32), Evaluation>,
    /// The scoped names evaluated, reused and (in check mode) evaluated
    /// anyway.
    #[serde(skip)]
    pub(super) reuse: ReuseCounts,
    /// Each owner, slot and definition's name at the last hour it was
    /// observed, which tells a name that changed at the next.
    #[serde(skip)]
    previous: BTreeMap<(String, u32, u32), (i32, Name)>,
    /// The scan starts after hours it did not see: a project an episode may hold open
    /// is named, whether it is open known only to whoever saw those hours.
    #[serde(skip)]
    pub(super) uncertain: bool,
    /// Each possibly open project named since last taken, with its name.
    #[serde(skip)]
    provisional: Vec<(NameKey, (u32, u32), Name)>,
}
/// Whether the name of `at` observed at `hour` is not the one `previous`
/// holds from the hour before (or it holds none), remembering it for the
/// next.
fn changed(
    previous: &mut BTreeMap<(String, u32, u32), (i32, Name)>,
    at: &(String, u32, u32),
    hour: i32,
    value: &Name,
) -> bool {
    let changed = previous
        .get(at)
        .is_none_or(|(last, name)| last.checked_add(1) != Some(hour) || name != value);
    previous.insert(at.clone(), (hour, value.clone()));
    changed
}

/// How a name scan reuses its scoped names' last evaluations.
#[derive(Clone, Copy)]
pub(super) enum Reuse<'a> {
    /// Evaluate every name.
    #[cfg_attr(
        not(any(test, feature = "test-support")),
        expect(dead_code, reason = "the recording oracle evaluates every name")
    )]
    Every,
    /// Reuse a name every input of whose last evaluation reads the same at
    /// the frame's hour, given these changes since the hour before (an
    /// input whose declared reads did not change reads the same without
    /// reading it); evaluate any other.
    Confirm(&'a dyn TableChanges),
}

impl NameScan {
    /// Observe the frame's projects' scoped names, reusing them as `reuse`
    /// says. The countries any of whose names changed since the hour before:
    /// one not named then, another name or refusal, or a name it no longer has.
    pub(super) fn observe(
        &mut self,
        frame: &DecodedFrame,
        input: &Actors,
        read: bool,
        content: &Content,
        reuse: Reuse<'_>,
        cancelled: &AtomicBool,
    ) -> io::Result<BTreeSet<String>> {
        check_cancel(cancelled)?;
        let before = std::mem::take(&mut self.active);
        let mut countries = BTreeSet::new();
        let Ok(rows) = frame.complete::<tables::Countries>() else {
            countries.extend(before.into_iter().map(|(_, tag)| tag));
            return Ok(countries);
        };
        let projects = frame.complete::<tables::Projects>().ok();
        let lexical = &frame.tables[TableId::LexicalTokens.index()];
        if self
            .lexical_view
            .as_ref()
            .is_none_or(|seen| !seen.is(lexical))
        {
            self.keys.clear();
            self.lexical_view = Some(lexical.identity());
        }
        for (tag, &(id, slot)) in &input.tags {
            let Ok(index) = rows.binary_search_by_key(&id, |country| country.id) else {
                continue;
            };
            let country = &rows[index];
            let identity = (slot, country.id);
            let uncertain = self.uncertain;
            let actor = self.actors.entry(tag.clone()).or_insert_with(|| Actor {
                seeding: uncertain,
                ..Actor::default()
            });
            if actor.last.is_none_or(|(old, hour)| {
                old != identity || hour.checked_add(1) != Some(frame.hour)
            }) {
                actor.admitted = false;
            }
            if actor.identity.is_some_and(|old| old != identity) {
                actor.open.clear();
                actor.provisional.clear();
                actor.seeding = false;
            }
            actor.identity = Some(identity);
            actor.last = Some((identity, frame.hour));
            let Some(projects) = projects else {
                actor.admitted = false;
                continue;
            };
            if read {
                actor.admitted = true;
            }
            if !actor.admitted {
                continue;
            }
            let Some(links) = input.links.get(&slot) else {
                continue;
            };
            let seeding = std::mem::take(&mut actor.seeding);
            let mut next = BTreeSet::new();
            let mut possible = BTreeSet::new();
            for project in owned_rows(projects, links, slot, |row| row.owner) {
                let key = (project.local_id, project.definition_id);
                let open = actor.open.contains(&key);
                let provisional = !open
                    && !visible(project, false)
                    && (seeding || actor.provisional.contains(&key));
                if provisional {
                    possible.insert(key);
                } else if visible(project, open) {
                    next.insert(key);
                } else {
                    continue;
                }
                let Ok(tag) = std::str::from_utf8(tag) else {
                    continue;
                };
                // Resolve each definition/country key once per lexical view;
                // only reached scoped names evaluate the complete World.
                let key = self
                    .keys
                    .entry((project.definition_id, tag.to_owned()))
                    .or_insert_with(|| scope_key(frame, content, project.definition_id, tag));
                let Some(key) = key else {
                    continue;
                };
                self.active.insert((project.definition_id, tag.to_owned()));
                let value = reused_name(
                    &mut self.evaluations,
                    &mut self.reuse,
                    (tag, slot, project.definition_id),
                    key,
                    frame,
                    content,
                    reuse,
                );
                let at = (tag.to_owned(), slot, project.definition_id);
                // The hour's first name of a key holds, as its runs keep it.
                if self
                    .previous
                    .get(&at)
                    .is_none_or(|(hour, _)| *hour != frame.hour)
                    && changed(&mut self.previous, &at, frame.hour, &value)
                {
                    countries.insert(tag.to_owned());
                }
                if provisional {
                    self.provisional
                        .push((at.clone(), (project.local_id, project.definition_id), value.clone()));
                }
                self.names.observe(at, frame.hour, value, !provisional);
            }
            actor.open = next;
            actor.provisional = possible;
        }
        // A name a project known open has is no provisional one.
        let names = &self.names;
        self.provisional
            .retain(|(at, _, _)| !names.counted_at(at, frame.hour));
        countries.extend(
            before
                .difference(&self.active)
                .map(|(_, tag)| tag.clone()),
        );
        Ok(countries)
    }
    /// Each possibly open project named since last taken, whose name no project known open
    /// has: its name's key, the project, and its name.
    pub(super) fn take_provisional(&mut self) -> Vec<(NameKey, (u32, u32), Name)> {
        std::mem::take(&mut self.provisional)
    }

    pub(super) fn take_current(&mut self) -> Names {
        Names {
            runs: std::mem::take(&mut self.names.runs),
            ..Default::default()
        }
    }
}
fn scope_key(
    frame: &DecodedFrame,
    content: &Content,
    definition: u32,
    tag: &str,
) -> Option<String> {
    let tokens = frame.complete::<tables::LexicalTokens>().ok()?;
    let token = tokens
        .binary_search_by_key(&definition, |row| row.token)
        .ok()
        .and_then(|index| std::str::from_utf8(&tokens[index].literal).ok())?;
    let definition = content.definitions.projects.get(token)?;
    if content.definitions.unsettled.is_some() || definition.refusal.is_some() {
        return None;
    }
    let key = country_key(&content.catalogue, tag, project_name_key(definition, token));
    needs_scope(&content.catalogue.localisation, &key).then_some(key)
}
fn name_at(frame: &DecodedFrame, slot: u32, key: &str, content: &Content) -> Name {
    evaluated_name(frame, slot, key, content).0
}

/// [`name_at`], and when it expanded scripted text, every input the
/// expansion read. The hour's text context is built outside the recorded
/// evaluation: what it holds is read only through recorded queries.
fn evaluated_name(
    frame: &DecodedFrame,
    slot: u32,
    key: &str,
    content: &Content,
) -> (Name, Option<reuse::Inputs>) {
    match content.catalogue.scripted_content() {
        Ok(scripted) => {
            let context = dashboard_text::scripted_text::ScriptedText::new(
                frame,
                &content.catalogue.localisation,
                scripted,
                slot,
            );
            let ((text, mut findings), inputs) = reuse::capture(frame, || {
                catalogue::localised_in_details(&content.catalogue.localisation, key, &[], &context)
            });
            let text = text.map_err(|finding| findings.push(*finding)).ok();
            let name = Name {
                text,
                warnings: findings
                    .into_iter()
                    .map(|finding| format!("{}: {}", finding.subject, finding.message))
                    .collect(),
            };
            (name, Some(inputs))
        }
        Err(reason) => (
            Name {
                text: None,
                warnings: vec![format!("Project name {key}: {reason}")],
            },
            None,
        ),
    }
}

/// The scoped name of `key` for `at` (owner, slot, definition) at the
/// frame's hour: with `changes` since the hour before, the last
/// evaluation's name while every input it read reads the same, else
/// evaluated. An input whose declared dependence did not change reads the
/// same without the hour's text context; the context is built only to read
/// an input again. The name is a function of what its evaluation read, and
/// a reused one notes nothing new: the content's refusals and readers are
/// sets, which its first evaluation noted. Check mode evaluates a reused
/// name anyway, and a change fails a test.
fn reused_name(
    evaluations: &mut BTreeMap<(String, u32, u32), Evaluation>,
    counts: &mut ReuseCounts,
    (owner, slot, definition): (&str, u32, u32),
    key: &str,
    frame: &DecodedFrame,
    content: &Content,
    reuse: Reuse<'_>,
) -> Name {
    let changes = match reuse {
        Reuse::Every => return name_at(frame, slot, key, content),
        Reuse::Confirm(changes) => changes,
    };
    let at = (owner.to_owned(), slot, definition);
    let since = Since::hour_before(frame, changes);
    let confirmed = match (
        evaluations.get_mut(&at),
        content.catalogue.scripted_content(),
    ) {
        (Some(last), Ok(scripted)) if last.key == key => {
            let text = std::cell::OnceCell::new();
            last.inputs.confirm_since(frame, since.as_ref(), |read| {
                text.get_or_init(|| {
                    dashboard_text::scripted_text::ScriptedText::new(
                        frame,
                        &content.catalogue.localisation,
                        scripted,
                        slot,
                    )
                })
                .reread(read)
            })
        }
        _ => false,
    };
    if confirmed {
        counts.reused += 1;
        if !reuse::check_reuse() {
            return evaluations[&at].name.clone();
        }
    }
    let (name, inputs) = evaluated_name(frame, slot, key, content);
    if confirmed {
        // Check mode: the reused name, evaluated anyway, must not change.
        counts.checked += 1;
        if evaluations[&at].name != name {
            counts.mismatched += 1;
            #[cfg(test)]
            panic!(
                "{owner}'s project {definition} name {key}: a reused evaluation changed at hour {}",
                frame.hour
            );
        }
    } else {
        counts.evaluated += 1;
    }
    match inputs {
        Some(inputs) => {
            evaluations.insert(
                at,
                Evaluation {
                    key: key.to_owned(),
                    name: name.clone(),
                    inputs,
                },
            );
        }
        None => {
            evaluations.remove(&at);
        }
    }
    name
}

#[cfg(any(test, feature = "test-support"))]
pub(super) fn scoped_names(
    replay: &Replay,
    scan: &BranchScan,
    branch: u64,
    content: &Content,
    cancelled: &AtomicBool,
) -> io::Result<Names> {
    let mut observer = NameScan::default();
    let mut at = 0;
    // Scripted names and their triggers read the trigger kit.
    let mut selected = SELECTED.to_vec();
    selected.extend_from_slice(dashboard_text::triggers::RECORDED_TABLES);
    selected.sort_unstable();
    selected.dedup();
    dashboard_core::blocks::scan_blocks(
        replay,
        replay.index.iter().filter(|block| block.branch == branch),
        &selected,
        cancelled,
        |blocks| {
            for (_, frames) in blocks {
                check_cancel(cancelled)?;
                #[cfg(any(test, feature = "test-support"))]
                {
                    observer.names.decoded_blocks += 1;
                }
                let mut decoder = FrameDecoder::for_tables(&selected);
                let Ok(frames) = frames else {
                    continue;
                };
                for frame in frames {
                    check_cancel(cancelled)?;
                    let Ok(frame) = decoder.advance(&frame) else {
                        break;
                    };
                    if validate_campaign(frame, replay).is_err() {
                        break;
                    }
                    while at < scan.spans.len() && scan.spans[at].last_hour < frame.hour {
                        at += 1;
                    }
                    let Some(span) = scan
                        .spans
                        .get(at)
                        .filter(|span| span.first_hour <= frame.hour)
                    else {
                        continue;
                    };
                    observer.observe(
                        frame,
                        &span.inputs.actors,
                        span.reads.binary_search(&frame.hour).is_ok(),
                        content,
                        Reuse::Every,
                        cancelled,
                    )?;
                }
            }
            Ok(())
        },
    )?;
    Ok(observer.names)
}
