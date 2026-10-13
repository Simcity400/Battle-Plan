//! Manual compaction of finished recordings, with progress and results. The
//! recorder compacts automatically after the game closes when File size is Small.
mod platform;
mod store;
#[cfg(test)]
mod tests;

use platform::{BackgroundPriority, game_running};
use replay::{
    container::{
        CompactionOutcome, CompactionResult, CompactionStage, compact_with, replay_revision,
    },
    recorder_settings::{RecorderSettings, RecordingCompression},
};
use serde::Serialize;
use std::{
    io,
    path::{Path, PathBuf},
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, Ordering},
    },
    thread::{self, JoinHandle},
};
use tauri::{AppHandle, Emitter, State};

#[derive(Clone, Default, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CompactionProgress {
    running: bool,
    game_running: bool,
    total: usize,
    completed: usize,
    compacted: usize,
    skipped: usize,
    bytes_saved: u64,
    current: Option<String>,
    blocks_total: usize,
    blocks_completed: usize,
    unchanged: Vec<CompactionFailure>,
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct CompactionFailure {
    path: String,
    reason: String,
}

#[derive(Default)]
struct Inner {
    progress: Mutex<CompactionProgress>,
    active: AtomicBool,
    closed: AtomicBool,
    store: Mutex<store::Store>,
    workers: Mutex<Vec<JoinHandle<()>>>,
}
#[derive(Default)]
pub struct CompactionHost(Arc<Inner>);

fn cancelled() -> io::Error {
    io::Error::new(io::ErrorKind::Interrupted, "Compaction cancelled")
}

impl CompactionHost {
    /// Load the compaction hints. Nothing is compacted until the user asks.
    pub fn start(&self) -> io::Result<()> {
        *self
            .0
            .store
            .lock()
            .map_err(|_| io::Error::other("Compaction state unavailable"))? =
            store::Store::load_from(&store::path());
        Ok(())
    }
    pub fn shutdown(&self) {
        self.0.closed.store(true, Ordering::Release);
        // Cancellation is cooperative; app shutdown must not join a codec or I/O
        // call. Workers own their state and clean their candidates when they yield.
        if let Ok(mut workers) = self.0.workers.lock() {
            workers.clear();
        }
    }
    fn launch(&self, paths: Vec<PathBuf>, app: AppHandle) -> Result<CompactionProgress, String> {
        if self.0.closed.load(Ordering::Acquire) {
            return Err("Compaction is closing".into());
        }
        if self
            .0
            .active
            .compare_exchange(false, true, Ordering::AcqRel, Ordering::Acquire)
            .is_err()
        {
            return Err(
                "Recording compaction is already running. See Settings for progress and results."
                    .into(),
            );
        }
        let initial = CompactionProgress {
            running: true,
            game_running: game_running().unwrap_or(false),
            total: paths.len(),
            ..Default::default()
        };
        *self.0.progress.lock().unwrap_or_else(|e| e.into_inner()) = initial.clone();
        let inner = Arc::clone(&self.0);
        let worker = match thread::Builder::new()
            .name("replay-manual-compaction".into())
            .spawn(move || {
                match BackgroundPriority::enter() {
                    Ok(_priority) => {
                        for path in paths {
                            if inner.closed.load(Ordering::Acquire) {
                                break;
                            }
                            {
                                let mut progress =
                                    inner.progress.lock().unwrap_or_else(|e| e.into_inner());
                                progress.current = Some(path.to_string_lossy().into_owned());
                                progress.blocks_total = 0;
                                progress.blocks_completed = 0;
                            }
                            let before = replay_revision(&path).ok();
                            let result = run_recording(&inner, &path);
                            if before.is_some_and(|before| {
                                replay_revision(&path).is_ok_and(|after| before != after)
                            }) {
                                // Both manual entry points refresh any changed revision through
                                // the frontend's existing campaign reconnect path.
                                if let Err(error) = app.emit(
                                    "recording-compacted",
                                    path.to_string_lossy().into_owned(),
                                ) {
                                    eprintln!(
                                        "Compacted campaign reload notification failed: {error}"
                                    );
                                }
                            }
                            record_manual_result(
                                &mut inner.progress.lock().unwrap_or_else(|e| e.into_inner()),
                                &path,
                                result,
                            );
                        }
                    }
                    Err(error) => inner
                        .progress
                        .lock()
                        .unwrap_or_else(|e| e.into_inner())
                        .unchanged
                        .push(CompactionFailure {
                            path: String::new(),
                            reason: error.to_string(),
                        }),
                }
                let mut progress = inner.progress.lock().unwrap_or_else(|e| e.into_inner());
                progress.running = false;
                progress.current = None;
                inner.active.store(false, Ordering::Release);
            }) {
            Ok(worker) => worker,
            Err(error) => {
                self.0.active.store(false, Ordering::Release);
                self.0
                    .progress
                    .lock()
                    .unwrap_or_else(|e| e.into_inner())
                    .running = false;
                return Err(error.to_string());
            }
        };
        let mut workers = self.0.workers.lock().unwrap_or_else(|e| e.into_inner());
        workers.retain(|worker| !worker.is_finished());
        workers.push(worker);
        Ok(initial)
    }
}

