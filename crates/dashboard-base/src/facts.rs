//! The game facts a recording carries, read from the recording itself.

use std::{io, sync::Arc};

use installed_content_core::content::facts::{Pack, PackSource};
use replay::campaign::Replay;

/// A recording's fact pack bytes, read where it holds them: its newest pack's.
struct RecordingFacts(Arc<Replay>, usize);

impl PackSource for RecordingFacts {
    fn read_at(&self, offset: u64, out: &mut [u8]) -> io::Result<()> {
        self.0.read_facts(self.1, offset, out)
    }
}

/// The game facts `replay` carries, or why it carries none.
pub fn recorded(replay: &Arc<Replay>) -> Result<Arc<Pack>, String> {
    let Some(at) = replay.fact_packs.len().checked_sub(1) else {
        return Err(
            "this recording carries no game facts; upgrade it with a game installation that matches it"
                .to_owned(),
        );
    };
    replay
        .verify_facts(at)
        .map_err(|error| format!("the recording's game facts cannot be read: {error}"))?;
    Pack::open(
        Arc::new(RecordingFacts(Arc::clone(replay), at)),
        0,
        replay.fact_packs[at].stored,
    )
    .map(Arc::new)
    .map_err(|error| format!("the recording's game facts cannot be read: {error}"))
}
