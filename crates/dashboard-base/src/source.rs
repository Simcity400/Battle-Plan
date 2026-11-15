//! One opened recording shared by every widget and map that reads it.

use job_priority::SpawnCarried as _;
use std::{
    any::{Any, TypeId},
    collections::HashMap,
    io,
    path::{Path, PathBuf},
    sync::{
        Arc, Mutex, Weak,
        atomic::{AtomicBool, AtomicU64, Ordering},
    },
};

use installed_content_core::content::diagnostics::{self, Diagnostics};
use installed_content_core::{ResolveOptions, content::ResolvedContent};
use replay::campaign::Replay;

use crate::{
    ReplayMetadata,
    content::{ContentKey, content_key, name_localisation},
    metadata::{self, MetadataRefusals, ResidentMetadata, Warnings},
    presentation::{Admitted, Binding, Cancel, Presentation, Presented},
    products::ContentOptions,
};
use installed_content_country_names::country_names::CountryNames;
use installed_content_localisation::localisation::Localisation;
use replay::campaign::SeekResult;

/// Identity of an opened recording revision inside the running service.
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub struct SourceKey {
    pub source_id: String,
    pub revision: String,
}

type SharedState = Arc<dyn Any + Send + Sync>;

struct StateEntry {
    value: SharedState,
    append: Option<fn(&SharedState) -> SharedState>,
}

/// The recording's own presentation as last mounted, over the content it was mounted on
/// ([`Source::recorded_presented`]).
#[derive(Default)]
struct RecordedMount(Option<(Weak<ResolvedContent>, Arc<Presented>)>);

/// The content resolved for the folders last asked for, with its read set, and the content of
/// earlier folders while work still holds it: asked for again, that is reused, so all work on
/// one folders reads, and reports, one read set.
#[derive(Default)]
struct ContentCache {
    bound: Option<(ResolveOptions, Arc<ResolvedContent>)>,
    held: Vec<(ResolveOptions, Weak<ResolvedContent>)>,
    /// Content a preparation resolves ahead of need ([`Source::resolving_ahead`]), unbound until
    /// [`Source::content`] asks for it, and released when that preparation ends.
    ahead: Option<ContentAhead>,
}

struct ContentAhead {
    options: ResolveOptions,
    /// The preparation that started it, and releases it.
    owner: u64,
    state: AheadState,
}

enum AheadState {
    /// `None` once stopped because its preparation ended, not because it was cancelled.
    Resolving(std::sync::mpsc::Receiver<Option<io::Result<ResolvedContent>>>),
    /// Waited for by [`Source::own_content`], which binds nothing.
    Resolved(Arc<ResolvedContent>),
}

impl ContentCache {
    /// The content of these folders work reads now, bound or held, if any.
    fn find(&self, options: &ResolveOptions) -> Option<Arc<ResolvedContent>> {
        match &self.bound {
            Some((old, content)) if same_folders(old, options) => Some(Arc::clone(content)),
            _ => self
                .held
                .iter()
                .filter(|(old, _)| same_folders(old, options))
                .find_map(|(_, content)| content.upgrade()),
        }
    }

    /// The content resolved ahead for these folders, waited for while it resolves, its error
    /// included (`Interrupted` when its preparation was cancelled); taken out of the cache when
    /// `claim`ed, else kept for whoever asks next. `None` when none is, or its resolution stopped
    /// as its preparation ended or panicked: it is resolved where asked for.
    fn ahead(
        &mut self,
        options: &ResolveOptions,
        claim: bool,
    ) -> Option<io::Result<Arc<ResolvedContent>>> {
        let ahead = self
            .ahead
            .take_if(|ahead| same_folders(&ahead.options, options))?;
        let content = match ahead.state {
            AheadState::Resolved(content) => content,
            AheadState::Resolving(receiver) => match receiver.recv() {
                Ok(Some(Ok(content))) => Arc::new(content),
                Ok(Some(Err(error))) => return Some(Err(error)),
                Ok(None) | Err(_) => return None,
            },
        };
        if !claim {
            self.ahead = Some(ContentAhead {
                options: ahead.options,
                owner: ahead.owner,
                state: AheadState::Resolved(Arc::clone(&content)),
            });
        }
        Some(Ok(content))
    }

    /// Binds `content`, holding what was bound while work still holds it.
    fn bind(&mut self, options: &ResolveOptions, content: &Arc<ResolvedContent>) {
        let _ = content.layers.report_uncovered_reads();
        self.held
            .retain(|(old, held)| held.strong_count() > 0 && !same_folders(old, options));
        let previous = self.bound.replace((options.clone(), Arc::clone(content)));
        if let Some((old, previous)) = previous
            && !Arc::ptr_eq(&previous, content)
        {
            self.held.push((old, Arc::downgrade(&previous)));
        }
    }
}

