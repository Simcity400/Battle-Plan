//! Prepared map source binding for the shared exact-hour reader.
use dashboard_base::prepared_map;
pub use dashboard_base::prepared_map::Reader as PreparedMap;
use replay::container::Replay;
use std::{
    io,
    path::PathBuf,
    sync::{Arc, Mutex, PoisonError},
    time::SystemTime,
};

/// Source-local read context: the change-run index serving the replay's map, bound or found
/// in the replay's store where preparation writes it, else committed prepared map
/// progress, which the host's preparation pipeline alone produces.
pub struct CheckpointSource {
    replay: Arc<Replay>,
    prepared: Option<(
        Arc<dashboard::Source>,
        Option<dashboard::products::ContentOptions>,
    )>,
    columnar: Option<Arc<crate::map_source::Columnar>>,
    /// Where preparation writes the index in the replay's store, placed when the source
    /// is bound from the recording it loaded, so no read of the map opens the recording;
    /// none for a replay held by name alone, or why it could not be placed.
    index: Result<Option<PathBuf>, (io::ErrorKind, String)>,
    /// The prepared index file found not to serve the replay, by its length and last write.
    refused: Mutex<Option<(u64, Option<SystemTime>)>>,
}

impl CheckpointSource {
    pub fn new(
        source: Arc<dashboard::Source>,
        content: Option<dashboard::products::ContentOptions>,
    ) -> Arc<Self> {
        let index = match dashboard::prepared::change_runs::path(&source) {
            Ok(path) => Ok(Some(path)),
            // A replay held by name alone has no store beside it.
            Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(None),
            Err(error) => Err((error.kind(), error.to_string())),
        };
        Arc::new(Self {
            replay: source.replay(),
            prepared: Some((source, content)),
            columnar: None,
            index,
            refused: Mutex::new(None),
        })
    }

    /// The map of `replay` served from `columnar` alone.
    pub fn columnar_only(
        replay: Arc<Replay>,
        columnar: Arc<crate::map_source::Columnar>,
    ) -> Arc<Self> {
        Arc::new(Self {
            replay,
            prepared: None,
            columnar: Some(columnar),
            index: Ok(None),
            refused: Mutex::new(None),
        })
    }

    /// This binding with the map served from `columnar` in place of the prepared map.
    pub fn with_columnar(&self, columnar: Arc<crate::map_source::Columnar>) -> Arc<Self> {
        Arc::new(Self {
            replay: Arc::clone(&self.replay),
            prepared: self.prepared.clone(),
            columnar: Some(columnar),
            index: Ok(None),
            refused: Mutex::new(None),
        })
    }

    pub fn replay_handle(&self) -> Arc<Replay> {
        Arc::clone(&self.replay)
    }

    /// The index serving the map: the one bound, else the one preparation wrote beside the
    /// replay's store when it serves this replay; a file found not to is not opened again
    /// until it changes. It reads the index's file, never the recording.
    pub fn columnar(&self) -> io::Result<Option<Arc<crate::map_source::Columnar>>> {
        if let Some(columnar) = &self.columnar {
            return Ok(Some(Arc::clone(columnar)));
        }
        let path = match &self.index {
            Ok(Some(path)) => path,
            Ok(None) => return Ok(None),
            Err((kind, error)) => return Err(io::Error::new(*kind, error.clone())),
        };
        let metadata = match std::fs::metadata(path) {
            Ok(metadata) => metadata,
            Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(None),
            Err(error) => return Err(error),
        };
        let identity = (metadata.len(), metadata.modified().ok());
        let mut refused = self.refused.lock().unwrap_or_else(PoisonError::into_inner);
        if *refused == Some(identity) {
            return Ok(None);
        }
        let file = match replay::columnar_runs::IndexFile::open(path) {
            Ok(file) => Arc::new(file),
            Err(error) if error.kind() == io::ErrorKind::InvalidData => {
                *refused = Some(identity);
                return Ok(None);
            }
            Err(error) => return Err(error),
        };
        let columnar = crate::map_source::Columnar::verify(&self.replay, file);
        if columnar.is_none() {
            *refused = Some(identity);
        }
        Ok(columnar)
    }

    /// The committed prepared map; none while a change-run index serves the map.
    pub fn open(&self) -> io::Result<Option<PreparedMap>> {
        let Some((source, content)) = self.prepared.as_ref().filter(|_| self.columnar.is_none())
        else {
            return Ok(None);
        };
        let Some((store, key, progress)) =
            dashboard::prepared::pipeline::map_checkpoint_binding(source, content.as_ref())?
        else {
            return Ok(None);
        };
        PreparedMap::new(
            store,
            key,
            progress,
            Arc::clone(&self.replay),
            prepared_map::selected_tables(),
        )
        .map(|map| {
            let (map, bound) = map
                .with_change_facts(engine_allocation::credit::display_products().clone())
                .with_committed_pages_reporting();
            if viewer_foundation::profile_writer::profiling() {
                viewer_foundation::profile_writer::profile_print(bind_line(&map, &bound));
            }
            Some(map)
        })
    }
}

fn bind_line(map: &PreparedMap, bound: &io::Result<()>) -> String {
    match bound {
        Err(error) => format!("PROFILE map_pages bind failed {error}"),
        Ok(()) => {
            let chains = map.page_chains();
            let detail: Vec<String> = chains
                .iter()
                .flat_map(|(branch, pages)| {
                    pages.chains.iter().map(move |chain| {
                        format!(
                            "{branch}/{}:{}:{:?}..{:?}",
                            chain.stride, chain.ranges, chain.first, chain.last
                        )
                    })
                })
                .collect();
            format!(
                "PROFILE map_pages bind ok branches {} chains {}",
                chains.len(),
                detail.join(",")
            )
        }
    }
}
#[cfg(test)]
mod tests;
