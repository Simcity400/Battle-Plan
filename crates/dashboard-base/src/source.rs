//! One opened recording shared by every widget and map that reads it.

use std::{
    any::{Any, TypeId},
    collections::HashMap,
    io,
    path::{Path, PathBuf},
    sync::{Arc, Mutex, atomic::AtomicBool},
};

use installed_content_core::{ResolveOptions, content::ResolvedContent, content::UncoveredReads};
use replay::container::Replay;

use crate::{
    ReplayMetadata,
    content::{ContentKey, content_key, name_localisation},
    metadata::{self, warn},
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

type ContentCache = Option<(ResolveOptions, Arc<ResolvedContent>, UncoveredReads)>;

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
    metadata: ReplayMetadata,
    cached_named: Option<metadata_cache::CachedNames>,
    pub(crate) cached_inputs: Option<store::SourceKey>,
    /// The metadata with country names resolved for the last folders it was asked for.
    named: Mutex<Option<(Option<ContentKey>, ReplayMetadata)>>,
    content: Mutex<ContentCache>,
    state: Mutex<HashMap<TypeId, StateEntry>>,
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
                content: Mutex::new(None),
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
        let metadata = metadata::read(&replay, &path, cancelled)?;
        Ok(Self {
            key,
            path,
            block_windows: block_windows(&replay),
            replay,
            metadata,
            cached_named: None,
            cached_inputs: None,
            named: Mutex::new(None),
            content: Mutex::new(None),
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
    pub fn metadata(&self) -> ReplayMetadata {
        let mut metadata = self
            .named
            .lock()
            .unwrap_or_else(|error| error.into_inner())
            .as_ref()
            .map_or_else(|| self.metadata.clone(), |(_, metadata)| metadata.clone());
        self.append_content_warnings(&mut metadata.warnings);
        metadata
    }

    /// Report the live read set even for cached metadata and product replies.
    pub fn append_content_warnings(&self, warnings: &mut Vec<String>) {
        if let Some((_, _, reads)) = self
            .content
            .lock()
            .unwrap_or_else(|error| error.into_inner())
            .as_ref()
        {
            reads.append_to(warnings);
        }
    }

    /// The resolved stack's live diagnostics, which every consumer's reads add
    /// to at its own pace: artwork-effect notes, then the uncovered-read set.
    /// They belong to the reply being served, never to a prepared family's
    /// published payloads ([`crate::catalogue::own_refusals`]), so a prepared
    /// reply reports the current ones, as a product reply reports the read set.
    pub fn append_stack_warnings(&self, warnings: &mut Vec<String>) {
        if let Some((_, content, reads)) = self
            .content
            .lock()
            .unwrap_or_else(|error| error.into_inner())
            .as_ref()
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
            .as_ref()
            .is_some_and(|(_, _, reads)| reads.warning().is_some())
    }

    /// Resolves the catalogue's names in the user's game language at each
    /// country's observed hour; without content they stay tags, with a warning.
    pub fn name_countries(&self, content: Option<&ContentOptions>) {
        let key = content_key(content);
        let mut named = self.named.lock().unwrap_or_else(|error| error.into_inner());
        if named.as_ref().is_some_and(|(cached, _)| *cached == key) {
            return;
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
                *named = Some((key, self.named_metadata_from(localisation, Some(metadata))));
                return;
            }
        }
        let metadata = self.named_metadata(name_localisation(self, content));
        *named = Some((key, metadata));
    }

    /// The catalogue named against `localisation`, or tagged with its warning.
    pub(crate) fn named_metadata(
        &self,
        localisation: Result<Arc<Localisation>, String>,
    ) -> ReplayMetadata {
        self.named_metadata_from(localisation, None)
    }

    fn named_metadata_from(
        &self,
        localisation: Result<Arc<Localisation>, String>,
        previous: Option<&ReplayMetadata>,
    ) -> ReplayMetadata {
        let mut metadata = self.metadata.clone();
        let saved_countries: HashMap<_, _> = previous
            .into_iter()
            .flat_map(|previous| &previous.countries)
            .map(|country| (country_observation(country), country))
            .collect();
        match localisation {
            Err(reason) => {
                if !reason.is_empty() {
                    warn(&mut metadata.warnings, reason);
                }
                if let Some(previous) = previous {
                    for country in &mut metadata.countries {
                        if let Some(saved) = saved_countries.get(&country_observation(country)) {
                            country.name.clone_from(&saved.name);
                        }
                    }
                    for reason in &previous.warnings {
                        warn(&mut metadata.warnings, reason.clone());
                    }
                }
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
                    let frame = frames.entry(at).or_insert_with(|| {
                        let branch = country.observed_branch_id.parse().ok()?;
                        let tables = Some(&installed_content_country_names::country_names::TABLES[..]);
                        match self
                            .replay
                            .seek_selected(branch, country.observed_hour, tables)
                            .ok()?
                        {
                            SeekResult::Frame(frame) => {
                                Some(CountryNames::read(&frame, &localisation))
                            }
                            SeekResult::Gap { .. } => None,
                        }
                    });
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
                    warn(
                        &mut metadata.warnings,
                        format!(
                            "{unnamed} country names could not be resolved from the recorded content and show their tag."
                        ),
                    );
                }
            }
        }
        metadata
    }

    /// The recording's verified game content for the configured folders,
    /// resolved once and reused until the folders change.
    pub fn content(&self, options: &ResolveOptions) -> io::Result<Arc<ResolvedContent>> {
        let mut cache = self
            .content
            .lock()
            .map_err(|error| io::Error::other(error.to_string()))?;
        if !cache.as_ref().is_some_and(|(old, _, _)| {
            old.game_dir == options.game_dir
                && old.user_dir == options.user_dir
                && old.cache_dir == options.cache_dir
        }) {
            let content = Arc::new(installed_content_core::resolve(&self.replay.identity, options)?);
            let reads = content.layers.report_uncovered_reads();
            *cache = Some((options.clone(), content, reads));
        }
        Ok(Arc::clone(&cache.as_ref().expect("content cached").1))
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

/// Content resolved by a test rather than from the configured folders.
#[cfg(any(test, feature = "test-support"))]
impl Source {
    pub fn set_test_content(
        &self,
        options: installed_content_core::ResolveOptions,
        content: Arc<installed_content_core::content::ResolvedContent>,
    ) {
        let reads = content.layers.report_uncovered_reads();
        *self.content.lock().unwrap() = Some((options, content, reads));
    }
}

#[cfg(test)]
mod tests;
