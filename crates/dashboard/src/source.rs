//! One opened recording shared by every widget and map that reads it.

use std::{
    any::{Any, TypeId},
    collections::HashMap,
    io,
    path::{Path, PathBuf},
    sync::{Arc, Mutex, atomic::AtomicBool},
};

use installed_content::{ResolveOptions, content::ResolvedContent};
use replay::container::Replay;

use crate::{ReplayMetadata, metadata};

/// Identity of an opened recording revision inside the running service.
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub struct SourceKey {
    pub source_id: String,
    pub revision: String,
}

/// The replay bytes, its presentation metadata, the resolved game content and
/// every product's per-source cache. Dropping the last handle releases it all.
pub struct Source {
    key: SourceKey,
    path: PathBuf,
    replay: Arc<Replay>,
    metadata: ReplayMetadata,
    content: Mutex<Option<(ResolveOptions, Arc<ResolvedContent>)>>,
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

    /// The initial observed catalogue, not a census of every campaign hour.
    pub fn metadata(&self) -> &ReplayMetadata {
        &self.metadata
    }

    /// The recording's verified game content for the configured folders,
    /// resolved once and reused until the folders change.
    pub fn content(&self, options: &ResolveOptions) -> io::Result<Arc<ResolvedContent>> {
        let mut cache = self
            .content
            .lock()
            .map_err(|error| io::Error::other(error.to_string()))?;
        if !cache.as_ref().is_some_and(|(old, _)| {
            old.game_dir == options.game_dir
                && old.user_dir == options.user_dir
                && old.cache_dir == options.cache_dir
        }) {
            *cache = Some((
                options.clone(),
                Arc::new(installed_content::resolve(&self.replay.identity, options)?),
            ));
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
