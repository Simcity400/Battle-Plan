//! Plain prepared pages and cancellable background preparation.

#[cfg(test)]
pub(crate) use dashboard_base::prepared::observer_tests;
pub use dashboard_base::prepared::{coverage, frame_changes, group_sum, series};
pub mod pipeline;
pub(crate) mod prepass;
pub mod query;
pub mod runner;

pub use crate::transport::Action;
pub use dashboard_base::prepared::{files, recording_store};
pub use pipeline::unprepared_blocks;

use crate::Source;
use serde::{Deserialize, Serialize};
use std::sync::{
    Arc, Condvar, Mutex,
    atomic::{AtomicBool, Ordering},
};
use ts_rs::TS;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Deserialize, Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(rename = "PreparationState")]
pub enum State {
    #[default]
    Idle,
    Queued,
    Preparing,
    Ready,
    Cancelled,
    Failed,
}

#[derive(Clone, Debug, Deserialize, Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(rename = "PreparationSection")]
pub struct Section {
    pub id: String,
    pub label: String,
    pub state: State,
    pub done: String,
    pub total: Option<String>,
    pub error: Option<String>,
}

#[derive(Clone, Debug, Deserialize, Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(rename = "PreparationReply")]
pub struct Reply {
    pub source_id: String,
    pub revision: String,
    pub request_id: String,
    pub background_enabled: bool,
    pub state: State,
    pub sections: Vec<Section>,
    pub coverage: std::collections::BTreeMap<String, pipeline::Coverage>,
    pub bytes: String,
    pub error: Option<String>,
}

#[derive(Clone, Copy, Debug, Deserialize, Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(rename = "PreparationPreferences")]
pub struct Preferences {
    pub background_enabled: bool,
}

/// Attached to an opened source; only current progress and current open files
/// survive a query. The recording's full timeline lives on disk.
pub struct Progress {
    pub state: State,
    pub sections: Vec<Section>,
    pub coverage: std::collections::BTreeMap<String, pipeline::Coverage>,
    pub bytes: u64,
    pub error: Option<String>,
    pub cancelled: Arc<AtomicBool>,
    pub running: bool,
    pub automatic: bool,
    pub cleared: bool,
    pub(crate) clearing: bool,
    pub(crate) finished: Arc<(Mutex<bool>, Condvar)>,
}

impl Default for Progress {
    fn default() -> Self {
        Self {
            state: State::Idle,
            sections: Vec::new(),
            coverage: Default::default(),
            bytes: 0,
            error: None,
            cancelled: Arc::new(AtomicBool::new(false)),
            running: false,
            automatic: false,
            cleared: false,
            clearing: false,
            finished: Arc::new((Mutex::new(true), Condvar::new())),
        }
    }
}

pub fn status(source: &Source, source_id: &str, request_id: &str) -> Reply {
    let progress = source.state::<Progress>();
    let progress = progress.lock().unwrap_or_else(|error| error.into_inner());
    Reply {
        source_id: source_id.to_owned(),
        revision: source.key().revision.clone(),
        request_id: request_id.to_owned(),
        background_enabled: true,
        state: progress.state,
        sections: progress.sections.clone(),
        coverage: progress.coverage.clone(),
        bytes: progress.bytes.to_string(),
        error: progress.error.clone(),
    }
}

pub fn cancel(source: &Source) {
    let progress = source.state::<Progress>();
    let mut progress = progress.lock().unwrap_or_else(|error| error.into_inner());
    progress.cancelled.store(true, Ordering::Release);
    // An active worker sets the final state after releasing its writer. Clear
    // cannot race an unacknowledged worker into publishing prepared data again.
    if !progress.running {
        progress.state = State::Cancelled;
    }
}