/// Ends a preparation's resolution ahead ([`Source::resolving_ahead`]), even when it unwinds.
struct ReleaseAhead<'scope> {
    source: &'scope Source,
    resolver: Option<(u64, std::thread::ScopedJoinHandle<'scope, ()>)>,
    finished: &'scope AtomicBool,
}

impl Drop for ReleaseAhead<'_> {
    fn drop(&mut self) {
        self.finished.store(true, Ordering::Release);
        if let Some((owner, resolver)) = self.resolver.take() {
            // Joined here, so a resolver that panicked leaves its content to be resolved where
            // asked for rather than panicking the preparation.
            let _ = resolver.join();
            let mut cache = self
                .source
                .content
                .lock()
                .unwrap_or_else(|error| error.into_inner());
            if cache
                .ahead
                .as_ref()
                .is_some_and(|ahead| ahead.owner == owner)
            {
                cache.ahead = None;
            }
        }
    }
}

/// A recording of an older layout, refused as the reason users see.
fn refuse_older(error: io::Error) -> io::Error {
    if replay::campaign::is_older_layout(&error) {
        dashboard_core::refused(error.to_string())
    } else {
        error
    }
}

fn block_windows(replay: &Replay) -> HashMap<u64, Vec<crate::span::HourSpan>> {
    let mut windows: HashMap<_, Vec<_>> = HashMap::new();
    for block in &replay.index {
        windows
            .entry(block.branch)
            .or_default()
            .push(crate::span::HourSpan {
                first_hour: block.first_hour,
                last_hour: block.last_hour,
            });
    }
    windows
}

/// The held read-only recording, its presentation metadata, resolved game content and
/// every product's per-source cache. Dropping the last handle releases it all.
pub struct Source {
    key: SourceKey,
    path: PathBuf,
    replay: Arc<Replay>,
    block_windows: HashMap<u64, Vec<crate::span::HourSpan>>,
    metadata: ResidentMetadata,
    /// The metadata with country names resolved for the last folders it was asked for.
    named: Mutex<Option<(NamesKey, ResidentMetadata, Diagnostics)>>,
    content: Mutex<ContentCache>,
    /// The game facts the recording carries, opened once ([`crate::facts::recorded`]).
    facts: std::sync::OnceLock<Result<Arc<installed_content_core::content::facts::Pack>, String>>,
    tables: std::sync::OnceLock<Option<engine_executable::executable_tables::Tables>>,
    /// What work on this source is bound to now ([`Self::bind`]), swapped in whole, never
    /// changed in place; `None` until the service binds it.
    binding: Mutex<Option<Arc<Binding>>>,
    state: Mutex<HashMap<TypeId, StateEntry>>,
}

/// What country names are resolved for: the folders. Their language is the recording's.
pub(crate) type NamesKey = Option<ContentKey>;

fn country_observation(country: &crate::CountryMetadata) -> (&str, &str, &str, i32) {
    (
        &country.id,
        &country.tag,
        &country.observed_branch_id,
        country.observed_hour,
    )
}

/// A recording's facts and the executable tables they carry.
type Recorded = (
    Result<Arc<installed_content_core::content::facts::Pack>, String>,
    Option<engine_executable::executable_tables::Tables>,
);

fn recorded(replay: &Arc<Replay>) -> Recorded {
    let facts = crate::facts::recorded(replay);
    let tables = facts.as_ref().ok().and_then(|pack| pack.tables().ok());
    (facts, tables)
}

impl Source {
    /// Reads the file and its initial country catalogue. Cancellation is
    /// checked between reads and decoded frames.
    pub fn open(key: SourceKey, path: PathBuf, cancelled: &AtomicBool) -> io::Result<Self> {
        metadata::check_cancel(cancelled)?;
        let replay = Arc::new(Replay::open_timelines(&path).map_err(refuse_older)?);
        metadata::check_cancel(cancelled)?;
        let (facts, tables) = recorded(&replay);
        // The catalogue is read with the recording's own executable tables.
        engine_executable::executable_tables::scope(tables, || {
            Self::read(key, path, replay, (facts, tables), cancelled)
        })
    }

    pub fn from_replay(
        key: SourceKey,
        path: PathBuf,
        replay: Arc<Replay>,
        cancelled: &AtomicBool,
    ) -> io::Result<Self> {
        let recorded = recorded(&replay);
        engine_executable::executable_tables::scope(recorded.1, || {
            Self::read(key, path, replay, recorded, cancelled)
        })
    }

