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
    // Automatic work stopped at a block boundary for a `PauseReason`; it
    // resumes from its accepted prefix once that reason has passed.
    Paused,
    Ready,
    Cancelled,
    Failed,
}

// Why automatic work is paused. (Plain comments: the generated contract
// carries no documentation for these types.)
#[derive(Clone, Copy, Debug, PartialEq, Eq, Deserialize, Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(rename = "PreparationPauseReason")]
pub enum PauseReason {
    // Preparation the user asked for, of another recording, is queued or running.
    Requested,
    // A newer recording behind its store is prepared first.
    NewerRecording,
    // The volume holding prepared data is below the free-space floor.
    DiskSpace,
    // the game is running.
    Game,
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
    // Set while `state` is `Paused`.
    pub pause_reason: Option<PauseReason>,
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
    /// The job was admitted by the library scan, ahead of the user: it yields
    /// to preparation the user asked for and lowers its threads
    /// ([`priority`](dashboard_base::prepared::priority)). An open that joins
    /// it clears the flag.
    pub background: Arc<AtomicBool>,
    pub cleared: bool,
    /// Why the scheduler, not the user, stopped the job: it ends `Paused`.
    pub(crate) paused: Option<PauseReason>,
    /// The user holds this job: an open or Prepare claimed it. The claim lasts
    /// across pauses of the same unfinished job and ends when the job reaches
    /// a terminal state (an open view keeps holding the recording on its own).
    pub(crate) claimed: bool,
    /// Moves on whenever something may change what a check of this source's
    /// store found: a content rebind, a Clear, a job queued to write the store
    /// and its end. A check records the epoch it read, and is used only while
    /// the epoch is unchanged.
    pub(crate) store_epoch: u64,
    /// Counts the user's Cancels and Clears, so work captured before one (a
    /// refresh restarting the job) never overrides it.
    pub(crate) user_stops: u64,
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
            background: Arc::new(AtomicBool::new(false)),
            cleared: false,
            paused: None,
            claimed: false,
            store_epoch: 0,
            user_stops: 0,
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
        pause_reason: progress.paused.filter(|_| progress.state == State::Paused),
    }
}

pub fn cancel(source: &Source) {
    stop(source, None);
}

/// Stop automatic work at its next block boundary for `reason`; it ends
/// `Paused` and keeps its accepted prefix, to resume once the reason passes.
pub fn pause(source: &Source, reason: PauseReason) {
    stop(source, Some(reason));
}

/// The epoch a check of `source`'s store records, read before the store.
pub fn store_epoch(source: &Source) -> u64 {
    source
        .state::<Progress>()
        .lock()
        .unwrap_or_else(|error| error.into_inner())
        .store_epoch
}

/// Invalidate every check of `source`'s store taken so far.
pub(crate) fn invalidate(progress: &mut Progress) {
    progress.store_epoch = progress.store_epoch.wrapping_add(1);
}

/// Every terminal transition (Ready, Failed, Cancelled) goes through here:
/// the job is finished, so its pause and the user's claim on it end.
pub(crate) fn finish(progress: &mut Progress, state: State) {
    debug_assert!(matches!(
        state,
        State::Ready | State::Failed | State::Cancelled
    ));
    progress.state = state;
    progress.paused = None;
    progress.claimed = false;
}

fn stop(source: &Source, pause: Option<PauseReason>) {
    let progress = source.state::<Progress>();
    let mut progress = progress.lock().unwrap_or_else(|error| error.into_inner());
    // A job already cancelled outright stays cancelled; a Cancel overrides a
    // pause, and a later pause names its own reason.
    let cancelled = progress.cancelled.swap(true, Ordering::AcqRel);
    progress.paused = pause.filter(|_| progress.paused.is_some() || !cancelled);
    // An active worker sets the final state after releasing its writer. Clear
    // cannot race an unacknowledged worker into publishing prepared data again.
    if !progress.running {
        if progress.paused.is_some() {
            progress.state = State::Paused;
        } else {
            finish(&mut progress, State::Cancelled);
        }
    }
}

