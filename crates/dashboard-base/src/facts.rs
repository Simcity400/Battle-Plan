//! The game facts a recording carries, read from the recording itself.

use std::{io, sync::Arc};

use installed_content_core::content::facts::{Pack, PackSource};
use replay::container::Replay;

/// A recording's fact pack bytes, read where it holds them.
struct RecordingFacts(Arc<Replay>);

impl PackSource for RecordingFacts {
    fn read_at(&self, offset: u64, out: &mut [u8]) -> io::Result<()> {
        self.0.read_facts_at(offset, out)
    }
}

/// The game facts `replay` carries, or why it carries none.
pub fn recorded(replay: &Arc<Replay>) -> Result<Arc<Pack>, String> {
    let Some(region) = replay.fact_packs.last().copied() else {
        return Err(
            "this recording was made before recordings carried their game facts; upgrade it with a game installation that matches it"
                .to_owned(),
        );
    };
    Pack::open(
        Arc::new(RecordingFacts(Arc::clone(replay))),
        region.offset,
        region.length,
    )
    .map(Arc::new)
    .map_err(|error| format!("the recording's game facts cannot be read: {error}"))
}