    fn read(
        key: SourceKey,
        path: PathBuf,
        replay: Arc<Replay>,
        (facts, tables): Recorded,
        cancelled: &AtomicBool,
    ) -> io::Result<Self> {
        let metadata = metadata::read_resident(&replay, &path, cancelled)?;
        Ok(Self {
            key,
            path,
            block_windows: block_windows(&replay),
            replay,
            metadata,
            named: Mutex::new(None),
            content: Mutex::default(),
            facts: facts.into(),
            tables: tables.into(),
            binding: Mutex::new(None),
            state: Mutex::new(HashMap::new()),
        })
    }

    pub fn key(&self) -> &SourceKey {
        &self.key
    }

    pub fn path(&self) -> &Path {
        &self.path
    }

    pub fn campaign_key(&self) -> &str {
        &self.metadata.campaign_key
    }

    pub fn replay(&self) -> Arc<Replay> {
        Arc::clone(&self.replay)
    }

    pub fn replay_ref(&self) -> &Replay {
        &self.replay
    }

    /// The game facts the recording carries, or why it carries none.
    pub fn facts(&self) -> Result<Arc<installed_content_core::content::facts::Pack>, String> {
        self.facts
            .get_or_init(|| crate::facts::recorded(&self.replay))
            .clone()
    }

    /// The executable tables the recording carries, which every piece of work on it reads in
    /// its scope ([`Self::in_scope`]); `None` when it carries none.
    pub fn tables(&self) -> Option<engine_executable::executable_tables::Tables> {
        *self
            .tables
            .get_or_init(|| self.facts().ok().and_then(|pack| pack.tables().ok()))
    }

    /// Runs `work` with the recording's executable tables in scope.
    pub fn in_scope<T>(&self, work: impl FnOnce() -> T) -> T {
        engine_executable::executable_tables::scope(self.tables(), work)
    }

    /// The recording's content played back over these folders: its own facts and the
    /// installed art ([`installed_content_core::playback`]).
    fn play(&self, options: &ResolveOptions) -> io::Result<ResolvedContent> {
        installed_content_core::playback(&self.replay.identity, self.facts(), options)
    }
    pub fn block_windows(&self, branch: u64) -> &[crate::span::HourSpan] {
        self.block_windows.get(&branch).map_or(&[], Vec::as_slice)
    }

    /// The initial observed catalogue, not a census of every campaign hour, with
    /// country names once [`Self::name_countries`] has run and tags before.
    pub fn metadata(&self) -> Result<ReplayMetadata, metadata::MetadataError> {
        let named = self.named.lock().unwrap_or_else(|error| error.into_inner());
        let mut metadata = match named.as_ref() {
            Some((_, metadata, diagnostics)) => {
                diagnostics.credit();
                let mut metadata = metadata.wire()?;
                diagnostics.append_to(&mut metadata.warnings);
                metadata
            }
            None => self.metadata.wire()?,
        };
        drop(named);
        self.append_content_warnings(&mut metadata.warnings);
        metadata.build_warnings = metadata::build_warnings(&self.replay);
        Ok(metadata)
    }

    /// Read resident catalogue fields without copying or formatting wire diagnostics.
    pub fn with_metadata<T>(&self, read: impl FnOnce(&ReplayMetadata) -> T) -> T {
        let named = self.named.lock().unwrap_or_else(|error| error.into_inner());
        read(
            named
                .as_ref()
                .map_or(&self.metadata.value, |(_, metadata, diagnostics)| {
                    diagnostics.credit();
                    &metadata.value
                }),
        )
    }

    /// Reports this request's reads and the bound content changes.
    pub fn append_content_warnings(&self, warnings: &mut Vec<String>) {
        diagnostics::current().append_to(warnings);
        if let Some((_, content)) = &self
            .content
            .lock()
            .unwrap_or_else(|error| error.into_inner())
            .bound
        {
            content.append_changed_to(warnings);
        }
    }

    /// The content of the folders work admitted under `options` reads: the bound content,
    /// content of these folders work still holds ([`ContentCache`]), or content resolved ahead
    /// for them. Resolved again when there is none, but never bound, so a reply's diagnostics
    /// never rebind what other work reads.
    fn own_content(&self, options: &ResolveOptions) -> io::Result<Arc<ResolvedContent>> {
        let mut cache = self
            .content
            .lock()
            .unwrap_or_else(|error| error.into_inner());
        if let Some(content) = cache.find(options) {
            return Ok(content);
        }
        if let Some(resolved) = cache.ahead(options, false) {
            return resolved;
        }
        drop(cache);
        Ok(Arc::new(self.play(options)?))
    }

