//! Source handles share immutable replay bytes with any native consumer.

use std::{
    io,
    mem::size_of,
    path::{Path, PathBuf},
    sync::Arc,
    time::UNIX_EPOCH,
};

use replay::container::Replay;

use crate::{
    Analytics, QueryError, QueryLimits, ReplayMetadata, SourceKey, query::metadata_heap_bytes,
};

pub(crate) struct SourceRecord {
    pub key: SourceKey,
    pub path: PathBuf,
    pub replay: Arc<Replay>,
    pub metadata: ReplayMetadata,
    pub bytes: usize,
}

#[derive(Clone)]
pub struct SourceHandle(pub(crate) Arc<SourceRecord>);

impl SourceHandle {
    pub fn key(&self) -> &SourceKey {
        &self.0.key
    }

    pub fn path(&self) -> &Path {
        &self.0.path
    }

    pub fn replay(&self) -> Arc<Replay> {
        Arc::clone(&self.0.replay)
    }

    /// Initial observed catalogue, not an assertion about every campaign hour.
    pub fn metadata(&self) -> &ReplayMetadata {
        &self.0.metadata
    }

    /// Compressed backing plus the exposed metadata and source handle payload.
    /// Replay's parsed internals and allocator overhead are not an RSS estimate.
    pub fn accounted_bytes(&self) -> usize {
        self.0.bytes
    }

    pub(crate) fn externally_held(&self) -> bool {
        Arc::strong_count(&self.0) > 1 || Arc::strong_count(&self.0.replay) > 1
    }
}

pub(crate) fn prepare(
    key: SourceKey,
    path: PathBuf,
    replay: Arc<Replay>,
    limits: &QueryLimits,
    metadata_budget: usize,
    check: impl Fn() -> Result<(), QueryError>,
) -> Result<SourceHandle, QueryError> {
    check()?;
    let fixed_bytes = size_of::<SourceRecord>()
        .saturating_add(size_of::<ReplayMetadata>())
        .saturating_add(path.capacity().saturating_mul(4))
        .saturating_add(key.source_id.capacity())
        .saturating_add(key.revision.capacity())
        .saturating_add(
            replay
                .branches
                .len()
                .saturating_mul(size_of::<crate::BranchMetadata>() + 128),
        )
        .saturating_add(32 * (1024 + size_of::<String>()));
    if fixed_bytes > metadata_budget {
        return Err(QueryError::limit(
            "Source metadata exceeds its admission budget",
        ));
    }
    // Covers the output catalogue and its temporary per-branch tree/vector.
    // Raw replay blocks and parsed replay metadata are separate allocations.
    let per_country = 2 * (size_of::<crate::CountryMetadata>() + 2 * 512 + 128);
    let max_countries = (metadata_budget - fixed_bytes) / per_country;
    for branch in &replay.branches {
        check()?;
        if replay
            .index
            .iter()
            .find(|block| block.branch == branch.id)
            .is_some_and(|block| block.raw_length > limits.max_block_raw_bytes as u64)
        {
            return Err(QueryError::limit(
                "Initial catalogue block exceeds the raw-block budget",
            ));
        }
    }
    let metadata =
        Analytics::from_shared_checked(Arc::clone(&replay), path.clone(), max_countries, || {
            check().map_err(io::Error::from)
        })?
        .into_metadata();
    check()?;
    let extra_bytes = metadata_bytes(&metadata)
        .saturating_add(size_of::<SourceRecord>())
        .saturating_add(path.capacity())
        .saturating_add(key.source_id.capacity())
        .saturating_add(key.revision.capacity());
    if extra_bytes > metadata_budget {
        return Err(QueryError::limit(
            "Source metadata exceeds its admission budget",
        ));
    }
    let bytes = replay.bytes().len().saturating_add(extra_bytes);
    Ok(SourceHandle(Arc::new(SourceRecord {
        key,
        path,
        replay,
        metadata,
        bytes,
    })))
}

/// Cheap file-change witness used by open_source. It includes no path and is
/// not a content hash; callers must retain SourceKey and the canonical path too.
pub fn source_revision(path: impl AsRef<Path>) -> io::Result<String> {
    let metadata = path.as_ref().metadata()?;
    if !metadata.is_file() {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "Recording is not a file",
        ));
    }
    let stamp = match metadata.modified()?.duration_since(UNIX_EPOCH) {
        Ok(duration) => format!("{}:{}", duration.as_secs(), duration.subsec_nanos()),
        Err(error) => {
            let duration = error.duration();
            format!("-{}:{}", duration.as_secs(), duration.subsec_nanos())
        }
    };
    Ok(format!("{}:{stamp}", metadata.len()))
}

fn metadata_bytes(metadata: &ReplayMetadata) -> usize {
    size_of::<ReplayMetadata>()
        + metadata.path.capacity()
        + metadata.name.capacity()
        + metadata
            .player_country_id
            .as_ref()
            .map_or(0, String::capacity)
        + metadata.branches.capacity() * size_of::<crate::BranchMetadata>()
        + metadata
            .branches
            .iter()
            .map(|branch| {
                branch.id.capacity()
                    + branch.session_id.capacity()
                    + branch.sample_boundary.capacity()
                    + branch.gaps.capacity() * size_of::<crate::RecordingGap>()
                    + branch
                        .gaps
                        .iter()
                        .map(|gap| gap.reason.capacity())
                        .sum::<usize>()
                    + branch.parent_id.as_ref().map_or(0, String::capacity)
            })
            .sum::<usize>()
        + metadata.countries.capacity() * size_of::<crate::CountryMetadata>()
        + metadata
            .countries
            .iter()
            .map(metadata_heap_bytes)
            .sum::<usize>()
        + metadata.warnings.capacity() * size_of::<String>()
        + metadata
            .warnings
            .iter()
            .map(String::capacity)
            .sum::<usize>()
}
