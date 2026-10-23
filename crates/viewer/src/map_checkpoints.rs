//! Prepared map source binding for the shared exact-hour reader.
use dashboard_base::prepared_map;
pub use dashboard_base::prepared_map::Reader as PreparedMap;
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
        .map(|map| {
            let (map, bound) = map.with_committed_pages_reporting();
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
