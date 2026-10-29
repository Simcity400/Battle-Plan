//! One opened recording shared by every widget and map that reads it.

use std::{
    any::{Any, TypeId},
    collections::HashMap,
    io,
    path::{Path, PathBuf},
    sync::{Arc, Mutex, Weak, atomic::AtomicBool},
};

use installed_content_core::{ResolveOptions, content::ResolvedContent, content::UncoveredReads};
use replay::container::Replay;

use crate::{
    ReplayMetadata,
    content::{ContentKey, content_key, language, name_localisation},
    metadata::{self, MetadataRefusals, ResidentMetadata, Warnings},
    presentation::{Admitted, Binding, Cancel, Presentation, Presented},
    products::ContentOptions,
};
use installed_content_country_names::country_names::CountryNames;
use installed_content_localisation::localisation::Localisation;
use replay::container::SeekResult;
pub mod metadata_cache;

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

/// The content resolved for the folders last asked for, with its read set, and the content of
/// earlier folders while work still holds it: asked for again, that is reused, so all work on
/// one folders reads, and reports, one read set.
#[derive(Default)]
struct ContentCache {
    bound: Option<(ResolveOptions, Arc<ResolvedContent>, UncoveredReads)>,
    held: Vec<(ResolveOptions, Weak<ResolvedContent>)>,
}

impl ContentCache {
    /// The content of these folders work reads now, bound or held, if any.
    fn find(&self, options: &ResolveOptions) -> Option<Arc<ResolvedContent>> {
        match &self.bound {
            Some((old, content, _)) if same_folders(old, options) => Some(Arc::clone(content)),
            _ => self
                .held
                .iter()
                .filter(|(old, _)| same_folders(old, options))
                .find_map(|(_, content)| content.upgrade()),
        }
    }

    /// Binds `content`, holding what was bound while work still holds it.
    fn bind(&mut self, options: &ResolveOptions, content: &Arc<ResolvedContent>) {
        let reads = content.layers.report_uncovered_reads();
        self.held
            .retain(|(old, held)| held.strong_count() > 0 && !same_folders(old, options));
        let previous = self
            .bound
            .replace((options.clone(), Arc::clone(content), reads));
        if let Some((old, previous, _)) = previous
            && !Arc::ptr_eq(&previous, content)
        {
            self.held.push((old, Arc::downgrade(&previous)));
        }
    }
}

