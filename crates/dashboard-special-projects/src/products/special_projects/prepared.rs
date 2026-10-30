//! One all-country projection. Queries build selected episodes from change intervals.
use super::*;
use dashboard_base::prepared::frame_changes::FrameChanges;
use dashboard_base::prepared::series::{self, Accumulator, Reader};
use std::collections::BTreeSet;

pub mod block;
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
/// The observations of holder `tag` that the changed-owner path skipped (the
/// hours of `skipped` from the first it missed), as observing it again with
/// the projects it held there makes them: its last observation becomes the
/// last such hour under its identity there, and its completions' compact
/// runs advance through the hours its completions would have been observed.
/// Everything else those observations make is unchanged or already counted
/// ([`completion_stream::Stream::repeat`]).
fn catch_up(
    countries: &mut BTreeMap<String, Country>,
    completions: &mut completion_stream::Stream,
    behind: &mut BTreeMap<String, (usize, (u32, u32))>,
    skipped: &[(i32, bool)],
    tag: &str,
) {
    let Some((first, identity)) = behind.remove(tag) else {
        return;
    };
    let hours = &skipped[first..];
    if let (Some(country), Some(&(last, _))) = (countries.get_mut(tag), hours.last()) {
        country.last = Some((last, identity));
    }
    let observed: Vec<i32> = hours
        .iter()
        .filter(|(_, observed)| *observed)
        .map(|&(hour, _)| hour)
        .collect();
    completions.advance(identity, &observed);
}

