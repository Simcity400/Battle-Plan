//! Durable work beside the forward publishers.
use crate::Source;
use crate::products::{ContentOptions, LoadContext};
use serde::{Deserialize, Serialize};
use std::{collections::BTreeSet, io};

const SECTION: &str = "preparation-side-work-v1";

#[derive(Default, Serialize, Deserialize)]
pub(super) struct Progress {
    pub pending: BTreeSet<String>,
}

pub(super) fn load(
    store: &store::Store,
    key: &store::SourceKey,
    saved: Option<&store::SourceKey>,
) -> io::Result<Option<Progress>> {
    let file = match saved {
        Some(previous) if previous != key => store.read_prefix(previous, key, SECTION, 0)?,
        _ => store.read(key, SECTION, 0)?,
    };
    file.map(|file| serde_json::from_reader(io::BufReader::new(file)).map_err(io::Error::other))
        .transpose()
}

fn staged(writer: &mut store::Writer) -> io::Result<Progress> {
    writer
        .read_at(SECTION, 0)?
        .map(|file| serde_json::from_reader(io::BufReader::new(file)).map_err(io::Error::other))
        .transpose()
        .map(Option::unwrap_or_default)
}

pub(super) fn write(writer: &mut store::Writer, progress: &Progress) -> io::Result<()> {
    super::json(writer, SECTION, progress)
}

/// Merge side publications after adopting their forks, in the canonical writer.
pub(super) fn update(writer: &mut store::Writer, finished: &[&str]) -> io::Result<()> {
    if !finished.is_empty() {
        let mut progress = staged(writer)?;
        for job in finished {
            progress.pending.remove(*job);
        }
        write(writer, &progress)?;
    }
    Ok(())
}

pub(super) fn complete(
    source: &Source,
    content: Option<&ContentOptions>,
    store: &store::Store,
    key: &store::SourceKey,
) -> io::Result<bool> {
    let Some(progress) = load(store, key, None)? else {
        return Ok(false);
    };
    Ok(progress.pending.is_empty()
        && super::production_scans::missing(
            source,
            store,
            key,
            &LoadContext {
                content: content.cloned(),
            },
        )?
        .is_empty())
}
