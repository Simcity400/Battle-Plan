//! Prepared map source binding for the shared exact-hour reader.
use dashboard::prepared_map;
pub use dashboard::prepared_map::Reader as PreparedMap;
use replay::container::Replay;
use std::{io, sync::Arc};

/// Source-local read context. It opens only committed map progress; the host's
/// preparation pipeline remains the sole producer.
pub struct CheckpointSource {
    source: Arc<dashboard::Source>,
    content: Option<dashboard::products::ContentOptions>,
}

impl CheckpointSource {
    pub fn new(
        source: Arc<dashboard::Source>,
        content: Option<dashboard::products::ContentOptions>,
    ) -> Arc<Self> {
        Arc::new(Self { source, content })
    }

    pub fn replay_handle(&self) -> Arc<Replay> {
        self.source.replay()
    }

    pub fn open(&self) -> io::Result<Option<PreparedMap>> {
        let Some((store, key, progress)) = dashboard::prepared::pipeline::map_checkpoint_binding(
            &self.source,
            self.content.as_ref(),
        )?
        else {
            return Ok(None);
        };
        PreparedMap::new(
            store,
            key,
            progress,
            self.source.replay(),
            prepared_map::selected_tables(),
        )
        .map(Some)
    }
}
#[cfg(test)]
mod tests;
