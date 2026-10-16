//! Shared recorded inputs, independent of a country's reply and content folders.
use super::*;
#[cfg(test)]
use crate::metadata::validate_campaign;
use crate::metadata::{check_cancel, tagged_countries};
use replay::rows::CountryRow;
#[cfg(test)]
use replay::{
    container::Replay,
    frame::{FrameDecoder, Metrics, SharedView, TableView},
    rows::{RowCodec, TableRows, TableSchema},
};
use std::collections::BTreeSet;

#[cfg(test)]
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
    pub(super) fn read(frame: &DecodedFrame) -> Self {
        let countries = frame
            .complete::<tables::Countries>()
            .ok()
            .map(crate::metadata::CountryKeys::read);
        let links = Self::link_operands(frame);
        Self::bind(countries.as_ref(), links.as_deref())
    }
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
        countries: Option<&crate::metadata::CountryKeys>,
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

/// The recording oracle and preparation count structural diagnostics once per
/// input span. Resume retains only an actor digest and rebinds decoder views.
#[derive(Clone, Default, Serialize, Deserialize)]
pub(super) struct SpanBoundary {
    hour: Option<i32>,
    actors: Option<[u8; 32]>,
    #[serde(skip)]
    views: Option<[ViewIdentity; TABLES.len()]>,
}
impl SpanBoundary {
    pub(super) fn restore_views(&mut self, frame: &DecodedFrame) {
        self.views = Some(TABLES.map(|table| frame.tables[table.index()].identity()));
    }
    #[cfg(test)]
    pub(super) fn observe(&mut self, frame: &DecodedFrame, actors: &Actors) -> bool {
        let actors = actors.digest();
        self.observe_digest(frame, actors)
    }
    pub(super) fn observe_changed(
        &mut self,
        frame: &DecodedFrame,
        actors: &Actors,
        identities_changed: bool,
    ) -> bool {
        let digest = if identities_changed {
            actors.digest()
        } else {
            self.actors.unwrap_or_else(|| actors.digest())
        };
        self.observe_digest(frame, digest)
    }
    pub(super) fn advance(&mut self, hour: i32) {
        self.hour = Some(hour);
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
#[cfg(test)]
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
#[cfg(test)]
fn projected<T: TableSchema>(rows: Option<Vec<T::Row>>) -> SharedView {
    rows.map_or_else(
        || TableView::Unavailable {
            reason: "Not recorded completely".into(),
        },
        |rows| TableView::Complete(T::wrap(rows)),
    )
    .into()
}

#[cfg(test)]
pub(super) struct Inputs {
    views: [SharedView; TABLES.len()],
    /// The recorded LexicalTokens read, which renews cached presentations.
    pub lexical: ViewIdentity,
    actors: Arc<Actors>,
}
#[cfg(test)]
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
                    project.program_reference as u32 == engine::nproject_cprogram::REFERENCE_TYPE
                })
                .map(|project| (project.program_reference >> 32) as u32),
        );
        let scientists: Vec<_> = programs
            .iter()
            .flatten()
            .filter(|program| program.scientist as u32 == engine::ccharacter::REFERENCE_TYPE)
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