fn block_windows(replay: &Replay) -> HashMap<u64, Vec<crate::products::country_groups::HourSpan>> {
    let mut windows: HashMap<_, Vec<_>> = HashMap::new();
    for block in &replay.index {
        windows
            .entry(block.branch)
            .or_default()
            .push(crate::products::country_groups::HourSpan {
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
    block_windows: HashMap<u64, Vec<crate::products::country_groups::HourSpan>>,
    metadata: ResidentMetadata,
    cached_named: Option<metadata_cache::CachedNames>,
    pub(crate) cached_inputs: Option<store::SourceKey>,
    /// The metadata with country names resolved for the last folders it was asked for.
    named: Mutex<Option<(NamesKey, ResidentMetadata)>>,
    content: Mutex<ContentCache>,
    /// What work on this source is bound to now ([`Self::bind`]), swapped in whole, never
    /// changed in place; `None` until the service binds it.
    binding: Mutex<Option<Arc<Binding>>>,
    state: Mutex<HashMap<TypeId, StateEntry>>,
}

/// What country names are resolved for: the folders, and the user's game language they are in
/// (or why it cannot be read).
pub(crate) type NamesKey = (Option<ContentKey>, Option<Result<String, String>>);

fn names_key(content: Option<&ContentOptions>) -> NamesKey {
    (
        content_key(content),
        content.map(|content| language(&content.options).map_err(|error| error.to_string())),
    )
}

fn country_observation(country: &crate::CountryMetadata) -> (&str, &str, &str, i32) {
    (
        &country.id,
        &country.tag,
        &country.observed_branch_id,
        country.observed_hour,
    )
}

impl Source {
    /// Reads the file and its initial country catalogue. Cancellation is
    /// checked between reads and decoded frames.
    pub fn open(key: SourceKey, path: PathBuf, cancelled: &AtomicBool) -> io::Result<Self> {
        metadata::check_cancel(cancelled)?;
        let replay = Arc::new(Replay::open_path(&path)?);
        replay.cache_decoded_sections(true);
        metadata::check_cancel(cancelled)?;
        if let Ok(Some(restored)) = metadata_cache::read(&path, &replay, cancelled) {
            metadata::check_cancel(cancelled)?;
            return Ok(Self {
                key,
                path,
                block_windows: block_windows(&replay),
                replay,
                metadata: restored.metadata,
                cached_named: restored.named,
                cached_inputs: Some(restored.inputs),
                named: Mutex::new(None),
                content: Mutex::default(),
                binding: Mutex::new(None),
                state: Mutex::new(HashMap::new()),
            });
        }
        Self::from_replay(key, path, replay, cancelled)
    }

    pub fn from_replay(
        key: SourceKey,
        path: PathBuf,
        replay: Arc<Replay>,
        cancelled: &AtomicBool,
    ) -> io::Result<Self> {
        replay.cache_decoded_sections(true);
        let metadata = metadata::read_resident(&replay, &path, cancelled)?;
        Ok(Self {
            key,
            path,
            block_windows: block_windows(&replay),
            replay,
            metadata,
            cached_named: None,
            cached_inputs: None,
            named: Mutex::new(None),
            content: Mutex::default(),
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
    pub fn block_windows(&self, branch: u64) -> &[crate::products::country_groups::HourSpan] {
        self.block_windows.get(&branch).map_or(&[], Vec::as_slice)
    }

    /// The initial observed catalogue, not a census of every campaign hour, with
    /// country names once [`Self::name_countries`] has run and tags before.
    pub fn metadata(&self) -> Result<ReplayMetadata, metadata::MetadataError> {
        let mut metadata = self
            .named
            .lock()
            .unwrap_or_else(|error| error.into_inner())
            .as_ref()
            .map_or_else(|| self.metadata.wire(), |(_, metadata)| metadata.wire())?;
        self.append_content_warnings(&mut metadata.warnings);
        Ok(metadata)
    }

    /// Read resident catalogue fields without copying or formatting wire diagnostics.
    pub fn with_metadata<T>(&self, read: impl FnOnce(&ReplayMetadata) -> T) -> T {
        let named = self.named.lock().unwrap_or_else(|error| error.into_inner());
        read(
            named
                .as_ref()
                .map_or(&self.metadata.value, |(_, metadata)| &metadata.value),
        )
    }

    /// Report the live read set and the changed content layers even for cached metadata and
    /// product replies.
    pub fn append_content_warnings(&self, warnings: &mut Vec<String>) {
        if let Some((_, content, reads)) = &self
            .content
            .lock()
            .unwrap_or_else(|error| error.into_inner())
            .bound
        {
            reads.append_to(warnings);
            content.append_changed_to(warnings);
        }
    }

    /// Report the live read set only: a reply that reads no game content is the same whatever
    /// layers changed, and is kept on disk without them.
    pub fn append_read_warnings(&self, warnings: &mut Vec<String>) {
        if let Some((_, _, reads)) = &self
            .content
            .lock()
            .unwrap_or_else(|error| error.into_inner())
            .bound
        {
            reads.append_to(warnings);
        }
    }

    /// The content of the folders work admitted under `options` reads: the bound content, or
    /// content of these folders work still holds ([`ContentCache`]). Resolved again when there
    /// is neither, but never bound, so a reply's diagnostics never rebind what other work reads.
    fn own_content(&self, options: &ResolveOptions) -> io::Result<Arc<ResolvedContent>> {
        let found = self
            .content
            .lock()
            .unwrap_or_else(|error| error.into_inner())
            .find(options);
        match found {
            Some(content) => Ok(content),
            None => Ok(Arc::new(installed_content_core::resolve(
                &self.replay.identity,
                options,
            )?)),
        }
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

    /// A content reply's live diagnostics: the read set and changed layers of its own folders'
    /// content, whatever the source has bound since; the bound read set without content.
    pub fn append_reply_warnings(
        &self,
        warnings: &mut Vec<String>,
        content: Option<&ContentOptions>,
    ) {
        match content.and_then(|content| self.own_content(&content.options).ok()) {
            Some(own) => {
                own.layers.read_warnings().append_to(warnings);
                own.append_changed_to(warnings);
            }
            None => self.append_read_warnings(warnings),
        }
    }

    /// Whether a content reply admitted under `content` read outside the recorded coverage: its
    /// own folders' content did, whatever the source has bound since ([`Self::reads_uncovered`]
    /// without content).
    pub fn reply_reads_uncovered(&self, content: Option<&ContentOptions>) -> bool {
        match content.and_then(|content| self.own_content(&content.options).ok()) {
            Some(own) => own.layers.read_warnings().warning().is_some(),
            None => self.reads_uncovered(),
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

    /// The resolved stack's live diagnostics, which every consumer's reads add
    /// to at its own pace: artwork-effect notes, then the uncovered-read set.
    /// They belong to the reply being served, never to a prepared family's
    /// published payloads ([`crate::catalogue::own_refusals`]), so a prepared
    /// reply reports the current ones, as a product reply reports the read set.
    pub fn append_stack_warnings(&self, warnings: &mut Vec<String>) {
        if let Some((_, content, reads)) = &self
            .content
            .lock()
            .unwrap_or_else(|error| error.into_inner())
            .bound
        {
            for line in content.layers.shader_warning_lines() {
                if !warnings.contains(&line) {
                    warnings.push(line);
                }
            }
            reads.append_to(warnings);
        }
    }

    /// Whether content outside the recorded coverage has been read for this source.
    pub fn reads_uncovered(&self) -> bool {
        self.content
            .lock()
            .unwrap_or_else(|error| error.into_inner())
            .bound
            .as_ref()
            .is_some_and(|(_, _, reads)| reads.warning().is_some())
    }

    /// Resolves the catalogue's names in the user's game language at each
    /// country's observed hour; without content they stay tags, with a warning.
    pub fn name_countries(&self, content: Option<&ContentOptions>) -> io::Result<()> {
        let key = names_key(content);
        let mut named = self.named.lock().unwrap_or_else(|error| error.into_inner());
        if named.as_ref().is_some_and(|(cached, _)| *cached == key) {
            return Ok(());
        }
        if let Some((inputs, saved, metadata)) = &self.cached_named
            && saved == &key
        {
            // Resolve today's complete stack/file list before accepting cached
            // labels, including newly added files and a changed language setting.
            if crate::prepared::files(self, content).is_ok_and(|(_, current)| &current == inputs) {
                let cached: std::collections::HashSet<_> =
                    metadata.countries.iter().map(country_observation).collect();
                let all_cached = self
                    .metadata
                    .countries
                    .iter()
                    .all(|country| cached.contains(&country_observation(country)));
                let localisation = if all_cached {
                    Err(String::new())
                } else {
                    name_localisation(self, content)
                };
                let metadata = self.named_metadata_from(localisation, Some(metadata))?;
                *named = Some((key, metadata));
                return Ok(());
            }
        }
        let metadata = self.named_metadata(name_localisation(self, content))?;
        *named = Some((key, metadata));
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
        let mut cache = self
            .content
            .lock()
            .map_err(|error| io::Error::other(error.to_string()))?;
        let content = match cache.find(options) {
            Some(content) => content,
            None => Arc::new(installed_content_core::resolve(&self.replay.identity, options)?),
        };
        if !cache
            .bound
            .as_ref()
            .is_some_and(|(_, bound, _)| Arc::ptr_eq(bound, &content))
        {
            cache.bind(options, &content);
        }
        Ok(content)
    }

    /// The content already resolved for these folders, without resolving it or replacing what
    /// is bound: `None` when the source's content is bound to other folders or to none. Work
    /// done ahead of a request uses it, so it never rebinds the content every reply reports
    /// ([`Self::append_content_warnings`], [`Self::reads_uncovered`]).
    pub fn bound_content(&self, options: &ResolveOptions) -> Option<Arc<ResolvedContent>> {
        self.content
            .lock()
            .unwrap_or_else(|error| error.into_inner())
            .bound
            .as_ref()
            .filter(|(old, _, _)| same_folders(old, options))
            .map(|(_, content, _)| Arc::clone(content))
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
        let mount = admitted.binding().mount.get_or_mount(admitted.cancel(), || {
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
            debug_assert!(false, "work showing visual mods reads art its entry did not resolve");
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
    /// families whose stored payloads hold artwork.
    pub fn recorded_presented(&self, options: &ResolveOptions) -> io::Result<Arc<Presented>> {
        let recorded = self.content(options)?;
        Presented::mount(
            &recorded,
            options,
            &self.replay,
            &Presentation::default(),
            &std::sync::atomic::AtomicBool::new(false),
        )
        .map(Arc::new)
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
            || old.keyframe_interval != new.keyframe_interval
            || old.index.len() > new.index.len()
        {
            return false;
        }
        for (a, b) in old.index.iter().zip(&new.index) {
            if a != b {
                return false;
            }
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
