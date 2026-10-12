//! App-owned automatic compaction and independent manual progress/results.
mod platform;
mod store;
#[cfg(test)]
mod tests;

use platform::{BackgroundPriority, game_running, recorder_running};
use replay::{
    container::{
        CompactionOutcome, CompactionResult, CompactionStage, compact_with, replay_revision,
    },
    recorder_settings::{RecorderSettings, RecordingCompression},
};
use serde::Serialize;
use std::{
    collections::BTreeMap,
    io,
    path::{Path, PathBuf},
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, Ordering},
    },
    thread::{self, JoinHandle},
    time::{Duration, Instant},
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
    // Automatic work never reads or writes the user's manual result snapshot.
    progress: Mutex<CompactionProgress>,
    manual_active: AtomicBool,
    closed: AtomicBool,
    serial: Mutex<()>,
    store: Mutex<store::Store>,
    loaded: Mutex<BTreeMap<String, usize>>,
    startup_recording: Mutex<Option<String>>,
    workers: Mutex<Vec<JoinHandle<()>>>,
}
#[derive(Default)]
pub struct CompactionHost(Arc<Inner>);

fn automatic_allowed(
    compression: RecordingCompression,
    game_running: bool,
    recorder_running: bool,
) -> bool {
    compression == RecordingCompression::Small && !game_running && !recorder_running
}
fn environment() -> io::Result<(RecordingCompression, bool, bool)> {
    Ok((
        RecorderSettings::load()?.compression,
        game_running()?,
        recorder_running()?,
    ))
}
fn cancelled() -> io::Error {
    io::Error::new(io::ErrorKind::Interrupted, "Compaction cancelled")
}

/// Manual work never consults the game/preference gate. Automatic work checks
/// it before starting, during streaming/verification, and before publication.
fn compact_in_environment(
    path: &Path,
    automatic: bool,
    mut environment: impl FnMut() -> io::Result<(RecordingCompression, bool, bool)>,
    mut checkpoint: impl FnMut(CompactionStage, &Path) -> io::Result<()>,
) -> io::Result<CompactionResult> {
    let mut check = || {
        let (preference, game, recorder) = environment()?;
        if automatic_allowed(preference, game, recorder) {
            Ok(())
        } else {
            Err(cancelled())
        }
    };
    if automatic {
        check()?;
    }
    let mut checked = Instant::now();
    compact_with(path, |stage, temporary| {
        checkpoint(stage, temporary)?;
        if automatic
            && (checked.elapsed() >= Duration::from_millis(250)
                || matches!(
                    stage,
                    CompactionStage::BeforeReplace | CompactionStage::BeforeSwap
                ))
        {
            check()?;
            checked = Instant::now();
        }
        Ok(())
    })
}