#[cfg(test)]
#[derive(Clone)]
pub(super) struct Span {
    pub first_hour: i32,
    pub last_hour: i32,
    pub inputs: Arc<Inputs>,
    pub reads: Vec<i32>,
}
#[cfg(test)]
#[derive(Clone, Default)]
pub(super) struct BranchScan {
    /// The recording's format, which dates its daily tables
    /// ([`crate::metadata::slot_read`]).
    pub(crate) format: replay::format::FormatVersion,
    pub campaign_start: Option<i32>,
    pub spans: Vec<Span>,
    blocks: usize,
    actors: Option<Arc<Actors>>,
    actor_views: Option<[ViewIdentity; 2]>,
    boundary: SpanBoundary,
}
#[cfg(test)]
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
        crate::blocks::scan_blocks(
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

#[cfg(test)]
#[derive(Clone, Default)]
pub(super) struct Scans {
    branches: Vec<(u64, Arc<BranchScan>)>,
    #[cfg(test)]
    pub decoded_blocks: usize,
}
#[cfg(test)]
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
    #[cfg(test)]
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

#[derive(Clone, PartialEq, Eq)]
struct Name {
    text: Option<String>,
    warnings: Vec<String>,
}
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
    #[cfg(test)]
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
    pub(super) fn warnings(&self, tag: &str) -> Vec<String> {
        self.counts
            .get(tag)
            .into_iter()
            .flatten()
            .map(|(reason, count)| format!("{reason} ({count} project-name observations)."))
            .collect()
    }
    fn observe(&mut self, key: (String, u32, u32), hour: i32, value: Name) {
        let owner = key.0.clone();
        let runs = self.runs.entry(key).or_default();
        // Several projects may use the same definition and owner at this hour.
        if runs.last().is_some_and(|last| last.last == hour) {
            return;
        }
        for reason in &value.warnings {
            *self
                .counts
                .entry(owner.clone())
                .or_default()
                .entry(reason.clone())
                .or_default() += 1;
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
}

#[derive(Default, Serialize, Deserialize)]
pub(super) struct NameScan {
    #[serde(with = "crate::prepared_production::map_pairs")]
    actors: BTreeMap<Vec<u8>, Actor>,
    #[serde(skip)]
    keys: BTreeMap<(u32, String), Option<String>>,
    #[serde(skip)]
    lexical_view: Option<ViewIdentity>,
    #[serde(skip)]
    active: BTreeSet<(u32, String)>,
    pub(super) names: Names,
}
impl NameScan {
    pub(super) fn observe(
        &mut self,
        frame: &DecodedFrame,
        input: &Actors,
        read: bool,
        content: &Content,
        cancelled: &AtomicBool,
    ) -> io::Result<()> {
        check_cancel(cancelled)?;
        self.active.clear();
        let Ok(countries) = frame.complete::<tables::Countries>() else {
            return Ok(());
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
            let Ok(index) = countries.binary_search_by_key(&id, |country| country.id) else {
                continue;
            };
            let country = &countries[index];
            let identity = (slot, country.id);
            let actor = self.actors.entry(tag.clone()).or_default();
            if actor.last.is_none_or(|(old, hour)| {
                old != identity || hour.checked_add(1) != Some(frame.hour)
            }) {
                actor.admitted = false;
            }
            if actor.identity.is_some_and(|old| old != identity) {
                actor.open.clear();
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
            let mut next = BTreeSet::new();
            for project in owned_rows(projects, links, slot, |row| row.owner) {
                let key = (project.local_id, project.definition_id);
                if !visible(project, actor.open.contains(&key)) {
                    continue;
                }
                next.insert(key);
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
                let value = name_at(frame, slot, key, content);
                self.names.observe(
                    (tag.to_owned(), slot, project.definition_id),
                    frame.hour,
                    value,
                );
            }
            actor.open = next;
        }
        Ok(())
    }
    pub(super) fn take_current(&mut self) -> Names {
        Names {
            runs: std::mem::take(&mut self.names.runs),
            ..Default::default()
        }
    }
    pub(super) fn rebind(
        &mut self,
        frame: &DecodedFrame,
        tag: &str,
        definition: u32,
        content: &Content,
    ) {
        let key = (definition, tag.to_owned());
        let value = scope_key(frame, content, definition, tag);
        if value.is_some() {
            self.active.insert(key.clone());
        } else {
            self.active.remove(&key);
        }
        self.keys.insert(key, value);
    }
    pub(super) fn active_countries(&self) -> impl Iterator<Item = String> + '_ {
        self.active.iter().map(|(_, tag)| tag.clone())
    }
    pub(super) fn evaluate_current(
        &mut self,
        frame: &DecodedFrame,
        actors: &Actors,
        content: &Content,
        cancelled: &AtomicBool,
    ) -> io::Result<BTreeSet<String>> {
        let mut countries = BTreeSet::new();
        for (definition, tag) in &self.active {
            check_cancel(cancelled)?;
            let Some((_, slot)) = actors.tags.get(tag.as_bytes()) else {
                continue;
            };
            let Some(Some(key)) = self.keys.get(&(*definition, tag.clone())) else {
                continue;
            };
            let value = name_at(frame, *slot, key, content);
            self.names
                .observe((tag.clone(), *slot, *definition), frame.hour, value);
            countries.insert(tag.clone());
        }
        Ok(countries)
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
    match content.catalogue.scripted_content() {
        Ok(scripted) => {
            let context = crate::scripted_text::ScriptedText::new(
                frame,
                &content.catalogue.localisation,
                scripted,
                slot,
            );
            let (text, mut findings) = catalogue::localised_in_details(
                &content.catalogue.localisation,
                key,
                &[],
                &context,
            );
            let text = text.map_err(|finding| findings.push(*finding)).ok();
            Name {
                text,
                warnings: findings
                    .into_iter()
                    .map(|finding| format!("{}: {}", finding.subject, finding.message))
                    .collect(),
            }
        }
        Err(reason) => Name {
            text: None,
            warnings: vec![format!("Project name {key}: {reason}")],
        },
    }
}

#[cfg(test)]
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
    selected.extend_from_slice(crate::triggers::RECORDED_TABLES);
    selected.sort_unstable();
    selected.dedup();
    crate::blocks::scan_blocks(
        replay,
        replay.index.iter().filter(|block| block.branch == branch),
        &selected,
        cancelled,
        |blocks| {
            for (_, frames) in blocks {
                check_cancel(cancelled)?;
                #[cfg(test)]
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
                        cancelled,
                    )?;
                }
            }
            Ok(())
        },
    )?;
    Ok(observer.names)
}