/// Every holder behind takes the observations it missed ([`catch_up`]), and
/// the skipped hours are forgotten.
fn catch_up_all(
    countries: &mut BTreeMap<String, Country>,
    completions: &mut completion_stream::Stream,
    behind: &mut BTreeMap<String, (usize, (u32, u32))>,
    skipped: &mut Vec<(i32, bool)>,
) {
    let tags: Vec<String> = behind.keys().cloned().collect();
    for tag in tags {
        catch_up(countries, completions, behind, skipped, &tag);
    }
    skipped.clear();
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
    #[serde(skip)]
    catalogue_pending: bool,
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
    /// Countries whose completions were observed with the projects they hold
    /// now: observing them again with those projects changes nothing but
    /// their counted warnings ([`completion_stream::Stream::repeat`]).
    #[serde(skip)]
    completions_current: BTreeSet<String>,
    /// The hours at which the changed-owner path skipped the holders whose
    /// rows did not change, in order, each with whether their completions
    /// would have been observed there; cleared once no holder is behind.
    #[serde(skip)]
    skipped: Vec<(i32, bool)>,
    /// Each holder skipped since then: the first of `skipped` it missed and
    /// its identity there. It takes those observations ([`catch_up`]) before
    /// it is observed again, before any hour that observes every holder, and
    /// before a flush.
    #[serde(skip)]
    behind: BTreeMap<String, (usize, (u32, u32))>,
    /// A kept hour's owners whose project rows changed since the hour
    /// before, found by its fold, which a recorded delta would name
    /// ([`Self::observe_kept`]).
    #[serde(skip)]
    kept_owners: Option<BTreeSet<u32>>,
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
    /// The countries whose names [`Self::refresh_names`] revisited, counted
    /// in tests.
    #[cfg(any(test, feature = "test-support"))]
    #[serde(skip)]
    refreshed: usize,
    #[cfg(any(test, feature = "test-support"))]
    #[serde(skip)]
    test_changes: Option<dashboard_base::prepared::frame_changes::FrameChangeTracker>,
}
const LEXICAL: u64 = 1;
/// The tables this observer reads of a frame, besides the countries' keys
/// and scoped names' inputs, which a block read by its changes watches
/// whole (`block::fields`): a frame built on the last one read holds their
/// hour's rows.
pub const WATCHED_WHOLE: [TableId; 7] = [
    TableId::Projects,
    TableId::ProjectPrograms,
    TableId::ProjectHistory,
    TableId::Characters,
    TableId::CharacterPortraits,
    TableId::LexicalTokens,
    TableId::CountryLinks,
];
/// A kept hour's changes as its scoped names' inputs read them: a table its
/// update rebuilt may have changed only in fields the block reader does not
/// watch (`watched`), and anywhere when it watches none of it; any other
/// table did not change ([`dashboard_text::triggers::inputs::QuietTables`]).
struct KeptChanges<'a> {
    rows_changed: &'a [bool; replay_data::rows::TABLE_COUNT],
    watched: &'a [replay::container::TableFields],
}
impl dashboard_text::triggers::inputs::TableChanges for KeptChanges<'_> {
    fn table(&self, table: TableId) -> dashboard_text::triggers::inputs::TableChange<'_> {
        use dashboard_text::triggers::inputs::TableChange;
        if !self.rows_changed[table.index()] {
            return TableChange::Same;
        }
        self.watched
            .iter()
            .filter(|watch| watch.table == table)
            .map(|watch| watch.fields)
            .reduce(|all, fields| all | fields)
            .map_or(TableChange::Any, |fields| TableChange::Fields(!fields))
    }
}
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
        #[cfg(any(test, feature = "test-support"))]
        {
            let mut tracker = dashboard_base::prepared::frame_changes::FrameChangeTracker::new();
            tracker.observe(frame, replay_data::format::FormatVersion::CURRENT);
            self.test_changes = Some(tracker);
        }
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
    /// Whether, once configured, an observation can evaluate scoped text,
    /// which reads the trigger kit ([`dashboard_text::triggers::RECORDED_TABLES`]):
    /// only when the bound content holds a project name key, or a
    /// country-prefixed variant of one, that needs scope. Every other text
    /// the observation resolves (unscoped names, specialisations, iteration
    /// counts, people) reads no frame, and a scoped name is evaluated only by
    /// the name scan, which runs only then.
    pub fn reads_trigger_world(&self) -> bool {
        self.scoped
    }
    fn configure_own(&mut self, source: &Source, context: &LoadContext) -> io::Result<()> {
        if self.content_ready {
            return Ok(());
        }
        self.replay = Some(source.replay());
        self.catalogue_pending = true;
        self.metadata.content_error = None;
        // Built from the recording's own catalogue alone, so bound to the recorded content: it
        // serves preparation and requests showing visual mods alike. Its load credits no note:
        // the replies that read it name its artwork by product.
        type BoundContent = Option<(
            Option<catalogue::RecordedBinding>,
            Arc<catalogue::CachedResult<Content>>,
        )>;
        let key = context.content.as_ref().map(catalogue::recorded_binding);
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
                .and_then(|options| {
                    recorded_politics_content(source, options)
                })
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
        let mut changes = self
            .test_changes
            .get_or_insert_with(dashboard_base::prepared::frame_changes::FrameChangeTracker::new)
            .observe(frame, source.replay_ref().format);
        if self.actors.is_none() {
            // Restoring the decoder preserves dictionary change evidence, but
            // the observer's skipped native joins still need their first read.
            let lexical_changed = changes.rows_changed[TableId::LexicalTokens.index()];
            changes.rows_changed.fill(true);
            changes.rows_changed[TableId::LexicalTokens.index()] = lexical_changed;
        }
        self.observe_changed(frame, &changes, cancelled)
    }
    pub fn observe_changed(
        &mut self,
        frame: &DecodedFrame,
        changes: &FrameChanges,
        cancelled: &AtomicBool,
    ) -> io::Result<()> {
        self.own(|state| state.observe_own_changes(frame, changes, None, cancelled))
    }
    /// Observe an hour of a block read by its changes (`block`): on its
    /// whole frame (`whole`), as [`Self::observe_changed`]; otherwise on a
    /// frame holding the hour's rows only of the tables the family watches
    /// whole and the countries' keys (an hour the block reader built on the
    /// last one read, a quiet hour on the last kept frame, or an hour it read
    /// whose frame carries tables built at an earlier hour), whose other
    /// tables changed at most as `changes` and `watched` tell. Those tables
    /// are all this observer reads of a frame but for scoped names, which
    /// then take their last evaluations, every one checked to read the same
    /// without the frame ([`history::NameScan::advances`]). False, having
    /// observed nothing, where that cannot be told without the whole frame:
    /// the hour is then observed on it. `owners` are the owners whose
    /// project rows changed since the hour before, where the fold found
    /// them, which the hour's recorded delta would name.
    pub fn observe_kept(
        &mut self,
        frame: &DecodedFrame,
        changes: &FrameChanges,
        whole: bool,
        watched: &[replay::container::TableFields],
        owners: Option<BTreeSet<u32>>,
        cancelled: &AtomicBool,
    ) -> io::Result<bool> {
        self.own(|state| {
            state.kept_owners = owners;
            let observed = state.observe_kept_own(frame, changes, whole, watched, cancelled);
            state.kept_owners = None;
            observed
        })
    }
    fn observe_kept_own(
        &mut self,
        frame: &DecodedFrame,
        changes: &FrameChanges,
        whole: bool,
        watched: &[replay::container::TableFields],
        cancelled: &AtomicBool,
    ) -> io::Result<bool> {
        let state = self;
        {
            if whole || !state.scoped {
                state.observe_own_changes(frame, changes, None, cancelled)?;
                return Ok(true);
            }
            let kept = KeptChanges {
                rows_changed: &changes.rows_changed,
                watched,
            };
            if !state.continues_kept(frame, changes, &kept) {
                return Ok(false);
            }
            state.observe_own_changes(frame, changes, Some(&kept), cancelled)?;
            Ok(true)
        }
    }
    /// Whether a kept frame's hour observes as its whole frame would: it
    /// follows the hour before, no table watched whole was decoded again
    /// (each is an hour this observer reacts to by its allocations, read
    /// on the whole frame), the country keys it compares are held, and
    /// every scoped name reads the same without the frame.
    fn continues_kept(
        &self,
        frame: &DecodedFrame,
        changes: &FrameChanges,
        kept: &KeptChanges<'_>,
    ) -> bool {
        let Some(actors) = self.actors.as_ref() else {
            return false;
        };
        self.last_hour.and_then(|hour| hour.checked_add(1)) == Some(frame.hour)
            && !WATCHED_WHOLE
                .iter()
                .any(|table| changes.rows_changed[table.index()])
            && (!changes.rows_changed[TableId::Countries.index()] || self.country_keys.is_some())
            && self.names.advances(frame, actors, kept)
    }
    fn observe_own_changes(
        &mut self,
        frame: &DecodedFrame,
        changes: &FrameChanges,
        kept: Option<&KeptChanges<'_>>,
        cancelled: &AtomicBool,
    ) -> io::Result<()> {
        let _cpu = ::replay_frame::cpu::scope_phase("projects:observe");
        let reuse = match kept {
            Some(kept) => history::Reuse::Advance(kept),
            None => history::Reuse::Confirm(changes),
        };
        dashboard_text::metadata::check_cancel(cancelled)?;
        self.values.advance(frame.hour)?;
        self.catalogue_pending = true;
        let previous_hour = self.last_hour.replace(frame.hour);
        let gap = previous_hour.and_then(|hour| hour.checked_add(1)) != Some(frame.hour);
        // Read evidence changes independently of row allocations and country facts.
        let read = changes.slot_read[TableId::Projects.index()];
        let marker = (read && frame.complete::<tables::Projects>().is_ok()).then_some(1);
        // Absence closes the held marker without replacing its last numeric
        // value. Reobserve this single branch-wide scalar so a successful read
        // after an unavailable interval reopens it even when that value is 1.
        self.values
            .observe_if_changed(0, frame.hour, marker, gap, false);
        // A fallback decoder can allocate a new view of the same supplying
        // dictionary. Only a recorded update advances its revision; equal
        // snapshots still count through the change tracker.
        let lexical_changed = self
            .lexical_view
            .as_ref()
            .is_none_or(|view| !view.is(&frame.tables[TableId::LexicalTokens.index()]))
            && (self.lexical_view.is_none()
                || changes.rows_changed[TableId::LexicalTokens.index()]);
        if lexical_changed {
            self.lexical += 1;
        }
        self.lexical_view = Some(frame.tables[TableId::LexicalTokens.index()].identity());
        if lexical_changed || gap {
            self.values
                .observe(LEXICAL, frame.hour, Some(i128::from(self.lexical)), gap);
        }
        let identity_changed = self.actors.is_none()
            || changes.rows_changed[TableId::CountryLinks.index()]
            || changes.rows_changed[TableId::Countries.index()]
                && self.country_keys.as_ref().is_none_or(|keys| {
                    frame.complete::<tables::Countries>().map_or(true, |rows| {
                        !keys.same_changes(rows, changes.rows(TableId::Countries))
                    })
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
                    reuse,
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
                    reuse,
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
                    .observe(frame, actors, read, content, reuse, cancelled)?;
            } else {
                self.names
                    .evaluate_current(frame, actors, content, reuse, cancelled)?;
            }
            content.scoped.clear();
            content
                .scoped
                .push((frame.branch, Arc::new(self.names.take_current())));
        }
        let previous_project_known = self.project_inputs.is_some();
        let previous_history = self.history_inputs.clone();
        let mut reached_dates = BTreeSet::new();
        let changed_owners = self.changed_owners(frame, changes, identity_changed || gap);
        if let Some(owners) = &changed_owners {
            // Only the changed owners' projections: every other owner's rows,
            // and so its projection, are the hour before's.
            let rows = frame
                .complete::<tables::Projects>()
                .expect("a recorded update of a complete table");
            let inputs = Arc::make_mut(
                self.project_inputs
                    .as_mut()
                    .expect("the hour before's projection"),
            );
            for &owner in owners {
                let first = rows.partition_point(|row| row.owner < owner);
                let count = rows[first..].partition_point(|row| row.owner == owner);
                #[cfg(any(test, feature = "test-support"))]
                {
                    self.work.row_visits += count;
                }
                if count == 0 {
                    inputs.remove(&owner);
                } else {
                    inputs.insert(
                        owner,
                        rows[first..first + count]
                            .iter()
                            .map(|row| ProjectInput {
                                facts: ProjectFacts::project(
                                    row,
                                    None,
                                    frame.hour,
                                    self.metadata.start,
                                ),
                                program_reference: row.program_reference,
                            })
                            .collect(),
                    );
                }
            }
            self.project_owners_unmapped = inputs
                .iter()
                .filter(|(owner, _)| actors.known && !self.linked_owners.contains(*owner))
                .map(|(_, rows)| rows.len())
                .sum();
        } else if changes.rows_changed[TableId::Projects.index()] {
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
        // Only some owners' rows changed: their countries alone are observed
        // again (with any whose history changed), and every other project
        // holder repeats what observing it again would count (below).
        let partial = changed_owners.is_some() && !history_status_changed;
        if partial {
            for owner in changed_owners.iter().flatten() {
                if let Some(tag) = self.owner_countries.get(owner) {
                    affected.insert(tag.clone());
                }
            }
        } else if project_change || history_status_changed {
            affected.extend(self.project_countries.iter().cloned());
            for owner in projects.as_ref().into_iter().flat_map(|rows| rows.keys()) {
                if let Some(tag) = self.owner_countries.get(owner) {
                    affected.insert(tag.clone());
                }
            }
        }
        if (partial || !(project_change || history_status_changed))
            && changes.rows_changed[TableId::ProjectHistory.index()]
        {
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
        // Holders the changed-owner path skipped take the observations it
        // stood in for before anything here reads their state.
        if partial {
            for tag in &affected {
                catch_up(
                    &mut self.countries,
                    &mut self.completions,
                    &mut self.behind,
                    &self.skipped,
                    tag,
                );
            }
        } else {
            catch_up_all(
                &mut self.countries,
                &mut self.completions,
                &mut self.behind,
                &mut self.skipped,
            );
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
                self.completions_current.insert(tag.to_owned());
            } else {
                self.completions_current.remove(tag);
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
        if partial {
            // The holders whose rows did not change: observing them again
            // with the projects they hold would change nothing but their
            // last observation, their open episodes' compact runs (where
            // their completions are observed) and the warnings counted where
            // the span changed. The counts go now; the rest when next read.
            let observed = projects.is_some() && read;
            self.skipped.push((frame.hour, observed));
            let at = self.skipped.len() - 1;
            for tag in self.project_countries.difference(&affected) {
                if let Some(&(country_id, slot)) = actors.tags.get(tag.as_bytes()) {
                    self.behind
                        .entry(tag.clone())
                        .or_insert((at, (slot, country_id)));
                    if observed {
                        self.completions.repeat(
                            (slot, country_id),
                            frame.hour,
                            span_changed && project_change,
                        );
                    }
                }
            }
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
    /// The owners whose Projects rows this hour's recorded update changed,
    /// when the hour can project those owners' rows alone: the table changed
    /// by one recorded update from the hour before, whose projection is held,
    /// without an identity change or a gap (`whole`); and every project
    /// holder whose rows did not change had its completions observed with the
    /// projects it holds, with warnings the stream knows, so observing it
    /// again would change nothing but the counts it repeats. None projects
    /// the whole table, as when no update is recorded.
    fn changed_owners(
        &self,
        frame: &DecodedFrame,
        changes: &FrameChanges,
        whole: bool,
    ) -> Option<BTreeSet<u32>> {
        if whole
            || !changes.rows_changed[TableId::Projects.index()]
            || self.project_inputs.is_none()
            || frame.complete::<tables::Projects>().is_err()
        {
            return None;
        }
        // Projects rows are keyed by owner, then project.
        let owners: BTreeSet<u32> = match changes.rows(TableId::Projects) {
            Some((changed, deleted)) => changed
                .iter()
                .map(|row| row.key)
                .chain(deleted.iter().copied())
                .map(|key| (key >> 32) as u32)
                .collect(),
            None => self.kept_owners.clone()?,
        };
        let actors = self.actors.as_ref()?;
        let touched: BTreeSet<&str> = owners
            .iter()
            .filter_map(|owner| self.owner_countries.get(owner))
            .map(String::as_str)
            .collect();
        self.project_countries
            .iter()
            .filter(|tag| !touched.contains(tag.as_str()))
            .all(|tag| {
                self.completions_current.contains(tag)
                    && actors
                        .tags
                        .get(tag.as_bytes())
                        .is_none_or(|&(country_id, slot)| {
                            self.completions.knows((slot, country_id))
                        })
            })
            .then_some(owners)
    }

    /// Every holder the changed-owner path skipped takes the observations it
    /// stood in for ([`catch_up`]).
    fn catch_up_all(&mut self) {
        catch_up_all(
            &mut self.countries,
            &mut self.completions,
            &mut self.behind,
            &mut self.skipped,
        );
    }

    pub fn buffered_bytes(&self) -> usize {
        self.values
            .bytes()
            .saturating_add(self.completions.buffered_bytes(
                self.content.as_ref(),
                self.metadata.content_error.as_deref(),
            ))
            .saturating_add(if self.catalogue_pending {
                let mut own = self.refusals.clone();
                catalogue::own_refusals(&mut own, || {
                    series::catalogue_bytes(&self.keys, &self.publication_metadata())
                })
            } else {
                0
            })
    }
    fn refresh_names(
        &mut self,
        frame: &DecodedFrame,
        affected: &BTreeSet<String>,
        cancelled: &AtomicBool,
    ) -> io::Result<()> {
        for tag in affected {
            dashboard_text::metadata::check_cancel(cancelled)?;
            #[cfg(any(test, feature = "test-support"))]
            {
                self.refreshed += 1;
            }
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
        // What is published and saved holds every skipped observation.
        self.catch_up_all();
        self.completions.flush(
            writer,
            branch,
            self.replay.as_deref(),
            self.content.as_mut(),
            self.metadata.content_error.as_deref(),
            cancelled,
        )?;
        self.values.flush(writer, &section(branch), 0, cancelled)?;
        self.metadata = self.publication_metadata();
        series::write_catalogue(
            writer,
            "special-projects",
            branch,
            &self.keys,
            &self.metadata,
        )?;
        self.metadata.progress.clear();
        self.metadata.scoped_warnings.clear();
        self.catalogue_pending = false;
        Ok(())
    }

    fn publication_metadata(&self) -> Metadata {
        let mut metadata = self.metadata.clone();
        metadata.progress = self
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
            metadata.definitions_empty = content.definitions.projects.is_empty();
            metadata.warnings = content.definitions.warnings.clone();
            metadata
                .warnings
                .extend(content.catalogue.verification.iter().cloned());
            metadata
                .warnings
                .extend(content.catalogue.warnings.iter().cloned());
            metadata
                .warnings
                .extend(content.catalogue.reader_warning_lines());
        }
        metadata.scoped_warnings = self
            .keys
            .keys()
            .filter_map(|tag| {
                let warnings = self.names.names.warnings(tag);
                (!warnings.is_empty()).then(|| (tag.clone(), warnings))
            })
            .collect();
        metadata
    }
}

struct Interval {
    first: i32,
    last: i32,
    observation: Observation,
}
/// The project observations of `key` through `through`, each interval ending by then.
fn intervals(
    reader: &mut Reader<'_>,
    key: u64,
    through: i32,
    cancelled: &AtomicBool,
) -> io::Result<Vec<Interval>> {
    let mut result = Vec::new();
    let _cpu = replay_frame::cpu::scope_phase("projects:query_intervals");
    let mut looks = Looks::default();
    let mut cursor = 0;
    loop {
        dashboard_text::metadata::check_cancel(cancelled)?;
        let page = reader.events(&[key], i32::MIN, through, cursor, 256)?;
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
                last: run.last_hour.min(through),
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
/// The identity runs of `key` through `through`, each ending by then.
fn identities(
    reader: &mut Reader<'_>,
    key: u64,
    through: i32,
    cancelled: &AtomicBool,
) -> io::Result<Vec<CountryIdentityRun>> {
    let _cpu = replay_frame::cpu::scope_phase("projects:query_identities");
    let mut identities = Vec::new();
    let mut cursor = 0;
    loop {
        dashboard_text::metadata::check_cancel(cancelled)?;
        let page = reader.events(&[key], i32::MIN, through, cursor, 256)?;
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
                .last_hour
                .min(through);
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

/// The reply `params` ask of the prepared pages, their history read through `through` alone.
fn complete_reply(
    store: &store::Store,
    source_key: &store::SourceKey,
    params: &SpecialProjectsParams,
    through: i32,
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
        Some(&key) => intervals(&mut reader, key, through, cancelled)?,
        None => Vec::new(),
    };
    let mut history = country_history::RecordedHistories::default();
    let mut all_ids = match metadata.progress.get(&params.tag) {
        Some(country) => identities(&mut reader, country.identity, through, cancelled)?,
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
            country.identities = Arc::new(identities(
                &mut reader,
                root_keys.identity,
                through,
                cancelled,
            )?);
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
    let cpu = replay_frame::cpu::scope_phase("projects:query_episodes");
    for interval in &selected_intervals {
        let _cpu = replay_frame::cpu::scope_phase("projects:query_interval");
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
    drop(cpu);
    let _cpu = replay_frame::cpu::scope_phase("projects:query_reads_finish");
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

/// The prepared reply, its art kept as the operands preparation recorded
/// ([`super::present_reply`] draws it).
/// `params`' reply from the prepared pages, their history read through `through`: the last hour
/// of the prefix the query admitted.
pub fn query(
    params: serde_json::Value,
    store: &store::Store,
    source_key: &store::SourceKey,
    through: i32,
    cancelled: &AtomicBool,
) -> io::Result<SpecialProjectsReply> {
    let params: SpecialProjectsParams = serde_json::from_value(params)?;
    SpecialProjects::validate(&params).map_err(dashboard_core::invalid)?;
    Ok(
        complete_reply(store, source_key, &params, through, cancelled)?.unwrap_or_else(|| {
            build_reply(Builder::default(), &params.country(), false, Vec::new()).0
        }),
    )
}

/// Current cards use preparation's observation, presentation and episode
/// builders. A fresh observation has no episode history: keep only in-progress
/// projects, and retain its observation bound rather than dating a start.
pub fn frame_reply(
    source: &Source,
    context: &LoadContext,
    params: &SpecialProjectsParams,
    hour: i32,
    cancelled: &AtomicBool,
) -> io::Result<SpecialProjectsReply> {
    use dashboard_base::prepared::{frame_changes::FrameChangeTracker, query::progressive};
    let branch = params.country().branch();
    let mut prepared = Prepared::new(source.replay_ref(), branch);
    prepared.configure(source, context)?;
    let mut tables = WATCHED_WHOLE.to_vec();
    tables.extend([TableId::Campaign, TableId::Countries]);
    if prepared.reads_trigger_world() {
        tables.extend_from_slice(dashboard_text::triggers::RECORDED_TABLES);
    }
    tables.sort_unstable();
    tables.dedup();
    let mut builder = Builder::default();
    let mut identities = Vec::new();
    let mut refusals = catalogue::Refusals::default();
    let mut selected = if params.selection_tags.is_empty() {
        None
    } else {
        let (store, key) = dashboard_base::prepared::files(source, context.content.as_ref())?;
        let accepted = dashboard_base::prepared::coverage::product_coverage(
            source,
            context.content.as_ref(),
            branch,
            "country-identities",
            cancelled,
        )?;
        let histories = progressive::histories(
            &store,
            &key,
            branch,
            accepted.as_ref(),
            &BTreeMap::new(),
            &BTreeMap::new(),
            source.replay_ref(),
            &params.selection_tags,
            cancelled,
        )?;
        Some(country_history::selected_identities(
            &histories,
            &params.selection_tags,
        ))
    };
    if let Some(frame) = progressive::frame_at(source, branch, hour, &tables, cancelled)? {
        let changes = FrameChangeTracker::new().observe(&frame, source.replay_ref().format);
        prepared.observe_changed(&frame, &changes, cancelled)?;
        if let Some(selected) = &mut selected {
            for tag in &params.selection_tags {
                if let Some(observation) = prepared
                    .countries
                    .get(tag)
                    .and_then(|country| country.current.as_ref())
                {
                    selected.insert(observation.identity);
                }
            }
        }
        if let Some(country) = prepared.countries.get(&params.tag)
            && let Some(mut observation) = country.current.clone()
        {
            if selected
                .as_ref()
                .is_some_and(|selected| !selected.contains(&observation.identity))
            {
                let mut reply = build_reply(builder, &params.country(), false, Vec::new()).0;
                reply.unavailable =
                    Some("The selected country's native holder is still being prepared.".into());
                return Ok(reply);
            }
            if let Some(projects) = &mut observation.projects {
                projects.retain(|(facts, _)| {
                    matches!(
                        facts.state,
                        ProjectState::Running | ProjectState::Paused | ProjectState::Unknown
                    )
                });
                // Structural observations keep progress in its numeric channel.
                // Current cards need that channel's value at the requested hour.
                for (facts, _) in projects {
                    let key = prepared.countries[&params.tag].progress
                        [&(facts.local_id, facts.definition_id)];
                    facts.progress = prepared
                        .values
                        .state(key)
                        .and_then(|state| state.last)
                        .map_or(0, |run| i32::try_from(run.value).expect("project progress"));
                }
            }
            let facts = observation
                .projects
                .as_ref()
                .into_iter()
                .flatten()
                .map(|(facts, _)| facts.clone())
                .collect::<Vec<_>>();
            identities.push(CountryIdentityRun {
                first_hour: hour,
                last_hour: hour,
                slot: observation.identity.0,
                country_id: observation.identity.1,
            });
            apply_interval(
                &mut builder,
                &params.tag,
                &Interval {
                    first: hour,
                    last: hour,
                    observation,
                },
                hour,
                hour,
                &facts,
                &mut refusals,
            );
            if !builder.available.is_empty() {
                builder.reads.push(hour);
            }
        }
    }
    let mut warnings = prepared.metadata.warnings;
    warnings.extend(refusals.lines("Project texts"));
    let mut reply = build_reply(
        builder,
        &params.country(),
        prepared.metadata.definitions_empty,
        warnings,
    )
    .0;
    if let Some(reason) = prepared.metadata.content_error {
        reply.warnings.push(reason);
    }
    for project in &mut reply.projects {
        project.id = format!("@{hour}:{}", project.id);
    }
    append_identities(&mut reply, &identities.iter().collect::<Vec<_>>())?;
    Ok(reply)
}
#[cfg(test)]
mod tests;
