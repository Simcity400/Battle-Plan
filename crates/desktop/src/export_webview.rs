use serde::{Deserialize, Serialize};
use std::{
    collections::HashMap,
    io::Cursor,
    path::PathBuf,
    sync::{
        Arc, Condvar, Mutex,
        atomic::{AtomicBool, AtomicU64, Ordering},
        mpsc,
    },
    time::{Duration, Instant},
};
use tauri::{AppHandle, Emitter, Manager, WebviewUrl, WebviewWindow, WebviewWindowBuilder};
use video_export::{
    AudioConfig, BackendChoice, Quality, VideoConfig,
    schedule::{
        FrameKind, FrameSchedule, RecordedStretch, ScheduleRequest, UnheldGap, build_schedule,
        merge_recorded_tracks, merge_unheld_gaps,
    },
};
use viewer::video_export::{
    ExportProgress, MapFrameStream, MapVideoOutcome, MapVideoRequest, PartialVideo,
    format_campaign_date,
};

pub const EXPORT_WINDOW_LABEL: &str = "video-export";
const MOUNT_EVENT: &str = "video-export-mount";
const METRICS_EVENT: &str = "video-export-metrics";
const STEP_EVENT: &str = "video-export-step";
const UNMOUNT_EVENT: &str = "video-export-unmount";
const STEP_TIMEOUT: Duration = Duration::from_secs(45);
const BOOT_TIMEOUT: Duration = Duration::from_secs(30);

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SurfaceTrackCoverage {
    pub stretches: Vec<RecordedStretch>,
    pub unheld: Vec<UnheldGap>,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct EmbeddedMapSpec {
    pub widget_id: String,
    #[serde(default)]
    pub title: String,
    pub path: String,
    pub revision: Option<String>,
    pub branch: u64,
    pub axis_offset: i32,
    #[serde(default = "default_map_time_mode")]
    pub time_mode: String,
    pub fixed_hour: Option<i32>,
    pub placement: Option<viewer::video_export::SavedPlacement>,
    pub source_viewport: Option<[u32; 2]>,
    pub ui_scale: Option<f32>,
    pub map_mode: Option<String>,
    pub perspective: Option<String>,
    #[serde(default)]
    pub perspective_slot: Option<u16>,
    pub fog: bool,
    pub day_night: Option<bool>,
    pub allied_plans: Option<bool>,
    pub counter_colors_country: Option<bool>,
    pub nato_counters: Option<bool>,
    #[serde(default)]
    pub map_settings: Option<dashboard_core::protocol::ResolvedMapSettings>,
}

fn default_map_time_mode() -> String {
    "follow".to_owned()
}

impl EmbeddedMapSpec {
    fn advances(&self, follow_pinned: bool) -> bool {
        self.time_mode == "follow" || (self.time_mode == "pin" && follow_pinned)
    }

    fn display_name(&self) -> &str {
        if self.title.is_empty() {
            &self.widget_id
        } else {
            &self.title
        }
    }
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SurfaceVideoRequest {
    pub kind: String,
    pub from_hour: i32,
    pub to_hour: i32,
    #[serde(default)]
    pub reference_offset: i32,
    pub fps: u32,
    pub hours_per_second: f64,
    pub skip_gaps: bool,
    pub cut_card_seconds: f64,
    pub width: u32,
    pub height: u32,
    pub quality: Quality,
    pub backend: BackendChoice,
    pub ffmpeg_path: Option<String>,
    pub framing: String,
    pub reflow_for_portrait: bool,
    pub source_width: u32,
    pub source_height: u32,
    pub scroll_top: f64,
    pub date_stamp: bool,
    pub follow_pinned: bool,
    #[serde(default)]
    pub has_rank_marker_tables: bool,
    pub audio_map_widget_id: Option<String>,
    #[serde(default)]
    pub allow_unverified: bool,
    pub tracks: Vec<SurfaceTrackCoverage>,
    pub embedded_maps: Vec<EmbeddedMapSpec>,
    pub payload: serde_json::Value,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ExportMapRect {
    pub widget_id: String,
    pub x: i32,
    pub y: i32,
    pub width: u32,
    pub height: u32,
}

#[derive(Clone, Copy, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct ExportClipRect {
    pub x: i32,
    pub y: i32,
    pub width: u32,
    pub height: u32,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PageAckPayload {
    pub seq: u64,
    pub kind: String,
    pub dirty: bool,
    pub content_width: Option<f64>,
    pub content_height: Option<f64>,
    pub min_text_px: Option<f64>,
    pub map_rects: Vec<ExportMapRect>,
    #[serde(default)]
    pub overlay_rects: Vec<ExportMapRect>,
    #[serde(default)]
    pub viewport_clip: Option<ExportClipRect>,
    #[serde(default)]
    pub bg_rgb: Option<[u8; 3]>,
    pub error: Option<String>,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SurfacePreviewResult {
    pub png_base64: String,
    pub min_text_px: Option<f64>,
    pub content_width: Option<f64>,
    pub content_height: Option<f64>,
    pub scale: f64,
}

#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
struct MountEventPayload {
    seq: u64,
    kind: String,
    framing: String,
    reflow_for_portrait: bool,
    width: u32,
    height: u32,
    css_width: u32,
    css_height: u32,
    dpr: f64,
    scale: f64,
    source_width: u32,
    source_height: u32,
    scroll_top: f64,
    fps: u32,
    hours_per_second: f64,
    date_stamp: bool,
    follow_pinned: bool,
    initial_hour: i32,
    payload: serde_json::Value,
}

#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
struct MetricsEventPayload {
    seq: u64,
    css_width: u32,
    css_height: u32,
    dpr: f64,
    scale: f64,
}

#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
struct StepEventPayload {
    seq: u64,
    frame_index: u64,
    virtual_now_ms: f64,
    mode: String,
    hour: Option<i32>,
    fraction: f64,
    reset_continuity: bool,
    date_text: Option<String>,
    card_heading: Option<String>,
    card_dates: Option<String>,
    card_detail: Option<String>,
    cut_banner: Option<String>,
    css_width: u32,
    css_height: u32,
    dpr: f64,
    scale: f64,
    measure_text: bool,
}

#[derive(Default)]
struct AckBus {
    booted: bool,
    acks: HashMap<u64, PageAckPayload>,
}

#[derive(Clone, Default)]
pub struct ExportWebviewState {
    bus: Arc<(Mutex<AckBus>, Condvar)>,
    next_seq: Arc<AtomicU64>,
    op_lock: Arc<Mutex<()>>,
    owned_sources: Arc<Mutex<Vec<String>>>,
}

impl ExportWebviewState {
    pub fn track_source(&self, source_id: &str) {
        self.owned_sources
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .push(source_id.to_owned());
    }

    pub fn untrack_source(&self, source_id: &str) {
        let mut owned = self.owned_sources.lock().unwrap_or_else(|e| e.into_inner());
        if let Some(pos) = owned.iter().position(|id| id == source_id) {
            owned.swap_remove(pos);
        }
    }

    fn release_owned_sources(&self, app: &AppHandle) {
        let leaked: Vec<String> = {
            let mut owned = self.owned_sources.lock().unwrap_or_else(|e| e.into_inner());
            std::mem::take(&mut *owned)
        };
        for source_id in leaked {
            crate::dashboard_service::release_export_source(app, source_id);
        }
    }

    pub fn record_ack(&self, payload: PageAckPayload) {
        let (lock, cv) = &*self.bus;
        let mut bus = lock.lock().unwrap_or_else(|e| e.into_inner());
        if payload.kind == "boot" {
            bus.booted = true;
        } else {
            bus.acks.insert(payload.seq, payload);
        }
        cv.notify_all();
    }

    pub fn close_window(&self, app: &AppHandle) {
        if let Some(window) = app.get_webview_window(EXPORT_WINDOW_LABEL) {
            let _ = window.destroy();
        }
        self.release_owned_sources(app);
        let (lock, cv) = &*self.bus;
        let mut bus = lock.lock().unwrap_or_else(|e| e.into_inner());
        bus.booted = false;
        bus.acks.clear();
        cv.notify_all();
    }

    fn next_seq(&self) -> u64 {
        self.next_seq.fetch_add(1, Ordering::AcqRel) + 1
    }

    fn wait_for_boot(&self, cancel: &AtomicBool) -> Result<(), String> {
        let (lock, cv) = &*self.bus;
        let deadline = Instant::now() + BOOT_TIMEOUT;
        let mut bus = lock.lock().unwrap_or_else(|e| e.into_inner());
        while !bus.booted {
            if cancel.load(Ordering::Acquire) {
                return Err("cancelled".to_owned());
            }
            let now = Instant::now();
            if now >= deadline {
                return Err("The hidden export page timed out while starting.".to_owned());
            }
            let wait = (deadline - now).min(Duration::from_millis(100));
            let (guard, _) = cv
                .wait_timeout(bus, wait)
                .unwrap_or_else(|e| e.into_inner());
            bus = guard;
        }
        Ok(())
    }

    fn unmount_window(&self, app: &AppHandle, window: &WebviewWindow) {
        let seq = self.next_seq();
        if window.emit(UNMOUNT_EVENT, serde_json::json!({ "seq": seq })).is_ok() {
            let never = AtomicBool::new(false);
            let _ = self.wait_for_ack_timeout(seq, &never, Duration::from_secs(5));
        }
        self.release_owned_sources(app);
    }

    fn wait_for_ack(&self, seq: u64, cancel: &AtomicBool) -> Result<PageAckPayload, String> {
        self.wait_for_ack_timeout(seq, cancel, STEP_TIMEOUT)
    }

    fn wait_for_ack_timeout(
        &self,
        seq: u64,
        cancel: &AtomicBool,
        timeout: Duration,
    ) -> Result<PageAckPayload, String> {
        let (lock, cv) = &*self.bus;
        let deadline = Instant::now() + timeout;
        let mut bus = lock.lock().unwrap_or_else(|e| e.into_inner());
        loop {
            if let Some(ack) = bus.acks.remove(&seq) {
                return Ok(ack);
            }
            if cancel.load(Ordering::Acquire) {
                return Err("cancelled".to_owned());
            }
            let now = Instant::now();
            if now >= deadline {
                return Err("The export page timed out waiting for widgets to finish drawing.".to_owned());
            }
            let wait = (deadline - now).min(Duration::from_millis(50));
            let (guard, _) = cv
                .wait_timeout(bus, wait)
                .unwrap_or_else(|e| e.into_inner());
            bus = guard;
        }
    }

    fn ensure_window(&self, app: &AppHandle, cancel: &AtomicBool) -> Result<WebviewWindow, String> {
        if let Some(existing) = app.get_webview_window(EXPORT_WINDOW_LABEL) {
            self.wait_for_boot(cancel)?;
            return Ok(existing);
        }
        {
            let (lock, _) = &*self.bus;
            let mut bus = lock.lock().unwrap_or_else(|e| e.into_inner());
            bus.booted = false;
            bus.acks.clear();
        }
        let app_clone = app.clone();
        let (tx, rx) = mpsc::channel();
        app.run_on_main_thread(move || {
            let built = WebviewWindowBuilder::new(
                &app_clone,
                EXPORT_WINDOW_LABEL,
                WebviewUrl::App("index.html?export=1".into()),
            )
            .title("BattlePlan Export")
            .visible(false)
            .skip_taskbar(true)
            .decorations(false)
            .resizable(false)
            .inner_size(1280.0, 720.0)
            .build()
            .map_err(|e| format!("Couldn’t create hidden export window: {e}"));
            let _ = tx.send(built);
        })
        .map_err(|e| format!("Couldn’t schedule hidden export window creation: {e}"))?;
        let window = rx
            .recv_timeout(Duration::from_secs(15))
            .map_err(|_| "Timed out creating hidden export window".to_owned())??;
        self.wait_for_boot(cancel)?;
        Ok(window)
    }
}

#[derive(Clone, Copy)]
struct ViewportMetrics {
    css_width: u32,
    css_height: u32,
    dpr: f64,
    scale: f64,
}

fn initial_metrics(req: &SurfaceVideoRequest) -> ViewportMetrics {
    let src_w = req.source_width.max(320);
    let out_w = req.width.max(16);
    let out_h = req.height.max(16);
    if req.kind == "widget" {
        let base_w = src_w.min(out_w).max(640.min(out_w));
        let dpr = (f64::from(out_w) / f64::from(base_w)).clamp(1.0, 4.0);
        let css_w = ((f64::from(out_w) / dpr).round() as u32).max(16);
        let css_h = ((f64::from(out_h) / dpr).round() as u32).max(16);
        return ViewportMetrics {
            css_width: css_w,
            css_height: css_h,
            dpr,
            scale: 1.0,
        };
    }
    if req.framing == "visible" {
        if req.reflow_for_portrait && out_h > out_w {
            let target_css_w = src_w.min(480).max(360);
            let ratio = f64::from(out_w) / f64::from(target_css_w);
            if ratio >= 1.0 {
                let dpr = ratio.clamp(1.0, 4.0);
                let css_w = ((f64::from(out_w) / dpr).round() as u32).max(16);
                let css_h = ((f64::from(out_h) / dpr).round() as u32).max(16);
                return ViewportMetrics {
                    css_width: css_w,
                    css_height: css_h,
                    dpr,
                    scale: 1.0,
                };
            }
            return ViewportMetrics {
                css_width: out_w,
                css_height: out_h,
                dpr: 1.0,
                scale: ratio.clamp(1e-4, 1.0),
            };
        }
        let src_h = req.source_height.max(240);
        let fit = (f64::from(out_w) / f64::from(src_w))
            .min(f64::from(out_h) / f64::from(src_h))
            .clamp(1e-4, 4.0);
        if fit >= 1.0 {
            let dpr = fit.clamp(1.0, 4.0);
            let css_w = ((f64::from(out_w) / dpr).round() as u32).max(16);
            let css_h = ((f64::from(out_h) / dpr).round() as u32).max(16);
            return ViewportMetrics {
                css_width: css_w,
                css_height: css_h,
                dpr,
                scale: 1.0,
            };
        }
        return ViewportMetrics {
            css_width: out_w,
            css_height: out_h,
            dpr: 1.0,
            scale: fit,
        };
    }
    ViewportMetrics {
        css_width: src_w,
        css_height: req.source_height.max(720),
        dpr: 1.0,
        scale: 1.0,
    }
}

fn finalized_metrics(
    req: &SurfaceVideoRequest,
    initial: ViewportMetrics,
    content_width: Option<f64>,
    content_height: Option<f64>,
) -> ViewportMetrics {
    if req.kind != "page" || req.framing != "full" {
        return initial;
    }
    let cw = content_width
        .filter(|v| v.is_finite() && *v >= 64.0)
        .unwrap_or_else(|| f64::from(req.source_width.max(320)));
    let ch = content_height
        .filter(|v| v.is_finite() && *v >= 64.0)
        .unwrap_or_else(|| f64::from(req.source_height.max(320)));
    let out_w = f64::from(req.width.max(16));
    let out_h = f64::from(req.height.max(16));
    let fit = (out_w / cw).min(out_h / ch).clamp(1e-4, 4.0);
    if fit >= 1.0 {
        let dpr = fit.clamp(1.0, 4.0);
        let css_w = (out_w / dpr).round().max(16.0) as u32;
        let css_h = (out_h / dpr).round().max(16.0) as u32;
        ViewportMetrics {
            css_width: css_w,
            css_height: css_h,
            dpr,
            scale: 1.0,
        }
    } else {
        ViewportMetrics {
            css_width: req.width.max(16),
            css_height: req.height.max(16),
            dpr: 1.0,
            scale: fit,
        }
    }
}

fn build_surface_schedule(req: &SurfaceVideoRequest) -> Result<FrameSchedule, String> {
    if req.tracks.is_empty() {
        return Err("No recorded track timeline is available for this export.".to_owned());
    }
    let track_stretches: Vec<Vec<RecordedStretch>> =
        req.tracks.iter().map(|t| t.stretches.clone()).collect();
    let track_unheld: Vec<Vec<UnheldGap>> = req.tracks.iter().map(|t| t.unheld.clone()).collect();
    let merged_stretches = merge_recorded_tracks(&track_stretches);
    let merged_unheld = merge_unheld_gaps(
        &merged_stretches,
        &track_unheld,
        req.from_hour,
        req.to_hour,
    );
    build_schedule(
        &merged_stretches,
        &merged_unheld,
        &ScheduleRequest {
            from_hour: req.from_hour,
            to_hour: req.to_hour,
            fps: req.fps,
            hours_per_second: req.hours_per_second,
            skip_gaps: req.skip_gaps,
            cut_card_seconds: req.cut_card_seconds,
        },
    )
    .map_err(|e| e.to_string())
}

fn step_payload_for_frame(
    seq: u64,
    frame_index: u64,
    virtual_now_ms: f64,
    kind: &FrameKind,
    request: &SurfaceVideoRequest,
    metrics: ViewportMetrics,
    measure_text: bool,
) -> (StepEventPayload, Option<String>, Option<i32>) {
    let ref_offset = request.reference_offset;
    match kind {
        FrameKind::Recorded {
            hour,
            fraction,
            reset_continuity,
            cut_banner,
        } => (
            StepEventPayload {
                seq,
                frame_index,
                virtual_now_ms,
                mode: "recorded".to_owned(),
                hour: Some(*hour),
                fraction: *fraction,
                reset_continuity: *reset_continuity,
                date_text: request
                    .date_stamp
                    .then(|| format_campaign_date(*hour + ref_offset)),
                card_heading: None,
                card_dates: None,
                card_detail: None,
                cut_banner: cut_banner.as_ref().map(|c| {
                    format!(
                        "Unrecorded gap skipped · {} -> {} ({} h omitted)",
                        format_campaign_date(c.from_hour + ref_offset),
                        format_campaign_date(c.to_hour + ref_offset),
                        c.omitted_hours,
                    )
                }),
                css_width: metrics.css_width,
                css_height: metrics.css_height,
                dpr: metrics.dpr,
                scale: metrics.scale,
                measure_text,
            },
            None,
            Some(*hour),
        ),
        FrameKind::GapCard {
            hour,
            gap_first,
            gap_last,
            reason,
            crossed_gap,
        } => {
            let dates = format!(
                "{}  (gap: {} to {})",
                format_campaign_date(*hour + ref_offset),
                format_campaign_date(*gap_first + ref_offset),
                format_campaign_date(*gap_last + ref_offset),
            );
            let detail = match crossed_gap {
                Some(cut) => format!(
                    "{reason} · earlier gap skipped: {} -> {} ({} h omitted)",
                    format_campaign_date(cut.from_hour + ref_offset),
                    format_campaign_date(cut.to_hour + ref_offset),
                    cut.omitted_hours,
                ),
                None => reason.clone(),
            };
            let key = format!("gap:{dates}:{detail}");
            (
                StepEventPayload {
                    seq,
                    frame_index,
                    virtual_now_ms,
                    mode: "card".to_owned(),
                    hour: Some(*hour),
                    fraction: 0.0,
                    reset_continuity: true,
                    date_text: None,
                    card_heading: Some("Unrecorded stretch".to_owned()),
                    card_dates: Some(dates),
                    card_detail: Some(detail),
                    cut_banner: None,
                    css_width: metrics.css_width,
                    css_height: metrics.css_height,
                    dpr: metrics.dpr,
                    scale: metrics.scale,
                    measure_text,
                },
                Some(key),
                Some(*hour),
            )
        }
        FrameKind::SkipCutCard(cut) => {
            let dates = format!(
                "{} -> {}",
                format_campaign_date(cut.from_hour + ref_offset),
                format_campaign_date(cut.to_hour + ref_offset),
            );
            let detail = format!("{} unrecorded hours omitted · {}", cut.omitted_hours, cut.reason);
            let key = format!("cut:{dates}:{detail}");
            (
                StepEventPayload {
                    seq,
                    frame_index,
                    virtual_now_ms,
                    mode: "card".to_owned(),
                    hour: Some(cut.to_hour),
                    fraction: 0.0,
                    reset_continuity: true,
                    date_text: None,
                    card_heading: Some("Unrecorded gap skipped".to_owned()),
                    card_dates: Some(dates),
                    card_detail: Some(detail),
                    cut_banner: None,
                    css_width: metrics.css_width,
                    css_height: metrics.css_height,
                    dpr: metrics.dpr,
                    scale: metrics.scale,
                    measure_text,
                },
                Some(key),
                Some(cut.to_hour),
            )
        }
    }
}

fn cdp_call(
    window: &WebviewWindow,
    method: &str,
    params_json: &str,
    cancel: &AtomicBool,
) -> Result<String, String> {
    let method_owned = method.to_owned();
    let params_owned = params_json.to_owned();
    let (tx, rx) = mpsc::channel::<Result<String, String>>();
    window
        .with_webview(move |webview| {
            let method_w = webview2_com::CoTaskMemPWSTR::from(method_owned.as_str());
            let params_w = webview2_com::CoTaskMemPWSTR::from(params_owned.as_str());
            let tx_cb = tx.clone();
            let handler = webview2_com::CallDevToolsProtocolMethodCompletedHandler::create(
                Box::new(move |err, result_json| {
                    let reply = match err {
                        Ok(()) => Ok(result_json),
                        Err(e) => Err(format!("WebView2 DevTools call failed: {e}")),
                    };
                    let _ = tx_cb.send(reply);
                    Ok(())
                }),
            );
            // SAFETY: `webview.controller()` and its `CoreWebView2` COM interface are accessed on
            // the UI thread inside `with_webview`, and both `CoTaskMemPWSTR` arguments outlive the
            // synchronous COM vtable invocation.
            let called = unsafe {
                webview.controller().CoreWebView2().and_then(|page| {
                    page.CallDevToolsProtocolMethod(
                        *method_w.as_ref().as_pcwstr(),
                        *params_w.as_ref().as_pcwstr(),
                        &handler,
                    )
                })
            };
            if let Err(e) = called {
                let _ = tx.send(Err(format!("Couldn’t invoke WebView2 DevTools method: {e}")));
            }
        })
        .map_err(|e| format!("Couldn’t reach export WebView: {e}"))?;

    let deadline = Instant::now() + STEP_TIMEOUT;
    loop {
        if cancel.load(Ordering::Acquire) {
            return Err("cancelled".to_owned());
        }
        match rx.recv_timeout(Duration::from_millis(50)) {
            Ok(reply) => return reply,
            Err(mpsc::RecvTimeoutError::Timeout) => {
                if Instant::now() >= deadline {
                    return Err(format!("WebView2 DevTools method {method} timed out"));
                }
            }
            Err(mpsc::RecvTimeoutError::Disconnected) => {
                return Err(format!("WebView2 DevTools method {method} dropped"));
            }
        }
    }
}

fn set_device_metrics(
    window: &WebviewWindow,
    metrics: ViewportMetrics,
    cancel: &AtomicBool,
) -> Result<(), String> {
    let params = serde_json::json!({
        "width": metrics.css_width,
        "height": metrics.css_height,
        "deviceScaleFactor": metrics.dpr,
        "mobile": false,
        "screenWidth": metrics.css_width,
        "screenHeight": metrics.css_height,
    });
    let _ = cdp_call(window, "Emulation.setDeviceMetricsOverride", &params.to_string(), cancel)?;
    Ok(())
}

fn capture_screenshot_png(
    window: &WebviewWindow,
    metrics: ViewportMetrics,
    cancel: &AtomicBool,
) -> Result<(String, Vec<u8>), String> {
    let params = serde_json::json!({
        "format": "png",
        "fromSurface": true,
        "captureBeyondViewport": false,
        "clip": {
            "x": 0,
            "y": 0,
            "width": metrics.css_width,
            "height": metrics.css_height,
            "scale": 1
        }
    });
    let raw = cdp_call(window, "Page.captureScreenshot", &params.to_string(), cancel)?;
    #[derive(Deserialize)]
    struct Shot {
        data: String,
    }
    let parsed: Shot = serde_json::from_str(&raw)
        .map_err(|e| format!("Couldn’t parse screenshot payload: {e}"))?;
    let bytes = decode_base64(&parsed.data)?;
    Ok((parsed.data, bytes))
}

fn decode_base64(input: &str) -> Result<Vec<u8>, String> {
    let bytes = input.as_bytes();
    let mut out = Vec::with_capacity(bytes.len() * 3 / 4);
    let mut acc: u32 = 0;
    let mut bits: u32 = 0;
    for &b in bytes {
        let val = match b {
            b'A'..=b'Z' => b - b'A',
            b'a'..=b'z' => b - b'a' + 26,
            b'0'..=b'9' => b - b'0' + 52,
            b'+' => 62,
            b'/' => 63,
            b'=' | b'\n' | b'\r' | b' ' | b'\t' => continue,
            _ => return Err("Invalid base64 byte in screenshot".to_owned()),
        };
        acc = (acc << 6) | u32::from(val);
        bits += 6;
        if bits >= 8 {
            bits -= 8;
            out.push(((acc >> bits) & 0xff) as u8);
        }
    }
    Ok(out)
}

fn decode_png_rgba(png_bytes: &[u8]) -> Result<(u32, u32, Vec<u8>), String> {
    let mut decoder = png::Decoder::new(Cursor::new(png_bytes));
    decoder.set_transformations(png::Transformations::EXPAND | png::Transformations::STRIP_16);
    let mut reader = decoder
        .read_info()
        .map_err(|e| format!("Couldn’t decode captured PNG header: {e}"))?;
    let mut buf = vec![0u8; reader.output_buffer_size().unwrap_or(0)];
    let info = reader
        .next_frame(&mut buf)
        .map_err(|e| format!("Couldn’t decode captured PNG pixels: {e}"))?;
    let raw = &buf[..info.buffer_size()];
    let rgba = match info.color_type {
        png::ColorType::Rgba => raw.to_vec(),
        png::ColorType::Rgb => {
            let mut out = Vec::with_capacity((info.width * info.height * 4) as usize);
            for chunk in raw.chunks_exact(3) {
                out.extend_from_slice(&[chunk[0], chunk[1], chunk[2], 255]);
            }
            out
        }
        png::ColorType::Grayscale => {
            let mut out = Vec::with_capacity((info.width * info.height * 4) as usize);
            for &g in raw {
                out.extend_from_slice(&[g, g, g, 255]);
            }
            out
        }
        png::ColorType::GrayscaleAlpha => {
            let mut out = Vec::with_capacity((info.width * info.height * 4) as usize);
            for chunk in raw.chunks_exact(2) {
                out.extend_from_slice(&[chunk[0], chunk[0], chunk[0], chunk[1]]);
            }
            out
        }
        png::ColorType::Indexed => {
            return Err("Unexpected indexed PNG after expansion".to_owned());
        }
    };
    Ok((info.width, info.height, rgba))
}

fn fit_rgba_to_exact(
    src: &[u8],
    src_w: u32,
    src_h: u32,
    dst_w: u32,
    dst_h: u32,
    bg_rgb: [u8; 3],
) -> Vec<u8> {
    if src_w == dst_w && src_h == dst_h && src.len() == (dst_w * dst_h * 4) as usize {
        return src.to_vec();
    }
    let mut dst = vec![0u8; (dst_w * dst_h * 4) as usize];
    for pixel in dst.chunks_exact_mut(4) {
        pixel[0] = bg_rgb[0];
        pixel[1] = bg_rgb[1];
        pixel[2] = bg_rgb[2];
        pixel[3] = 255;
    }
    let copy_w = src_w.min(dst_w) as usize;
    let copy_h = src_h.min(dst_h) as usize;
    let src_stride = (src_w * 4) as usize;
    let dst_stride = (dst_w * 4) as usize;
    let row_bytes = copy_w * 4;
    for y in 0..copy_h {
        let s0 = y * src_stride;
        let d0 = y * dst_stride;
        dst[d0..d0 + row_bytes].copy_from_slice(&src[s0..s0 + row_bytes]);
    }
    dst
}

struct SavedPatch {
    x: i32,
    y: i32,
    width: u32,
    height: u32,
    rgba: Vec<u8>,
}

fn save_overlay_patches(
    frame: &[u8],
    frame_w: u32,
    frame_h: u32,
    rects: &[ExportMapRect],
) -> Vec<SavedPatch> {
    let mut patches = Vec::new();
    let stride = (frame_w * 4) as usize;
    for rect in rects {
        let x0 = rect.x.max(0);
        let y0 = rect.y.max(0);
        let x1 = (rect.x + rect.width as i32).min(frame_w as i32);
        let y1 = (rect.y + rect.height as i32).min(frame_h as i32);
        if x1 <= x0 || y1 <= y0 {
            continue;
        }
        let w = (x1 - x0) as u32;
        let h = (y1 - y0) as u32;
        let mut rgba = Vec::with_capacity((w * h * 4) as usize);
        let row_bytes = (w * 4) as usize;
        for row in 0..h as usize {
            let s = (y0 as usize + row) * stride + (x0 as usize) * 4;
            if s + row_bytes <= frame.len() {
                rgba.extend_from_slice(&frame[s..s + row_bytes]);
            }
        }
        if rgba.len() == (w * h * 4) as usize {
            patches.push(SavedPatch {
                x: x0,
                y: y0,
                width: w,
                height: h,
                rgba,
            });
        }
    }
    patches
}

fn restore_overlay_patches(
    frame: &mut [u8],
    frame_w: u32,
    frame_h: u32,
    patches: &[SavedPatch],
) {
    for patch in patches {
        blit_rgba(
            frame,
            frame_w,
            frame_h,
            &patch.rgba,
            patch.width,
            patch.height,
            patch.x,
            patch.y,
        );
    }
}

fn blit_rgba(
    dst: &mut [u8],
    dst_w: u32,
    dst_h: u32,
    src: &[u8],
    src_w: u32,
    src_h: u32,
    dst_x: i32,
    dst_y: i32,
) {
    blit_rgba_clipped(dst, dst_w, dst_h, src, src_w, src_h, dst_x, dst_y, None);
}

#[expect(
    clippy::too_many_arguments,
    reason = "pixel blitter takes source and destination buffers, dimensions, offset, and optional clip rectangle"
)]
fn blit_rgba_clipped(
    dst: &mut [u8],
    dst_w: u32,
    dst_h: u32,
    src: &[u8],
    src_w: u32,
    src_h: u32,
    dst_x: i32,
    dst_y: i32,
    clip: Option<ExportClipRect>,
) {
    if src_w == 0 || src_h == 0 || dst_w == 0 || dst_h == 0 {
        return;
    }
    let (cx0, cy0, cx1, cy1) = match clip {
        Some(rect) => (
            rect.x.max(0),
            rect.y.max(0),
            (rect.x + rect.width as i32).min(dst_w as i32),
            (rect.y + rect.height as i32).min(dst_h as i32),
        ),
        None => (0, 0, dst_w as i32, dst_h as i32),
    };
    let x0 = dst_x.max(cx0);
    let y0 = dst_y.max(cy0);
    let x1 = (dst_x + src_w as i32).min(cx1);
    let y1 = (dst_y + src_h as i32).min(cy1);
    if x1 <= x0 || y1 <= y0 {
        return;
    }
    let sx0 = (x0 - dst_x) as usize;
    let sy0 = (y0 - dst_y) as usize;
    let copy_w = (x1 - x0) as usize;
    let src_stride = (src_w * 4) as usize;
    let dst_stride = (dst_w * 4) as usize;
    let row_bytes = copy_w * 4;
    for row in 0..(y1 - y0) as usize {
        let s = (sy0 + row) * src_stride + sx0 * 4;
        let d = (y0 as usize + row) * dst_stride + (x0 as usize) * 4;
        if s + row_bytes <= src.len() && d + row_bytes <= dst.len() {
            dst[d..d + row_bytes].copy_from_slice(&src[s..s + row_bytes]);
        }
    }
}

#[expect(
    clippy::too_many_arguments,
    reason = "pixel blitter takes source and destination buffers, dimensions, target rectangle, and optional clip rectangle"
)]
fn blit_rgba_scaled_clipped(
    dst: &mut [u8],
    dst_w: u32,
    dst_h: u32,
    src: &[u8],
    src_w: u32,
    src_h: u32,
    dst_x: i32,
    dst_y: i32,
    dst_rect_w: u32,
    dst_rect_h: u32,
    clip: Option<ExportClipRect>,
) {
    if src_w == 0 || src_h == 0 || dst_w == 0 || dst_h == 0 || dst_rect_w == 0 || dst_rect_h == 0 {
        return;
    }
    if src_w == dst_rect_w && src_h == dst_rect_h {
        blit_rgba_clipped(dst, dst_w, dst_h, src, src_w, src_h, dst_x, dst_y, clip);
        return;
    }
    let (cx0, cy0, cx1, cy1) = match clip {
        Some(rect) => (
            rect.x.max(0),
            rect.y.max(0),
            (rect.x + rect.width as i32).min(dst_w as i32),
            (rect.y + rect.height as i32).min(dst_h as i32),
        ),
        None => (0, 0, dst_w as i32, dst_h as i32),
    };
    let x0 = dst_x.max(cx0);
    let y0 = dst_y.max(cy0);
    let x1 = (dst_x + dst_rect_w as i32).min(cx1);
    let y1 = (dst_y + dst_rect_h as i32).min(cy1);
    if x1 <= x0 || y1 <= y0 {
        return;
    }
    let src_stride = (src_w * 4) as usize;
    let dst_stride = (dst_w * 4) as usize;
    for y in y0..y1 {
        let local_y = (y - dst_y) as u64;
        let sy = ((local_y * u64::from(src_h)) / u64::from(dst_rect_h)).min(u64::from(src_h - 1)) as usize;
        let dst_row = y as usize * dst_stride;
        let src_row = sy * src_stride;
        for x in x0..x1 {
            let local_x = (x - dst_x) as u64;
            let sx = ((local_x * u64::from(src_w)) / u64::from(dst_rect_w)).min(u64::from(src_w - 1)) as usize;
            let s = src_row + sx * 4;
            let d = dst_row + x as usize * 4;
            if s + 4 <= src.len() && d + 4 <= dst.len() {
                dst[d..d + 4].copy_from_slice(&src[s..s + 4]);
            }
        }
    }
}