    /// Name the layers that changed since the recording in the content a reply was admitted
    /// under: its own folders' content, whatever the source has bound since, so a reply never
    /// names another content's changes nor loses its own.
    pub fn append_changed_warnings(
        &self,
        warnings: &mut Vec<String>,
        content: Option<&ContentOptions>,
    ) {
        if let Some(own) = content.and_then(|content| self.own_content(&content.options).ok()) {
            own.append_changed_to(warnings);
        }
    }

    /// What the binding a content reply was admitted under cannot show
    /// ([`Presented::append_to`]), from the mount its entry resolved, never the source's binding
    /// now. Work that reads no content or shows no visual mod names nothing, and work cancelled
    /// while the mount was resolved names nothing either.
    pub fn append_presentation_warnings(
        &self,
        warnings: &mut Vec<String>,
        content: Option<&ContentOptions>,
    ) {
        if let Some(content) = content.filter(|content| {
            content.presentation.content_work() && content.presentation.shows_mods()
        }) && let Ok(presented) = self.presented_for(content)
        {
            presented.append_to(warnings);
        }
    }

    /// Resolves the catalogue's names in the recording's language at each
    /// country's observed hour; without content they stay tags, with a warning.
    pub fn name_countries(&self, content: Option<&ContentOptions>) -> io::Result<()> {
        let key = content_key(content);
        let mut named = self.named.lock().unwrap_or_else(|error| error.into_inner());
        if let Some((cached, _, diagnostics)) = named.as_ref()
            && *cached == key
        {
            diagnostics.credit();
            return Ok(());
        }
        let (metadata, diagnostics) =
            diagnostics::capture(|| self.named_metadata(name_localisation(self, content)));
        diagnostics.credit();
        *named = Some((key, metadata?, diagnostics));
        Ok(())
    }

    /// The catalogue named against `localisation`, or tagged with its warning.
    pub(crate) fn named_metadata(
        &self,
        localisation: Result<Arc<Localisation>, String>,
    ) -> io::Result<ResidentMetadata> {
        self.named_metadata_from(localisation, None)
    }

    fn named_metadata_from(
        &self,
        localisation: Result<Arc<Localisation>, String>,
        previous: Option<&ResidentMetadata>,
    ) -> io::Result<ResidentMetadata> {
        self.try_named_metadata_from(localisation, previous)
            .map_err(|error| io::Error::other(format!("Country metadata naming failed: {error}")))
    }

    fn try_named_metadata_from(
        &self,
        localisation: Result<Arc<Localisation>, String>,
        previous: Option<&ResidentMetadata>,
    ) -> io::Result<ResidentMetadata> {
        let mut metadata = self.metadata.copy_presentation()?;
        let saved_countries: HashMap<_, _> = previous
            .into_iter()
            .flat_map(|previous| &previous.countries)
            .map(|country| (country_observation(country), country))
            .collect();
        let reuse_warnings = localisation.is_err();
        let reason = match localisation {
            Err(reason) => {
                for country in &mut metadata.countries {
                    if let Some(saved) = saved_countries.get(&country_observation(country)) {
                        country.name.clone_from(&saved.name);
                    }
                }
                reason
            }
            Ok(localisation) => {
                let mut unnamed = 0;
                let mut frames = std::collections::BTreeMap::new();
                for country in &mut metadata.countries {
                    if let Some(saved) = saved_countries.get(&country_observation(country)) {
                        country.name.clone_from(&saved.name);
                        continue;
                    }
                    let at = (country.observed_branch_id.clone(), country.observed_hour);
                    let frame = match frames.entry(at) {
                        std::collections::btree_map::Entry::Occupied(entry) => entry.into_mut(),
                        std::collections::btree_map::Entry::Vacant(entry) => {
                            let frame = if let Ok(branch) = country.observed_branch_id.parse() {
                                let tables =
                                    Some(&installed_content_country_names::country_names::TABLES[..]);
                                match self.replay.seek_selected(
                                    branch,
                                    country.observed_hour,
                                    tables,
                                ) {
                                    Ok(SeekResult::Frame(frame)) => {
                                        Some(CountryNames::read(&frame, &localisation))
                                    }
                                    Err(error)
                                        if engine_allocation::is_allocation_error(&error) =>
                                    {
                                        return Err(error);
                                    }
                                    _ => None,
                                }
                            } else {
                                None
                            };
                            entry.insert(frame)
                        }
                    };
                    let name = frame
                        .as_ref()
                        .zip(country.id.parse::<u32>().ok())
                        .and_then(|(names, slot)| names.name(slot).ok());
                    match name {
                        Some(name) => country.name = metadata::short_text(&name, 512),
                        None => unnamed += 1,
                    }
                }
                if unnamed > 0 {
                    replay_data::codec::try_format(format_args!(
                        "{unnamed} country names could not be resolved from the recorded content and show their tag."
                    ))?
                } else {
                    String::new()
                }
            }
        };
        let mut warnings = Warnings::new()?;
        warnings.import(&self.metadata.warnings, &self.metadata.refusals.warnings)?;
        if !reason.is_empty() {
            warnings.warn(&reason)?;
        }
        if reuse_warnings && let Some(previous) = previous {
            warnings.import(&previous.warnings, &previous.refusals.warnings)?;
        }
        let mut reasons = Warnings::new()?;
        if let Some(damaged) = &self.metadata.damaged {
            reasons.import(&damaged.reasons, &self.metadata.refusals.reasons)?;
        }
        let (values, warning_refusals) = warnings.finish();
        metadata.warnings = values;
        let (values, reason_refusals) = reasons.finish();
        if let Some(damaged) = &mut metadata.damaged {
            damaged.reasons = values;
        }
        Ok(ResidentMetadata {
            value: metadata,
            refusals: MetadataRefusals {
                warnings: warning_refusals,
                reasons: reason_refusals,
            },
        })
    }

