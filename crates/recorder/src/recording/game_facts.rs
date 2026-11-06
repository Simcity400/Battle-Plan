//! The game facts a session's recordings carry, gathered beside the capture.

use installed_content_core::content::{GatheredContent, facts};
use replay_data::identity::Identity;
use std::{
    path::{Path, PathBuf},
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
    thread::JoinHandle,
};

/// Packs the gathered content's facts into the session folder's facts file on a background
/// thread at the lowest priority, on half the cores, so the game keeps the rest. Stopped and
/// waited for when dropped, so no packing outlives its capture.
pub(super) struct GameFacts {
    cancelled: Arc<AtomicBool>,
    thread: Option<JoinHandle<Result<String, String>>>,
}

impl GameFacts {
    pub(super) fn start(content: &GatheredContent, identity: &Identity, session: &Path) -> Self {
        let cancelled = Arc::new(AtomicBool::new(false));
        let stopping = Arc::clone(&cancelled);
        let layers = content.layers.clone();
        let identity = identity.clone();
        let file = session.join(replay::container::SESSION_FACTS_FILE);
        let cache = replay::recorder_settings::directory();
        let thread = std::thread::Builder::new()
            .name("game facts".into())
            .spawn(move || {
                job_priority::background_thread();
                job_priority::confined(|| pack(&layers, &identity, &cache, &file, &stopping))
            })
            .ok();
        Self { cancelled, thread }
    }

    /// Waits for the facts to be packed and says what they hold, or why there are none.
    pub(super) fn finish(mut self) -> Result<String, String> {
        match self.thread.take() {
            Some(thread) => thread
                .join()
                .unwrap_or_else(|_| Err("the game facts worker panicked".to_owned())),
            None => Err("the game facts worker could not start".to_owned()),
        }
    }
}

impl Drop for GameFacts {
    fn drop(&mut self) {
        if let Some(thread) = self.thread.take() {
            self.cancelled.store(true, Ordering::Release);
            let _ = thread.join();
        }
    }
}

fn pack(
    layers: &installed_content_core::ResolvedLayers,
    identity: &Identity,
    cache: &Path,
    file: &PathBuf,
    cancelled: &AtomicBool,
) -> Result<String, String> {
    let source = facts::Source {
        layers,
        withheld: Default::default(),
    };
    let executable = engine_executable::executable_tables::recorded_facts();
    let (pack, stats) = facts::write(&source, identity, executable, cache, &|| {
        cancelled.load(Ordering::Acquire)
    })
    .map_err(|error| error.to_string())?;
    let bytes = replay::container::encode_session_facts(identity, &pack).map_err(|e| e.to_string())?;
    let staged = file.with_extension("bin.tmp");
    std::fs::write(&staged, &bytes)
        .and_then(|()| std::fs::rename(&staged, file))
        .map_err(|error| format!("{}: {error}", file.display()))?;
    Ok(format!(
        "{} files, {:.1} MB, carried in {:.1} MB ({} of {} chunks reused)",
        stats.files,
        stats.raw_bytes as f64 / 1e6,
        stats.pack_bytes as f64 / 1e6,
        stats.cached_chunks,
        stats.chunks
    ))
}