fn encode_rgba_png(rgba: &[u8], width: u32, height: u32) -> Result<Vec<u8>, String> {
    let mut encoded = Vec::new();
    let mut encoder = png::Encoder::new(&mut encoded, width, height);
    encoder.set_color(png::ColorType::Rgba);
    encoder.set_depth(png::BitDepth::Eight);
    encoder.set_source_srgb(png::SrgbRenderingIntent::Perceptual);
    let mut writer = encoder.write_header().map_err(|e| e.to_string())?;
    writer.write_image_data(rgba).map_err(|e| e.to_string())?;
    writer.finish().map_err(|e| e.to_string())?;
    Ok(encoded)
}

fn encode_base64(input: &[u8]) -> String {
    const TABLE: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut out = String::with_capacity(input.len().div_ceil(3) * 4);
    for chunk in input.chunks(3) {
        let b0 = u32::from(chunk[0]);
        let b1 = if chunk.len() > 1 { u32::from(chunk[1]) } else { 0 };
        let b2 = if chunk.len() > 2 { u32::from(chunk[2]) } else { 0 };
        let n = (b0 << 16) | (b1 << 8) | b2;
        out.push(TABLE[((n >> 18) & 63) as usize] as char);
        out.push(TABLE[((n >> 12) & 63) as usize] as char);
        out.push(if chunk.len() > 1 {
            TABLE[((n >> 6) & 63) as usize] as char
        } else {
            '='
        });
        out.push(if chunk.len() > 2 {
            TABLE[(n & 63) as usize] as char
        } else {
            '='
        });
    }
    out
}

