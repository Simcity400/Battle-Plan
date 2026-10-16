//! One all-country projection. Queries build selected episodes from change intervals.
use super::*;
use dashboard_base::prepared::frame_changes::FrameChanges;
use dashboard_base::prepared::series::{self, Accumulator, Reader};
use std::collections::BTreeSet;

pub mod completion_stream;

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
struct Observation {
    identity: (u32, u32),
    projects: Option<Vec<(ProjectFacts, Look)>>,
}
/// An observation as its event stores it. Each distinct presentation and
/// person (icons, portraits, names) is published once, as the only event of
/// its own key, and the observation names that key, so a changed project
/// republishes its country's small facts rather than every project's art.
#[derive(Serialize, Deserialize)]
struct StoredObservation {
    identity: (u32, u32),
    projects: Option<Vec<(ProjectFacts, StoredLook)>>,
}
#[derive(Serialize, Deserialize)]
struct StoredLook {
    needs_scope: bool,
    program_reference: u64,
    person: u64,
    info: u64,
    warnings: BTreeMap<String, usize>,
    refusals: catalogue::Refusals,
    iteration_text: Option<String>,
    iteration_refusals: catalogue::Refusals,
}
/// The key publishing `value`: an earlier one for equal text, else a new key
/// whose only event is `value`.
fn look_key(
    values: &mut Accumulator,
    looks: &mut BTreeMap<String, u64>,
    next_key: &mut u64,
    hour: i32,
    value: &impl Serialize,
) -> io::Result<u64> {
    let hash = blake3::hash(&serde_json::to_vec(value)?)
        .to_hex()
        .to_string();
    if let Some(&key) = looks.get(&hash) {
        return Ok(key);
    }
    let key = *next_key;
    *next_key += 1;
    values.event(key, hour, value)?;
    looks.insert(hash, key);
    Ok(key)
}
/// Publishes `observation` as the event of `key` at `hour`.
fn emit(
    values: &mut Accumulator,
    looks: &mut BTreeMap<String, u64>,
    next_key: &mut u64,
    key: u64,
    hour: i32,
    observation: &Observation,
) -> io::Result<()> {
    let mut projects = None;
    if let Some(observed) = &observation.projects {
        let mut stored = Vec::with_capacity(observed.len());
        for (facts, look) in observed {
            stored.push((
                facts.clone(),
                StoredLook {
                    needs_scope: look.needs_scope,
                    program_reference: look.program_reference,
                    person: look_key(values, looks, next_key, hour, &*look.person)?,
                    info: look_key(values, looks, next_key, hour, &*look.info)?,
                    warnings: look.warnings.clone(),
                    refusals: look.refusals.clone(),
                    iteration_text: look.iteration_text.clone(),
                    iteration_refusals: look.iteration_refusals.clone(),
                },
            ));
        }
        projects = Some(stored);
    }
    values.event(
        key,
        hour,
        &StoredObservation {
            identity: observation.identity,
            projects,
        },
    )
}
/// Presentations and people a query has read, by key.
#[derive(Default)]
struct Looks {
    infos: BTreeMap<u64, Arc<ProjectPresentation>>,
    people: BTreeMap<u64, Arc<ProjectPerson>>,
}
fn published<T: serde::de::DeserializeOwned>(
    reader: &mut Reader<'_>,
    held: &mut BTreeMap<u64, Arc<T>>,
    key: u64,
) -> io::Result<Arc<T>> {
    if let Some(value) = held.get(&key) {
        return Ok(Arc::clone(value));
    }
    let row = reader
        .events(&[key], i32::MIN, i32::MAX, 0, 1)?
        .rows
        .into_iter()
        .next()
        .ok_or_else(|| dashboard_core::invalid("Prepared project presentation is missing"))?;
    let value = Arc::new(serde_json::from_value(row.row)?);
    held.insert(key, Arc::clone(&value));
    Ok(value)
}
impl Looks {
    fn observation(
        &mut self,
        reader: &mut Reader<'_>,
        stored: StoredObservation,
    ) -> io::Result<Observation> {
        let mut projects = None;
        if let Some(stored) = stored.projects {
            let mut observed = Vec::with_capacity(stored.len());
            for (facts, look) in stored {
                observed.push((
                    facts,
                    Look {
                        needs_scope: look.needs_scope,
                        program_reference: look.program_reference,
                        person: published(reader, &mut self.people, look.person)?,
                        info: published(reader, &mut self.infos, look.info)?,
                        warnings: look.warnings,
                        refusals: look.refusals,
                        iteration_text: look.iteration_text,
                        iteration_refusals: look.iteration_refusals,
                    },
                ));
            }
            projects = Some(observed);
        }
        Ok(Observation {
            identity: stored.identity,
            projects,
        })
    }
}
#[derive(Default, Serialize, Deserialize)]
struct Country {
    #[serde(skip)]
    current: Option<Observation>,
    identity_key: u64,
    identity_hour: Option<i32>,
    hash: Option<[u8; 32]>,
    event_hour: Option<i32>,
    last: Option<(i32, (u32, u32))>,
    #[serde(with = "dashboard_core::map_pairs")]
    progress: BTreeMap<(u32, u32), u64>,
    active: BTreeSet<u64>,
}
#[derive(Clone, Default, Serialize, Deserialize)]
struct ProgressKeys {
    identity: u64,
    #[serde(with = "dashboard_core::map_pairs")]
    keys: BTreeMap<(u32, u32), u64>,
}
#[derive(Clone, Default, Serialize, Deserialize)]
struct Metadata {
    start: i32,
    warnings: Vec<String>,
    content_error: Option<String>,
    definitions_empty: bool,
    scoped_warnings: BTreeMap<String, Vec<String>>,
    progress: BTreeMap<String, ProgressKeys>,
}
/// Current resolved project and history operands; resume rebuilds them through A3.
#[derive(Clone)]
struct ProjectInput {
    facts: ProjectFacts,
    program_reference: u64,
}
impl ProjectInput {
    fn presentation_row(&self, owner: u32) -> ProjectRow {
        let mut row = ProjectRow {
            owner,
            local_id: self.facts.local_id,
            definition_id: self.facts.definition_id,
            program_reference: self.program_reference,
            ..Default::default()
        };
        if self.facts.state == ProjectState::Completed {
            row.state.current_offset = 0xe8;
            row.state.not_completed = 0;
        }
        row
    }
}
type ProjectInputs = Option<Arc<BTreeMap<u32, Vec<ProjectInput>>>>;
type HistoryInputs = Option<Arc<BTreeMap<(u32, u32), Vec<(u32, bool, i32)>>>>;
#[derive(Default, Serialize, Deserialize)]
pub struct Prepared {
    values: Accumulator,
    keys: BTreeMap<String, u64>,
    countries: BTreeMap<String, Country>,
    active_countries: BTreeSet<String>,
    next_key: u64,
    /// Published presentations and people by text hash, to their keys.
    looks: BTreeMap<String, u64>,
    lexical: u64,
    metadata: Metadata,
    names: history::NameScan,
    completions: completion_stream::Stream,
    #[serde(skip)]
    replay: Option<Arc<replay::container::Replay>>,
    #[serde(skip)]
    lexical_view: Option<ViewIdentity>,
    #[serde(skip)]
    naming: Naming,
    #[serde(skip)]
    content: Option<Content>,
    #[serde(skip)]
    content_ready: bool,
    last_hour: Option<i32>,
    next_completion: Option<i32>,
    /// The content refusals this family's own reads produced and the lazy
    /// readers they used, whose warnings its metadata reports.
    #[serde(default)]
    refusals: catalogue::OwnRefusals,
    #[serde(skip)]
    scoped: bool,
    #[serde(skip)]
    actors: Option<history::Actors>,
    #[serde(skip)]
    country_keys: Option<dashboard_text::metadata::CountryKeys>,
    #[serde(skip)]
    link_inputs: Option<Vec<(u32, u32)>>,
    #[serde(skip)]
    project_inputs: ProjectInputs,
    #[serde(skip)]
    history_inputs: HistoryInputs,
    #[serde(skip)]
    owner_countries: BTreeMap<u32, String>,
    #[serde(skip)]
    linked_owners: BTreeSet<u32>,
    #[serde(skip)]
    project_countries: BTreeSet<String>,
    #[serde(skip)]
    project_owners_unmapped: usize,
    #[serde(skip)]
    history_owners_unmapped: usize,
    #[serde(skip)]
    dependencies: BTreeMap<u32, (Option<Vec<u8>>, BTreeSet<String>)>,
    #[serde(skip)]
    country_dependencies: BTreeMap<String, BTreeSet<u32>>,
    #[serde(skip)]
    known_tokens: BTreeSet<u32>,
    #[serde(skip)]
    program_countries: BTreeMap<u32, BTreeSet<String>>,
    #[serde(skip)]
    program_scientists: Option<BTreeMap<u32, u64>>,
    #[serde(skip)]
    person_countries: BTreeMap<u32, BTreeSet<String>>,
    #[serde(skip)]
    country_people: BTreeMap<String, BTreeSet<u32>>,
    #[serde(skip)]
    character_inputs: Option<BTreeMap<u32, (Vec<u8>, i32)>>,
    #[allow(clippy::type_complexity)]
    #[serde(skip)]
    portrait_inputs: Option<BTreeMap<u32, Vec<(u32, u32, u8, u32, Vec<u8>)>>>,
    #[cfg(any(test, feature = "test-support"))]
    #[serde(skip)]
    work: dashboard_base::prepared::observer_tests::Work,
    #[cfg(any(test, feature = "test-support"))]
    #[serde(skip)]
    test_changes: Option<dashboard_base::prepared::frame_changes::FrameChangeTracker>,
}
const LEXICAL: u64 = 1;
fn section(branch: u64) -> String {
    format!("special-projects-{branch}")
}
impl Prepared {
    pub fn new(replay: &replay::container::Replay, branch: u64) -> Self {
        let start = replay
            .index
            .iter()
            .filter(|block| block.branch == branch)
            .map(|block| block.first_hour)
            .min()
            .unwrap_or(0);
        Self {
            next_key: 2,
            metadata: Metadata {
                start,
                ..Default::default()
            },
            ..Default::default()
        }
    }
    /// Bind weak identities to the already restored decoder, without replaying it.
    pub fn restore_views(&mut self, frame: &DecodedFrame) {
        self.lexical_view = Some(frame.tables[TableId::LexicalTokens.index()].identity());
        self.completions.restore_views(frame);
    }
    /// Runs `run` as this family's own consumer of the shared content
    /// ([`catalogue::own_refusals`]).
    fn own<T>(&mut self, run: impl FnOnce(&mut Self) -> T) -> T {
        let mut own = std::mem::take(&mut self.refusals);
        let value = catalogue::own_refusals(&mut own, || run(self));
        self.refusals = own;
        value
    }
    pub fn configure(&mut self, source: &Source, context: &LoadContext) -> io::Result<()> {
        self.own(|state| state.configure_own(source, context))
    }
    fn configure_own(&mut self, source: &Source, context: &LoadContext) -> io::Result<()> {
        if self.content_ready {
            return Ok(());
        }
        self.replay = Some(source.replay());
        self.metadata.content_error = None;
        type BoundContent = Option<(
            Option<catalogue::Binding>,
            Arc<catalogue::CachedResult<Content>>,
        )>;
        let key = context.content.as_ref().map(catalogue::binding);
        let slot = source.state::<BoundContent>();
        let cache = {
            let mut slot = slot.lock().unwrap_or_else(|error| error.into_inner());
            if slot.as_ref().is_none_or(|(binding, _)| *binding != key) {
                *slot = Some((key, Arc::new(catalogue::CachedResult::default())));
            }
            Arc::clone(&slot.as_ref().expect("project content binding").1)
        };
        let loaded = cache.get_or_init(|| {
            #[cfg(any(test, feature = "test-support"))]
            {
                self.work.content_loads += 1;
            }
            let politics = context
                .content()
                .and_then(|options| politics_content(source, options))
                .map_err(|error| {
                    io::Error::new(
                        error.kind(),
                        format!("Project content, names and art unavailable: {error}"),
                    )
                })?;
            Content::load(politics).map_err(|error| {
                io::Error::new(
                    error.kind(),
                    format!("Special-project content unavailable: {error}"),
                )
            })
        });
        if loaded
            .as_ref()
            .is_err_and(|error| error.kind() == io::ErrorKind::Interrupted)
        {
            return loaded.map(|_| ());
        }
        self.content = match loaded {
            Ok(content) => Some((*content).clone()),
            Err(reason) => {
                self.metadata.content_error = Some(reason.to_string());
                None
            }
        };
        self.content_ready = true;
        self.scoped = self.content.as_ref().is_some_and(|content| {
            completions::scripted_names(&content.definitions, &content.catalogue.localisation)
        });
        Ok(())
    }
    #[cfg(any(test, feature = "test-support"))]
    pub fn observe(
        &mut self,
        source: &Source,
        context: &LoadContext,
        frame: &DecodedFrame,
        cancelled: &AtomicBool,
    ) -> io::Result<()> {
        self.configure(source, context)?;
        let changes = self
            .test_changes
            .get_or_insert_with(dashboard_base::prepared::frame_changes::FrameChangeTracker::new)
            .observe(frame, source.replay_ref().format);
        self.observe_changed(frame, &changes, cancelled)
    }
    pub fn observe_changed(
        &mut self,
        frame: &DecodedFrame,
        changes: &FrameChanges,
        cancelled: &AtomicBool,
    ) -> io::Result<()> {
        self.own(|state| state.observe_own_changes(frame, changes, cancelled))
    }
    fn observe_own_changes(
        &mut self,
        frame: &DecodedFrame,
        changes: &FrameChanges,
        cancelled: &AtomicBool,
    ) -> io::Result<()> {
        dashboard_text::metadata::check_cancel(cancelled)?;
        self.values.advance(frame.hour)?;
        let previous_hour = self.last_hour.replace(frame.hour);
        let gap = previous_hour.and_then(|hour| hour.checked_add(1)) != Some(frame.hour);
        // Read evidence changes independently of row allocations and country facts.
        let read = changes.slot_read[TableId::Projects.index()];
        let marker = (read && frame.complete::<tables::Projects>().is_ok()).then_some(1);
        // Absence closes the held marker without replacing its last numeric
        // value. Reobserve this single branch-wide scalar so a successful read
        // after an unavailable interval reopens it even when that value is 1.
        self.values.observe(0, frame.hour, marker, gap);
        let lexical_changed = self
            .lexical_view
            .as_ref()
            .is_none_or(|view| !view.is(&frame.tables[TableId::LexicalTokens.index()]));
        if lexical_changed {
            self.lexical += 1;
            self.lexical_view = Some(frame.tables[TableId::LexicalTokens.index()].identity());
        }
        if lexical_changed || gap {
            self.values
                .observe(LEXICAL, frame.hour, Some(i128::from(self.lexical)), gap);
        }
        let identity_changed = self.actors.is_none()
            || changes.rows_changed[TableId::CountryLinks.index()]
            || changes.rows_changed[TableId::Countries.index()]
                && self.country_keys.as_ref().is_none_or(|keys| {
                    frame
                        .complete::<tables::Countries>()
                        .map_or(true, |rows| !keys.same(rows))
                });
        if identity_changed {
            if changes.rows_changed[TableId::Countries.index()] {
                self.country_keys = frame
                    .complete::<tables::Countries>()
                    .ok()
                    .map(dashboard_text::metadata::CountryKeys::read);
                #[cfg(any(test, feature = "test-support"))]
                {
                    self.work.row_visits += frame
                        .complete::<tables::Countries>()
                        .map_or(0, |rows| rows.len());
                }
            }
            if changes.rows_changed[TableId::CountryLinks.index()] {
                self.link_inputs = history::Actors::link_operands(frame);
                #[cfg(any(test, feature = "test-support"))]
                {
                    self.work.row_visits += self.link_inputs.as_ref().map_or(0, Vec::len);
                }
            }
            self.actors = Some(history::Actors::bind(
                self.country_keys.as_ref(),
                self.link_inputs.as_deref(),
            ));
            self.owner_countries.clear();
            let actors = self.actors.as_ref().unwrap();
            self.linked_owners = actors
                .links
                .values()
                .flatten()
                .map(|link| link.id)
                .collect();
            for (tag, &(_, slot)) in &actors.tags {
                if let Ok(tag) = std::str::from_utf8(tag) {
                    for link in &actors.links[&slot] {
                        self.owner_countries.insert(link.id, tag.to_owned());
                    }
                }
            }
        }
        let structural = identity_changed
            || self.next_completion.is_some_and(|hour| hour <= frame.hour)
            || [TableId::Projects, TableId::ProjectHistory]
                .iter()
                .any(|table| changes.rows_changed[table.index()]);
        // Scoped text may read the admitted hour even when its row views hold.
        let scoped_changed = self.scoped;
        let mut names_affected = BTreeSet::new();
        if changes.rows_changed[TableId::ProjectPrograms.index()] {
            let next = frame
                .complete::<tables::ProjectPrograms>()
                .ok()
                .map(|rows| {
                    #[cfg(any(test, feature = "test-support"))]
                    {
                        self.work.row_visits += rows.len();
                    }
                    rows.iter()
                        .map(|row| (row.id, row.scientist))
                        .collect::<BTreeMap<_, _>>()
                });
            let ids: BTreeSet<_> = if self.program_scientists.is_some() != next.is_some() {
                self.program_countries.keys().copied().collect()
            } else {
                self.program_scientists
                    .as_ref()
                    .into_iter()
                    .flat_map(|rows| rows.keys())
                    .chain(next.as_ref().into_iter().flat_map(|rows| rows.keys()))
                    .copied()
                    .collect()
            };
            for id in ids {
                let Some(countries) = self.program_countries.get(&id) else {
                    continue;
                };
                if self
                    .program_scientists
                    .as_ref()
                    .and_then(|rows| rows.get(&id))
                    != next.as_ref().and_then(|rows| rows.get(&id))
                    || self.program_scientists.is_some() != next.is_some()
                {
                    names_affected.extend(countries.iter().cloned());
                }
            }
            self.program_scientists = next;
        }
        if changes.rows_changed[TableId::Characters.index()] {
            let next = frame.complete::<tables::Characters>().ok().map(|rows| {
                #[cfg(any(test, feature = "test-support"))]
                {
                    self.work.row_visits += rows.len();
                }
                rows.iter()
                    .map(|row| (row.id, (row.token.clone(), row.name_token_id)))
                    .collect::<BTreeMap<_, _>>()
            });
            let ids: BTreeSet<_> = if self.character_inputs.is_some() != next.is_some() {
                self.person_countries.keys().copied().collect()
            } else {
                self.character_inputs
                    .as_ref()
                    .into_iter()
                    .flat_map(|rows| rows.keys())
                    .chain(next.as_ref().into_iter().flat_map(|rows| rows.keys()))
                    .copied()
                    .collect()
            };
            for id in ids {
                let Some(countries) = self.person_countries.get(&id) else {
                    continue;
                };
                if self
                    .character_inputs
                    .as_ref()
                    .and_then(|rows| rows.get(&id))
                    != next.as_ref().and_then(|rows| rows.get(&id))
                    || self.character_inputs.is_some() != next.is_some()
                {
                    names_affected.extend(countries.iter().cloned());
                }
            }
            self.character_inputs = next;
        }
        if changes.rows_changed[TableId::CharacterPortraits.index()] {
            let next = frame
                .complete::<tables::CharacterPortraits>()
                .ok()
                .map(|rows| {
                    #[cfg(any(test, feature = "test-support"))]
                    {
                        self.work.row_visits += rows.len();
                    }
                    rows.iter()
                        .map(|row| {
                            (
                                row.character,
                                row.portraits
                                    .iter()
                                    .map(|portrait| {
                                        (
                                            portrait.category,
                                            portrait.size,
                                            portrait.is_set,
                                            portrait.kind,
                                            portrait.identity.clone(),
                                        )
                                    })
                                    .collect::<Vec<_>>(),
                            )
                        })
                        .collect::<BTreeMap<_, _>>()
                });
            let ids: BTreeSet<_> = if self.portrait_inputs.is_some() != next.is_some() {
                self.person_countries.keys().copied().collect()
            } else {
                self.portrait_inputs
                    .as_ref()
                    .into_iter()
                    .flat_map(|rows| rows.keys())
                    .chain(next.as_ref().into_iter().flat_map(|rows| rows.keys()))
                    .copied()
                    .collect()
            };
            for id in ids {
                let Some(countries) = self.person_countries.get(&id) else {
                    continue;
                };
                if self.portrait_inputs.as_ref().and_then(|rows| rows.get(&id))
                    != next.as_ref().and_then(|rows| rows.get(&id))
                    || self.portrait_inputs.is_some() != next.is_some()
                {
                    names_affected.extend(countries.iter().cloned());
                }
            }
            self.portrait_inputs = next;
        }
        if changes.rows_changed[TableId::LexicalTokens.index()] {
            let mut found = BTreeSet::new();
            if let Ok(rows) = frame.complete::<tables::LexicalTokens>() {
                #[cfg(any(test, feature = "test-support"))]
                {
                    self.work.row_visits += rows.len();
                }
                for row in rows {
                    if let Some((literal, countries)) = self.dependencies.get_mut(&row.token) {
                        found.insert(row.token);
                        if literal.as_deref() != Some(row.literal.as_slice()) {
                            *literal = Some(row.literal.clone());
                            names_affected.extend(countries.iter().cloned());
                        }
                    }
                }
            }
            for token in self.known_tokens.difference(&found) {
                if let Some((literal, countries)) = self.dependencies.get_mut(token)
                    && literal.take().is_some()
                {
                    names_affected.extend(countries.iter().cloned());
                }
            }
            self.known_tokens = found;
        }
        if !structural && scoped_changed && !changes.rows_changed[TableId::LexicalTokens.index()] {
            if let Some(content) = &mut self.content {
                names_affected.extend(self.names.evaluate_current(
                    frame,
                    self.actors.as_ref().expect("project identities"),
                    content,
                    cancelled,
                )?);
                content.scoped.clear();
                content
                    .scoped
                    .push((frame.branch, Arc::new(self.names.take_current())));
            }
            self.refresh_names(frame, &names_affected, cancelled)?;
            if self.completions.observe_inputs(
                frame,
                self.actors.as_ref().expect("project identities"),
                false,
            ) {
                self.completions.repeat_source_counts(frame.hour);
            }
            self.completions.through(frame.hour);
            return Ok(());
        }
        if !structural
            && !scoped_changed
            && !changes.rows_changed[TableId::LexicalTokens.index()]
            && [
                TableId::ProjectPrograms,
                TableId::Characters,
                TableId::CharacterPortraits,
            ]
            .iter()
            .any(|table| changes.rows_changed[table.index()])
        {
            self.refresh_names(frame, &names_affected, cancelled)?;
            if self.completions.observe_inputs(
                frame,
                self.actors.as_ref().expect("held project identities"),
                false,
            ) {
                self.completions.repeat_source_counts(frame.hour);
            }
            self.completions.through(frame.hour);
            return Ok(());
        }
        if !structural && changes.rows_changed[TableId::LexicalTokens.index()] {
            let mut affected = names_affected;
            if self.scoped
                && let Some(content) = &mut self.content
            {
                for tag in &affected {
                    if let Some(projects) = self.countries[tag]
                        .current
                        .as_ref()
                        .and_then(|row| row.projects.as_ref())
                    {
                        for (fact, _) in projects {
                            self.names.rebind(frame, tag, fact.definition_id, content);
                        }
                    }
                }
                affected.extend(self.names.evaluate_current(
                    frame,
                    self.actors.as_ref().expect("project identities"),
                    content,
                    cancelled,
                )?);
                content.scoped.clear();
                content
                    .scoped
                    .push((frame.branch, Arc::new(self.names.take_current())));
            }
            self.refresh_names(frame, &affected, cancelled)?;
            if self.completions.observe_inputs(
                frame,
                self.actors.as_ref().expect("held project identities"),
                false,
            ) {
                self.completions.repeat_source_counts(frame.hour);
            }
            self.completions.through(frame.hour);
            return Ok(());
        }
        if !structural && !scoped_changed && !changes.rows_changed[TableId::LexicalTokens.index()] {
            self.completions.through(frame.hour);
            return Ok(());
        }
        if identity_changed {
            self.dependencies.clear();
            self.country_dependencies.clear();
            self.known_tokens.clear();
            self.program_countries.clear();
            self.person_countries.clear();
            self.country_people.clear();
        }
        let actors = self.actors.as_ref().expect("project actor projection");
        let span_changed = self
            .completions
            .observe_inputs(frame, actors, identity_changed);
        if self.scoped
            && let Some(content) = &mut self.content
        {
            if identity_changed || gap || changes.rows_changed[TableId::Projects.index()] {
                self.names
                    .observe(frame, actors, read, content, cancelled)?;
            } else {
                self.names
                    .evaluate_current(frame, actors, content, cancelled)?;
            }
            content.scoped.clear();
            content
                .scoped
                .push((frame.branch, Arc::new(self.names.take_current())));
        }
        let previous_project_known = self.project_inputs.is_some();
        let previous_history = self.history_inputs.clone();
        let mut reached_dates = BTreeSet::new();
        if changes.rows_changed[TableId::Projects.index()] {
            self.project_owners_unmapped = 0;
            self.project_inputs = frame.complete::<tables::Projects>().ok().map(|rows| {
                #[cfg(any(test, feature = "test-support"))]
                {
                    self.work.row_visits += rows.len();
                }
                let mut inputs = BTreeMap::<u32, Vec<ProjectInput>>::new();
                for row in rows {
                    self.project_owners_unmapped +=
                        usize::from(actors.known && !self.linked_owners.contains(&row.owner));
                    inputs.entry(row.owner).or_default().push(ProjectInput {
                        facts: ProjectFacts::project(row, None, frame.hour, self.metadata.start),
                        program_reference: row.program_reference,
                    });
                }
                Arc::new(inputs)
            });
        }
        if changes.rows_changed[TableId::ProjectHistory.index()] {
            self.next_completion = None;
            self.history_owners_unmapped = 0;
            self.history_inputs = frame.complete::<tables::ProjectHistory>().ok().map(|rows| {
                #[cfg(any(test, feature = "test-support"))]
                {
                    self.work.row_visits += rows.len();
                }
                let mut inputs = BTreeMap::<(u32, u32), Vec<(u32, bool, i32)>>::new();
                for row in rows {
                    self.history_owners_unmapped +=
                        usize::from(actors.known && !self.linked_owners.contains(&row.owner));
                    let operand = history_operand(row);
                    if operand.1 && operand.2 > frame.hour {
                        self.next_completion = Some(
                            self.next_completion
                                .map_or(operand.2, |old| old.min(operand.2)),
                        );
                    }
                    inputs
                        .entry((row.owner, row.project))
                        .or_default()
                        .push(operand);
                }
                Arc::new(inputs)
            });
        } else if self.next_completion.is_some_and(|hour| hour <= frame.hour) {
            self.next_completion = None;
            if let Some(history) = &self.history_inputs {
                for (&key, rows) in &**history {
                    for &(_, completion, date) in rows {
                        if completion && date > frame.hour {
                            self.next_completion =
                                Some(self.next_completion.map_or(date, |old| old.min(date)));
                        } else if completion && previous_hour.is_none_or(|hour| date > hour) {
                            reached_dates.insert(key.0);
                        }
                    }
                }
            }
        }
        if identity_changed {
            self.project_owners_unmapped = self
                .project_inputs
                .as_ref()
                .into_iter()
                .flat_map(|rows| rows.iter())
                .filter(|(owner, _)| actors.known && !self.linked_owners.contains(*owner))
                .map(|(_, rows)| rows.len())
                .sum();
            self.history_owners_unmapped = self
                .history_inputs
                .as_ref()
                .into_iter()
                .flat_map(|rows| rows.iter())
                .filter(|(key, _)| actors.known && !self.linked_owners.contains(&key.0))
                .map(|(_, rows)| rows.len())
                .sum();
        }
        let projects = self.project_inputs.clone();
        let history = self.history_inputs.clone();
        let full = identity_changed || gap || previous_project_known != projects.is_some();
        let project_change = changes.rows_changed[TableId::Projects.index()] || full;
        let history_status_changed = previous_history.is_some() != history.is_some();
        let mut affected = if full {
            actors
                .tags
                .keys()
                .filter_map(|tag| std::str::from_utf8(tag).ok().map(str::to_owned))
                .collect::<BTreeSet<_>>()
        } else {
            BTreeSet::new()
        };
        if project_change || history_status_changed {
            affected.extend(self.project_countries.iter().cloned());
            for owner in projects.as_ref().into_iter().flat_map(|rows| rows.keys()) {
                if let Some(tag) = self.owner_countries.get(owner) {
                    affected.insert(tag.clone());
                }
            }
        } else if changes.rows_changed[TableId::ProjectHistory.index()] {
            // Compare only the projected history table's current owners. Removed
            // history inputs revisit their former owner; no country catalogue sweep.
            let owners: BTreeSet<_> = previous_history
                .as_ref()
                .into_iter()
                .flat_map(|rows| rows.keys())
                .chain(history.as_ref().into_iter().flat_map(|rows| rows.keys()))
                .copied()
                .collect();
            for key in owners {
                if previous_history.as_ref().and_then(|rows| rows.get(&key))
                    != history.as_ref().and_then(|rows| rows.get(&key))
                    && let Some(tag) = self.owner_countries.get(&key.0)
                {
                    affected.insert(tag.clone());
                }
            }
        }
        affected.extend(
            reached_dates
                .iter()
                .filter_map(|owner| self.owner_countries.get(owner).cloned()),
        );
        affected.extend(names_affected.iter().cloned());
        if scoped_changed {
            affected.extend(self.names.active_countries());
        }
        if span_changed {
            if project_change {
                self.completions.reset_source_counts();
            }
            self.completions.set_source_count(
                completion_stream::UNMAPPED_PROJECT_OWNER,
                self.project_owners_unmapped,
            );
            self.completions.set_source_count(
                completion_stream::UNMAPPED_HISTORY_OWNER,
                self.history_owners_unmapped,
            );
            self.completions.repeat_source_counts(frame.hour);
        }
        let mut seen = BTreeSet::new();
        for tag in &affected {
            let Some(&(country_id, slot)) = actors.tags.get(tag.as_bytes()) else {
                continue;
            };
            dashboard_text::metadata::check_cancel(cancelled)?;
            let tag = tag.as_str();
            let key = *self.keys.entry(tag.to_owned()).or_insert_with(|| {
                let key = self.next_key;
                self.next_key += 1;
                key
            });
            if let Some(old) = self
                .countries
                .get(tag)
                .and_then(|country| country.current.as_ref())
                .and_then(|row| row.projects.as_ref())
            {
                for (facts, look) in old {
                    if identity_changed || names_affected.contains(tag) {
                        self.naming
                            .definitions
                            .remove(&(tag.to_string(), facts.definition_id));
                        self.naming.people.remove(&look.program_reference);
                    }
                    if look.program_reference as u32 == 83 {
                        let id = (look.program_reference >> 32) as u32;
                        if self
                            .program_countries
                            .get_mut(&id)
                            .is_some_and(|countries| {
                                countries.remove(tag);
                                countries.is_empty()
                            })
                        {
                            self.program_countries.remove(&id);
                        }
                    }
                }
            }
            let country = self.countries.entry(tag.to_owned()).or_insert_with(|| {
                let identity_key = self.next_key;
                self.next_key += 1;
                Country {
                    identity_key,
                    ..Default::default()
                }
            });
            let identity = (slot, country_id);
            let continuing = country.last.is_some_and(|(_, old)| {
                old == identity
                    && previous_hour.and_then(|hour| hour.checked_add(1)) == Some(frame.hour)
            });
            if !continuing {
                country.identity_hour = Some(frame.hour);
                self.values.event(
                    country.identity_key,
                    frame.hour,
                    &CountryIdentityRun {
                        first_hour: frame.hour,
                        last_hour: frame.hour,
                        slot,
                        country_id,
                    },
                )?;
            }
            if !continuing {
                self.values.observe(
                    country.identity_key,
                    frame.hour,
                    country.identity_hour.map(i128::from),
                    true,
                );
            }
            let links = &actors.links[&slot];
            let mut observed = Vec::new();
            let mut completion_rows = Vec::new();
            let mut active = BTreeSet::new();
            for link in links {
                for input in projects
                    .as_ref()
                    .and_then(|rows| rows.get(&link.id))
                    .into_iter()
                    .flatten()
                {
                    #[cfg(any(test, feature = "test-support"))]
                    if !project_change {
                        self.work.history_visits += 1;
                    }
                    let row = input.presentation_row(link.id);
                    let mut rows = Vec::new();
                    for owner in links {
                        rows.extend(
                            history
                                .as_ref()
                                .and_then(|rows| rows.get(&(owner.id, row.local_id)))
                                .into_iter()
                                .flatten()
                                .copied(),
                        );
                    }
                    let mut facts = input.facts.clone();
                    facts.observe_history(
                        history.as_ref().map(|_| rows.as_slice()),
                        frame.hour,
                        self.metadata.start,
                    );
                    let progress_key = *country
                        .progress
                        .entry((row.local_id, row.definition_id))
                        .or_insert_with(|| {
                            let key = self.next_key;
                            self.next_key += 1;
                            key
                        });
                    let value = i128::from(facts.progress);
                    if !continuing
                        || !country.active.contains(&progress_key)
                        || self
                            .values
                            .state(progress_key)
                            .and_then(|state| state.last)
                            .is_none_or(|run| run.value != value)
                    {
                        self.values
                            .observe(progress_key, frame.hour, Some(value), !continuing);
                    }
                    active.insert(progress_key);
                    facts.progress = 0;
                    let look = self.naming.look(
                        frame,
                        tag,
                        slot,
                        &row,
                        facts.iterations,
                        self.content.as_mut(),
                    );
                    observed.push((facts, look));
                    if row.program_reference as u32 == 83 {
                        self.program_countries
                            .entry((row.program_reference >> 32) as u32)
                            .or_default()
                            .insert(tag.to_owned());
                    }
                    completion_rows.push(row);
                }
            }
            if let Some(old) = country
                .current
                .as_ref()
                .and_then(|row| row.projects.as_ref())
            {
                let definitions: BTreeSet<_> = observed
                    .iter()
                    .map(|(fact, _)| fact.definition_id)
                    .collect();
                let programs: BTreeSet<_> = observed
                    .iter()
                    .map(|(_, look)| look.program_reference)
                    .collect();
                for (fact, look) in old {
                    if !definitions.contains(&fact.definition_id) {
                        self.naming
                            .definitions
                            .remove(&(tag.to_owned(), fact.definition_id));
                    }
                    if !programs.contains(&look.program_reference) {
                        self.naming.people.remove(&look.program_reference);
                    }
                }
            }
            for &progress_key in &country.active {
                if !active.contains(&progress_key) {
                    self.values.observe(progress_key, frame.hour, None, false);
                }
            }
            country.active = active;
            if observed.is_empty() {
                self.project_countries.remove(tag);
            } else {
                self.project_countries.insert(tag.to_owned());
            }
            if projects.is_some() && read {
                self.completions.observe(
                    identity,
                    frame.hour,
                    &observed,
                    &completion_rows.iter().collect::<Vec<_>>(),
                    span_changed && project_change,
                );
            }
            let observation = Observation {
                identity,
                projects: projects.as_ref().map(|_| observed),
            };
            if !continuing || country.current.as_ref() != Some(&observation) {
                let hash = *blake3::hash(&serde_json::to_vec(&observation)?).as_bytes();
                if country.hash != Some(hash) || !continuing {
                    emit(
                        &mut self.values,
                        &mut self.looks,
                        &mut self.next_key,
                        key,
                        frame.hour,
                        &observation,
                    )?;
                    #[cfg(any(test, feature = "test-support"))]
                    {
                        self.work.emitted += 1;
                    }
                    country.hash = Some(hash);
                    country.event_hour = Some(frame.hour);
                }
                country.current = Some(observation);
            }
            let value = country.event_hour.map(i128::from);
            if !continuing
                || self
                    .values
                    .state(key)
                    .and_then(|state| state.last)
                    .map(|run| run.value)
                    != value
            {
                self.values.observe(key, frame.hour, value, !continuing);
            }
            country.last = Some((frame.hour, identity));
            seen.insert(tag.to_owned());
        }
        if full {
            for tag in self.active_countries.difference(&seen) {
                let country = self
                    .countries
                    .get_mut(tag)
                    .expect("previous project holder");
                self.values.observe(self.keys[tag], frame.hour, None, false);
                for &key in &country.active {
                    self.values.observe(key, frame.hour, None, false);
                }
                if let Some(old) = country
                    .current
                    .as_ref()
                    .and_then(|row| row.projects.as_ref())
                {
                    for (fact, look) in old {
                        self.naming
                            .definitions
                            .remove(&(tag.clone(), fact.definition_id));
                        self.naming.people.remove(&look.program_reference);
                    }
                }
                country.active.clear();
                self.project_countries.remove(tag);
                self.values
                    .observe(country.identity_key, frame.hour, None, false);
                country.last = None;
            }
            self.active_countries = seen.clone();
        }
        for tag in seen {
            self.reindex_country(&tag, frame);
        }
        self.completions.through(frame.hour);
        self.values.through(frame.hour);
        Ok(())
    }
    pub fn buffered_bytes(&self) -> usize {
        self.values
            .bytes()
            .saturating_add(self.completions.buffered_bytes())
    }
    fn refresh_names(
        &mut self,
        frame: &DecodedFrame,
        affected: &BTreeSet<String>,
        cancelled: &AtomicBool,
    ) -> io::Result<()> {
        for tag in affected {
            dashboard_text::metadata::check_cancel(cancelled)?;
            let country = self
                .countries
                .get_mut(tag)
                .expect("live lexical dependency");
            let Some(mut observation) = country.current.take() else {
                continue;
            };
            if let Some(projects) = &mut observation.projects {
                for (facts, look) in projects {
                    #[cfg(any(test, feature = "test-support"))]
                    {
                        self.work.history_visits += 1;
                    }
                    self.naming
                        .definitions
                        .remove(&(tag.to_string(), facts.definition_id));
                    self.naming.people.remove(&look.program_reference);
                    let row = ProjectRow {
                        local_id: facts.local_id,
                        definition_id: facts.definition_id,
                        program_reference: look.program_reference,
                        ..Default::default()
                    };
                    *look = self.naming.look(
                        frame,
                        tag,
                        observation.identity.0,
                        &row,
                        facts.iterations,
                        self.content.as_mut(),
                    );
                }
            }
            let hash = *blake3::hash(&serde_json::to_vec(&observation)?).as_bytes();
            if country.hash != Some(hash) {
                let key = self.keys[tag];
                emit(
                    &mut self.values,
                    &mut self.looks,
                    &mut self.next_key,
                    key,
                    frame.hour,
                    &observation,
                )?;
                self.values
                    .observe(key, frame.hour, Some(i128::from(frame.hour)), false);
                country.hash = Some(hash);
                country.event_hour = Some(frame.hour);
                #[cfg(any(test, feature = "test-support"))]
                {
                    self.work.emitted += 1;
                }
            }
            country.current = Some(observation);
            self.reindex_country(tag, frame);
        }
        Ok(())
    }
    fn reindex_country(&mut self, tag: &str, frame: &DecodedFrame) {
        let previous = self.country_dependencies.remove(tag).unwrap_or_default();
        for &token in &previous {
            if let Some((_, countries)) = self.dependencies.get_mut(&token) {
                countries.remove(tag);
            }
        }
        let previous_people = self.country_people.remove(tag).unwrap_or_default();
        let mut people = BTreeSet::new();
        let mut tokens = BTreeSet::new();
        if let Some(projects) = self.countries[tag]
            .current
            .as_ref()
            .and_then(|observation| observation.projects.as_ref())
        {
            for (facts, look) in projects {
                tokens.insert(facts.definition_id);
                if look.program_reference as u32 == 83
                    && let Some(reference) = self
                        .program_scientists
                        .as_ref()
                        .and_then(|rows| rows.get(&((look.program_reference >> 32) as u32)))
                        .filter(|reference| **reference as u32 == 73)
                {
                    let person = (reference >> 32) as u32;
                    people.insert(person);
                    if let Some((_saved, token)) = self
                        .character_inputs
                        .as_ref()
                        .and_then(|rows| rows.get(&person))
                        && let Ok(token) = u32::try_from(*token)
                    {
                        tokens.insert(token);
                    }
                }
            }
        }
        for &token in &tokens {
            self.dependencies
                .entry(token)
                .or_insert_with(|| {
                    let literal = frame
                        .complete::<tables::LexicalTokens>()
                        .ok()
                        .and_then(|rows| {
                            rows.binary_search_by_key(&token, |row| row.token)
                                .ok()
                                .map(|index| rows[index].literal.clone())
                        });
                    (literal, BTreeSet::new())
                })
                .1
                .insert(tag.to_owned());
            if self.dependencies[&token].0.is_some() {
                self.known_tokens.insert(token);
            }
        }
        for token in previous.difference(&tokens) {
            if self
                .dependencies
                .get(token)
                .is_some_and(|(_, countries)| countries.is_empty())
            {
                self.dependencies.remove(token);
                self.known_tokens.remove(token);
            }
        }
        self.country_dependencies.insert(tag.to_owned(), tokens);
        for person in previous_people.difference(&people) {
            if self
                .person_countries
                .get_mut(person)
                .is_some_and(|countries| {
                    countries.remove(tag);
                    countries.is_empty()
                })
            {
                self.person_countries.remove(person);
            }
        }
        for &person in &people {
            self.person_countries
                .entry(person)
                .or_default()
                .insert(tag.to_owned());
        }
        self.country_people.insert(tag.to_owned(), people);
    }
    pub fn flush(
        &mut self,
        writer: &mut store::Writer,
        branch: u64,
        cancelled: &AtomicBool,
    ) -> io::Result<()> {
        self.own(|state| state.flush_own(writer, branch, cancelled))
    }
    fn flush_own(
        &mut self,
        writer: &mut store::Writer,
        branch: u64,
        cancelled: &AtomicBool,
    ) -> io::Result<()> {
        self.completions.flush(
            writer,
            branch,
            self.replay.as_deref(),
            self.content.as_mut(),
            self.metadata.content_error.as_deref(),
            cancelled,
        )?;
        self.values.flush(writer, &section(branch), 0, cancelled)?;
        self.metadata.progress = self
            .countries
            .iter()
            .map(|(tag, country)| {
                (
                    tag.clone(),
                    ProgressKeys {
                        identity: country.identity_key,
                        keys: country.progress.clone(),
                    },
                )
            })
            .collect();
        if let Some(content) = &self.content {
            self.metadata.definitions_empty = content.definitions.projects.is_empty();
            self.metadata.warnings = content.definitions.warnings.clone();
            self.metadata
                .warnings
                .extend(content.catalogue.verification.iter().cloned());
            self.metadata
                .warnings
                .extend(content.catalogue.warnings.iter().cloned());
            self.metadata
                .warnings
                .extend(content.catalogue.reader_warning_lines());
        }
        self.metadata.scoped_warnings = self
            .keys
            .keys()
            .filter_map(|tag| {
                let warnings = self.names.names.warnings(tag);
                (!warnings.is_empty()).then(|| (tag.clone(), warnings))
            })
            .collect();
        series::write_catalogue(
            writer,
            "special-projects",
            branch,
            &self.keys,
            &self.metadata,
        )?;
        self.metadata.progress.clear();
        self.metadata.scoped_warnings.clear();
        Ok(())
    }
}