/// Admit one source job. Request cancellation applies to admission only; closing
/// a panel never owns another consumer's background preparation.
pub fn start(
    runner: &runner::Runner,
    source: Arc<Source>,
    content: Option<crate::products::ContentOptions>,
    cancel_request: &AtomicBool,
) -> std::io::Result<()> {
    crate::metadata::check_cancel(cancel_request)?;
    let progress = source.state::<Progress>();
    let state = progress.lock().unwrap_or_else(|error| error.into_inner());
    if state.clearing {
        return Err(std::io::Error::new(
            std::io::ErrorKind::WouldBlock,
            "Prepared data is being cleared",
        ));
    }
    if state.running {
        return Ok(());
    }
    drop(state);
    let restored = match pipeline::restore(&source, content.as_ref(), cancel_request) {
        Ok(value) => value,
        Err(error) if error.kind() == std::io::ErrorKind::Interrupted => return Err(error),
        Err(error) => {
            let mut state = progress.lock().unwrap_or_else(|error| error.into_inner());
            if !state.running && !state.clearing {
                state.state = State::Failed;
                state.error = Some(error.to_string());
                state.sections =
                    pipeline::sections(State::Failed, 0, Some(source.replay_ref().index.len()));
                for section in &mut state.sections {
                    section.error = Some(error.to_string());
                }
            }
            return Ok(());
        }
    };
    let mut state = progress.lock().unwrap_or_else(|error| error.into_inner());
    if state.clearing {
        return Err(std::io::Error::new(
            std::io::ErrorKind::WouldBlock,
            "Prepared data is being cleared",
        ));
    }
    if state.running {
        return Ok(());
    }
    if let Some(restored) = restored {
        state.cleared = false;
        state.state = State::Ready;
        state.sections = restored.sections;
        state.coverage = restored.coverage;
        state.bytes = restored.bytes;
        state.error = None;
        return Ok(());
    }
    state.cleared = false;
    state.cancelled = Arc::new(AtomicBool::new(false));
    let cancelled = Arc::clone(&state.cancelled);
    let finished = Arc::clone(&state.finished);
    *finished.0.lock().unwrap_or_else(|error| error.into_inner()) = false;
    state.running = true;
    state.state = State::Queued;
    state.error = None;
    state.sections = pipeline::sections(State::Queued, 0, None);
    drop(state);
    let worker_source = Arc::clone(&source);
    let worker_finished = Arc::clone(&finished);
    let admitted = runner.submit(cancelled, move |cancelled| {
        let progress = worker_source.state::<Progress>();
        progress
            .lock()
            .unwrap_or_else(|error| error.into_inner())
            .state = State::Preparing;
        let outcome = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            pipeline::run(&worker_source, content.as_ref(), cancelled, |accepted| {
                let mut state = progress.lock().unwrap_or_else(|error| error.into_inner());
                state.bytes = accepted.bytes;
                state.sections = accepted.sections.clone();
                state.coverage = accepted.coverage.clone();
            })
        }));
        // All writer handles belong to run and are gone before acknowledgement.
        let mut state = progress.lock().unwrap_or_else(|error| error.into_inner());
        state.running = false;
        match outcome {
            Ok(Ok(accepted)) => {
                state.state = State::Ready;
                state.bytes = accepted.bytes;
                state.sections = accepted.sections;
                state.coverage = accepted.coverage;
                state.error = None;
            }
            Ok(Err(error)) => {
                state.state = if error.kind() == std::io::ErrorKind::Interrupted {
                    State::Cancelled
                } else {
                    State::Failed
                };
                state.error = Some(error.to_string());
                let phase = state.state;
                for section in &mut state.sections {
                    if matches!(section.state, State::Queued | State::Preparing) {
                        section.state = phase;
                    }
                }
            }
            Err(_) => {
                state.state = State::Failed;
                state.error = Some("Preparation worker failed".into());
            }
        }
        *worker_finished
            .0
            .lock()
            .unwrap_or_else(|error| error.into_inner()) = true;
        drop(state);
        worker_finished.1.notify_all();
    });
    if let Err(error) = admitted {
        let mut state = progress.lock().unwrap_or_else(|error| error.into_inner());
        state.running = false;
        state.state = State::Failed;
        state.error = Some(error.to_string());
        for section in &mut state.sections {
            section.state = State::Failed;
            section.error = Some(error.to_string());
        }
        *finished.0.lock().unwrap_or_else(|error| error.into_inner()) = true;
        finished.1.notify_all();
        return Err(error);
    }
    Ok(())
}

/// Cancel and wait for the writer to close before removing prepared names. Open
/// range readers keep their existing files; new readers see an empty store.
pub fn clear(
    source: &Source,
    content: Option<&crate::products::ContentOptions>,
    cancel_request: &AtomicBool,
) -> std::io::Result<()> {
    let progress = source.state::<Progress>();
    let finished = {
        let mut state = progress.lock().unwrap_or_else(|error| error.into_inner());
        if state.clearing {
            return Err(std::io::Error::new(
                std::io::ErrorKind::WouldBlock,
                "Prepared data is being cleared",
            ));
        }
        state.clearing = true;
        state.cancelled.store(true, Ordering::Release);
        Arc::clone(&state.finished)
    };
    let result = (|| {
        let mut done = finished.0.lock().unwrap_or_else(|error| error.into_inner());
        while !*done {
            crate::metadata::check_cancel(cancel_request)?;
            done = finished
                .1
                .wait_timeout(done, std::time::Duration::from_millis(50))
                .unwrap_or_else(|error| error.into_inner())
                .0;
        }
        drop(done);
        crate::metadata::check_cancel(cancel_request)?;
        let (store, _) = files(source, content)?;
        store.clear()
    })();
    let mut state = progress.lock().unwrap_or_else(|error| error.into_inner());
    state.clearing = false;
    if result.is_ok() {
        state.cleared = true;
        state.state = State::Idle;
        state.sections.clear();
        state.coverage.clear();
        state.bytes = 0;
        state.error = None;
    }
    result
}

/// Wait after cancellation across all revisions of a recording. The service keeps
/// its short Start/Clear admission lock while choosing and cancelling those sources.
pub fn wait_cancelled(source: &Source, cancel_request: &AtomicBool) -> std::io::Result<()> {
    cancel(source);
    let progress = source.state::<Progress>();
    let finished = Arc::clone(
        &progress
            .lock()
            .unwrap_or_else(|error| error.into_inner())
            .finished,
    );
    let mut done = finished.0.lock().unwrap_or_else(|error| error.into_inner());
    while !*done {
        crate::metadata::check_cancel(cancel_request)?;
        done = finished
            .1
            .wait_timeout(done, std::time::Duration::from_millis(50))
            .unwrap_or_else(|error| error.into_inner())
            .0;
    }
    Ok(())
}
