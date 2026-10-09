//! Shared recorded inputs, independent of a country's reply and content folders.
use super::*;
use crate::metadata::{check_cancel, tagged_countries, validate_campaign};
use replay::{
    container::Replay,
    frame::{FrameDecoder, Metrics, SharedView, TableView},
    rows::{CountryRow, TableRows},
};
use std::collections::BTreeSet;

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
struct Actors {
    known: bool,
    unresolved: usize,
    tags: BTreeMap<Vec<u8>, (u32, u32)>,
    links: BTreeMap<u32, Vec<CountryLinkRow>>,
}
impl Actors {
    fn read(frame: &DecodedFrame) -> Self {
        let mut actors = Self::default();
        if let (Ok(countries), Ok(links)) = (
            frame.complete::<tables::Countries>(),
            frame.complete::<tables::CountryLinks>(),
        ) {
            actors.known = true;
            for link in links {
                actors
                    .links
                    .entry(link.canonical_slot)
                    .or_default()
                    .push(link.clone());
            }
            for (tag, found) in tagged_countries(countries, links) {
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

pub(super) struct Inputs {
    views: [SharedView; TABLES.len()],
    actors: Arc<Actors>,
}
impl Inputs {
    fn new(frame: &DecodedFrame, actors: Arc<Actors>) -> Self {
        Self {
            views: TABLES.map(|table| frame.tables[table.index()].clone()),
            actors,
        }
    }
    fn same(&self, frame: &DecodedFrame, actors: &Arc<Actors>) -> bool {
        Arc::ptr_eq(&self.actors, actors)
            && TABLES
                .iter()
                .zip(&self.views)
                .all(|(table, seen)| SharedView::ptr_eq(seen, &frame.tables[table.index()]))
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

#[derive(Clone)]
pub(super) struct Span {
    pub first_hour: i32,
    pub last_hour: i32,
    pub inputs: Arc<Inputs>,
    pub reads: Vec<i32>,
}
#[derive(Clone, Default)]
pub(super) struct BranchScan {
    pub campaign_start: Option<i32>,
    pub spans: Vec<Span>,
    blocks: usize,
    actors: Option<Arc<Actors>>,
    actor_views: Option<[ViewIdentity; 2]>,
}
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
        let read = slot_read(&frame.metrics, TableId::Projects, frame.hour);
        if let Some(span) = self.spans.last_mut()
            && span.last_hour.checked_add(1) == Some(frame.hour)
            && span.inputs.same(frame, &actors)
        {
            span.last_hour = frame.hour;
            if read {
                span.reads.push(frame.hour);
            }
            return;
        }
        let inputs = Arc::new(Inputs::new(frame, actors));
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
        self.campaign_start = replay
            .index
            .iter()
            .filter(|block| block.branch == branch)
            .map(|block| block.first_hour)
            .min();
        let mut decoded_blocks = 0;
        for block in replay
            .index
            .iter()
            .skip(self.blocks)
            .filter(|block| block.branch == branch)
        {
            check_cancel(cancelled)?;
            decoded_blocks += 1;
            let mut decoder = FrameDecoder::for_tables(&SELECTED);
            let Ok(frames) = replay.read_block_selected(block, &SELECTED) else {
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
        self.blocks = replay.index.len();
        Ok(decoded_blocks)
    }
}

#[derive(Clone, Default)]
pub(super) struct Scans {
    branches: Vec<(u64, Arc<BranchScan>)>,
    #[cfg(test)]
    pub decoded_blocks: usize,
}
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
#[derive(Default)]
pub(super) struct Names {
    runs: BTreeMap<(String, u32, u32), Vec<NameRun>>,
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
        let mut warnings = BTreeMap::<&str, usize>::new();
        for ((owner, _, _), runs) in &self.runs {
            if owner != tag {
                continue;
            }
            for run in runs {
                for warning in &run.value.warnings {
                    *warnings.entry(warning).or_default() +=
                        (i64::from(run.last) - i64::from(run.first) + 1) as usize;
                }
            }
        }
        warnings
            .into_iter()
            .map(|(reason, count)| format!("{reason} ({count} project-name observations)."))
            .collect()
    }
    fn observe(&mut self, key: (String, u32, u32), hour: i32, value: Name) {
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

#[derive(Default)]
struct Actor {
    last: Option<((u32, u32), i32)>,
    identity: Option<(u32, u32)>,
    admitted: bool,
    open: BTreeSet<(u32, u32)>,
}

pub(super) fn scoped_names(
    replay: &Replay,
    scan: &BranchScan,
    branch: u64,
    content: &Content,
    cancelled: &AtomicBool,
) -> io::Result<Names> {
    let mut names = Names::default();
    let mut actors = BTreeMap::<Vec<u8>, Actor>::new();
    let mut keys = BTreeMap::<(u32, String), Option<String>>::new();
    let mut lexical_view: Option<ViewIdentity> = None;
    let mut at = 0;
    for block in replay.index.iter().filter(|block| block.branch == branch) {
        check_cancel(cancelled)?;
        #[cfg(test)]
        {
            names.decoded_blocks += 1;
        }
        let mut decoder = FrameDecoder::new();
        let Ok(frames) = replay.read_block_selected(block, &TableId::ALL) else {
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
            let Ok(countries) = frame.complete::<tables::Countries>() else {
                continue;
            };
            let projects = frame.complete::<tables::Projects>().ok();
            let tokens = frame.complete::<tables::LexicalTokens>().ok();
            let lexical = &frame.tables[TableId::LexicalTokens.index()];
            if lexical_view.as_ref().is_none_or(|seen| !seen.is(lexical)) {
                keys.clear();
                lexical_view = Some(lexical.identity());
            }
            for (tag, &(id, slot)) in &span.inputs.actors.tags {
                let Ok(index) = countries.binary_search_by_key(&id, |country| country.id) else {
                    continue;
                };
                let country = &countries[index];
                let identity = (slot, country.id);
                let actor = actors.entry(tag.clone()).or_default();
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
                if span.reads.binary_search(&frame.hour).is_ok() {
                    actor.admitted = true;
                }
                if !actor.admitted {
                    continue;
                }
                let Some(links) = span.inputs.actors.links.get(&slot) else {
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
                    let key = keys
                        .entry((project.definition_id, tag.to_owned()))
                        .or_insert_with(|| {
                            let tokens = tokens?;
                            let token = tokens
                                .binary_search_by_key(&project.definition_id, |row| row.token)
                                .ok()
                                .and_then(|index| {
                                    std::str::from_utf8(&tokens[index].literal).ok()
                                })?;
                            let definition = content.definitions.projects.get(token)?;
                            if content.definitions.unsettled.is_some()
                                || definition.refusal.is_some()
                            {
                                return None;
                            }
                            let key = country_key(
                                &content.catalogue,
                                tag,
                                project_name_key(definition, token),
                            );
                            needs_scope(&content.catalogue.localisation, &key).then_some(key)
                        });
                    let Some(key) = key else {
                        continue;
                    };
                    let value = match content.catalogue.scripted_content() {
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
                                    .map(|finding| {
                                        format!("{}: {}", finding.subject, finding.message)
                                    })
                                    .collect(),
                            }
                        }
                        Err(reason) => Name {
                            text: None,
                            warnings: vec![format!("Project name {key}: {reason}")],
                        },
                    };
                    names.observe(
                        (tag.to_owned(), slot, project.definition_id),
                        frame.hour,
                        value,
                    );
                }
                actor.open = next;
            }
        }
    }
    Ok(names)
}