struct Interval {
    first: i32,
    last: i32,
    observation: Observation,
}
fn intervals(
    reader: &mut Reader<'_>,
    key: u64,
    cancelled: &AtomicBool,
) -> io::Result<Vec<Interval>> {
    let mut result = Vec::new();
    let mut looks = Looks::default();
    let mut cursor = 0;
    loop {
        dashboard_text::metadata::check_cancel(cancelled)?;
        let page = reader.events(&[key], i32::MIN, i32::MAX, cursor, 256)?;
        if page
            .cursor
            .is_some_and(|next| next <= cursor || page.rows.is_empty())
        {
            return Err(dashboard_core::invalid(
                "Special-project event page made no progress",
            ));
        }
        for row in page.rows {
            let run = reader.value_at(key, row.hour)?.ok_or_else(|| {
                dashboard_core::invalid("Prepared project observation has no interval")
            })?;
            let stored = serde_json::from_value(row.row)?;
            result.push(Interval {
                first: row.hour,
                last: run.last_hour,
                observation: looks.observation(reader, stored)?,
            });
        }
        let Some(next) = page.cursor else {
            break;
        };
        cursor = next;
    }
    Ok(result)
}
fn identities(
    reader: &mut Reader<'_>,
    key: u64,
    cancelled: &AtomicBool,
) -> io::Result<Vec<CountryIdentityRun>> {
    let mut identities = Vec::new();
    let mut cursor = 0;
    loop {
        dashboard_text::metadata::check_cancel(cancelled)?;
        let page = reader.events(&[key], i32::MIN, i32::MAX, cursor, 256)?;
        if page
            .cursor
            .is_some_and(|next| next <= cursor || page.rows.is_empty())
        {
            return Err(dashboard_core::invalid(
                "Special-project event page made no progress",
            ));
        }
        for event in page.rows {
            let mut identity: CountryIdentityRun = serde_json::from_value(event.row)?;
            identity.last_hour = reader
                .value_at(key, event.hour)?
                .ok_or_else(|| {
                    dashboard_core::invalid("Prepared project identity has no interval")
                })?
                .last_hour;
            identities.push(identity);
        }
        let Some(next) = page.cursor else {
            break;
        };
        cursor = next;
    }
    Ok(identities)
}