    /// The recording's verified game content for the configured folders,
    /// resolved once and reused until the folders change, and while work holds it after.
    pub fn content(&self, options: &ResolveOptions) -> io::Result<Arc<ResolvedContent>> {
        loop {
            let (content, rebind) = {
                let mut cache = self.content.lock().map_err(|error| io::Error::other(error.to_string()))?;
                match cache.find(options) {
                    Some(content) => {
                        let rebind = !cache.bound.as_ref().is_some_and(|(_, bound)| Arc::ptr_eq(bound, &content));
                        (content, rebind)
                    }
                    None => {
                        let content = match cache.ahead(options, true) {
                            Some(resolved) => resolved?,
                            None => {
                                let _memory = engine_allocation::labels::enter(
                                    engine_allocation::labels::Owner::RUN,
                                    engine_allocation::labels::Kind::Content,
                                );
                                Arc::new(self.play(options)?)
                            }
                        };
                        cache.bind(options, &content);
                        (content, false)
                    }
                }
            };
            if content.layers.admit_generation().is_err() {
                return self.refresh_content(options, &content);
            }
            if !rebind { return Ok(content); }
            let mut cache = self.content.lock().map_err(|error| io::Error::other(error.to_string()))?;
            if !cache.find(options).is_some_and(|current| Arc::ptr_eq(&current, &content)) {
                continue;
            }
            if !cache.bound.as_ref().is_some_and(|(_, bound)| Arc::ptr_eq(bound, &content)) {
                cache.bind(options, &content);
            }
            return Ok(content);
        }
    }

    pub fn refresh_content(
        &self,
        options: &ResolveOptions,
        expected: &Arc<ResolvedContent>,
    ) -> io::Result<Arc<ResolvedContent>> {
        let mut expected = Arc::clone(expected);
        loop {
            let mut cache = self.content.lock().map_err(|error| io::Error::other(error.to_string()))?;
            if let Some(current) = cache.find(options)
                && !Arc::ptr_eq(&current, &expected)
            {
                drop(cache);
                if current.layers.admit_generation().is_ok() { return self.content(options); }
                expected = current;
                continue;
            }
            let _generation = installed_content_core::content::Generation::enter();
            let current = Arc::new(self.play(options)?);
            cache.bind(options, &current);
            drop(cache);
            current.layers.admit_generation()?;
            return Ok(current);
        }
    }