/// Library traversal stops when the app closes.
fn scan_checkpoint(inner: &Inner) -> Result<(), String> {
    if inner.closed.load(Ordering::Acquire) {
        return Err(cancelled().to_string());
    }
    Ok(())
}

fn compact_in_host(
    inner: &Inner,
    path: &Path,
    mut observe: impl FnMut(CompactionStage) -> io::Result<()>,
) -> io::Result<CompactionResult> {
    compact_with(path, |stage, _| {
        checkpoint(inner, stage)?;
        observe(stage)
    })
}

fn run_recording(inner: &Inner, path: &Path) -> io::Result<CompactionResult> {
    let result = compact_in_host(inner, path, |_| Ok(()))?;
    if matches!(
        result.outcome,
        CompactionOutcome::Compacted | CompactionOutcome::AlreadySmall
    ) {
        let mut store = inner.store.lock().unwrap_or_else(|e| e.into_inner());
        store.remember(path, &result);
        if let Err(error) = store.save_to(&store::path()) {
            eprintln!("Compaction tracking could not be saved: {error}");
        }
    }
    Ok(result)
}
fn checkpoint(inner: &Inner, stage: CompactionStage) -> io::Result<()> {
    if inner.closed.load(Ordering::Acquire) {
        return Err(cancelled());
    }
    let mut progress = inner.progress.lock().unwrap_or_else(|e| e.into_inner());
    match stage {
        CompactionStage::Started { blocks } => progress.blocks_total = blocks,
        CompactionStage::BlockWritten(position) => progress.blocks_completed = position + 1,
        _ => {}
    }
    Ok(())
}
fn record_manual_result(
    progress: &mut CompactionProgress,
    path: &Path,
    result: io::Result<CompactionResult>,
) {
    progress.completed += 1;
    let reason = match result {
        Ok(result) if result.outcome == CompactionOutcome::Compacted => {
            progress.compacted += 1;
            progress.bytes_saved += result.bytes_before.saturating_sub(result.bytes_after);
            return;
        }
        Ok(result) if result.outcome == CompactionOutcome::AlreadySmall => {
            "Already as small as it gets".into()
        }
        Ok(_) => "Changed while compacting; original kept".into(),
        Err(error) if error.kind() == io::ErrorKind::WouldBlock => "Busy; try again later".into(),
        Err(error) => error.to_string(),
    };
    progress.skipped += 1;
    progress.unchanged.push(CompactionFailure {
        path: path.to_string_lossy().into_owned(),
        reason,
    });
}

#[tauri::command]
pub async fn recorder_settings() -> Result<RecorderSettings, String> {
    tauri::async_runtime::spawn_blocking(RecorderSettings::load)
        .await
        .map_err(|e| e.to_string())?
        .map_err(|e| e.to_string())
}
#[tauri::command]
pub async fn set_recording_compression(
    compression: RecordingCompression,
) -> Result<RecorderSettings, String> {
    tauri::async_runtime::spawn_blocking(move || {
        RecorderSettings::update(|settings| settings.compression = compression)
            .map(|(settings, ())| settings)
    })
    .await
    .map_err(|e| e.to_string())?
    .map_err(|e| e.to_string())
}
#[tauri::command]
pub fn compact_recording(
    path: String,
    host: State<'_, CompactionHost>,
    app: AppHandle,
) -> Result<CompactionProgress, String> {
    host.launch(vec![PathBuf::from(path)], app)
}
#[tauri::command]
pub async fn compact_replay_folder(
    host: State<'_, CompactionHost>,
    app: AppHandle,
) -> Result<CompactionProgress, String> {
    let inner = Arc::clone(&host.0);
    tauri::async_runtime::spawn_blocking(move || {
        let folder = crate::replay_library::configured_folder()?;
        let paths = crate::replay_library::scan_with(&folder, || scan_checkpoint(&inner))?
            .files
            .into_iter()
            .map(|file| PathBuf::from(file.path))
            .collect();
        CompactionHost(inner).launch(paths, app)
    })
    .await
    .map_err(|e| e.to_string())?
}
#[tauri::command]
pub async fn compaction_progress(
    host: State<'_, CompactionHost>,
) -> Result<CompactionProgress, String> {
    let mut progress = host.0.progress.lock().map_err(|e| e.to_string())?.clone();
    tauri::async_runtime::spawn_blocking(move || {
        progress.game_running = game_running().unwrap_or(false);
        Ok(progress)
    })
    .await
    .map_err(|e| e.to_string())?
}
#[tauri::command]
pub async fn recording_compaction_status(
    path: String,
    host: State<'_, CompactionHost>,
) -> Result<bool, String> {
    let inner = Arc::clone(&host.0);
    tauri::async_runtime::spawn_blocking(move || {
        let path = PathBuf::from(path);
        let revision = replay_revision(&path).map_err(|e| e.to_string())?;
        Ok(!inner
            .store
            .lock()
            .map_err(|e| e.to_string())?
            .eligible(&path, &revision))
    })
    .await
    .map_err(|e| e.to_string())?
}