fn apply_interval(
    builder: &mut Builder,
    tag: &str,
    interval: &Interval,
    first: i32,
    last: i32,
    facts: &[ProjectFacts],
    refusals: &mut catalogue::Refusals,
) {
    let Some(projects) = &interval.observation.projects else {
        builder.last = Some((interval.observation.identity, last));
        builder.admitted = false;
        return;
    };
    if builder
        .identity
        .is_some_and(|old| old != interval.observation.identity)
    {
        builder.open.clear();
    }
    builder.identity = Some(interval.observation.identity);
    // Admit the first hour before processing the attribution boundary. Extending
    // coverage now would make that second observation overlap the whole interval.
    if builder
        .available
        .last()
        .is_some_and(|span| span.last_hour.checked_add(1) == Some(first))
    {
        builder.available.last_mut().unwrap().last_hour = first;
    } else {
        builder.available.push(HourSpan {
            first_hour: first,
            last_hour: first,
        });
    }
    builder.last = Some((interval.observation.identity, first));
    let mut present = |builder: &mut Builder,
                       ordinal: usize,
                       _index: usize,
                       fact: &ProjectFacts,
                       previous: Option<&SpecialProjectRun>,
                       _: Option<&ProjectDate>| {
        let look = &projects[ordinal].1;
        let id = builder.present_look(
            look,
            fact.definition_id,
            tag,
            fact.state == ProjectState::Completed,
            first,
            previous,
            refusals,
        );
        if let Some(count) = fact.iterations
            && !builder.iteration_texts.contains_key(&count)
        {
            builder
                .iteration_texts
                .insert(count, look.iteration_text.clone());
            refusals.merge(&look.iteration_refusals);
        }
        id
    };
    builder.observe_projects(first, facts, &mut present);
    // A completion first seen with a real scientist gains dated attribution on
    // its next consecutive observation. Process that boundary once, then extend.
    let second_needed = last > first
        && builder.open.values().any(|index| {
            let run = builder.episodes[*index].runs.last().unwrap();
            let info = &builder.presentations[run.presentation as usize];
            run.state == ProjectState::Completed
                && info.scientist_as_of.is_none()
                && builder.people[info.scientist as usize].id.is_some()
        });
    let processed = if second_needed {
        let second = first + 1;
        let one = Interval {
            first: second,
            last: second,
            observation: interval.observation.clone(),
        };
        apply_interval(builder, tag, &one, second, second, facts, refusals);
        second
    } else {
        first
    };
    for fact in facts {
        let key = (fact.local_id, fact.definition_id);
        if let Some(&index) = builder.open.get(&key) {
            builder.episodes[index].runs.last_mut().unwrap().last_hour = last;
            if let Some(reason) = &fact.warning {
                *builder.warnings.entry(reason.clone()).or_default() +=
                    usize::try_from(i64::from(last) - i64::from(processed))
                        .expect("nonnegative interval");
            }
        }
    }
    builder.available.last_mut().unwrap().last_hour = last;
    builder.last = Some((interval.observation.identity, last));
}

