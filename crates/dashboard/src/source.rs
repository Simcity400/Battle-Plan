//! One opened recording shared by every widget and map that reads it.

use std::{
    any::{Any, TypeId},
    collections::HashMap,
    io,
    path::{Path, PathBuf},
    sync::{Arc, Mutex, atomic::AtomicBool},
};

use installed_content::{
    ResolveOptions,
    content::{ResolvedContent, UncoveredReads},
};
use replay::container::Replay;

use crate::{
    ReplayMetadata,
    content::{ContentKey, content_key, name_localisation},
    metadata::{self, warn},
    products::ContentOptions,
};
use installed_content::{country_names::CountryNames, localisation::Localisation};
use replay::container::SeekResult;

/// Identity of an opened recording revision inside the running service.
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub struct SourceKey {
    pub source_id: String,
    pub revision: String,
}

type ContentCache = Option<(ResolveOptions, Arc<ResolvedContent>, UncoveredReads)>;

/// The replay bytes, its presentation metadata, the resolved game content and
/// every product's per-source cache. Dropping the last handle releases it all.
pub struct Source {
    key: SourceKey,
    path: PathBuf,
    replay: Arc<Replay>,
    metadata: ReplayMetadata,
    /// The metadata with country names resolved for the last folders it was asked for.
    named: Mutex<Option<(Option<ContentKey>, ReplayMetadata)>>,
    content: Mutex<ContentCache>,
    state: Mutex<HashMap<TypeId, Arc<dyn Any + Send + Sync>>>,
}

impl Source {
    /// Reads the file and its initial country catalogue. Cancellation is
    /// checked between reads and decoded frames.
    pub fn open(key: SourceKey, path: PathBuf, cancelled: &AtomicBool) -> io::Result<Self> {
        metadata::check_cancel(cancelled)?;
        let bytes = std::fs::read(&path)?;
        metadata::check_cancel(cancelled)?;
        Self::from_replay(key, path, Arc::new(Replay::open(bytes)?), cancelled)
    }

    pub fn from_replay(
        key: SourceKey,
        path: PathBuf,
        replay: Arc<Replay>,
        cancelled: &AtomicBool,
    ) -> io::Result<Self> {
        let metadata = metadata::read(&replay, &path, cancelled)?;
        Ok(Self {
            key,
            path,
            replay,
            metadata,
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

    pub fn replay(&self) -> Arc<Replay> {
        Arc::clone(&self.replay)
    }

    pub fn replay_ref(&self) -> &Replay {
        &self.replay
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
    pub(crate) fn append_content_warnings(&self, warnings: &mut Vec<String>) {
        if let Some((_, _, reads)) = self
            .content
            .lock()
            .unwrap_or_else(|error| error.into_inner())
            .as_ref()
        {
            reads.append_to(warnings);
        }
    }

    /// Resolves the catalogue's names in the user's game language at each
    /// country's observed hour; without content they stay tags, with a warning.
    pub fn name_countries(&self, content: Option<&ContentOptions>) {
        let key = content_key(content);
        let mut named = self.named.lock().unwrap_or_else(|error| error.into_inner());
        if named.as_ref().is_some_and(|(cached, _)| *cached == key) {
            return;
        }
        let metadata = self.named_metadata(name_localisation(self, content));
        *named = Some((key, metadata));
    }

    /// The catalogue named against `localisation`, or tagged with its warning.
    pub(crate) fn named_metadata(
        &self,
        localisation: Result<Arc<Localisation>, String>,
    ) -> ReplayMetadata {
        let mut metadata = self.metadata.clone();
        match localisation {
            Err(reason) => warn(&mut metadata.warnings, reason),
            Ok(localisation) => {
                let mut unnamed = 0;
                let mut frames = std::collections::BTreeMap::new();
                for country in &mut metadata.countries {
                    let at = (country.observed_branch_id.clone(), country.observed_hour);
                    let frame = frames.entry(at).or_insert_with(|| {
                        let branch = country.observed_branch_id.parse().ok()?;
                        match self.replay.seek(branch, country.observed_hour).ok()? {
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
            let content = Arc::new(installed_content::resolve(&self.replay.identity, options)?);
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
            .or_insert_with(|| Arc::new(Mutex::new(T::default())) as Arc<dyn Any + Send + Sync>);
        Arc::clone(entry)
            .downcast::<Mutex<T>>()
            .expect("state is keyed by its own type")
    }
}

#[cfg(test)]
mod tests;