fn embedded_map_render_size(rect: &ExportMapRect) -> (u32, u32) {
    let rw = rect.width.max(1);
    let rh = rect.height.max(1);
    let min_dim = rw.min(rh);
    let (scaled_w, scaled_h) = if min_dim < 16 {
        let factor = 16.0 / f64::from(min_dim);
        (
            (f64::from(rw) * factor).round() as u32,
            (f64::from(rh) * factor).round() as u32,
        )
    } else {
        (rw, rh)
    };
    let even = |v: u32| {
        let clamped = v.max(16);
        if clamped.is_multiple_of(2) { clamped } else { clamped + 1 }
    };
    (even(scaled_w), even(scaled_h))
}

fn map_request_for_embedded(
    spec: &EmbeddedMapSpec,
    rect: &ExportMapRect,
    req: &SurfaceVideoRequest,
    audio_enabled: bool,
) -> MapVideoRequest {
    let (w, h) = embedded_map_render_size(rect);
    let advances = spec.advances(req.follow_pinned);
    let from = if advances {
        req.from_hour + spec.axis_offset
    } else {
        spec.fixed_hour.unwrap_or(req.from_hour + spec.axis_offset)
    };
    let to = if advances {
        req.to_hour + spec.axis_offset
    } else {
        spec.fixed_hour.unwrap_or(req.to_hour + spec.axis_offset)
    };
    MapVideoRequest {
        path: spec.path.clone(),
        revision: spec.revision.clone(),
        branch: spec.branch,
        from_hour: from,
        to_hour: to,
        fps: req.fps,
        hours_per_second: req.hours_per_second,
        skip_gaps: req.skip_gaps,
        cut_card_seconds: req.cut_card_seconds,
        width: w,
        height: h,
        quality: req.quality,
        backend: req.backend,
        ffmpeg_path: req.ffmpeg_path.clone(),
        placement: spec.placement,
        source_viewport: spec.source_viewport,
        crop: None,
        ui_scale: spec.ui_scale,
        map_mode: spec.map_mode.clone(),
        perspective: spec.perspective.clone(),
        perspective_slot: spec.perspective_slot,
        fog: spec.fog,
        day_night: spec.day_night,
        allied_plans: spec.allied_plans,
        counter_colors_country: spec.counter_colors_country,
        nato_counters: spec.nato_counters,
        map_settings: spec.map_settings.clone(),
        date_stamp: false,
        audio_enabled,
        allow_unverified: req.allow_unverified,
    }
}