fn complete_reply(
    store: &store::Store,
    source_key: &store::SourceKey,
    params: &SpecialProjectsParams,
    cancelled: &AtomicBool,
) -> io::Result<Option<SpecialProjectsReply>> {
    let branch = params.country().branch();
    let Some((keys, metadata)) =
        series::read_catalogue::<Metadata>(store, source_key, "special-projects", branch)?
    else {
        return Ok(None);
    };
    let mut reader = Reader::new(store, source_key, section(branch), cancelled)?;
    let selected_intervals = match keys.get(&params.tag) {
        Some(&key) => intervals(&mut reader, key, cancelled)?,
        None => Vec::new(),
    };
    let mut history = country_history::RecordedHistories::default();
    let mut all_ids = match metadata.progress.get(&params.tag) {
        Some(country) => identities(&mut reader, country.identity, cancelled)?,
        None => Vec::new(),
    };
    let mut selected_country = country_history::RecordedCountry::default();
    selected_country.identities = Arc::new(all_ids.clone());
    history
        .countries
        .insert(params.tag.clone(), selected_country);
    for root in &params.selection_tags {
        if history.countries.contains_key(root) {
            continue;
        }
        if let Some(root_keys) = metadata.progress.get(root) {
            let mut country = country_history::RecordedCountry::default();
            country.identities = Arc::new(identities(&mut reader, root_keys.identity, cancelled)?);
            history.countries.insert(root.clone(), country);
        }
    }
    let selected = country_history::selected_identities(&history, &params.selection_tags);
    all_ids.retain(|id| {
        params.selection_tags.is_empty() || selected.contains(&(id.slot, id.country_id))
    });
    let mut builder = Builder {
        campaign_start: Some(metadata.start),
        ..Default::default()
    };
    let mut lexical = None;
    let mut refusals = catalogue::Refusals::default();
    for interval in &selected_intervals {
        dashboard_text::metadata::check_cancel(cancelled)?;
        if !params.selection_tags.is_empty() && !selected.contains(&interval.observation.identity) {
            builder.last = None;
            builder.admitted = false;
            continue;
        }
        let Some(projects) = &interval.observation.projects else {
            if builder
                .identity
                .is_some_and(|old| old != interval.observation.identity)
            {
                builder.open.clear();
            }
            builder.identity = Some(interval.observation.identity);
            builder.last = Some((interval.observation.identity, interval.last));
            builder.admitted = false;
            continue;
        };
        // Format 102's Projects cadence is table-level Sliced: each accepted
        // complete hour supplies it. The shared branch marker dates those reads.
        let mut hour = interval.first;
        loop {
            let mut end = interval.last;
            let revision = reader.value_at(LEXICAL, hour)?.ok_or_else(|| {
                dashboard_core::invalid("Prepared project lexical revision is missing")
            })?;
            if lexical != Some(revision.value) {
                builder.content_ids.clear();
                lexical = Some(revision.value);
            }
            end = end.min(revision.last_hour);
            let mut facts: Vec<_> = projects.iter().map(|(facts, _)| facts.clone()).collect();
            for fact in &mut facts {
                let key = metadata.progress[&params.tag].keys[&(fact.local_id, fact.definition_id)];
                let run = reader.value_at(key, hour)?.ok_or_else(|| {
                    dashboard_core::invalid("Prepared project progress is missing")
                })?;
                fact.progress = i32::try_from(run.value)
                    .map_err(|_| dashboard_core::invalid("Prepared project progress is invalid"))?;
                end = end.min(run.last_hour);
            }
            apply_interval(
                &mut builder,
                &params.tag,
                interval,
                hour,
                end,
                &facts,
                &mut refusals,
            );
            if end == interval.last {
                break;
            }
            hour = end
                .checked_add(1)
                .ok_or_else(|| dashboard_core::invalid("Project interval overflow"))?;
        }
    }
    for span in &builder.available {
        let mut hour = span.first_hour;
        while hour <= span.last_hour {
            dashboard_text::metadata::check_cancel(cancelled)?;
            if let Some(run) = reader.value_at(0, hour)? {
                let last = run.last_hour.min(span.last_hour);
                let count = usize::try_from(i64::from(last) - i64::from(hour) + 1)
                    .map_err(|_| dashboard_core::invalid("Project read interval is invalid"))?;
                // Even a one-digit hour plus its separator takes two JSON bytes.
                if builder
                    .reads
                    .len()
                    .saturating_add(count)
                    .saturating_mul(2)
                    .saturating_sub(1)
                    > MAX_PRODUCT_REPLY_BYTES
                {
                    return Err(io::Error::other(
                        "Special projects reply exceeds its transport reply budget",
                    ));
                }
                builder.reads.try_reserve(count).map_err(|error| {
                    io::Error::other(format!(
                        "{}: {error}",
                        dashboard_core::transport::CAPACITY_REACHED
                    ))
                })?;
                builder.reads.extend(hour..=last);
                let Some(next) = last.checked_add(1) else {
                    break;
                };
                hour = next;
            } else if let Some(next) = reader.next_change(0, f64::from(hour), true)? {
                if next <= hour {
                    return Err(dashboard_core::invalid(
                        "Special-project read marker made no progress",
                    ));
                }
                hour = next;
            } else {
                break;
            }
        }
    }
    let mut warnings = metadata.warnings;
    warnings.extend(refusals.lines("Project texts"));
    warnings.extend(
        metadata
            .scoped_warnings
            .get(&params.tag)
            .into_iter()
            .flatten()
            .cloned(),
    );
    let mut reply = build_reply(
        builder,
        &params.country(),
        metadata.definitions_empty,
        warnings,
    )
    .0;
    if let Some(reason) = metadata.content_error {
        reply.warnings.push(reason);
    }
    if !reply.projects.is_empty() {
        reply.warnings.push(
            "Project icons and portraits are visual assets outside content hash coverage.".into(),
        );
    }
    append_identities(&mut reply, &all_ids.iter().collect::<Vec<_>>())?;
    Ok(Some(reply))
}

pub fn query(
    params: serde_json::Value,
    store: &store::Store,
    source_key: &store::SourceKey,
    cancelled: &AtomicBool,
) -> io::Result<serde_json::Value> {
    let params: SpecialProjectsParams = serde_json::from_value(params)?;
    SpecialProjects::validate(&params).map_err(dashboard_core::invalid)?;
    serde_json::to_value(complete_reply(store, source_key, &params, cancelled)?).map_err(Into::into)
}
#[cfg(test)]
mod tests;