    /// Runs `work`, a preparation, while these folders' content resolves on a thread beside it at
    /// the caller's priority, unless it is resolved or being resolved, or `cancelled` is set: the
    /// [`Self::content`] asked for them meanwhile waits for it and returns what it returned, its
    /// error included, rather than resolving again. Nothing is bound before then. The resolution
    /// stops when `cancelled` is set or `work` ends, which waits for it, and a result nothing
    /// claimed is released then.
    pub fn resolving_ahead<T>(
        &self,
        options: Option<&ResolveOptions>,
        cancelled: &AtomicBool,
        work: impl FnOnce() -> T,
    ) -> T {
        static OWNERS: AtomicU64 = AtomicU64::new(0);
        let finished = AtomicBool::new(false);
        std::thread::scope(|scope| {
            let resolver = options.and_then(|options| {
                let mut cache = self
                    .content
                    .lock()
                    .unwrap_or_else(|error| error.into_inner());
                if cancelled.load(Ordering::Acquire)
                    || cache.find(options).is_some()
                    || cache
                        .ahead
                        .as_ref()
                        .is_some_and(|ahead| same_folders(&ahead.options, options))
                {
                    return None;
                }
                let (sender, receiver) = std::sync::mpsc::sync_channel(1);
                let finished = &finished;
                let caller = job_priority::Caller::current();
                let cpu = job_priority::accounting::Context::capture();
                // Not started, it is resolved when asked for.
                let handle = std::thread::Builder::new()
                    .name("content resolve".into())
                    .spawn_scoped_carried(
                        scope,
                        installed_content_core::content::carry_generation(move || {
                            caller.enter();
                            let _cpu = cpu.enter("content resolve");
                            let _memory = engine_allocation::labels::enter(
                                engine_allocation::labels::Owner::RUN,
                                engine_allocation::labels::Kind::Content,
                            );
                            let resolved = if cancelled.load(Ordering::Acquire)
                                || finished.load(Ordering::Acquire)
                            {
                                Err(io::Error::new(io::ErrorKind::Interrupted, "stopped"))
                            } else {
                                self.play(options)
                            };
                            // Stopped as the preparation ended: whoever asks still resolves. A
                            // cancelled preparation's waiters are told so instead of hashing again.
                            let stopped = matches!(&resolved, Err(error)
                            if error.kind() == io::ErrorKind::Interrupted
                                && !cancelled.load(Ordering::Acquire));
                            let _ = sender.send((!stopped).then_some(resolved));
                        }),
                    )
                    .ok()?;
                let owner = OWNERS.fetch_add(1, Ordering::Relaxed);
                cache.ahead = Some(ContentAhead {
                    options: options.clone(),
                    owner,
                    state: AheadState::Resolving(receiver),
                });
                Some((owner, handle))
            });
            let _release = ReleaseAhead {
                source: self,
                resolver,
                finished: &finished,
            };
            work()
        })
    }

    /// The content already resolved for these folders, without resolving it or replacing what
    /// is bound: `None` when the source's content is bound to other folders or to none. Work
    /// done ahead of a request uses it, so it never rebinds the content every reply reports
    /// ([`Self::append_content_warnings`]).
    pub fn bound_content(&self, options: &ResolveOptions) -> Option<Arc<ResolvedContent>> {
        self.content
            .lock()
            .unwrap_or_else(|error| error.into_inner())
            .bound
            .as_ref()
            .filter(|(old, _)| same_folders(old, options))
            .map(|(_, content)| Arc::clone(content))
    }

    /// What work on this source is bound to now, `None` until the service binds it.
    pub fn binding(&self) -> Option<Arc<Binding>> {
        self.binding
            .lock()
            .unwrap_or_else(|error| error.into_inner())
            .clone()
    }

    /// The visual mods this source's widgets show now. Work reads the binding it was admitted
    /// under from its `ContentOptions::presentation`, never this, so nothing changed while it
    /// runs reaches it.
    pub fn presentation(&self) -> Presentation {
        self.binding()
            .map(|binding| binding.presentation.clone())
            .unwrap_or_default()
    }

    /// Makes work on this source bound to `options`, `allow_unverified` and `presentation`, as
    /// one swap of the whole binding, returning whether anything changed. Unchanged, the current
    /// binding and its mount are kept. `None` options unbind it.
    pub fn bind(
        &self,
        options: Option<(&ResolveOptions, bool)>,
        presentation: Presentation,
    ) -> bool {
        let mut binding = self
            .binding
            .lock()
            .unwrap_or_else(|error| error.into_inner());
        let Some((options, allow_unverified)) = options else {
            let retired = binding.take();
            retired.iter().for_each(|retired| retired.retire());
            return retired.is_some();
        };
        if binding
            .as_ref()
            .is_some_and(|current| current.binds(options, allow_unverified, &presentation))
        {
            return false;
        }
        let replaced = binding.replace(Arc::new(Binding::new(
            options.clone(),
            allow_unverified,
            presentation,
        )));
        replaced.iter().for_each(|replaced| replaced.retire());
        true
    }

    /// Admits work on this source: its current binding, taken whole under one short lock, so
    /// work never reads one binding's folders with another's mods. `None` until the source is
    /// bound.
    pub fn admit(&self, work: Work) -> Option<ContentOptions> {
        let binding = self.binding()?;
        let options = binding.options.clone();
        let allow_unverified = binding.allow_unverified;
        let presentation = match work {
            Work::Content(cancel) => Admitted::admitted(binding, true, cancel),
            Work::Ahead => {
                let cancel = Arc::clone(&binding) as Arc<dyn Cancel>;
                Admitted::admitted(binding, false, cancel)
            }
            Work::Recorded => Admitted::recorded(binding),
        };
        Some(ContentOptions {
            options,
            allow_unverified,
            presentation,
        })
    }