fn render_embedded_map_still(
    config: dashboard_core::config::Config,
    map_req: MapVideoRequest,
    hour: i32,
    fraction: f64,
    fps: u32,
    hours_per_second: f64,
    cancel: &AtomicBool,
) -> Result<(u32, u32, Vec<u8>), String> {
    let width = map_req.width;
    let height = map_req.height;
    let single = FrameSchedule::single_preview(hour, fraction, fps, hours_per_second);
    let stream = MapFrameStream::spawn(config, map_req, single)?;
    let chunk = stream.next_frame(cancel)?;
    let rgba = chunk
        .rgba
        .ok_or_else(|| "Embedded map renderer produced no pixels for this hour".to_owned())?;
    Ok((width, height, rgba))
}

enum EmbeddedMapSource {
    Stream {
        rect: ExportMapRect,
        stream: MapFrameStream,
        audio: bool,
    },
    Static {
        rect: ExportMapRect,
        rgba: Vec<u8>,
        width: u32,
        height: u32,
    },
}

pub fn preview_surface_video(
    app: &AppHandle,
    webview_state: &ExportWebviewState,
    config: dashboard_core::config::Config,
    request: SurfaceVideoRequest,
    preview_hour: i32,
    cancel: Arc<AtomicBool>,
) -> Result<SurfacePreviewResult, String> {
    let _guard = webview_state
        .op_lock
        .lock()
        .unwrap_or_else(|e| e.into_inner());
    let schedule = build_surface_schedule(&request)?;
    let first_recorded_hour = schedule
        .first_recorded()
        .map_or(request.from_hour, |(hour, _)| hour);
    let preview = schedule.preview_frame_with_history(preview_hour, request.has_rank_marker_tables);
    let (preview_recorded_hour, preview_fraction) = match &preview.frame.kind {
        FrameKind::Recorded { hour, fraction, .. } => (*hour, *fraction),
        _ => (preview_hour, 0.0),
    };
    let mount_initial_hour = preview
        .seed_frames
        .first()
        .and_then(|f| match f.kind {
            FrameKind::Recorded { hour, .. } => Some(hour),
            _ => None,
        })
        .unwrap_or(first_recorded_hour);
    let window = webview_state.ensure_window(app, &cancel)?;
    let initial = initial_metrics(&request);
    set_device_metrics(&window, initial, &cancel)?;

    let mount_seq = webview_state.next_seq();
    let mount = MountEventPayload {
        seq: mount_seq,
        kind: request.kind.clone(),
        framing: request.framing.clone(),
        reflow_for_portrait: request.reflow_for_portrait,
        width: request.width,
        height: request.height,
        css_width: initial.css_width,
        css_height: initial.css_height,
        dpr: initial.dpr,
        scale: initial.scale,
        source_width: request.source_width,
        source_height: request.source_height,
        scroll_top: request.scroll_top,
        fps: request.fps,
        hours_per_second: request.hours_per_second,
        date_stamp: request.date_stamp,
        follow_pinned: request.follow_pinned,
        initial_hour: mount_initial_hour,
        payload: request.payload.clone(),
    };
    window
        .emit(MOUNT_EVENT, mount)
        .map_err(|e| format!("Couldn’t send mount payload to export page: {e}"))?;
    let mount_ack = match webview_state.wait_for_ack(mount_seq, &cancel) {
        Ok(ack) => ack,
        Err(error) => {
            webview_state.unmount_window(app, &window);
            return Err(error);
        }
    };
    if let Some(error) = mount_ack.error {
        webview_state.unmount_window(app, &window);
        return Err(error);
    }

    let metrics = finalized_metrics(
        &request,
        initial,
        mount_ack.content_width,
        mount_ack.content_height,
    );
    let mut viewport_clip = mount_ack.viewport_clip;
    let mut min_text_px = mount_ack.min_text_px;
    if metrics.css_width != initial.css_width
        || metrics.css_height != initial.css_height
        || (metrics.dpr - initial.dpr).abs() > 1e-4
        || (metrics.scale - initial.scale).abs() > 1e-4
    {
        if let Err(error) = set_device_metrics(&window, metrics, &cancel) {
            webview_state.unmount_window(app, &window);
            return Err(error);
        }
        let metrics_seq = webview_state.next_seq();
        if let Err(e) = window.emit(
            METRICS_EVENT,
            MetricsEventPayload {
                seq: metrics_seq,
                css_width: metrics.css_width,
                css_height: metrics.css_height,
                dpr: metrics.dpr,
                scale: metrics.scale,
            },
        ) {
            webview_state.unmount_window(app, &window);
            return Err(format!("Couldn’t apply export viewport metrics: {e}"));
        }
        let metrics_ack = match webview_state.wait_for_ack(metrics_seq, &cancel) {
            Ok(ack) => ack,
            Err(error) => {
                webview_state.unmount_window(app, &window);
                return Err(error);
            }
        };
        if let Some(error) = metrics_ack.error {
            webview_state.unmount_window(app, &window);
            return Err(error);
        }
        if metrics_ack.viewport_clip.is_some() {
            viewport_clip = metrics_ack.viewport_clip;
        }
        if metrics_ack.min_text_px.is_some() {
            min_text_px = metrics_ack.min_text_px;
        }
    }

    for seed in &preview.seed_frames {
        let seed_seq = webview_state.next_seq();
        let seed_now_ms = (seed.frame_index as f64 * 1000.0) / f64::from(schedule.fps);
        let (seed_step, _, _) = step_payload_for_frame(
            seed_seq,
            seed.frame_index,
            seed_now_ms,
            &seed.kind,
            &request,
            metrics,
            false,
        );
        window.emit(STEP_EVENT, seed_step).map_err(|e| {
            webview_state.unmount_window(app, &window);
            format!("Couldn’t seed export page for preview: {e}")
        })?;
        let seed_ack = match webview_state.wait_for_ack(seed_seq, &cancel) {
            Ok(ack) => ack,
            Err(error) => {
                webview_state.unmount_window(app, &window);
                return Err(error);
            }
        };
        if let Some(error) = seed_ack.error {
            webview_state.unmount_window(app, &window);
            return Err(error);
        }
    }

    let step_seq = webview_state.next_seq();
    let preview_now_ms = (preview.frame.frame_index as f64 * 1000.0) / f64::from(schedule.fps);
    let (step, card_key, _) = step_payload_for_frame(
        step_seq,
        preview.frame.frame_index,
        preview_now_ms,
        &preview.frame.kind,
        &request,
        metrics,
        true,
    );
    window.emit(STEP_EVENT, step).map_err(|e| {
        webview_state.unmount_window(app, &window);
        format!("Couldn’t step export page for preview: {e}")
    })?;
    let step_ack = match webview_state.wait_for_ack(step_seq, &cancel) {
        Ok(ack) => ack,
        Err(error) => {
            webview_state.unmount_window(app, &window);
            return Err(error);
        }
    };
    if let Some(error) = step_ack.error {
        webview_state.unmount_window(app, &window);
        return Err(error);
    }
    if step_ack.viewport_clip.is_some() {
        viewport_clip = step_ack.viewport_clip;
    }

    let (png_base64, png_bytes) = match capture_screenshot_png(&window, metrics, &cancel) {
        Ok(pair) => pair,
        Err(error) => {
            webview_state.unmount_window(app, &window);
            return Err(error);
        }
    };
    let mut final_base64 = png_base64;
    let bg_rgb = step_ack.bg_rgb.or(mount_ack.bg_rgb).unwrap_or([15, 19, 26]);

    if card_key.is_none() && !step_ack.map_rects.is_empty() && !request.embedded_maps.is_empty() {
        let (src_w, src_h, raw_rgba) = match decode_png_rgba(&png_bytes) {
            Ok(decoded) => decoded,
            Err(error) => {
                webview_state.unmount_window(app, &window);
                return Err(error);
            }
        };
        let mut rgba = fit_rgba_to_exact(&raw_rgba, src_w, src_h, request.width, request.height, bg_rgb);
        let overlay_patches =
            save_overlay_patches(&rgba, request.width, request.height, &step_ack.overlay_rects);
        for rect in &step_ack.map_rects {
            if let Some(spec) = request
                .embedded_maps
                .iter()
                .find(|m| m.widget_id == rect.widget_id)
                && rect.width > 0
                && rect.height > 0
            {
                let advances = spec.advances(request.follow_pinned);
                let map_hour = if advances {
                    Some(preview_recorded_hour + spec.axis_offset)
                } else {
                    spec.fixed_hour
                };
                let Some(map_hour) = map_hour else {
                    continue;
                };
                let map_fraction = if advances { preview_fraction } else { 0.0 };
                let map_req = map_request_for_embedded(spec, rect, &request, false);
                let (mw, mh, mrgba) = match render_embedded_map_still(
                    config.clone(),
                    map_req,
                    map_hour,
                    map_fraction,
                    request.fps,
                    request.hours_per_second,
                    &cancel,
                ) {
                    Ok(rendered) => rendered,
                    Err(error) => {
                        webview_state.unmount_window(app, &window);
                        return Err(format!(
                            "Couldn’t preview embedded map “{}”: {error}",
                            spec.display_name()
                        ));
                    }
                };
                blit_rgba_scaled_clipped(
                    &mut rgba,
                    request.width,
                    request.height,
                    &mrgba,
                    mw,
                    mh,
                    rect.x,
                    rect.y,
                    rect.width,
                    rect.height,
                    viewport_clip,
                );
            }
        }
        restore_overlay_patches(&mut rgba, request.width, request.height, &overlay_patches);
        let composited = match encode_rgba_png(&rgba, request.width, request.height) {
            Ok(bytes) => bytes,
            Err(error) => {
                webview_state.unmount_window(app, &window);
                return Err(error);
            }
        };
        final_base64 = encode_base64(&composited);
    }

    webview_state.unmount_window(app, &window);
    Ok(SurfacePreviewResult {
        png_base64: final_base64,
        min_text_px: step_ack.min_text_px.or(min_text_px),
        content_width: mount_ack.content_width,
        content_height: mount_ack.content_height,
        scale: metrics.scale * metrics.dpr,
    })
}