impl CompactionHost {
    pub fn start(&self) -> io::Result<()> {
        *self
            .0
            .store
            .lock()
            .map_err(|_| io::Error::other("Compaction state unavailable"))? =
            store::Store::load_from(&store::path());
        // The viewer opens a command-line recording before frontend leases exist.
        // Transfer this initial retention to its first frontend open.
        if let Some(path) = std::env::args_os().skip(1).map(PathBuf::from).find(|path| {
            path.extension()
                .is_some_and(|ext| ext.eq_ignore_ascii_case("battleplan") || ext.eq_ignore_ascii_case("hoi4replay"))
        }) {
            let key = store::key(&path);
            self.0
                .loaded
                .lock()
                .unwrap_or_else(|error| error.into_inner())
                .insert(key.clone(), 1);
            *self
                .0
                .startup_recording
                .lock()
                .unwrap_or_else(|error| error.into_inner()) = Some(key);
        }
        let inner = Arc::clone(&self.0);
        let worker = thread::Builder::new()
            .name("replay-auto-compaction".into())
            .spawn(move || {
                let _priority = match BackgroundPriority::enter() {
                    Ok(priority) => priority,
                    Err(error) => {
                        eprintln!("Automatic compaction: {error}");
                        return;
                    }
                };
                let mut deferred = BTreeMap::new();
                while !inner.closed.load(Ordering::Acquire) {
                    let mut scan_checked = Instant::now();
                    if !inner.manual_active.load(Ordering::Acquire)
                        && environment().is_ok_and(|(preference, game, recorder)| {
                            automatic_allowed(preference, game, recorder)
                        })
                        && let Ok(_serial) = inner.serial.try_lock()
                        && let Ok(Some(folder)) = crate::replay_library::configured_folder()
                        && let Ok(library) = crate::replay_library::scan_with(&folder, || {
                            scan_checkpoint(&inner, true, &mut scan_checked)
                        })
                    {
                        for file in library.files {
                            if inner.manual_active.load(Ordering::Acquire)
                                || inner.closed.load(Ordering::Acquire)
                            {
                                break;
                            }
                            let path = PathBuf::from(file.path);
                            if is_loaded(&inner, &path) {
                                continue;
                            }
                            let Ok(revision) = replay_revision(&path) else {
                                continue;
                            };
                            if !inner
                                .store
                                .lock()
                                .unwrap_or_else(|e| e.into_inner())
                                .eligible(&path, &revision)
                            {
                                continue;
                            }
                            // Failed/busy files remain eligible, with a short retry
                            // delay so a held file cannot monopolize background work.
                            if deferred.get(&path).is_some_and(|(old, until)| {
                                old == &revision && Instant::now() < *until
                            }) {
                                continue;
                            }
                            let result = run_recording(&inner, &path, true);
                            if result
                                .as_ref()
                                .is_err_and(|error| error.kind() == io::ErrorKind::Interrupted)
                            {
                                break;
                            }
                            deferred
                                .insert(path, (revision, Instant::now() + Duration::from_secs(60)));
                        }
                    }
                    for _ in 0..20 {
                        if inner.closed.load(Ordering::Acquire) {
                            break;
                        }
                        thread::sleep(Duration::from_millis(100));
                    }
                }
            })?;
        self.0
            .workers
            .lock()
            .map_err(|_| io::Error::other("Compaction state unavailable"))?
            .push(worker);
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
            .manual_active
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
                // A manual request promptly cancels the current automatic candidate;
                // the UI never waits for this worker's serial lock.
                let _serial = inner.serial.lock().unwrap_or_else(|e| e.into_inner());
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
                            let result = run_recording(&inner, &path, false);
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
                inner.manual_active.store(false, Ordering::Release);
            }) {
            Ok(worker) => worker,
            Err(error) => {
                self.0.manual_active.store(false, Ordering::Release);
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

fn is_loaded(inner: &Inner, path: &Path) -> bool {
    inner
        .loaded
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .contains_key(&store::key(path))
}

fn scan_checkpoint(inner: &Inner, automatic: bool, checked: &mut Instant) -> Result<(), String> {
    checkpoint(inner, automatic, CompactionStage::Reading).map_err(|error| error.to_string())?;
    if automatic && checked.elapsed() >= Duration::from_millis(250) {
        *checked = Instant::now();
        let (preference, game, recorder) = environment().map_err(|error| error.to_string())?;
        if !automatic_allowed(preference, game, recorder) {
            return Err(cancelled().to_string());
        }
    }
    Ok(())
}

/// The frontend retains pending opens and every live source lease. Automatic
/// compaction excludes these paths; manual compaction always remains available.
#[tauri::command]
pub async fn set_compaction_recording_open(
    path: String,
    open: bool,
    host: State<'_, CompactionHost>,
) -> Result<(), String> {
    let inner = Arc::clone(&host.0);
    tauri::async_runtime::spawn_blocking(move || {
        let key = store::key(Path::new(&path));
        let mut loaded = inner.loaded.lock().map_err(|error| error.to_string())?;
        if open {
            let mut startup = inner
                .startup_recording
                .lock()
                .map_err(|error| error.to_string())?;
            if startup.as_ref() == Some(&key) {
                *startup = None;
            } else {
                *loaded.entry(key).or_default() += 1;
            }
        } else if let Some(count) = loaded.get_mut(&key) {
            *count -= 1;
            if *count == 0 {
                loaded.remove(&key);
            }
        }
        Ok(())
    })
    .await
    .map_err(|error| error.to_string())?
}

fn compact_in_host(
    inner: &Inner,
    path: &Path,
    automatic: bool,
    environment: impl FnMut() -> io::Result<(RecordingCompression, bool, bool)>,
    mut observe: impl FnMut(CompactionStage) -> io::Result<()>,
) -> io::Result<CompactionResult> {
    let mut publication_guard = None;
    let result = compact_in_environment(path, automatic, environment, |stage, _| {
        checkpoint(inner, automatic, stage)?;
        observe(stage)?;
        if automatic {
            let loaded = inner.loaded.lock().unwrap_or_else(|e| e.into_inner());
            if loaded.contains_key(&store::key(path)) {
                return Err(cancelled());
            }
            if stage == CompactionStage::BeforeSwap {
                // Retention and the final publication cannot pass one another.
                // Keep this short guard until compact_with returns.
                publication_guard = Some(loaded);
            }
        }
        Ok(())
    });
    drop(publication_guard);
    result
}

fn run_recording(inner: &Inner, path: &Path, automatic: bool) -> io::Result<CompactionResult> {
    let result = compact_in_host(inner, path, automatic, environment, |_| Ok(()))?;
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
fn checkpoint(inner: &Inner, automatic: bool, stage: CompactionStage) -> io::Result<()> {
    if inner.closed.load(Ordering::Acquire)
        || automatic && inner.manual_active.load(Ordering::Acquire)
    {
        return Err(cancelled());
    }
    if !automatic {
        let mut progress = inner.progress.lock().unwrap_or_else(|e| e.into_inner());
        match stage {
            CompactionStage::Started { blocks } => progress.blocks_total = blocks,
            CompactionStage::BlockWritten(position) => progress.blocks_completed = position + 1,
            _ => {}
        }
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
        let mut settings = RecorderSettings::load()?;
        settings.compression = compression;
        settings.save()?;
        Ok::<_, io::Error>(settings)
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
        let folder =
            crate::replay_library::configured_folder()?.ok_or("Choose a replay folder first")?;
        let mut checked = Instant::now();
        let paths = crate::replay_library::scan_with(&folder, || {
            scan_checkpoint(&inner, false, &mut checked)
        })?
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