    /// Whether work admitted under `content` still holds the source's current binding: no
    /// folder, choice or visual mod change has replaced it. Work admitted outside the service
    /// (the source never bound) holds it while its presentation is the source's.
    pub fn still_admitted(&self, content: &ContentOptions) -> bool {
        match self.binding() {
            Some(current) => Arc::ptr_eq(&current, content.presentation.binding()),
            None => content.presentation.identity().is_empty(),
        }
    }

    /// [`Source::admit`], with the mount the work draws resolved before any of its code runs
    /// ([`Source::resolve`]): what every entry of work (a product request, a prepared query,
    /// read-ahead) admits with.
    pub fn admit_resolved(&self, work: Work) -> Option<ContentOptions> {
        let mut content = self.admit(work)?;
        self.resolve(&mut content);
        Some(content)
    }

    /// Resolves the mount work admitted under `content` draws, at its entry and outside every
    /// lock, so no code of the work, under whatever cache lock, ever mounts: its binding's mount,
    /// computed once by the first work that needs it, the others waiting on it while watching
    /// their own cancellation ([`Binding::mount`]). Work showing no visual mod needs none.
    pub fn resolve(&self, content: &mut ContentOptions) {
        if !content.presentation.shows_mods() {
            return;
        }
        let admitted = &content.presentation;
        let mount = admitted
            .binding()
            .mount
            .get_or_mount(admitted.cancel(), || {
                let recorded = self.content(&content.options)?;
                Presented::mount(
                    &recorded,
                    &content.options,
                    &self.replay,
                    admitted.identity(),
                    admitted.cancel(),
                )
                .map(Arc::new)
            });
        content.presentation.set_mount(&mount);
    }

    /// The mount the art reads of work admitted under `content` draw from, as its entry resolved
    /// it ([`Source::resolve`]); it never mounts. Recorded-only work and work showing no visual
    /// mod get the recording's own artwork.
    pub fn presented_for(&self, content: &ContentOptions) -> io::Result<Arc<Presented>> {
        if !content.presentation.shows_mods() {
            return self.recorded_presented(&content.options);
        }
        content.presentation.mount().unwrap_or_else(|| {
            debug_assert!(
                false,
                "work showing visual mods reads art its entry did not resolve"
            );
            Err(io::Error::other(
                "The selected visual mods were not prepared for this work",
            ))
        })
    }

    /// [`Source::resolve`] at the entry of work done ahead of a request: only over content
    /// already resolved for its folders, and only while `content` holds the source's current
    /// binding, so it never mounts over content or for a presentation a request has replaced.
    pub fn resolve_bound(&self, content: &mut ContentOptions) {
        if self.bound_content(&content.options).is_some() && self.still_admitted(content) {
            self.resolve(content);
        }
    }

    /// The mount of work done ahead of a request, as its entry resolved it
    /// ([`Source::resolve_bound`]), while `content` still holds the source's current binding over
    /// content already resolved for its folders: it never binds content or a presentation a
    /// request has replaced. `None` otherwise, or when its entry resolved none.
    pub fn bound_presented_for(&self, content: &ContentOptions) -> Option<Arc<Presented>> {
        self.bound_content(&content.options)?;
        if !self.still_admitted(content) {
            return None;
        }
        if content.presentation.shows_mods() {
            return content.presentation.mount()?.ok();
        }
        self.recorded_presented(&content.options).ok()
    }

    /// The recording's own artwork for these folders: no visual mod mounted, for prepared
    /// families whose stored payloads hold artwork. One mount per recorded content, so what
    /// artwork keeps by its mount (flag files, cells) lasts across requests.
    pub fn recorded_presented(&self, options: &ResolveOptions) -> io::Result<Arc<Presented>> {
        let recorded = self.content(options)?;
        let state = self.state::<RecordedMount>();
        let mut kept = state.lock().unwrap_or_else(|error| error.into_inner());
        // The weak handle keeps the content's allocation, so its address names it alone.
        if let Some((content, presented)) = &kept.0
            && std::ptr::eq(content.as_ptr(), Arc::as_ptr(&recorded))
        {
            return Ok(Arc::clone(presented));
        }
        let presented = Arc::new(Presented::mount(
            &recorded,
            options,
            &self.replay,
            &Presentation::default(),
            &std::sync::atomic::AtomicBool::new(false),
        )?);
        kept.0 = Some((Arc::downgrade(&recorded), Arc::clone(&presented)));
        Ok(presented)
    }