pub fn run_surface_video_export(
    app: AppHandle,
    webview_state: ExportWebviewState,
    config: dashboard_core::config::Config,
    request: SurfaceVideoRequest,
    staged_path: PathBuf,
    cancel: Arc<AtomicBool>,
    progress: Option<Arc<dyn Fn(ExportProgress) + Send + Sync>>,
) -> MapVideoOutcome {
    let _guard = webview_state
        .op_lock
        .lock()
        .unwrap_or_else(|e| e.into_inner());
    let video_config = VideoConfig {
        width: request.width,
        height: request.height,
        fps: request.fps,
        quality: request.quality,
        backend: request.backend,
        ffmpeg_path: request
            .ffmpeg_path
            .as_deref()
            .filter(|s| !s.is_empty())
            .map(PathBuf::from),
    };
    if let Err(e) = video_config.validate() {
        return MapVideoOutcome::Failed {
            error: e.to_string(),
            campaign_hour: None,
            partial: None,
        };
    }
    let schedule = match build_surface_schedule(&request) {
        Ok(s) => s,
        Err(error) => {
            return MapVideoOutcome::Failed {
                error,
                campaign_hour: None,
                partial: None,
            };
        }
    };
    let first_recorded_hour = schedule
        .first_recorded()
        .map_or(request.from_hour, |(hour, _)| hour);

    let window = match webview_state.ensure_window(&app, &cancel) {
        Ok(w) => w,
        Err(error) => {
            if cancel.load(Ordering::Acquire) || error == "cancelled" {
                return MapVideoOutcome::Cancelled;
            }
            return MapVideoOutcome::Failed {
                error,
                campaign_hour: None,
                partial: None,
            };
        }
    };

    let initial = initial_metrics(&request);
    if let Err(error) = set_device_metrics(&window, initial, &cancel) {
        webview_state.unmount_window(&app, &window);
        return if cancel.load(Ordering::Acquire) || error == "cancelled" {
            MapVideoOutcome::Cancelled
        } else {
            MapVideoOutcome::Failed {
                error,
                campaign_hour: None,
                partial: None,
            }
        };
    }

    let mount_seq = webview_state.next_seq();
    let mount = MountEventPayload {
        seq: mount_seq,
        kind: request.kind.clone(),
        framing: request.framing.clone(),
        reflow_for_portrait: request.reflow_for_portrait,
        width: request.width,
        height: request.height,
        css_width: initial.css_width,
        css_height: initial.css_height,
        dpr: initial.dpr,
        scale: initial.scale,
        source_width: request.source_width,
        source_height: request.source_height,
        scroll_top: request.scroll_top,
        fps: request.fps,
        hours_per_second: request.hours_per_second,
        date_stamp: request.date_stamp,
        follow_pinned: request.follow_pinned,
        initial_hour: first_recorded_hour,
        payload: request.payload.clone(),
    };
    if let Err(e) = window.emit(MOUNT_EVENT, mount) {
        webview_state.unmount_window(&app, &window);
        return MapVideoOutcome::Failed {
            error: format!("Couldn’t mount export page: {e}"),
            campaign_hour: None,
            partial: None,
        };
    }
    let mount_ack = match webview_state.wait_for_ack(mount_seq, &cancel) {
        Ok(ack) => ack,
        Err(error) => {
            webview_state.unmount_window(&app, &window);
            return if cancel.load(Ordering::Acquire) || error == "cancelled" {
                MapVideoOutcome::Cancelled
            } else {
                MapVideoOutcome::Failed {
                    error,
                    campaign_hour: None,
                    partial: None,
                }
            };
        }
    };
    if let Some(error) = mount_ack.error {
        webview_state.unmount_window(&app, &window);
        return MapVideoOutcome::Failed {
            error,
            campaign_hour: None,
            partial: None,
        };
    }

    let metrics = finalized_metrics(
        &request,
        initial,
        mount_ack.content_width,
        mount_ack.content_height,
    );
    let mut final_map_rects = mount_ack.map_rects;
    let mut viewport_clip = mount_ack.viewport_clip;
    if metrics.css_width != initial.css_width
        || metrics.css_height != initial.css_height
        || (metrics.dpr - initial.dpr).abs() > 1e-4
        || (metrics.scale - initial.scale).abs() > 1e-4
    {
        if let Err(error) = set_device_metrics(&window, metrics, &cancel) {
            webview_state.unmount_window(&app, &window);
            return if cancel.load(Ordering::Acquire) || error == "cancelled" {
                MapVideoOutcome::Cancelled
            } else {
                MapVideoOutcome::Failed {
                    error,
                    campaign_hour: None,
                    partial: None,
                }
            };
        }
        let metrics_seq = webview_state.next_seq();
        if let Err(e) = window.emit(
            METRICS_EVENT,
            MetricsEventPayload {
                seq: metrics_seq,
                css_width: metrics.css_width,
                css_height: metrics.css_height,
                dpr: metrics.dpr,
                scale: metrics.scale,
            },
        ) {
            webview_state.unmount_window(&app, &window);
            return MapVideoOutcome::Failed {
                error: format!("Couldn’t apply export viewport metrics: {e}"),
                campaign_hour: None,
                partial: None,
            };
        }
        match webview_state.wait_for_ack(metrics_seq, &cancel) {
            Ok(ack) => {
                if let Some(error) = ack.error {
                    webview_state.unmount_window(&app, &window);
                    return MapVideoOutcome::Failed {
                        error,
                        campaign_hour: None,
                        partial: None,
                    };
                }
                final_map_rects = ack.map_rects;
                if ack.viewport_clip.is_some() {
                    viewport_clip = ack.viewport_clip;
                }
            }
            Err(error) => {
                webview_state.unmount_window(&app, &window);
                return if cancel.load(Ordering::Acquire) || error == "cancelled" {
                    MapVideoOutcome::Cancelled
                } else {
                    MapVideoOutcome::Failed {
                        error,
                        campaign_hour: None,
                        partial: None,
                    }
                };
            }
        }
    }

    let mut map_sources: Vec<EmbeddedMapSource> = Vec::new();
    let mut audio_enabled = false;
    let mut bg_rgb = mount_ack.bg_rgb.unwrap_or([15, 19, 26]);
    if !final_map_rects.is_empty() && !request.embedded_maps.is_empty() {
        for rect in &final_map_rects {
            let Some(spec) = request
                .embedded_maps
                .iter()
                .find(|m| m.widget_id == rect.widget_id)
            else {
                continue;
            };
            if rect.width == 0 || rect.height == 0 {
                continue;
            }
            let advances = spec.advances(request.follow_pinned);
            let wants_audio = advances
                && request
                    .audio_map_widget_id
                    .as_deref()
                    .is_some_and(|id| id == spec.widget_id);
            if !advances {
                let Some(fixed) = spec.fixed_hour else {
                    continue;
                };
                let map_req = map_request_for_embedded(spec, rect, &request, false);
                match render_embedded_map_still(
                    config.clone(),
                    map_req,
                    fixed,
                    0.0,
                    request.fps,
                    request.hours_per_second,
                    &cancel,
                ) {
                    Ok((mw, mh, rgba)) => map_sources.push(EmbeddedMapSource::Static {
                        rect: rect.clone(),
                        rgba,
                        width: mw,
                        height: mh,
                    }),
                    Err(error) => {
                        webview_state.unmount_window(&app, &window);
                        return if cancel.load(Ordering::Acquire) || error == "cancelled" {
                            MapVideoOutcome::Cancelled
                        } else {
                            MapVideoOutcome::Failed {
                                error: format!(
                                    "Couldn’t render embedded map “{}”: {error}",
                                    spec.display_name()
                                ),
                                campaign_hour: Some(fixed),
                                partial: None,
                            }
                        };
                    }
                }
            } else {
                let map_req = map_request_for_embedded(spec, rect, &request, wants_audio);
                let map_schedule = schedule.shifted(spec.axis_offset);
                match MapFrameStream::spawn(config.clone(), map_req, map_schedule) {
                    Ok(stream) => {
                        if wants_audio {
                            audio_enabled = true;
                        }
                        map_sources.push(EmbeddedMapSource::Stream {
                            rect: rect.clone(),
                            stream,
                            audio: wants_audio,
                        });
                    }
                    Err(error) => {
                        webview_state.unmount_window(&app, &window);
                        return MapVideoOutcome::Failed {
                            error,
                            campaign_hour: None,
                            partial: None,
                        };
                    }
                }
            }
        }
    }

    let audio_cfg = audio_enabled.then(AudioConfig::default);
    let mut sink = match video_export::open(&video_config, audio_cfg, &staged_path) {
        Ok(s) => s,
        Err(error) => {
            drop(map_sources);
            webview_state.unmount_window(&app, &window);
            return MapVideoOutcome::Failed {
                error: error.to_string(),
                campaign_hour: None,
                partial: None,
            };
        }
    };
    let backend_name = sink.backend_name().to_owned();
    let started = Instant::now();
    let mut last_progress = started;
    let total_frames = schedule.summary.total_frames;
    let mut last_recorded_hour: Option<i32> = None;
    let mut warnings: Vec<String> = Vec::new();
    let mut cached_page_rgba: Option<(Vec<u8>, Vec<ExportMapRect>)> = None;
    let mut cached_card: Option<(String, Vec<u8>)> = None;

    let emit_progress = |phase: &str, frame: u64, hour: Option<i32>, last: &mut Instant, force: bool| {
        let Some(cb) = &progress else {
            return;
        };
        let now = Instant::now();
        if !force && now.duration_since(*last).as_millis() < 120 {
            return;
        }
        *last = now;
        let elapsed_ms = now.duration_since(started).as_millis() as u64;
        let eta_ms = if frame >= 5 && total_frames > frame {
            Some(elapsed_ms.saturating_mul(total_frames - frame) / frame)
        } else {
            None
        };
        cb(ExportProgress {
            phase: phase.to_owned(),
            frame,
            total_frames,
            campaign_hour: hour,
            elapsed_ms,
            eta_ms,
            backend: Some(backend_name.clone()),
        });
    };

    let finalize_partial = |sink: Box<dyn video_export::VideoSink>,
                            error: String,
                            last_hour: Option<i32>|
     -> MapVideoOutcome {
        if cancel.load(Ordering::Acquire) {
            sink.abort();
            return MapVideoOutcome::Cancelled;
        }
        let written = sink.frames_written();
        let partial = if written > 0 {
            match sink.finish_cancellable(&cancel) {
                Ok(finished) => {
                    if cancel.load(Ordering::Acquire) {
                        let _ = std::fs::remove_file(&finished.path);
                        return MapVideoOutcome::Cancelled;
                    }
                    let playable = video_export::mp4_boxes::inspect_mp4(&finished.path)
                        .is_ok_and(|summary| summary.has_avc1);
                    if cancel.load(Ordering::Acquire) {
                        let _ = std::fs::remove_file(&finished.path);
                        return MapVideoOutcome::Cancelled;
                    }
                    if playable {
                        Some(PartialVideo {
                            staged_path: finished.path,
                            frames: finished.frames,
                            through_hour: last_hour.unwrap_or(schedule.summary.from_hour),
                            duration_seconds: finished.frames as f64 / f64::from(schedule.fps),
                            backend: finished.backend,
                        })
                    } else {
                        let _ = std::fs::remove_file(&finished.path);
                        None
                    }
                }
                Err(finish_err)
                    if finish_err.kind() == std::io::ErrorKind::Interrupted
                        || cancel.load(Ordering::Acquire) =>
                {
                    let _ = std::fs::remove_file(&staged_path);
                    return MapVideoOutcome::Cancelled;
                }
                Err(_) => {
                    let _ = std::fs::remove_file(&staged_path);
                    None
                }
            }
        } else {
            sink.abort();
            None
        };
        MapVideoOutcome::Failed {
            error,
            campaign_hour: last_hour,
            partial,
        }
    };

    for scheduled in schedule.cursor() {
        if cancel.load(Ordering::Acquire) {
            drop(map_sources);
            sink.abort();
            webview_state.unmount_window(&app, &window);
            webview_state.close_window(&app);
            return MapVideoOutcome::Cancelled;
        }
        let virtual_now_ms = (scheduled.frame_index as f64 * 1000.0) / f64::from(schedule.fps);
        let (step_payload, card_key, frame_hour) = step_payload_for_frame(
            webview_state.next_seq(),
            scheduled.frame_index,
            virtual_now_ms,
            &scheduled.kind,
            &request,
            metrics,
            false,
        );

        let mut frame_audio: Option<Vec<i16>> = None;
        let mut embedded_frames: Vec<(ExportMapRect, Vec<u8>, u32, u32)> = Vec::new();
        for source in &map_sources {
            match source {
                EmbeddedMapSource::Stream {
                    rect,
                    stream,
                    audio,
                } => match stream.next_frame(&cancel) {
                    Ok(chunk) => {
                        if *audio {
                            frame_audio = Some(chunk.audio);
                            for warning in chunk.warnings {
                                if !warnings.contains(&warning) {
                                    warnings.push(warning);
                                }
                            }
                        }
                        if let Some(rgba) = chunk.rgba {
                            let (mw, mh) = embedded_map_render_size(rect);
                            embedded_frames.push((rect.clone(), rgba, mw, mh));
                        }
                    }
                    Err(error) => {
                        drop(map_sources);
                        webview_state.unmount_window(&app, &window);
                        webview_state.close_window(&app);
                        return if cancel.load(Ordering::Acquire) || error == "cancelled" {
                            sink.abort();
                            MapVideoOutcome::Cancelled
                        } else {
                            finalize_partial(sink, error, last_recorded_hour)
                        };
                    }
                },
                EmbeddedMapSource::Static {
                    rect,
                    rgba,
                    width,
                    height,
                } => {
                    if card_key.is_none() {
                        embedded_frames.push((rect.clone(), rgba.clone(), *width, *height));
                    }
                }
            }
        }

        let (mut frame_rgba, frame_overlay_rects) = if let Some(key) = &card_key {
            if cached_card.as_ref().is_some_and(|(k, _)| k == key) {
                (cached_card.as_ref().expect("checked").1.clone(), Vec::new())
            } else {
                let seq = step_payload.seq;
                if let Err(e) = window.emit(STEP_EVENT, step_payload) {
                    drop(map_sources);
                    webview_state.unmount_window(&app, &window);
                    webview_state.close_window(&app);
                    return finalize_partial(sink, format!("Couldn’t step export card: {e}"), last_recorded_hour);
                }
                match webview_state.wait_for_ack(seq, &cancel) {
                    Ok(ack) if ack.error.is_none() => {
                        if let Some(rgb) = ack.bg_rgb {
                            bg_rgb = rgb;
                        }
                    }
                    Ok(ack) => {
                        drop(map_sources);
                        webview_state.unmount_window(&app, &window);
                        webview_state.close_window(&app);
                        return finalize_partial(
                            sink,
                            ack.error.unwrap_or_else(|| "Export page error".to_owned()),
                            last_recorded_hour,
                        );
                    }
                    Err(error) => {
                        drop(map_sources);
                        webview_state.unmount_window(&app, &window);
                        webview_state.close_window(&app);
                        return if cancel.load(Ordering::Acquire) || error == "cancelled" {
                            sink.abort();
                            MapVideoOutcome::Cancelled
                        } else {
                            finalize_partial(sink, error, last_recorded_hour)
                        };
                    }
                }
                let (_, png_bytes) = match capture_screenshot_png(&window, metrics, &cancel) {
                    Ok(pair) => pair,
                    Err(error) => {
                        drop(map_sources);
                        webview_state.unmount_window(&app, &window);
                        webview_state.close_window(&app);
                        return if cancel.load(Ordering::Acquire) || error == "cancelled" {
                            sink.abort();
                            MapVideoOutcome::Cancelled
                        } else {
                            finalize_partial(sink, error, last_recorded_hour)
                        };
                    }
                };
                let (sw, sh, raw) = match decode_png_rgba(&png_bytes) {
                    Ok(v) => v,
                    Err(error) => {
                        drop(map_sources);
                        webview_state.unmount_window(&app, &window);
                        webview_state.close_window(&app);
                        return finalize_partial(sink, error, last_recorded_hour);
                    }
                };
                let exact = fit_rgba_to_exact(&raw, sw, sh, request.width, request.height, bg_rgb);
                cached_card = Some((key.clone(), exact.clone()));
                (exact, Vec::new())
            }
        } else {
            let seq = step_payload.seq;
            if let Err(e) = window.emit(STEP_EVENT, step_payload) {
                drop(map_sources);
                webview_state.unmount_window(&app, &window);
                webview_state.close_window(&app);
                return finalize_partial(sink, format!("Couldn’t step export page: {e}"), last_recorded_hour);
            }
            let ack = match webview_state.wait_for_ack(seq, &cancel) {
                Ok(ack) => ack,
                Err(error) => {
                    drop(map_sources);
                    webview_state.unmount_window(&app, &window);
                    webview_state.close_window(&app);
                    return if cancel.load(Ordering::Acquire) || error == "cancelled" {
                        sink.abort();
                        MapVideoOutcome::Cancelled
                    } else {
                        finalize_partial(sink, error, last_recorded_hour)
                    };
                }
            };
            if let Some(error) = ack.error {
                drop(map_sources);
                webview_state.unmount_window(&app, &window);
                webview_state.close_window(&app);
                return finalize_partial(sink, error, last_recorded_hour);
            }
            if let Some(rgb) = ack.bg_rgb {
                bg_rgb = rgb;
            }
            if ack.viewport_clip.is_some() {
                viewport_clip = ack.viewport_clip;
            }
            if !ack.dirty
                && let Some((cached, cached_overlays)) = &cached_page_rgba
            {
                (cached.clone(), cached_overlays.clone())
            } else {
                let (_, png_bytes) = match capture_screenshot_png(&window, metrics, &cancel) {
                    Ok(pair) => pair,
                    Err(error) => {
                        drop(map_sources);
                        webview_state.unmount_window(&app, &window);
                        webview_state.close_window(&app);
                        return if cancel.load(Ordering::Acquire) || error == "cancelled" {
                            sink.abort();
                            MapVideoOutcome::Cancelled
                        } else {
                            finalize_partial(sink, error, last_recorded_hour)
                        };
                    }
                };
                let (sw, sh, raw) = match decode_png_rgba(&png_bytes) {
                    Ok(v) => v,
                    Err(error) => {
                        drop(map_sources);
                        webview_state.unmount_window(&app, &window);
                        webview_state.close_window(&app);
                        return finalize_partial(sink, error, last_recorded_hour);
                    }
                };
                let exact = fit_rgba_to_exact(&raw, sw, sh, request.width, request.height, bg_rgb);
                cached_page_rgba = Some((exact.clone(), ack.overlay_rects.clone()));
                (exact, ack.overlay_rects)
            }
        };

        if card_key.is_none() {
            if !embedded_frames.is_empty() {
                let overlay_patches =
                    save_overlay_patches(&frame_rgba, request.width, request.height, &frame_overlay_rects);
                for (rect, mrgba, mw, mh) in &embedded_frames {
                    blit_rgba_scaled_clipped(
                        &mut frame_rgba,
                        request.width,
                        request.height,
                        mrgba,
                        *mw,
                        *mh,
                        rect.x,
                        rect.y,
                        rect.width,
                        rect.height,
                        viewport_clip,
                    );
                }
                restore_overlay_patches(&mut frame_rgba, request.width, request.height, &overlay_patches);
            }
            last_recorded_hour = frame_hour;
        }

        if let Err(error) = sink.push_video_rgba(&frame_rgba) {
            drop(map_sources);
            webview_state.unmount_window(&app, &window);
            webview_state.close_window(&app);
            sink.abort();
            return MapVideoOutcome::Failed {
                error: error.to_string(),
                campaign_hour: last_recorded_hour,
                partial: None,
            };
        }
        if audio_enabled {
            let samples = frame_audio
                .unwrap_or_else(|| vec![0i16; scheduled.audio_frames * 2]);
            if let Err(error) = sink.push_audio_stereo_i16(&samples) {
                drop(map_sources);
                webview_state.unmount_window(&app, &window);
                webview_state.close_window(&app);
                sink.abort();
                return MapVideoOutcome::Failed {
                    error: error.to_string(),
                    campaign_hour: last_recorded_hour,
                    partial: None,
                };
            }
        }
        emit_progress(
            "rendering",
            scheduled.frame_index + 1,
            frame_hour,
            &mut last_progress,
            false,
        );
    }

    emit_progress("finalizing", total_frames, last_recorded_hour, &mut last_progress, true);
    drop(map_sources);
    webview_state.unmount_window(&app, &window);
    webview_state.close_window(&app);

    if cancel.load(Ordering::Acquire) {
        sink.abort();
        return MapVideoOutcome::Cancelled;
    }

    match sink.finish_cancellable(&cancel) {
        Ok(finished) => {
            if cancel.load(Ordering::Acquire) {
                let _ = std::fs::remove_file(&finished.path);
                return MapVideoOutcome::Cancelled;
            }
            MapVideoOutcome::Completed {
                staged_path: finished.path,
                frames: finished.frames,
                audio_frames: finished.audio_frames,
                backend: finished.backend,
                warnings,
            }
        }
        Err(error)
            if error.kind() == std::io::ErrorKind::Interrupted
                || cancel.load(Ordering::Acquire) =>
        {
            MapVideoOutcome::Cancelled
        }
        Err(error) => MapVideoOutcome::Failed {
            error: error.to_string(),
            campaign_hour: last_recorded_hour,
            partial: None,
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn base64_round_trips_binary_payloads() {
        let sample: Vec<u8> = (0u8..=255).collect();
        let encoded = encode_base64(&sample);
        let decoded = decode_base64(&encoded).unwrap();
        assert_eq!(decoded, sample);
    }

    #[test]
    fn blit_rgba_clips_partially_offscreen_embedded_map_rect() {
        let mut dst = vec![0u8; 4 * 4 * 4];
        let src = vec![255u8; 2 * 2 * 4];
        blit_rgba(&mut dst, 4, 4, &src, 2, 2, 3, 3);
        let idx = (3 * 4 + 3) * 4;
        assert_eq!(&dst[idx..idx + 4], &[255, 255, 255, 255]);
        assert_eq!(&dst[0..4], &[0, 0, 0, 0]);
    }

    #[test]
    fn blit_rgba_clipped_confines_pixels_to_visible_viewport_clip() {
        let mut dst = vec![0u8; 6 * 6 * 4];
        let mut src = vec![0u8; 4 * 4 * 4];
        for y in 0..4 {
            for x in 0..4 {
                let p = (y * 4 + x) * 4;
                src[p] = (x as u8 + 1) * 10;
                src[p + 1] = (y as u8 + 1) * 20;
                src[p + 2] = 77;
                src[p + 3] = 255;
            }
        }
        let clip = ExportClipRect { x: 1, y: 1, width: 3, height: 2 };
        blit_rgba_clipped(&mut dst, 6, 6, &src, 4, 4, 2, 2, Some(clip));
        assert_eq!(&dst[(2 * 6 + 2) * 4..(2 * 6 + 2) * 4 + 4], &[10, 20, 77, 255]);
        assert_eq!(&dst[(2 * 6 + 3) * 4..(2 * 6 + 3) * 4 + 4], &[20, 20, 77, 255]);
        assert_eq!(&dst[(2 * 6 + 4) * 4..(2 * 6 + 4) * 4 + 4], &[0, 0, 0, 0]);
        assert_eq!(&dst[(3 * 6 + 2) * 4..(3 * 6 + 2) * 4 + 4], &[0, 0, 0, 0]);
    }

    #[test]
    fn blit_rgba_scaled_clipped_scales_sub_minimum_slot_within_clip() {
        let mut dst = vec![0u8; 16 * 16 * 4];
        let src = vec![90u8; 16 * 16 * 4];
        let clip = ExportClipRect { x: 2, y: 2, width: 8, height: 6 };
        blit_rgba_scaled_clipped(&mut dst, 16, 16, &src, 16, 16, 2, 2, 10, 8, Some(clip));
        assert_eq!(&dst[(2 * 16 + 2) * 4..(2 * 16 + 2) * 4 + 4], &[90, 90, 90, 90]);
        assert_eq!(&dst[(7 * 16 + 9) * 4..(7 * 16 + 9) * 4 + 4], &[90, 90, 90, 90]);
        assert_eq!(&dst[(7 * 16 + 10) * 4..(7 * 16 + 10) * 4 + 4], &[0, 0, 0, 0]);
        assert_eq!(&dst[(8 * 16 + 2) * 4..(8 * 16 + 2) * 4 + 4], &[0, 0, 0, 0]);
        assert_eq!(
            embedded_map_render_size(&ExportMapRect {
                widget_id: "m".into(),
                x: 0,
                y: 0,
                width: 2,
                height: 12,
            }),
            (16, 96)
        );
    }

    #[test]
    fn fit_rgba_pads_or_crops_to_exact_encoder_dimensions() {
        let src = vec![99u8; 3 * 2 * 4];
        let fitted = fit_rgba_to_exact(&src, 3, 2, 4, 4, [240, 242, 245]);
        assert_eq!(fitted.len(), 4 * 4 * 4);
        assert_eq!(&fitted[0..4], &[99, 99, 99, 99]);
        assert_eq!(&fitted[(3 * 4 + 3) * 4..(3 * 4 + 3) * 4 + 4], &[240, 242, 245, 255]);
    }
}