/// Admit one source job. Request cancellation applies to admission only; closing
/// a panel never owns another consumer's background preparation.
///
/// `restored` is what [`pipeline::restore`] found, read by the caller before it
/// took the service's admission lock at store epoch `epoch`, so admission does
/// no store I/O: a complete store is shown Ready, a store that cannot be read
/// fails with its reason, and anything else is queued for preparation. An
/// outcome whose epoch has moved on is dropped and the store is prepared.
pub fn start(
    runner: &runner::Runner,
    source: Arc<Source>,
    content: Option<crate::products::ContentOptions>,
    restored: std::io::Result<Option<pipeline::Accepted>>,
    epoch: u64,
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
    // Every bump happens under the admission lock the caller holds, or ends a
    // running job, so the outcome stays valid until it is used below.
    let restored = if state.store_epoch == epoch {
        restored
    } else {
        Ok(None)
    };
    drop(state);
    let restored = match restored {
        Ok(value) => value,
        Err(error) if error.kind() == std::io::ErrorKind::Interrupted => return Err(error),
        Err(error) => {
            let mut state = progress.lock().unwrap_or_else(|error| error.into_inner());
            if !state.running && !state.clearing {
                finish(&mut state, State::Failed);
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
        finish(&mut state, State::Ready);
        state.sections = restored.sections;
        state.coverage = restored.coverage;
        state.bytes = restored.bytes;
        state.error = None;
        return Ok(());
    }
    state.cleared = false;
    state.paused = None;
    // The job writes the store: checks taken before it are stale.
    invalidate(&mut state);
    state.cancelled = Arc::new(AtomicBool::new(false));
    let cancelled = Arc::clone(&state.cancelled);
    let background = Arc::clone(&state.background);
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
            dashboard_base::prepared::priority::JobPriority::run(background, || {
                pipeline::run(&worker_source, content.as_ref(), cancelled, |accepted| {
                    let mut state = progress.lock().unwrap_or_else(|error| error.into_inner());
                    state.bytes = accepted.bytes;
                    state.sections = accepted.sections.clone();
                    state.coverage = accepted.coverage.clone();
                })
            })
        }));
        // All writer handles belong to run and are gone before acknowledgement.
        let mut state = progress.lock().unwrap_or_else(|error| error.into_inner());
        state.running = false;
        invalidate(&mut state);
        match outcome {
            Ok(Ok(accepted)) => {
                finish(&mut state, State::Ready);
                state.bytes = accepted.bytes;
                state.sections = accepted.sections;
                state.coverage = accepted.coverage;
                state.error = None;
            }
            Ok(Err(error)) => {
                if error.kind() != std::io::ErrorKind::Interrupted {
                    finish(&mut state, State::Failed);
                } else if state.paused.is_some() {
                    // A pause keeps the same unfinished job and its claim.
                    state.state = State::Paused;
                } else {
                    finish(&mut state, State::Cancelled);
                }
                state.error = Some(error.to_string());
                let phase = state.state;
                for section in &mut state.sections {
                    if matches!(section.state, State::Queued | State::Preparing) {
                        section.state = phase;
                    }
                }
            }
            Err(_) => {
                finish(&mut state, State::Failed);
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
        finish(&mut state, State::Failed);
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
        // The store from the recording's path alone: a damaged manifest that
        // cannot be read or bound is cleared too.
        let _ = content;
        recording_store(source.path())?.clear()
    })();
    let mut state = progress.lock().unwrap_or_else(|error| error.into_inner());
    state.clearing = false;
    invalidate(&mut state);
    if result.is_ok() {
        state.cleared = true;
        state.state = State::Idle;
        state.claimed = false;
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
    wait_finished(source, cancel_request)
}

/// Wait for the source's admitted job to finish, without stopping it.
pub fn wait_finished(source: &Source, cancel_request: &AtomicBool) -> std::io::Result<()> {
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