    /// A product's own cache for this source, created on first use. Requests
    /// for the same product serialise on it; different products run in parallel.
    pub fn state<T: Default + Send + 'static>(&self) -> Arc<Mutex<T>> {
        let mut states = self.state.lock().unwrap_or_else(|error| error.into_inner());
        let entry = states
            .entry(TypeId::of::<T>())
            .or_insert_with(|| StateEntry {
                value: Arc::new(Mutex::new(T::default())),
                append: None,
            });
        Arc::clone(&entry.value)
            .downcast::<Mutex<T>>()
            .expect("state is keyed by its own type")
    }

    /// Opt in to verified append inheritance. Clone must share immutable history
    /// and give the new revision its own mutable continuation.
    pub fn append_state<T: Default + Clone + Send + 'static>(&self) -> Arc<Mutex<T>> {
        fn fork<T: Clone + Send + 'static>(value: &SharedState) -> SharedState {
            let value = value.downcast_ref::<Mutex<T>>().expect("state type");
            Arc::new(Mutex::new(
                value
                    .lock()
                    .unwrap_or_else(|error| error.into_inner())
                    .clone(),
            ))
        }
        let value = self.state::<T>();
        self.state
            .lock()
            .unwrap_or_else(|error| error.into_inner())
            .get_mut(&TypeId::of::<T>())
            .expect("state created")
            .append = Some(fork::<T>);
        value
    }

    /// Called outside the service's sources lock. Unused and revision-local
    /// state needs neither a prefix comparison nor a product lock.
    pub fn inherit_append_states(&self, previous: &Self) {
        let opted: Vec<_> = previous
            .state
            .lock()
            .unwrap_or_else(|error| error.into_inner())
            .iter()
            .filter_map(|(id, entry)| {
                entry
                    .append
                    .map(|fork| (*id, Arc::clone(&entry.value), fork))
            })
            .collect();
        if opted.is_empty() || !self.is_append_of(previous) {
            return;
        }
        for (id, value, fork) in opted {
            let value = fork(&value);
            self.state
                .lock()
                .unwrap_or_else(|error| error.into_inner())
                .insert(
                    id,
                    StateEntry {
                        value,
                        append: Some(fork),
                    },
                );
        }
    }

    pub fn is_append_of(&self, previous: &Self) -> bool {
        let old = previous.replay_ref();
        let new = self.replay_ref();
        if old.campaign_id != new.campaign_id
            || old.identity != new.identity
            || old.format != new.format
            || old.index.len() > new.index.len()
        {
            return false;
        }
        // A save adds pages to the runs it continues as well as new runs, and timelines to the
        // ones read before; a changed join takes blocks away.
        let held: std::collections::HashSet<&replay::campaign::BlockIndex> =
            new.index.iter().collect();
        if old.index.iter().any(|block| !held.contains(block)) {
            return false;
        }
        if old
            .sessions
            .iter()
            .any(|session| !new.sessions.contains(session))
        {
            return false;
        }
        if old
            .branches
            .iter()
            .any(|branch| !new.branches.contains(branch))
            || old.gaps.iter().any(|gap| !new.gaps.contains(gap))
        {
            return false;
        }
        !new.gaps
            .iter()
            .filter(|gap| !old.gaps.contains(gap))
            .any(|gap| {
                old.index
                    .iter()
                    .any(|block| block.branch == gap.branch && gap.first_hour <= block.last_hour)
            })
    }
}

/// What work a source admits ([`Source::admit`]).
pub enum Work {
    /// Work answering a request, cancelled by the request: its art follows the binding's visual
    /// mods, and its replies name what their mount cannot show.
    Content(Arc<dyn Cancel>),
    /// Work done ahead of requests (read-ahead): its art follows the binding's visual mods, and
    /// it is cancelled once its binding is retired.
    Ahead,
    /// Work that reads the recording's own artwork whatever its source shows, and that nobody
    /// cancels: preparation and the store checks it makes, and work that reads no content. It
    /// never mounts a binding's visual mods.
    Recorded,
}

/// A dropped source retires its binding, cancelling work still done ahead under it.
impl Drop for Source {
    fn drop(&mut self) {
        if let Some(binding) = self
            .binding
            .get_mut()
            .unwrap_or_else(|error| error.into_inner())
            .take()
        {
            binding.retire();
        }
    }
}

/// Whether content resolved for `old` folders is the content of `options`.
fn same_folders(old: &ResolveOptions, options: &ResolveOptions) -> bool {
    old.game_dir == options.game_dir
        && old.user_dir == options.user_dir
        && old.cache_dir == options.cache_dir
}

/// Content resolved by a test rather than from the configured folders.
#[cfg(any(test, feature = "test-support"))]
impl Source {
    pub fn set_test_content(
        &self,
        options: installed_content_core::ResolveOptions,
        content: Arc<installed_content_core::content::ResolvedContent>,
    ) {
        self.content.lock().unwrap().bind(&options, &content);
    }
}

#[cfg(test)]
mod tests;
