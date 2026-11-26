use crate::{export_settings, native::NativeHost, popout};
use serde::{Deserialize, Serialize};
use std::{
    path::PathBuf,
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, AtomicU64, Ordering},
    },
};
use tauri::{AppHandle, Emitter, State, WebviewWindow};
use viewer::video_export::{
    ExportProgress, MapExportSnapshot, MapVideoOutcome, MapVideoRequest, PartialVideo,
    render_map_video_preview, run_map_video_export,
};

const PROGRESS_EVENT: &str = "video-export-progress";

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PartialExportInfo {
    pub through_hour: i32,
    pub frames: u64,
    pub duration_seconds: f64,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ExportJobSnapshot {
    pub id: u64,
    pub state: String,
    pub target_label: String,
    pub destination: Option<String>,
    pub phase: String,
    pub frame: u64,
    pub total_frames: u64,
    pub campaign_hour: Option<i32>,
    pub elapsed_ms: u64,
    pub eta_ms: Option<u64>,
    pub backend: Option<String>,
    pub warnings: Vec<String>,
    pub error: Option<String>,
    pub partial: Option<PartialExportInfo>,
}

impl Default for ExportJobSnapshot {
    fn default() -> Self {
        Self {
            id: 0,
            state: "idle".to_owned(),
            target_label: String::new(),
            destination: None,
            phase: String::new(),
            frame: 0,
            total_frames: 0,
            campaign_hour: None,
            elapsed_ms: 0,
            eta_ms: None,
            backend: None,
            warnings: Vec::new(),
            error: None,
            partial: None,
        }
    }
}

struct ActiveJob {
    id: u64,
    cancel: Arc<AtomicBool>,
    committing: bool,
    staged_path: PathBuf,
    join: Option<std::thread::JoinHandle<()>>,
}

struct PendingPartial {
    id: u64,
    staged_path: PathBuf,
    destination: PathBuf,
    saving: bool,
}

#[derive(Default)]
struct Inner {
    snapshot: ExportJobSnapshot,
    active: Option<ActiveJob>,
    pending_partial: Option<PendingPartial>,
    preview_cancel: Option<Arc<AtomicBool>>,
}

#[derive(Clone, Default)]
pub struct ExportJobs {
    inner: Arc<Mutex<Inner>>,
    next_id: Arc<AtomicU64>,
}

impl ExportJobs {
    fn emit(&self, app: &AppHandle, snapshot: ExportJobSnapshot) {
        let _ = app.emit(PROGRESS_EVENT, snapshot);
    }

    pub fn snapshot(&self) -> ExportJobSnapshot {
        self.inner
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .snapshot
            .clone()
    }

    pub fn is_running(&self) -> bool {
        self.inner
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .active
            .is_some()
    }

    pub fn cancel_and_cleanup_for_shutdown(&self) {
        let (join, staged, partial_stage) = {
            let mut inner = self.inner.lock().unwrap_or_else(|e| e.into_inner());
            if let Some(cancel) = inner.preview_cancel.take() {
                cancel.store(true, Ordering::Release);
            }
            let (join, staged) = if let Some(mut active) = inner.active.take() {
                active.cancel.store(true, Ordering::Release);
                (active.join.take(), Some(active.staged_path))
            } else {
                (None, None)
            };
            let partial_stage = inner.pending_partial.take().map(|p| p.staged_path);
            (join, staged, partial_stage)
        };
        if let Some(handle) = join {
            let deadline = std::time::Instant::now() + std::time::Duration::from_secs(3);
            while !handle.is_finished() && std::time::Instant::now() < deadline {
                std::thread::sleep(std::time::Duration::from_millis(25));
            }
            if handle.is_finished() {
                let _ = handle.join();
            }
        }
        if let Some(path) = staged {
            cleanup_stage(&path);
        }
        if let Some(path) = partial_stage {
            cleanup_stage(&path);
        }
    }
}

fn cleanup_stage(path: &std::path::Path) {
    if export_settings::remove_stage_with_sidecars(path) {
        export_settings::clear_pending_stage(path);
    }
}

fn partial_preferred_path(destination: &std::path::Path) -> PathBuf {
    let stem = destination
        .file_stem()
        .and_then(|s| s.to_str())
        .unwrap_or("video");
    let ext = destination
        .extension()
        .and_then(|s| s.to_str())
        .unwrap_or("mp4");
    destination.with_file_name(format!("{stem}-partial.{ext}"))
}

#[tauri::command]
pub fn video_export_state(jobs: State<'_, ExportJobs>) -> ExportJobSnapshot {
    jobs.snapshot()
}

#[tauri::command]
pub fn video_export_active(jobs: State<'_, ExportJobs>) -> bool {
    jobs.is_running()
}

#[tauri::command]
pub async fn map_export_snapshot(
    host: State<'_, NativeHost>,
    popouts: State<'_, popout::PopOuts>,
    view_id: Option<String>,
) -> Result<MapExportSnapshot, String> {
    let answered = if view_id.as_deref() == Some(popout::VIEW_ID) {
        let generation = popouts.current_generation();
        popout::snapshot(&popouts, generation)?
    } else {
        host.snapshot(view_id)?
    };
    answered
        .await
        .map_err(|_| "The map stopped before its snapshot arrived".to_owned())?
}

#[tauri::command]
pub async fn choose_video_export_path(
    window: WebviewWindow,
    default_name: String,
) -> Result<Option<String>, String> {
    let file_name = if default_name.ends_with(".mp4") {
        default_name
    } else {
        format!("{default_name}.mp4")
    };
    tauri::async_runtime::spawn_blocking(move || {
        rfd::FileDialog::new()
            .set_parent(&window)
            .set_title("Export MP4 video")
            .add_filter("MP4 video", &["mp4"])
            .set_file_name(&file_name)
            .save_file()
            .map(|p| p.to_string_lossy().into_owned())
    })
    .await
    .map_err(|e| e.to_string())
}

#[tauri::command]
pub async fn preview_map_video(
    jobs: State<'_, ExportJobs>,
    mut request: MapVideoRequest,
    preview_hour: i32,
) -> Result<tauri::ipc::Response, String> {
    let cancel = Arc::new(AtomicBool::new(false));
    {
        let mut inner = jobs.inner.lock().unwrap_or_else(|e| e.into_inner());
        if let Some(prev) = inner.preview_cancel.replace(Arc::clone(&cancel)) {
            prev.store(true, Ordering::Release);
        }
    }
    let settings = export_settings::load();
    request.backend = settings.backend;
    if request.ffmpeg_path.is_none() {
        request.ffmpeg_path = settings.ffmpeg_path;
    }
    let (config, allow_unverified) = crate::dashboard_service::host_config()?;
    request.allow_unverified = allow_unverified;
    let bytes = tauri::async_runtime::spawn_blocking(move || {
        render_map_video_preview(config, request, preview_hour, cancel)
    })
    .await
    .map_err(|e| e.to_string())??;
    Ok(tauri::ipc::Response::new(bytes))
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct StartMapVideoExportArgs {
    pub request: MapVideoRequest,
    pub destination: String,
    pub target_label: String,
}

#[tauri::command]
pub fn start_map_video_export(
    app: AppHandle,
    jobs: State<'_, ExportJobs>,
    args: StartMapVideoExportArgs,
) -> Result<ExportJobSnapshot, String> {
    let destination = PathBuf::from(args.destination.trim());
    if destination.as_os_str().is_empty() {
        return Err("Choose a destination file before exporting.".into());
    }
    let mut request = args.request;
    viewer::video_export::ensure_distinct_export_path(
        std::path::Path::new(&request.path),
        &destination,
    )?;
    let settings = export_settings::load();
    request.backend = settings.backend;
    if request.ffmpeg_path.is_none() {
        request.ffmpeg_path = settings.ffmpeg_path;
    }
    let (config, allow_unverified) = crate::dashboard_service::host_config()?;
    request.allow_unverified = allow_unverified;
    let staged_path = replay_settings::exit::stage_path(&destination)
        .map_err(|e| format!("Couldn’t prepare staging file beside destination: {e}"))?;
    export_settings::record_pending_stage(&staged_path);

    let job_id = jobs.next_id.fetch_add(1, Ordering::AcqRel) + 1;
    let cancel = Arc::new(AtomicBool::new(false));

    let (initial_snapshot, old_partial_stage) = {
        let mut inner = jobs.inner.lock().unwrap_or_else(|e| e.into_inner());
        if inner.active.is_some() {
            drop(inner);
            cleanup_stage(&staged_path);
            return Err("Another video export is already running.".into());
        }
        if inner.pending_partial.as_ref().is_some_and(|p| p.saving) {
            drop(inner);
            cleanup_stage(&staged_path);
            return Err("Wait for the partial export save to finish before starting a new export.".into());
        }
        let old_partial = inner.pending_partial.take().map(|p| p.staged_path);
        inner.active = Some(ActiveJob {
            id: job_id,
            cancel: Arc::clone(&cancel),
            committing: false,
            staged_path: staged_path.clone(),
            join: None,
        });
        inner.snapshot = ExportJobSnapshot {
            id: job_id,
            state: "running".to_owned(),
            target_label: args.target_label,
            destination: Some(destination.to_string_lossy().into_owned()),
            phase: "preparing".to_owned(),
            frame: 0,
            total_frames: 0,
            campaign_hour: Some(request.from_hour),
            elapsed_ms: 0,
            eta_ms: None,
            backend: None,
            warnings: Vec::new(),
            error: None,
            partial: None,
        };
        (inner.snapshot.clone(), old_partial)
    };
    if let Some(old_stage) = old_partial_stage {
        cleanup_stage(&old_stage);
    }

    let jobs_cloned = jobs.inner().clone();
    let app_for_progress = app.clone();
    let jobs_for_progress = jobs_cloned.clone();
    let progress_cb: Arc<dyn Fn(ExportProgress) + Send + Sync> = Arc::new(move |prog| {
        let updated = {
            let mut inner = jobs_for_progress.inner.lock().unwrap_or_else(|e| e.into_inner());
            if inner.snapshot.id != job_id || inner.snapshot.state != "running" {
                return;
            }
            inner.snapshot.phase = prog.phase;
            inner.snapshot.frame = prog.frame;
            inner.snapshot.total_frames = prog.total_frames;
            inner.snapshot.campaign_hour = prog.campaign_hour;
            inner.snapshot.elapsed_ms = prog.elapsed_ms;
            inner.snapshot.eta_ms = prog.eta_ms;
            if prog.backend.is_some() {
                inner.snapshot.backend = prog.backend;
            }
            inner.snapshot.clone()
        };
        jobs_for_progress.emit(&app_for_progress, updated);
    });

    let app_for_worker = app.clone();
    let worker_staged = staged_path.clone();
    let worker_handle = std::thread::Builder::new()
        .name(format!("video-export-{job_id}"))
        .spawn(move || {
            let outcome = run_map_video_export(
                config,
                request,
                worker_staged.clone(),
                Arc::clone(&cancel),
                Some(progress_cb),
            );
            let final_snapshot = match outcome {
                MapVideoOutcome::Completed {
                    staged_path: finished_stage,
                    frames,
                    audio_frames: _,
                    backend,
                    mut warnings,
                } => {
                    let proceed = {
                        let mut inner = jobs_cloned.inner.lock().unwrap_or_else(|e| e.into_inner());
                        let cancelled = cancel.load(Ordering::Acquire)
                            || !inner.active.as_ref().is_some_and(|a| a.id == job_id);
                        if cancelled {
                            if inner.active.as_ref().is_some_and(|a| a.id == job_id) {
                                inner.active = None;
                            }
                            if inner.snapshot.id == job_id {
                                inner.snapshot.state = "cancelled".to_owned();
                                inner.snapshot.phase = "cancelled".to_owned();
                                inner.snapshot.error = None;
                                inner.snapshot.partial = None;
                            }
                            false
                        } else {
                            if let Some(active) = inner.active.as_mut() {
                                active.committing = true;
                            }
                            true
                        }
                    };
                    if !proceed {
                        cleanup_stage(&finished_stage);
                        if finished_stage != worker_staged {
                            cleanup_stage(&worker_staged);
                        }
                        jobs_cloned.snapshot()
                    } else {
                        let published = replay_settings::exit::EXIT
                            .admit(replay_settings::exit::What::Export)
                            .map_err(|e| e.to_string())
                            .and_then(|receipt| {
                                receipt
                                    .replace_staged(&finished_stage, &destination)
                                    .map_err(|e| e.to_string())
                            });
                        let (snapshot, publish_ok) = {
                            let mut inner =
                                jobs_cloned.inner.lock().unwrap_or_else(|e| e.into_inner());
                            if inner.active.as_ref().is_some_and(|a| a.id == job_id) {
                                inner.active = None;
                            }
                            let ok = match published {
                                Ok(applied) => {
                                    if let Some(w) = applied.warning {
                                        warnings.push(w);
                                    }
                                    if inner.snapshot.id == job_id {
                                        inner.snapshot.state = "completed".to_owned();
                                        inner.snapshot.phase = "done".to_owned();
                                        inner.snapshot.frame = frames;
                                        inner.snapshot.total_frames = frames;
                                        inner.snapshot.eta_ms = Some(0);
                                        inner.snapshot.backend = Some(backend);
                                        inner.snapshot.warnings = warnings;
                                        inner.snapshot.error = None;
                                        inner.snapshot.partial = None;
                                    }
                                    true
                                }
                                Err(error) => {
                                    if inner.snapshot.id == job_id {
                                        inner.snapshot.state = "failed".to_owned();
                                        inner.snapshot.phase = "failed".to_owned();
                                        inner.snapshot.error = Some(format!(
                                            "Rendered video couldn’t be published to {}: {error}",
                                            destination.display()
                                        ));
                                        inner.snapshot.partial = None;
                                    }
                                    false
                                }
                            };
                            (inner.snapshot.clone(), ok)
                        };
                        if publish_ok {
                            cleanup_stage(&worker_staged);
                            if finished_stage != worker_staged {
                                export_settings::clear_pending_stage(&finished_stage);
                            }
                        } else {
                            cleanup_stage(&finished_stage);
                            if finished_stage != worker_staged {
                                cleanup_stage(&worker_staged);
                            }
                        }
                        snapshot
                    }
                }
                MapVideoOutcome::Cancelled => {
                    cleanup_stage(&worker_staged);
                    let mut inner = jobs_cloned.inner.lock().unwrap_or_else(|e| e.into_inner());
                    if inner.active.as_ref().is_some_and(|a| a.id == job_id) {
                        inner.active = None;
                    }
                    if inner.snapshot.id == job_id {
                        inner.snapshot.state = "cancelled".to_owned();
                        inner.snapshot.phase = "cancelled".to_owned();
                        inner.snapshot.error = None;
                        inner.snapshot.partial = None;
                    }
                    inner.snapshot.clone()
                }
                MapVideoOutcome::Failed {
                    error,
                    campaign_hour,
                    partial,
                } => {
                    let mut discard_paths = Vec::new();
                    let snapshot = {
                        let mut inner = jobs_cloned.inner.lock().unwrap_or_else(|e| e.into_inner());
                        let cancelled = cancel.load(Ordering::Acquire)
                            || !inner.active.as_ref().is_some_and(|a| a.id == job_id);
                        if inner.active.as_ref().is_some_and(|a| a.id == job_id) {
                            inner.active = None;
                        }
                        if cancelled {
                            if inner.snapshot.id == job_id {
                                inner.snapshot.state = "cancelled".to_owned();
                                inner.snapshot.phase = "cancelled".to_owned();
                                inner.snapshot.error = None;
                                inner.snapshot.partial = None;
                            }
                            if let Some(p) = partial {
                                discard_paths.push(p.staged_path);
                            }
                            discard_paths.push(worker_staged.clone());
                        } else if inner.snapshot.id == job_id {
                            inner.snapshot.state = "failed".to_owned();
                            inner.snapshot.phase = "failed".to_owned();
                            inner.snapshot.campaign_hour =
                                campaign_hour.or(inner.snapshot.campaign_hour);
                            inner.snapshot.error = Some(error);
                            let mut kept_stage = None;
                            inner.snapshot.partial = partial.map(
                                |PartialVideo {
                                     staged_path: partial_stage,
                                     frames,
                                     through_hour,
                                     duration_seconds,
                                     backend,
                                 }| {
                                    inner.snapshot.backend = Some(backend);
                                    kept_stage = Some(partial_stage.clone());
                                    inner.pending_partial = Some(PendingPartial {
                                        id: job_id,
                                        staged_path: partial_stage,
                                        destination: destination.clone(),
                                        saving: false,
                                    });
                                    PartialExportInfo {
                                        through_hour,
                                        frames,
                                        duration_seconds,
                                    }
                                },
                            );
                            if let Some(kept) = kept_stage {
                                if kept != worker_staged {
                                    export_settings::record_pending_stage(&kept);
                                    discard_paths.push(worker_staged.clone());
                                }
                            } else {
                                discard_paths.push(worker_staged.clone());
                            }
                        } else {
                            if let Some(p) = partial {
                                discard_paths.push(p.staged_path);
                            }
                            discard_paths.push(worker_staged.clone());
                        }
                        inner.snapshot.clone()
                    };
                    for path in discard_paths {
                        cleanup_stage(&path);
                    }
                    snapshot
                }
            };
            jobs_cloned.emit(&app_for_worker, final_snapshot);
        })
        .map_err(|e| {
            let mut inner = jobs.inner.lock().unwrap_or_else(|e| e.into_inner());
            inner.active = None;
            drop(inner);
            cleanup_stage(&staged_path);
            format!("Couldn’t start export worker thread: {e}")
        })?;
    {
        let mut inner = jobs.inner.lock().unwrap_or_else(|e| e.into_inner());
        if let Some(active) = inner.active.as_mut()
            && active.id == job_id
        {
            active.join = Some(worker_handle);
        }
    }

    jobs.emit(&app, initial_snapshot.clone());
    Ok(initial_snapshot)
}

#[tauri::command]
pub fn cancel_video_export(app: AppHandle, jobs: State<'_, ExportJobs>) -> ExportJobSnapshot {
    let snapshot = {
        let mut inner = jobs.inner.lock().unwrap_or_else(|e| e.into_inner());
        if let Some(active) = &inner.active
            && !active.committing
        {
            active.cancel.store(true, Ordering::Release);
            inner.snapshot.phase = "cancelling".to_owned();
        }
        inner.snapshot.clone()
    };
    jobs.emit(&app, snapshot.clone());
    snapshot
}

#[tauri::command]
pub fn save_partial_video_export(
    app: AppHandle,
    jobs: State<'_, ExportJobs>,
    job_id: u64,
) -> Result<String, String> {
    let (staged_path, destination) = {
        let mut inner = jobs.inner.lock().unwrap_or_else(|e| e.into_inner());
        let Some(pending) = inner.pending_partial.as_mut().filter(|p| p.id == job_id) else {
            return Err("No finalized partial export is available to save.".into());
        };
        if pending.saving {
            return Err("Partial export is already being saved.".into());
        }
        pending.saving = true;
        (pending.staged_path.clone(), pending.destination.clone())
    };
    let preferred = partial_preferred_path(&destination);
    let published = replay_settings::exit::EXIT
        .admit(replay_settings::exit::What::Export)
        .map_err(|e| e.to_string())
        .and_then(|receipt| {
            receipt
                .publish_distinct(&staged_path, &preferred)
                .map_err(|e| format!("Couldn’t save partial video: {e}"))
        });
    match published {
        Ok(applied) => {
            cleanup_stage(&staged_path);
            let saved_path = applied.value.to_string_lossy().into_owned();
            let updated = {
                let mut inner = jobs.inner.lock().unwrap_or_else(|e| e.into_inner());
                if inner.pending_partial.as_ref().is_some_and(|p| p.id == job_id) {
                    inner.pending_partial = None;
                }
                if inner.snapshot.id == job_id {
                    inner.snapshot.state = "completed".to_owned();
                    inner.snapshot.phase = "done".to_owned();
                    inner.snapshot.destination = Some(saved_path.clone());
                    inner.snapshot.partial = None;
                    inner.snapshot.error = None;
                    if let Some(w) = applied.warning {
                        inner.snapshot.warnings.push(w);
                    }
                }
                inner.snapshot.clone()
            };
            jobs.emit(&app, updated);
            Ok(saved_path)
        }
        Err(error) => {
            let mut inner = jobs.inner.lock().unwrap_or_else(|e| e.into_inner());
            if let Some(pending) = inner.pending_partial.as_mut().filter(|p| p.id == job_id) {
                pending.saving = false;
            }
            Err(error)
        }
    }
}

#[tauri::command]
pub fn dismiss_video_export(app: AppHandle, jobs: State<'_, ExportJobs>) -> ExportJobSnapshot {
    let (snapshot, discarded_stage) = {
        let mut inner = jobs.inner.lock().unwrap_or_else(|e| e.into_inner());
        if inner.active.is_some() || inner.pending_partial.as_ref().is_some_and(|p| p.saving) {
            return inner.snapshot.clone();
        }
        let discarded = inner.pending_partial.take().map(|p| p.staged_path);
        inner.snapshot = ExportJobSnapshot::default();
        (inner.snapshot.clone(), discarded)
    };
    if let Some(path) = discarded_stage {
        cleanup_stage(&path);
    }
    jobs.emit(&app, snapshot.clone());
    snapshot
}

#[tauri::command]
pub fn reveal_export_file(path: String) -> Result<(), String> {
    let target = PathBuf::from(&path);
    if !target.exists() {
        return Err("Exported video file was not found.".into());
    }
    #[cfg(windows)]
    {
        std::process::Command::new("explorer.exe")
            .arg(format!("/select,{}", target.display()))
            .spawn()
            .map_err(|e| format!("Couldn’t open File Explorer: {e}"))?;
        Ok(())
    }
    #[cfg(not(windows))]
    {
        let _ = target;
        Err("Revealing exported files in the file manager is supported on Windows.".into())
    }
}
