use crate::{
    Options,
    app::Viewer,
    document::Document,
    host::{self, Lost},
};
use dashboard_core::config::Config;
use dashboard_query::timeline::{Join, Timeline};
use eframe::{egui, egui_wgpu, wgpu};
use replay_data::{BranchId, Hour};
use serde::{Deserialize, Serialize};
use shader_pipeline::gpu::stats::CountedPasses as _;
use std::{
    io,
    path::{Path, PathBuf},
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, Ordering},
    },
    time::Instant,
};
use video_export::{
    AudioConfig, BackendChoice, Quality, VideoConfig, VideoSink,
    schedule::{
        CutNotice, FrameKind, OwnedFrameCursor, RecordedStretch, ScheduleRequest, ScheduledFrame,
        StretchJoin, UnheldGap, build_schedule,
    },
};
use viewer_foundation::perspective::Choice;
use viewer_gpu::{
    camera::{Camera, Free, Lens, MapSize, Placement, Pose},
    gpu_contract::{OUTPUT_FORMAT, read_texture},
};
use viewer_map_modes::map_mode::MapMode;
use viewer_world::scene::Scene;

#[derive(Clone, Copy, Debug, PartialEq, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CropRect {
    pub x: f32,
    pub y: f32,
    pub width: f32,
    pub height: f32,
}

impl Default for CropRect {
    fn default() -> Self {
        Self {
            x: 0.0,
            y: 0.0,
            width: 1.0,
            height: 1.0,
        }
    }
}

impl CropRect {
    pub fn normalized(self) -> Self {
        let w = self.width.clamp(0.05, 1.0);
        let h = self.height.clamp(0.05, 1.0);
        let x = self.x.clamp(0.0, 1.0 - w);
        let y = self.y.clamp(0.0, 1.0 - h);
        Self { x, y, width: w, height: h }
    }

    pub fn is_full(self) -> bool {
        self.x.abs() < 1e-4
            && self.y.abs() < 1e-4
            && (self.width - 1.0).abs() < 1e-4
            && (self.height - 1.0).abs() < 1e-4
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Deserialize, Serialize)]
#[serde(tag = "kind", rename_all = "camelCase")]
pub enum SavedPose {
    Map,
    Free {
        yaw: f32,
        pitch: f32,
        roll: f32,
        fov_y: f32,
        near: f32,
        far: f32,
    },
}

#[derive(Clone, Copy, Debug, PartialEq, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SavedPlacement {
    pub center: [f32; 2],
    pub height: f32,
    pub pose: SavedPose,
}

impl From<Placement> for SavedPlacement {
    fn from(value: Placement) -> Self {
        let pose = match value.pose {
            Pose::Map => SavedPose::Map,
            Pose::Free(free) => SavedPose::Free {
                yaw: free.yaw,
                pitch: free.pitch,
                roll: free.roll,
                fov_y: free.lens.fov_y,
                near: free.lens.near,
                far: free.lens.far,
            },
        };
        Self {
            center: value.center,
            height: value.height,
            pose,
        }
    }
}

impl From<SavedPlacement> for Placement {
    fn from(value: SavedPlacement) -> Self {
        let pose = match value.pose {
            SavedPose::Map => Pose::Map,
            SavedPose::Free {
                yaw,
                pitch,
                roll,
                fov_y,
                near,
                far,
            } => {
                let mut free = Free::new(yaw, pitch, Lens { fov_y, near, far });
                free.roll = roll;
                Pose::Free(free)
            }
        };
        Self {
            center: value.center,
            height: value.height,
            pose,
        }
    }
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct MapExportSnapshot {
    pub path: String,
    pub revision: Option<String>,
    pub branch: BranchId,
    pub current_hour: Hour,
    pub placement: SavedPlacement,
    pub viewport: [u32; 2],
    pub ui_scale: f32,
    pub map_mode: String,
    pub perspective: Option<String>,
    #[serde(default)]
    pub perspective_slot: Option<u32>,
    pub fog: bool,
    pub day_night: bool,
    pub allied_plans: bool,
    pub counter_colors_country: bool,
    pub nato_counters: bool,
    pub audio_muted: bool,
    #[serde(default)]
    pub map_settings: Option<dashboard_core::protocol::ResolvedMapSettings>,
    pub max_texture_dimension: u32,
    pub stretches: Vec<RecordedStretch>,
    pub unheld: Vec<UnheldGap>,
    pub first_hour: Hour,
    pub last_hour: Hour,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct MapVideoRequest {
    pub path: String,
    pub revision: Option<String>,
    pub branch: BranchId,
    pub from_hour: Hour,
    pub to_hour: Hour,
    pub fps: u32,
    pub hours_per_second: f64,
    pub skip_gaps: bool,
    pub cut_card_seconds: f64,
    pub width: u32,
    pub height: u32,
    pub quality: Quality,
    pub backend: BackendChoice,
    pub ffmpeg_path: Option<String>,
    pub placement: Option<SavedPlacement>,
    pub source_viewport: Option<[u32; 2]>,
    pub crop: Option<CropRect>,
    pub ui_scale: Option<f32>,
    pub map_mode: Option<String>,
    pub perspective: Option<String>,
    #[serde(default)]
    pub perspective_slot: Option<u32>,
    pub fog: bool,
    pub day_night: Option<bool>,
    pub allied_plans: Option<bool>,
    pub counter_colors_country: Option<bool>,
    pub nato_counters: Option<bool>,
    #[serde(default)]
    pub map_settings: Option<dashboard_core::protocol::ResolvedMapSettings>,
    pub date_stamp: bool,
    pub audio_enabled: bool,
    pub allow_unverified: bool,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ExportProgress {
    pub phase: String,
    pub frame: u64,
    pub total_frames: u64,
    pub campaign_hour: Option<Hour>,
    pub elapsed_ms: u64,
    pub eta_ms: Option<u64>,
    pub backend: Option<String>,
}

#[derive(Clone, Debug)]
pub struct PartialVideo {
    pub staged_path: PathBuf,
    pub frames: u64,
    pub through_hour: Hour,
    pub duration_seconds: f64,
    pub backend: String,
}

#[derive(Clone, Debug)]
pub enum MapVideoOutcome {
    Completed {
        staged_path: PathBuf,
        frames: u64,
        audio_frames: u64,
        backend: String,
        warnings: Vec<String>,
    },
    Cancelled,
    Failed {
        error: String,
        campaign_hour: Option<Hour>,
        partial: Option<PartialVideo>,
    },
}

pub fn branch_schedule_inputs(
    path: &Path,
    branch: BranchId,
) -> Result<(Vec<RecordedStretch>, Vec<UnheldGap>), String> {
    let replay = replay::campaign::Replay::open_timelines(path)
        .map_err(|e| format!("Couldn’t open recording for export: {e}"))?;
    let timeline = dashboard_query::timeline::resolve(&replay, branch)
        .map_err(|e| format!("Couldn’t read branch timeline: {e}"))?;
    Ok(timeline_to_schedule_inputs(&timeline))
}

pub fn timeline_to_schedule_inputs(timeline: &Timeline) -> (Vec<RecordedStretch>, Vec<UnheldGap>) {
    let stretches = timeline
        .stretches
        .iter()
        .map(|s| RecordedStretch {
            first: s.first,
            last: s.last,
            join: match s.join {
                Join::Start => StretchJoin::Start,
                Join::Load { .. } => StretchJoin::Load,
                Join::Break => StretchJoin::Break,
            },
        })
        .collect();
    let unheld = timeline
        .unheld
        .iter()
        .map(|g| UnheldGap {
            first: g.first,
            last: g.last,
            reason: g.reason.clone(),
        })
        .collect();
    (stretches, unheld)
}

pub fn cropped_placement(camera: &Camera, source_viewport: [u32; 2], crop: CropRect) -> Placement {
    let crop = crop.normalized();
    if crop.is_full() || source_viewport[0] == 0 || source_viewport[1] == 0 {
        return camera.placement();
    }
    let cx = (crop.x + crop.width * 0.5) * source_viewport[0] as f32;
    let cy = (crop.y + crop.height * 0.5) * source_viewport[1] as f32;
    match camera.pose {
        Pose::Map => {
            let center = ray_ground_hit(camera, source_viewport, [cx, cy]).unwrap_or(camera.center);
            let [min_h, max_h] = camera.defines.heights;
            let height = (camera.height * crop.height).clamp(min_h, max_h);
            Placement {
                center,
                height,
                pose: Pose::Map,
            }
        }
        Pose::Free(mut free) => {
            let half_fov = (free.lens.fov_y * 0.5).tan();
            let dir = camera.ray([cx, cy], source_viewport);
            let horiz = dir[0].hypot(dir[2]);
            if dir.iter().all(|v| v.is_finite()) && horiz.hypot(dir[1]) > 1e-6 {
                free.yaw = dir[0].atan2(dir[2]);
                free.pitch = dir[1]
                    .atan2(horiz)
                    .clamp(-viewer_gpu::camera::PITCH_LIMIT, viewer_gpu::camera::PITCH_LIMIT);
            }
            free.lens.fov_y = (2.0 * (half_fov * crop.height).atan()).clamp(0.05, 3.0);
            Placement {
                center: camera.center,
                height: camera.height,
                pose: Pose::Free(free),
            }
        }
    }
}

pub fn ensure_distinct_export_path(input: &Path, output: &Path) -> Result<(), String> {
    let canonical_in = std::fs::canonicalize(input)
        .map_err(|e| format!("Couldn’t inspect input recording path: {e}"))?;
    let same_target = std::fs::canonicalize(output)
        .map(|canon_out| canon_out == canonical_in)
        .unwrap_or_else(|_| {
            std::path::absolute(output)
                .ok()
                .zip(std::path::absolute(input).ok())
                .is_some_and(|(a, b)| a == b)
        });
    if same_target {
        return Err("Export destination cannot overwrite the input recording.".to_owned());
    }
    Ok(())
}

fn ray_ground_hit(camera: &Camera, viewport: [u32; 2], pixel: [f32; 2]) -> Option<[f32; 2]> {
    let inv = camera.inverse(viewport)?;
    let ndc_x = (pixel[0] / viewport[0] as f32) * 2.0 - 1.0;
    let ndc_y = 1.0 - (pixel[1] / viewport[1] as f32) * 2.0;
    let unproject = |z: f32| -> Option<[f32; 3]> {
        let mut out = [0.0f32; 4];
        for col in 0..4 {
            out[col] = ndc_x * inv[0][col] + ndc_y * inv[1][col] + z * inv[2][col] + inv[3][col];
        }
        (out[3].abs() > 1e-6).then_some([out[0] / out[3], out[1] / out[3], out[2] / out[3]])
    };
    let near = unproject(0.0)?;
    let far = unproject(1.0)?;
    let dy = far[1] - near[1];
    if dy.abs() < 1e-6 {
        return None;
    }
    let t = (viewer_gpu::camera::WATER_HEIGHT - near[1]) / dy;
    if !(0.0..1e6).contains(&t) {
        return None;
    }
    let hit_x = (near[0] + (far[0] - near[0]) * t).rem_euclid(camera.map.width as f32);
    let hit_z = (near[2] + (far[2] - near[2]) * t).clamp(0.0, camera.map.height as f32);
    Some([hit_x, hit_z])
}

pub fn proportional_ui_scale(
    base_ui_scale: f32,
    source_height: u32,
    output_height: u32,
    crop_height: f32,
) -> f32 {
    if source_height == 0 || output_height == 0 || crop_height <= 0.0 {
        return base_ui_scale.clamp(
            *dashboard_core::protocol::MAP_UI_SCALE_RANGE.start(),
            *dashboard_core::protocol::MAP_UI_SCALE_RANGE.end(),
        );
    }
    let ratio = output_height as f32 / (source_height as f32 * crop_height);
    (base_ui_scale * ratio).clamp(
        *dashboard_core::protocol::MAP_UI_SCALE_RANGE.start(),
        *dashboard_core::protocol::MAP_UI_SCALE_RANGE.end(),
    )
}

pub fn format_campaign_date(hour: Hour) -> String {
    let elapsed = i64::from(hour) - 43_800_000;
    if elapsed < 0 {
        return format!("Hour {hour}");
    }
    let day_index = elapsed / 24;
    let clock = (elapsed % 24) + 1;
    let year = day_index / 365;
    let mut doy = day_index % 365;
    const MONTHS: [(&str, i64); 12] = [
        ("Jan", 31),
        ("Feb", 28),
        ("Mar", 31),
        ("Apr", 30),
        ("May", 31),
        ("Jun", 30),
        ("Jul", 31),
        ("Aug", 31),
        ("Sep", 30),
        ("Oct", 31),
        ("Nov", 30),
        ("Dec", 31),
    ];
    let mut month_name = "Jan";
    for (name, days) in MONTHS {
        if doy < days {
            month_name = name;
            break;
        }
        doy -= days;
    }
    let day = doy + 1;
    format!("{day:02} {month_name} {year} · {clock:02}:00")
}

struct OverlayBurner {
    context: egui::Context,
    renderer: egui_wgpu::Renderer,
    card_texture: Option<(wgpu::Texture, wgpu::TextureView, [u32; 2])>,
}

impl OverlayBurner {
    fn new(device: &wgpu::Device) -> Self {
        let context = egui::Context::default();
        let renderer = egui_wgpu::Renderer::new(
            device,
            OUTPUT_FORMAT,
            egui_wgpu::RendererOptions {
                dithering: false,
                ..Default::default()
            },
        );
        Self {
            context,
            renderer,
            card_texture: None,
        }
    }

    fn draw_on_target(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        target: &wgpu::TextureView,
        size: [u32; 2],
        clear: Option<wgpu::Color>,
        paint: impl FnMut(&mut egui::Ui),
    ) {
        let [width, height] = size;
        let mut output = self.context.run_ui(
            egui::RawInput {
                screen_rect: Some(egui::Rect::from_min_size(
                    egui::Pos2::ZERO,
                    egui::vec2(width as f32, height as f32),
                )),
                ..Default::default()
            },
            paint,
        );
        let jobs = self.context.tessellate(output.shapes, output.pixels_per_point);
        let screen = egui_wgpu::ScreenDescriptor {
            size_in_pixels: size,
            pixels_per_point: output.pixels_per_point,
        };
        for (id, deltas) in output.textures_delta.set.drain() {
            for delta in &deltas {
                self.renderer.update_texture(device, queue, id, delta);
            }
        }
        let mut encoder = device.create_command_encoder(&wgpu::CommandEncoderDescriptor {
            label: Some("video export overlay"),
        });
        let commands = self
            .renderer
            .update_buffers(device, queue, &mut encoder, &jobs, &screen);
        {
            let load = match clear {
                Some(color) => wgpu::LoadOp::Clear(color),
                None => wgpu::LoadOp::Load,
            };
            let mut pass = encoder
                .counted_render_pass(&wgpu::RenderPassDescriptor {
                    label: Some("video export overlay pass"),
                    color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                        view: target,
                        depth_slice: None,
                        resolve_target: None,
                        ops: wgpu::Operations {
                            load,
                            store: wgpu::StoreOp::Store,
                        },
                    })],
                    depth_stencil_attachment: None,
                    timestamp_writes: None,
                    occlusion_query_set: None,
                    multiview_mask: None,
                })
                .forget_lifetime();
            self.renderer.render(&mut pass, &jobs, &screen);
        }
        queue.submit(commands.into_iter().chain([encoder.finish()]));
        for id in output.textures_delta.free.drain() {
            self.renderer.free_texture(&id);
        }
    }

    fn burn_map_overlays(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        scene: &Scene,
        date_text: Option<&str>,
        cut_banner: Option<&CutNotice>,
        unverified: Option<&str>,
    ) -> io::Result<()> {
        if date_text.is_none() && cut_banner.is_none() && unverified.is_none() {
            return Ok(());
        }
        let targets = scene
            .targets()
            .ok_or_else(|| io::Error::other("Map target unavailable for overlay burn-in"))?;
        let size = targets.size();
        let scale = (size[1] as f32 / 1080.0).clamp(0.65, 2.5);
        self.draw_on_target(device, queue, targets.output_view(), size, None, |ui| {
            let painter = ui.painter();
            if let Some(text) = unverified {
                let galley = painter.layout(
                    format!("UNVERIFIED CONTENT\n{text}"),
                    egui::FontId::proportional(16.0 * scale),
                    egui::Color32::BLACK,
                    (size[0] as f32 - 24.0 * scale).max(1.0),
                );
                painter.rect_filled(
                    egui::Rect::from_min_size(
                        egui::Pos2::ZERO,
                        egui::vec2(size[0] as f32, galley.size().y + 20.0 * scale),
                    ),
                    0.0,
                    egui::Color32::from_rgb(238, 173, 49),
                );
                painter.galley(
                    egui::pos2(12.0 * scale, 10.0 * scale),
                    galley,
                    egui::Color32::BLACK,
                );
            }
            if let Some(date) = date_text {
                let font = egui::FontId::proportional(20.0 * scale);
                let galley = painter.layout_no_wrap(
                    date.to_owned(),
                    font,
                    egui::Color32::from_rgb(240, 243, 248),
                );
                let pad_x = 14.0 * scale;
                let pad_y = 8.0 * scale;
                let margin = 18.0 * scale;
                let pill_size = egui::vec2(galley.size().x + pad_x * 2.0, galley.size().y + pad_y * 2.0);
                let top_left = egui::pos2(
                    (size[0] as f32 - pill_size.x - margin).max(margin),
                    margin + if unverified.is_some() { 54.0 * scale } else { 0.0 },
                );
                let rect = egui::Rect::from_min_size(top_left, pill_size);
                painter.rect_filled(
                    rect,
                    6.0 * scale,
                    egui::Color32::from_rgba_unmultiplied(15, 19, 26, 212),
                );
                painter.rect_stroke(
                    rect,
                    6.0 * scale,
                    egui::Stroke::new(1.0 * scale, egui::Color32::from_rgba_unmultiplied(140, 160, 190, 110)),
                    egui::StrokeKind::Inside,
                );
                painter.galley(
                    egui::pos2(rect.min.x + pad_x, rect.min.y + pad_y),
                    galley,
                    egui::Color32::from_rgb(240, 243, 248),
                );
            }
            if let Some(cut) = cut_banner {
                let label = format!(
                    "Unrecorded gap skipped · {} -> {} ({} h omitted)",
                    format_campaign_date(cut.from_hour),
                    format_campaign_date(cut.to_hour),
                    cut.omitted_hours,
                );
                let galley = painter.layout_no_wrap(
                    label,
                    egui::FontId::proportional(17.0 * scale),
                    egui::Color32::from_rgb(246, 232, 196),
                );
                let pad_x = 16.0 * scale;
                let pad_y = 8.0 * scale;
                let pill_size = egui::vec2(galley.size().x + pad_x * 2.0, galley.size().y + pad_y * 2.0);
                let rect = egui::Rect::from_min_size(
                    egui::pos2(
                        ((size[0] as f32 - pill_size.x) * 0.5).max(12.0),
                        size[1] as f32 - pill_size.y - 24.0 * scale,
                    ),
                    pill_size,
                );
                painter.rect_filled(
                    rect,
                    6.0 * scale,
                    egui::Color32::from_rgba_unmultiplied(24, 20, 14, 224),
                );
                painter.galley(
                    egui::pos2(rect.min.x + pad_x, rect.min.y + pad_y),
                    galley,
                    egui::Color32::from_rgb(246, 232, 196),
                );
            }
        });
        Ok(())
    }

    fn render_card_rgba(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        size: [u32; 2],
        heading: &str,
        dates: &str,
        detail: &str,
    ) -> io::Result<Vec<u8>> {
        let needs_alloc = self
            .card_texture
            .as_ref()
            .is_none_or(|(_, _, held)| *held != size);
        if needs_alloc {
            let texture = device.create_texture(&wgpu::TextureDescriptor {
                label: Some("video export gap card"),
                size: wgpu::Extent3d {
                    width: size[0],
                    height: size[1],
                    depth_or_array_layers: 1,
                },
                mip_level_count: 1,
                sample_count: 1,
                dimension: wgpu::TextureDimension::D2,
                format: OUTPUT_FORMAT,
                usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::COPY_SRC,
                view_formats: &[],
            });
            let view = texture.create_view(&wgpu::TextureViewDescriptor::default());
            self.card_texture = Some((texture, view, size));
        }
        let scale = (size[1] as f32 / 1080.0).clamp(0.65, 2.5);
        let clear = Some(wgpu::Color {
            r: 0.055,
            g: 0.068,
            b: 0.09,
            a: 1.0,
        });
        let (_, view, _) = self.card_texture.as_ref().expect("allocated above");
        let view = view.clone();
        self.draw_on_target(device, queue, &view, size, clear, |ui| {
            let painter = ui.painter();
            let max_w = (size[0] as f32 * 0.72).max(200.0);
            let h_galley = painter.layout(
                heading.to_owned(),
                egui::FontId::proportional(34.0 * scale),
                egui::Color32::from_rgb(240, 243, 248),
                max_w,
            );
            let d_galley = painter.layout(
                dates.to_owned(),
                egui::FontId::proportional(24.0 * scale),
                egui::Color32::from_rgb(198, 212, 232),
                max_w,
            );
            let b_galley = painter.layout(
                detail.to_owned(),
                egui::FontId::proportional(18.0 * scale),
                egui::Color32::from_rgb(154, 168, 188),
                max_w,
            );
            let gap = 14.0 * scale;
            let total_h = h_galley.size().y + d_galley.size().y + b_galley.size().y + gap * 2.0;
            let mut y = ((size[1] as f32 - total_h) * 0.5).max(20.0);
            for (galley, color) in [
                (h_galley, egui::Color32::from_rgb(240, 243, 248)),
                (d_galley, egui::Color32::from_rgb(198, 212, 232)),
                (b_galley, egui::Color32::from_rgb(154, 168, 188)),
            ] {
                let x = ((size[0] as f32 - galley.size().x) * 0.5).max(16.0);
                let h = galley.size().y;
                painter.galley(egui::pos2(x, y), galley, color);
                y += h + gap;
            }
        });
        let (texture, _, _) = self.card_texture.as_ref().expect("allocated above");
        read_texture(device, queue, texture, 4)
    }
}

pub(crate) enum VideoMode {
    Encode {
        sink: Option<Box<dyn VideoSink>>,
        staged_path: PathBuf,
        outcome: Arc<Mutex<Option<MapVideoOutcome>>>,
    },
    Preview {
        png_out: Arc<Mutex<Option<Result<Vec<u8>, String>>>>,
    },
    Frames {
        tx: std::sync::mpsc::SyncSender<Result<MapFrameChunk, String>>,
    },
}

#[derive(Clone, Debug)]
pub struct MapFrameChunk {
    pub rgba: Option<Vec<u8>>,
    pub audio: Vec<i16>,
}

pub struct MapFrameStream {
    rx: std::sync::mpsc::Receiver<Result<MapFrameChunk, String>>,
    cancel: Arc<AtomicBool>,
    handle: Option<std::thread::JoinHandle<()>>,
}

impl MapFrameStream {
    pub fn spawn(
        config: Config,
        request: MapVideoRequest,
        schedule: FrameSchedule,
        cancel: Arc<AtomicBool>,
    ) -> Result<Self, String> {
        let (tx, rx) = std::sync::mpsc::sync_channel(2);
        let thread_cancel = Arc::clone(&cancel);
        let tx_err = tx.clone();
        let handle = std::thread::Builder::new()
            .name("embedded-map-export".into())
            .spawn(move || {
                if let Err(error) = run_map_video_inner(
                    config,
                    request,
                    VideoTarget::Frames { tx, schedule },
                    thread_cancel,
                    None,
                ) {
                    let _ = tx_err.send(Err(error));
                }
            })
            .map_err(|e| format!("Couldn’t start embedded map export thread: {e}"))?;
        Ok(Self {
            rx,
            cancel,
            handle: Some(handle),
        })
    }

    pub fn next_frame(&self) -> Result<MapFrameChunk, String> {
        self.rx
            .recv()
            .map_err(|_| "Embedded map renderer stopped before finishing".to_owned())?
    }
}

impl Drop for MapFrameStream {
    fn drop(&mut self) {
        self.cancel.store(true, Ordering::Release);
        if let Some(handle) = self.handle.take() {
            let _ = handle.join();
        }
    }
}

pub(crate) struct VideoSession {
    cursor: OwnedFrameCursor,
    pending: Option<ScheduledFrame>,
    frames_done: u64,
    pub size: [u32; 2],
    pub date_stamp: bool,
    pub audio_enabled: bool,
    audio_warning_recorded: bool,
    pub cancel: Arc<AtomicBool>,
    pub progress: Option<Arc<dyn Fn(ExportProgress) + Send + Sync>>,
    pub started: Instant,
    pub last_progress: Instant,
    pub mixer: Option<viewer_audio::OfflineMixer>,
    pub audio_buf: Vec<i16>,
    pub last_recorded_hour: Option<Hour>,
    pub warnings: Vec<String>,
    pub mode: VideoMode,
    burner: Option<OverlayBurner>,
    cached_card: Option<(String, Vec<u8>)>,
}

impl VideoSession {
    pub(crate) fn new(
        mut cursor: OwnedFrameCursor,
        size: [u32; 2],
        date_stamp: bool,
        audio_enabled: bool,
        cancel: Arc<AtomicBool>,
        progress: Option<Arc<dyn Fn(ExportProgress) + Send + Sync>>,
        mode: VideoMode,
    ) -> Self {
        let pending = cursor.next();
        let now = Instant::now();
        Self {
            cursor,
            pending,
            frames_done: 0,
            size,
            date_stamp,
            audio_enabled,
            audio_warning_recorded: false,
            cancel,
            progress,
            started: now,
            last_progress: now,
            mixer: None,
            audio_buf: Vec::new(),
            last_recorded_hour: None,
            warnings: Vec::new(),
            mode,
            burner: None,
            cached_card: None,
        }
    }

    pub(crate) fn fps(&self) -> u32 {
        self.cursor.schedule().fps
    }

    pub(crate) fn current(&self) -> Option<&ScheduledFrame> {
        self.pending.as_ref()
    }

    fn take_current(&mut self) -> Option<ScheduledFrame> {
        let cur = self.pending.take();
        if cur.is_some() {
            self.pending = self.cursor.next();
        }
        cur
    }

    pub(crate) fn emit_progress(&mut self, phase: &str, hour: Option<Hour>, force: bool) {
        let Some(cb) = &self.progress else {
            return;
        };
        let now = Instant::now();
        if !force && now.duration_since(self.last_progress).as_millis() < 120 {
            return;
        }
        self.last_progress = now;
        let frame = self.frames_done;
        let total_frames = self.cursor.schedule().summary.total_frames;
        let elapsed_ms = now.duration_since(self.started).as_millis() as u64;
        let eta_ms = if frame >= 5 && total_frames > frame {
            Some(elapsed_ms.saturating_mul(total_frames - frame) / frame)
        } else {
            None
        };
        let backend = match &self.mode {
            VideoMode::Encode { sink, .. } => sink.as_ref().map(|s| s.backend_name().to_owned()),
            VideoMode::Preview { .. } | VideoMode::Frames { .. } => None,
        };
        cb(ExportProgress {
            phase: phase.to_owned(),
            frame,
            total_frames,
            campaign_hour: hour,
            elapsed_ms,
            eta_ms,
            backend,
        });
    }

    pub(crate) fn drain_non_recorded(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
    ) -> Result<bool, String> {
        while let Some(scheduled) = self.pending.as_ref() {
            if self.cancel.load(Ordering::Acquire) {
                self.finish_cancelled();
                return Ok(true);
            }
            if matches!(scheduled.kind, FrameKind::Recorded { .. }) {
                return Ok(false);
            }
            let scheduled = self.take_current().expect("checked above");
            match scheduled.kind {
                FrameKind::Recorded { .. } => unreachable!(),
                FrameKind::GapCard {
                    hour,
                    gap_first,
                    gap_last,
                    reason,
                    crossed_gap,
                } => {
                    if matches!(self.mode, VideoMode::Frames { .. }) {
                        self.push_non_recorded_stream(scheduled.audio_frames)?;
                        self.cursor += 1;
                        continue;
                    }
                    let dates = format!(
                        "{}  (gap: {} to {})",
                        format_campaign_date(hour),
                        format_campaign_date(gap_first),
                        format_campaign_date(gap_last),
                    );
                    let detail = match crossed_gap {
                        Some(cut) => format!(
                            "{reason} · earlier gap skipped: {} -> {} ({} h omitted)",
                            format_campaign_date(cut.from_hour),
                            format_campaign_date(cut.to_hour),
                            cut.omitted_hours,
                        ),
                        None => reason,
                    };
                    let key = format!("gap:{dates}:{detail}");
                    let rgba = if self.cached_card.as_ref().is_some_and(|(k, _)| k == &key) {
                        self.cached_card.as_ref().expect("checked").1.clone()
                    } else {
                        let burner = self.burner.get_or_insert_with(|| OverlayBurner::new(device));
                        let rendered = burner
                            .render_card_rgba(device, queue, self.size, "Unrecorded stretch", &dates, &detail)
                            .map_err(|e| e.to_string())?;
                        self.cached_card = Some((key, rendered.clone()));
                        rendered
                    };
                    self.push_frame(&rgba, None, scheduled.audio_frames, Some(hour))?;
                    self.frames_done += 1;
                    self.emit_progress("rendering", Some(hour), false);
                }
                FrameKind::SkipCutCard(cut) => {
                    if matches!(self.mode, VideoMode::Frames { .. }) {
                        self.push_non_recorded_stream(scheduled.audio_frames)?;
                        self.cursor += 1;
                        continue;
                    }
                    let dates = format!(
                        "{} -> {}",
                        format_campaign_date(cut.from_hour),
                        format_campaign_date(cut.to_hour),
                    );
                    let detail = format!("{} unrecorded hours omitted · {}", cut.omitted_hours, cut.reason);
                    let key = format!("cut:{dates}:{detail}");
                    let rgba = if self.cached_card.as_ref().is_some_and(|(k, _)| k == &key) {
                        self.cached_card.as_ref().expect("checked").1.clone()
                    } else {
                        let burner = self.burner.get_or_insert_with(|| OverlayBurner::new(device));
                        let rendered = burner
                            .render_card_rgba(device, queue, self.size, "Unrecorded gap skipped", &dates, &detail)
                            .map_err(|e| e.to_string())?;
                        self.cached_card = Some((key, rendered.clone()));
                        rendered
                    };
                    self.push_frame(&rgba, None, scheduled.audio_frames, Some(cut.to_hour))?;
                    self.frames_done += 1;
                    self.emit_progress("rendering", Some(cut.to_hour), false);
                }
            }
        }
        self.finish_complete()?;
        Ok(true)
    }

    pub(crate) fn consume_recorded(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        scene: &Scene,
        document: &Document,
        camera: &Camera,
        audio_frame: Option<viewer_audio::Frame>,
        unverified: Option<&str>,
    ) -> Result<bool, String> {
        if self.cancel.load(Ordering::Acquire) {
            self.finish_cancelled();
            return Ok(true);
        }
        let Some(scheduled) = self.pending.as_ref() else {
            self.finish_complete()?;
            return Ok(true);
        };
        if !matches!(scheduled.kind, FrameKind::Recorded { .. }) {
            return self.drain_non_recorded(device, queue);
        }
        let scheduled = self.take_current().expect("checked above");
        let FrameKind::Recorded {
            hour,
            cut_banner,
            ..
        } = &scheduled.kind
        else {
            unreachable!()
        };
        let date_text = self.date_stamp.then(|| format_campaign_date(*hour));
        let burner = self.burner.get_or_insert_with(|| OverlayBurner::new(device));
        burner
            .burn_map_overlays(
                device,
                queue,
                scene,
                date_text.as_deref(),
                cut_banner.as_ref(),
                unverified,
            )
            .map_err(|e| e.to_string())?;
        let targets = scene
            .targets()
            .ok_or_else(|| "Map render target unavailable".to_owned())?;
        let rgba = read_texture(device, queue, targets.output(), 4).map_err(|e| e.to_string())?;
        let _ = (document, camera);
        self.push_frame(&rgba, audio_frame.as_ref(), scheduled.audio_frames, Some(*hour))?;
        self.last_recorded_hour = Some(*hour);
        self.frames_done += 1;
        self.emit_progress("rendering", Some(*hour), false);
        self.drain_non_recorded(device, queue)
    }

    fn push_non_recorded_stream(&mut self, audio_samples: usize) -> Result<(), String> {
        let audio = if self.audio_enabled {
            if let Some(mixer) = &mut self.mixer {
                mixer.silence_frame(audio_samples, &mut self.audio_buf);
            } else {
                self.audio_buf.clear();
                self.audio_buf.resize(audio_samples * 2, 0);
            }
            self.audio_buf.clone()
        } else {
            Vec::new()
        };
        if let VideoMode::Frames { tx } = &self.mode {
            tx.send(Ok(MapFrameChunk { rgba: None, audio }))
                .map_err(|_| "Embedded map consumer closed".to_owned())?;
        }
        Ok(())
    }

    fn push_frame(
        &mut self,
        rgba: &[u8],
        audio_frame: Option<&viewer_audio::Frame>,
        audio_samples: usize,
        hour: Option<Hour>,
    ) -> Result<(), String> {
        match &mut self.mode {
            VideoMode::Encode { sink, .. } => {
                let sink = sink
                    .as_mut()
                    .ok_or_else(|| "Video encoder is already closed".to_owned())?;
                sink.push_video_rgba(rgba).map_err(|e| e.to_string())?;
                if self.audio_enabled {
                    if let (Some(mixer), Some(frame)) = (&mut self.mixer, audio_frame) {
                        mixer.step_frame(frame, audio_samples, &mut self.audio_buf);
                    } else if let Some(mixer) = &mut self.mixer {
                        mixer.silence_frame(audio_samples, &mut self.audio_buf);
                    } else {
                        self.audio_buf.clear();
                        self.audio_buf.resize(audio_samples * 2, 0);
                    }
                    sink.push_audio_stereo_i16(&self.audio_buf)
                        .map_err(|e| e.to_string())?;
                }
                Ok(())
            }
            VideoMode::Preview { png_out } => {
                let mut encoded = Vec::new();
                let mut encoder = png::Encoder::new(&mut encoded, self.size[0], self.size[1]);
                encoder.set_color(png::ColorType::Rgba);
                encoder.set_depth(png::BitDepth::Eight);
                encoder.set_source_srgb(png::SrgbRenderingIntent::Perceptual);
                let mut writer = encoder.write_header().map_err(|e| e.to_string())?;
                writer.write_image_data(rgba).map_err(|e| e.to_string())?;
                writer.finish().map_err(|e| e.to_string())?;
                *png_out.lock().unwrap_or_else(|e| e.into_inner()) = Some(Ok(encoded));
                let _ = hour;
                Ok(())
            }
            VideoMode::Frames { tx } => {
                let audio = if self.audio_enabled {
                    if let (Some(mixer), Some(frame)) = (&mut self.mixer, audio_frame) {
                        mixer.step_frame(frame, audio_samples, &mut self.audio_buf);
                    } else if let Some(mixer) = &mut self.mixer {
                        mixer.silence_frame(audio_samples, &mut self.audio_buf);
                    } else {
                        self.audio_buf.clear();
                        self.audio_buf.resize(audio_samples * 2, 0);
                    }
                    self.audio_buf.clone()
                } else {
                    Vec::new()
                };
                tx.send(Ok(MapFrameChunk {
                    rgba: Some(rgba.to_vec()),
                    audio,
                }))
                .map_err(|_| "Embedded map consumer closed".to_owned())?;
                let _ = hour;
                Ok(())
            }
        }
    }

    pub(crate) fn ensure_mixer(&mut self, document: &Document) {
        if !self.audio_enabled {
            return;
        }
        if self.mixer.is_none() {
            if let Some(source) = &document.audio {
                self.mixer = Some(viewer_audio::OfflineMixer::new(Arc::clone(source)));
            } else if !self.audio_warning_recorded {
                self.audio_warning_recorded = true;
                let detail = document
                    .warnings
                    .iter()
                    .find(|w| w.to_ascii_lowercase().contains("sound") || w.to_ascii_lowercase().contains("audio"))
                    .cloned()
                    .unwrap_or_else(|| "Map sound catalog is unavailable; exported audio track is silent.".to_owned());
                self.warnings.push(detail);
            }
        }
    }

    pub(crate) fn finish_complete(&mut self) -> Result<(), String> {
        if self.cancel.load(Ordering::Acquire) {
            self.finish_cancelled();
            return Ok(());
        }
        if matches!(self.mode, VideoMode::Encode { .. }) {
            self.emit_progress("finalizing", self.last_recorded_hour, true);
        }
        match &mut self.mode {
            VideoMode::Encode {
                sink,
                staged_path: _,
                outcome,
            } => {
                let Some(sink) = sink.take() else {
                    return Ok(());
                };
                match sink.finish_cancellable(&self.cancel) {
                    Ok(finished) => {
                        if self.cancel.load(Ordering::Acquire) {
                            let _ = std::fs::remove_file(&finished.path);
                            *outcome.lock().unwrap_or_else(|e| e.into_inner()) =
                                Some(MapVideoOutcome::Cancelled);
                            return Ok(());
                        }
                        if let Some(mixer) = &self.mixer {
                            self.warnings.extend(mixer.notes.iter().cloned());
                        }
                        *outcome.lock().unwrap_or_else(|e| e.into_inner()) =
                            Some(MapVideoOutcome::Completed {
                                staged_path: finished.path,
                                frames: finished.frames,
                                audio_frames: finished.audio_frames,
                                backend: finished.backend,
                                warnings: std::mem::take(&mut self.warnings),
                            });
                        Ok(())
                    }
                    Err(error)
                        if error.kind() == io::ErrorKind::Interrupted
                            || self.cancel.load(Ordering::Acquire) =>
                    {
                        *outcome.lock().unwrap_or_else(|e| e.into_inner()) =
                            Some(MapVideoOutcome::Cancelled);
                        Ok(())
                    }
                    Err(error) => {
                        let msg = error.to_string();
                        *outcome.lock().unwrap_or_else(|e| e.into_inner()) =
                            Some(MapVideoOutcome::Failed {
                                error: msg.clone(),
                                campaign_hour: self.last_recorded_hour,
                                partial: None,
                            });
                        Err(msg)
                    }
                }
            }
            VideoMode::Preview { .. } | VideoMode::Frames { .. } => Ok(()),
        }
    }

    pub(crate) fn finish_cancelled(&mut self) {
        match &mut self.mode {
            VideoMode::Encode { sink, outcome, .. } => {
                if let Some(sink) = sink.take() {
                    sink.abort();
                }
                *outcome.lock().unwrap_or_else(|e| e.into_inner()) = Some(MapVideoOutcome::Cancelled);
            }
            VideoMode::Preview { png_out } => {
                *png_out.lock().unwrap_or_else(|e| e.into_inner()) =
                    Some(Err("Preview cancelled".to_owned()));
            }
            VideoMode::Frames { .. } => {}
        }
    }

    pub(crate) fn finish_render_failure(&mut self, error: String) {
        let fps = self.fps();
        let from_hour = self.cursor.schedule().summary.from_hour;
        let last_recorded_hour = self.last_recorded_hour;
        match &mut self.mode {
            VideoMode::Encode {
                sink,
                staged_path,
                outcome,
            } => {
                if self.cancel.load(Ordering::Acquire) {
                    if let Some(sink) = sink.take() {
                        sink.abort();
                    }
                    *outcome.lock().unwrap_or_else(|e| e.into_inner()) =
                        Some(MapVideoOutcome::Cancelled);
                    return;
                }
                let mut cancelled = false;
                let partial = if let Some(sink) = sink.take() {
                    let frames = sink.frames_written();
                    if frames > 0 {
                        match sink.finish_cancellable(&self.cancel) {
                            Ok(finished) => {
                                if self.cancel.load(Ordering::Acquire) {
                                    let _ = std::fs::remove_file(&finished.path);
                                    cancelled = true;
                                    None
                                } else {
                                    let playable = video_export::mp4_boxes::inspect_mp4(&finished.path)
                                        .is_ok_and(|summary| summary.has_avc1);
                                    if self.cancel.load(Ordering::Acquire) {
                                        let _ = std::fs::remove_file(&finished.path);
                                        cancelled = true;
                                        None
                                    } else if playable {
                                        Some(PartialVideo {
                                            staged_path: finished.path,
                                            frames: finished.frames,
                                            through_hour: last_recorded_hour.unwrap_or(from_hour),
                                            duration_seconds: finished.frames as f64 / f64::from(fps),
                                            backend: finished.backend,
                                        })
                                    } else {
                                        let _ = std::fs::remove_file(&finished.path);
                                        None
                                    }
                                }
                            }
                            Err(err)
                                if err.kind() == io::ErrorKind::Interrupted
                                    || self.cancel.load(Ordering::Acquire) =>
                            {
                                let _ = std::fs::remove_file(&*staged_path);
                                cancelled = true;
                                None
                            }
                            Err(_) => {
                                let _ = std::fs::remove_file(&*staged_path);
                                None
                            }
                        }
                    } else {
                        sink.abort();
                        None
                    }
                } else {
                    None
                };
                *outcome.lock().unwrap_or_else(|e| e.into_inner()) = Some(if cancelled {
                    MapVideoOutcome::Cancelled
                } else {
                    MapVideoOutcome::Failed {
                        error,
                        campaign_hour: last_recorded_hour,
                        partial,
                    }
                });
            }
            VideoMode::Preview { png_out } => {
                *png_out.lock().unwrap_or_else(|e| e.into_inner()) = Some(Err(error));
            }
            VideoMode::Frames { tx } => {
                let _ = tx.send(Err(error));
            }
        }
    }
}

pub fn run_map_video_export(
    config: Config,
    request: MapVideoRequest,
    staged_path: PathBuf,
    cancel: Arc<AtomicBool>,
    progress: Option<Arc<dyn Fn(ExportProgress) + Send + Sync>>,
) -> MapVideoOutcome {
    let outcome = Arc::new(Mutex::new(None));
    let result = run_map_video_inner(
        config,
        request,
        VideoTarget::File {
            staged_path: staged_path.clone(),
            outcome: Arc::clone(&outcome),
        },
        cancel,
        progress,
    );
    if let Some(finished) = outcome.lock().unwrap_or_else(|e| e.into_inner()).take() {
        return finished;
    }
    match result {
        Ok(()) => MapVideoOutcome::Cancelled,
        Err(error) => {
            let _ = std::fs::remove_file(&staged_path);
            MapVideoOutcome::Failed {
                error,
                campaign_hour: None,
                partial: None,
            }
        }
    }
}

pub fn render_map_video_preview(
    config: Config,
    request: MapVideoRequest,
    preview_hour: Hour,
    cancel: Arc<AtomicBool>,
) -> Result<Vec<u8>, String> {
    let png_out = Arc::new(Mutex::new(None));
    let preview_request = MapVideoRequest {
        from_hour: preview_hour,
        to_hour: preview_hour,
        audio_enabled: false,
        skip_gaps: false,
        ..request
    };
    let result = run_map_video_inner(
        config,
        preview_request,
        VideoTarget::Preview(Arc::clone(&png_out)),
        cancel,
        None,
    );
    if let Some(answer) = png_out.lock().unwrap_or_else(|e| e.into_inner()).take() {
        return answer;
    }
    result.and_then(|()| Err("Preview produced no frame".to_owned()))
}

enum VideoTarget {
    File {
        staged_path: PathBuf,
        outcome: Arc<Mutex<Option<MapVideoOutcome>>>,
    },
    Preview(Arc<Mutex<Option<Result<Vec<u8>, String>>>>),
    Frames {
        tx: std::sync::mpsc::SyncSender<Result<MapFrameChunk, String>>,
        schedule: FrameSchedule,
    },
}

fn run_map_video_inner(
    config: Config,
    request: MapVideoRequest,
    target: VideoTarget,
    cancel: Arc<AtomicBool>,
    progress: Option<Arc<dyn Fn(ExportProgress) + Send + Sync>>,
) -> Result<(), String> {
    let handle = std::thread::Builder::new()
        .name("viewer-video-export".to_owned())
        .stack_size(64 << 20)
        .spawn(move || run_map_video_on_thread(config, request, target, cancel, progress))
        .map_err(|e| format!("Couldn’t start headless map renderer: {e}"))?;
    handle
        .join()
        .map_err(|_| "Headless map renderer stopped unexpectedly".to_owned())?
}

fn run_map_video_on_thread(
    config: Config,
    request: MapVideoRequest,
    target: VideoTarget,
    cancel: Arc<AtomicBool>,
    progress: Option<Arc<dyn Fn(ExportProgress) + Send + Sync>>,
) -> Result<(), String> {
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
    if !matches!(target, VideoTarget::Frames { .. }) {
        video_config.validate().map_err(|e| e.to_string())?;
    }
    if cancel.load(Ordering::Acquire) {
        return Ok(());
    }

    let source_path = Path::new(&request.path);
    let initial_revision = dashboard_core::protocol::replay_revision(source_path)
        .map_err(|e| format!("Couldn’t inspect recording revision: {e}"))?;
    if let Some(expected) = request.revision.as_deref()
        && expected != initial_revision
    {
        return Err(
            "The recording changed on disk since this export was opened. Re-open the export dialog to use the updated file."
                .to_owned(),
        );
    }

    let replay = Arc::new(
        replay::campaign::Replay::open_timelines(source_path)
            .map_err(|e| format!("Couldn’t open recording for export: {e}"))?,
    );
    let timeline = dashboard_query::timeline::resolve(&replay, request.branch)
        .map_err(|e| format!("Couldn’t read branch timeline: {e}"))?;
    let (map_stretches, map_unheld) = timeline_to_schedule_inputs(&timeline);
    let (schedule, mode) = match target {
        VideoTarget::File {
            staged_path,
            outcome,
        } => {
            let schedule = build_schedule(
                &map_stretches,
                &map_unheld,
                &ScheduleRequest {
                    from_hour: request.from_hour,
                    to_hour: request.to_hour,
                    fps: request.fps,
                    hours_per_second: request.hours_per_second,
                    skip_gaps: request.skip_gaps,
                    cut_card_seconds: request.cut_card_seconds,
                },
            )
            .map_err(|e| e.to_string())?;
            let audio_cfg = request.audio_enabled.then(AudioConfig::default);
            let sink = video_export::open(&video_config, audio_cfg, &staged_path)
                .map_err(|e| e.to_string())?;
            (
                schedule,
                VideoMode::Encode {
                    sink: Some(sink),
                    staged_path,
                    outcome,
                },
            )
        }
        VideoTarget::Preview(png_out) => {
            let schedule = build_schedule(
                &map_stretches,
                &map_unheld,
                &ScheduleRequest {
                    from_hour: request.from_hour,
                    to_hour: request.to_hour,
                    fps: request.fps,
                    hours_per_second: request.hours_per_second,
                    skip_gaps: request.skip_gaps,
                    cut_card_seconds: request.cut_card_seconds,
                },
            )
            .map_err(|e| e.to_string())?;
            (schedule, VideoMode::Preview { png_out })
        }
        VideoTarget::Frames { tx, schedule } => (
            schedule,
            VideoMode::Frames {
                tx,
                stretches: map_stretches,
                unheld: map_unheld,
            },
        ),
    };

    let after_schedule_revision = dashboard_core::protocol::replay_revision(source_path)
        .map_err(|e| format!("Couldn’t verify recording revision: {e}"))?;
    if after_schedule_revision != initial_revision {
        return Err(
            "The recording changed on disk while preparing this export. Re-open the export dialog to retry."
                .to_owned(),
        );
    }
    if cancel.load(Ordering::Acquire) {
        return Ok(());
    }

    let first_recorded = schedule
        .first_recorded()
        .ok_or_else(|| "Selected range has no recorded hours".to_owned())?;

    let crop = request.crop.map(CropRect::normalized).unwrap_or_default();
    let effective_ui_scale = match (request.ui_scale, request.source_viewport) {
        (Some(base), Some([_, src_h])) => {
            Some(proportional_ui_scale(base, src_h, request.height, crop.height))
        }
        (Some(base), None) => Some(base),
        (None, _) => Some(1.0),
    };

    let perspective = match (request.perspective.as_deref(), request.perspective_slot) {
        (tag, Some(slot)) => Choice {
            country: viewer_foundation::perspective::Country::Slot {
                slot,
                tag: tag.unwrap_or_default().to_owned(),
            },
            fog: request.fog,
        },
        (Some(tag), None) if !tag.is_empty() => Choice {
            country: viewer_foundation::perspective::Country::Tag(tag.to_owned()),
            fog: request.fog,
        },
        _ => Choice {
            country: viewer_foundation::perspective::Country::Recorded,
            fog: request.fog,
        },
    };
    let map_mode = request
        .map_mode
        .as_deref()
        .and_then(MapMode::parse)
        .unwrap_or_default();

    let placement = request.placement.map(Placement::from);
    let source_viewport = request.source_viewport;
    let size = [request.width, request.height];
    let map_settings = request.map_settings.clone();
    let visual_mods = map_settings
        .as_ref()
        .and_then(|s| s.visual_mods.clone())
        .unwrap_or_default();

    let mut options = Options {
        replay: Some(PathBuf::from(&request.path)),
        branch: Some(request.branch),
        hour: Some(first_recorded.0),
        hour_fraction: first_recorded.1 as f32,
        size,
        camera: placement,
        ui_scale: effective_ui_scale,
        map_mode,
        perspective,
        day_night: request.day_night,
        allied_plans: request.allied_plans,
        counter_colors: request.counter_colors_country.map(|country| {
            if country {
                viewer_counter_kit::CounterColorMode::Country
            } else {
                viewer_counter_kit::CounterColorMode::Allegiance
            }
        }),
        nato_counters: request.nato_counters,
        visual_mods,
        allow_unverified: request.allow_unverified,
        window_dump: true,
        postfx: true,
        ..Options::default()
    };
    options.out = Some(PathBuf::from("video-export"));

    let session = VideoSession::new(
        schedule.into_cursor(),
        size,
        request.date_stamp,
        request.audio_enabled,
        Arc::clone(&cancel),
        progress,
        mode,
    );

    let speed = request.hours_per_second;
    let lost = Lost::default();
    let failure: Arc<Mutex<Option<String>>> = Arc::new(Mutex::new(None));
    let reported = Arc::clone(&failure);
    let res = host::driver::run_headless(size, &lost, cancel, move |ctx, state| {
        let max_dim = state.device.limits().max_texture_dimension_2d;
        if size[0] > max_dim || size[1] > max_dim {
            return Err(format!(
                "Requested {}x{} exceeds this graphics device’s maximum texture size of {} px",
                size[0], size[1], max_dim
            ));
        }
        let _generation = content_reader_layers::content::Generation::enter();
        let mut viewer = Viewer::new_for_video_export(
            ctx,
            Some(state),
            config,
            options,
            reported,
            Arc::clone(&replay),
            initial_revision.clone(),
            map_settings.clone(),
        );
        viewer.configure_video_export(session, speed, source_viewport, crop);
        Ok(viewer)
    });
    if let Err(error) = res {
        return Err(error);
    }
    if let Some(error) = failure.lock().unwrap_or_else(|e| e.into_inner()).take() {
        return Err(error);
    }
    Ok(())
}

pub(crate) fn apply_crop_to_placement(
    placement: Placement,
    map_size: MapSize,
    defines: viewer_gpu::camera::MapDefines,
    source_viewport: Option<[u32; 2]>,
    crop: CropRect,
) -> Placement {
    if crop.is_full() {
        return placement;
    }
    let Some(viewport) = source_viewport else {
        return placement;
    };
    let camera = placement.on(map_size, defines);
    cropped_placement(&camera, viewport, crop)
}

pub(crate) fn dispatch(args: &[std::ffi::OsString]) -> Option<io::Result<()>> {
    if !args.iter().any(|arg| arg == "--export-video") {
        return None;
    }
    Some(run_cli(args))
}

fn run_cli(args: &[std::ffi::OsString]) -> io::Result<()> {
    let mut iter = args.iter().cloned();
    let mut replay: Option<PathBuf> = None;
    let mut out: Option<PathBuf> = None;
    let mut branch: Option<BranchId> = None;
    let mut from_hour: Option<Hour> = None;
    let mut to_hour: Option<Hour> = None;
    let mut fps: u32 = 60;
    let mut speed: f64 = 24.0;
    let mut width: u32 = 1920;
    let mut height: u32 = 1080;
    let mut quality = Quality::High;
    let mut backend = BackendChoice::Auto;
    let mut ffmpeg_path: Option<String> = None;
    let mut skip_gaps = false;
    let mut date_stamp = true;
    let mut audio_enabled = false;
    let mut camera: Option<Placement> = None;
    let mut game_dir: Option<PathBuf> = None;
    let mut user_dir: Option<PathBuf> = None;
    let mut allow_unverified = false;
    let mut map_mode: Option<String> = None;
    let mut perspective: Option<String> = None;
    let mut fog = true;
    let mut day_night: Option<bool> = None;
    let invalid = |msg: &str| io::Error::new(io::ErrorKind::InvalidInput, msg);

    while let Some(arg) = iter.next() {
        let Some(flag) = arg.to_str() else {
            return Err(invalid("non-UTF-8 argument to --export-video"));
        };
        let next_str = |iter: &mut std::iter::Cloned<std::slice::Iter<'_, std::ffi::OsString>>, name: &str| -> io::Result<String> {
            iter.next()
                .and_then(|v| v.into_string().ok())
                .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidInput, format!("{name} requires a value")))
        };
        match flag {
            "--export-video" => {
                replay = Some(PathBuf::from(iter.next().ok_or_else(|| invalid("--export-video requires a replay path"))?));
            }
            "--out" => {
                out = Some(PathBuf::from(iter.next().ok_or_else(|| invalid("--out requires an output path"))?));
            }
            "--branch" => {
                branch = Some(next_str(&mut iter, "--branch")?.parse().map_err(|_| invalid("invalid --branch"))?);
            }
            "--from-hour" => {
                from_hour = Some(next_str(&mut iter, "--from-hour")?.parse().map_err(|_| invalid("invalid --from-hour"))?);
            }
            "--to-hour" => {
                to_hour = Some(next_str(&mut iter, "--to-hour")?.parse().map_err(|_| invalid("invalid --to-hour"))?);
            }
            "--fps" => {
                fps = next_str(&mut iter, "--fps")?.parse().map_err(|_| invalid("invalid --fps"))?;
            }
            "--speed" => {
                speed = next_str(&mut iter, "--speed")?.parse().map_err(|_| invalid("invalid --speed"))?;
            }
            "--width" => {
                width = next_str(&mut iter, "--width")?.parse().map_err(|_| invalid("invalid --width"))?;
            }
            "--height" => {
                height = next_str(&mut iter, "--height")?.parse().map_err(|_| invalid("invalid --height"))?;
            }
            "--quality" => {
                let raw = next_str(&mut iter, "--quality")?;
                quality = match raw.to_ascii_lowercase().as_str() {
                    "standard" => Quality::Standard,
                    "high" => Quality::High,
                    "max" | "maximum" => Quality::Maximum,
                    _ => return Err(invalid("--quality must be standard, high, or max")),
                };
            }
            "--backend" => {
                let raw = next_str(&mut iter, "--backend")?;
                backend = match raw.to_ascii_lowercase().as_str() {
                    "auto" => BackendChoice::Auto,
                    "media-foundation" | "mf" => BackendChoice::MediaFoundation,
                    "ffmpeg" => BackendChoice::Ffmpeg,
                    _ => return Err(invalid("--backend must be auto, media-foundation, or ffmpeg")),
                };
            }
            "--ffmpeg" => ffmpeg_path = Some(next_str(&mut iter, "--ffmpeg")?),
            "--skip-gaps" => skip_gaps = true,
            "--date-stamp" => {
                let raw = next_str(&mut iter, "--date-stamp")?;
                date_stamp = raw.eq_ignore_ascii_case("on") || raw.eq_ignore_ascii_case("true");
            }
            "--audio" => {
                let raw = next_str(&mut iter, "--audio")?;
                audio_enabled = raw.eq_ignore_ascii_case("on") || raw.eq_ignore_ascii_case("true");
            }
            "--camera" => {
                let raw = next_str(&mut iter, "--camera")?;
                camera = Some(crate::parse_placement(&raw)?);
            }
            "--map-mode" => map_mode = Some(next_str(&mut iter, "--map-mode")?),
            "--perspective" => perspective = Some(next_str(&mut iter, "--perspective")?),
            "--fog" => {
                let raw = next_str(&mut iter, "--fog")?;
                fog = raw.eq_ignore_ascii_case("on") || raw.eq_ignore_ascii_case("true");
            }
            "--day-night" => {
                let raw = next_str(&mut iter, "--day-night")?;
                day_night = Some(raw.eq_ignore_ascii_case("on") || raw.eq_ignore_ascii_case("true"));
            }
            "--game-dir" => game_dir = Some(PathBuf::from(iter.next().ok_or_else(|| invalid("--game-dir requires a path"))?)),
            "--user-dir" => user_dir = Some(PathBuf::from(iter.next().ok_or_else(|| invalid("--user-dir requires a path"))?)),
            "--allow-unverified-content" => allow_unverified = true,
            other => return Err(io::Error::new(io::ErrorKind::InvalidInput, format!("unknown --export-video option: {other}"))),
        }
    }

    let replay_path = replay.ok_or_else(|| invalid("--export-video requires a replay path"))?;
    let out_path = out.ok_or_else(|| invalid("--export-video requires --out <video.mp4>"))?;
    ensure_distinct_export_path(&replay_path, &out_path)
        .map_err(|msg| io::Error::new(io::ErrorKind::InvalidInput, msg))?;
    let canonical_in = std::fs::canonicalize(&replay_path)?;
    let mut config = Config::load().unwrap_or_default();
    if let Some(dir) = game_dir {
        config.game_dir = dir;
    }
    if let Some(dir) = user_dir {
        config.user_dir = dir;
    }

    let opened = replay::campaign::Replay::open_timelines(&canonical_in)?;
    let branch_id = branch.unwrap_or_else(|| {
        opened
            .timelines()
            .map(|t| t.main)
            .unwrap_or_else(|| opened.branches.first().map_or(0, |b| b.id))
    });
    let timeline = dashboard_query::timeline::resolve(&opened, branch_id)
        .map_err(|e| io::Error::other(e.to_string()))?;
    let first_default = timeline.stretches.first().map_or(0, |s| s.first);
    let last_default = timeline.stretches.last().map_or(first_default, |s| s.last);
    drop(opened);

    let req = MapVideoRequest {
        path: canonical_in.to_string_lossy().into_owned(),
        revision: None,
        branch: branch_id,
        from_hour: from_hour.unwrap_or(first_default),
        to_hour: to_hour.unwrap_or(last_default),
        fps,
        hours_per_second: speed,
        skip_gaps,
        cut_card_seconds: 1.0,
        width,
        height,
        quality,
        backend,
        ffmpeg_path,
        placement: camera.map(SavedPlacement::from),
        source_viewport: None,
        crop: None,
        ui_scale: Some(1.0),
        map_mode,
        perspective,
        perspective_slot: None,
        fog,
        day_night,
        allied_plans: None,
        counter_colors_country: None,
        nato_counters: None,
        map_settings: None,
        date_stamp,
        audio_enabled,
        allow_unverified,
    };

    let staged = replay_settings::exit::stage_path(&out_path)?;
    let cancel = Arc::new(AtomicBool::new(false));
    let outcome = run_map_video_export(config, req, staged.clone(), cancel, None);
    match outcome {
        MapVideoOutcome::Completed {
            staged_path,
            frames,
            audio_frames,
            backend,
            warnings,
        } => {
            let receipt = replay_settings::exit::EXIT
                .admit(replay_settings::exit::What::Export)
                .map_err(|e| io::Error::other(e.to_string()))?;
            let applied = receipt
                .replace_staged(&staged_path, &out_path)
                .map_err(|e| {
                    let _ = std::fs::remove_file(&staged_path);
                    io::Error::other(e.to_string())
                })?;
            if let Some(warning) = applied.warning {
                eprintln!("Warning: {warning}");
            }
            for warning in warnings {
                eprintln!("Warning: {warning}");
            }
            println!(
                "Exported {} frames ({} audio frames) via {} to {}",
                frames,
                audio_frames,
                backend,
                out_path.display()
            );
            Ok(())
        }
        MapVideoOutcome::Cancelled => Err(io::Error::new(io::ErrorKind::Interrupted, "Export cancelled")),
        MapVideoOutcome::Failed { error, partial, .. } => {
            if let Some(partial) = partial {
                let _ = std::fs::remove_file(partial.staged_path);
            }
            let _ = std::fs::remove_file(&staged);
            Err(io::Error::other(error))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn formats_hoi4_campaign_dates_from_engine_hour() {
        const START_1936: Hour = 43_800_000 + 1936 * 365 * 24;
        assert_eq!(format_campaign_date(START_1936 + 11), "01 Jan 1936 · 12:00");
        assert_eq!(format_campaign_date(START_1936 + 31 * 24 + 11), "01 Feb 1936 · 12:00");
        assert_eq!(format_campaign_date(START_1936 + 365 * 24), "01 Jan 1937 · 01:00");
        assert_eq!(format_campaign_date(0), "Hour 0");
    }

    #[test]
    fn proportional_ui_scale_keeps_counter_ratio_across_output_resolutions() {
        let s4k = proportional_ui_scale(1.0, 1080, 2160, 1.0);
        assert!((s4k - 2.0).abs() < 1e-4);
        let cropped = proportional_ui_scale(1.0, 1080, 1080, 0.5);
        assert!((cropped - 2.0).abs() < 1e-4);
    }

    #[test]
    fn crop_rect_normalizes_and_detects_full_frame() {
        assert!(CropRect::default().is_full());
        let clamped = CropRect { x: 0.9, y: -0.2, width: 0.5, height: 0.4 }.normalized();
        assert!((clamped.x - 0.5).abs() < 1e-5);
        assert!((clamped.y - 0.0).abs() < 1e-5);
        assert!(!clamped.is_full());
    }

    #[test]
    fn centered_free_crop_preserves_yaw_pitch_and_roll() {
        let mut free = viewer_gpu::camera::Free::new(0.42, -0.35, viewer_gpu::camera::Lens::MAP);
        free.roll = 0.15;
        let camera = Camera {
            center: [100.0, 200.0],
            height: 250.0,
            pose: Pose::Free(free),
            defines: viewer_gpu::camera::MapDefines::test(),
            map: MapSize::TEST,
        };
        let cropped = cropped_placement(
            &camera,
            [1920, 1080],
            CropRect { x: 0.25, y: 0.25, width: 0.5, height: 0.5 },
        );
        let Pose::Free(after) = cropped.pose else {
            panic!("expected Pose::Free");
        };
        assert!((after.yaw - free.yaw).abs() < 1e-4, "yaw drifted: {} vs {}", after.yaw, free.yaw);
        assert!((after.pitch - free.pitch).abs() < 1e-4, "pitch drifted: {} vs {}", after.pitch, free.pitch);
        assert!((after.roll - free.roll).abs() < 1e-5);
        assert!(after.lens.fov_y < free.lens.fov_y);
    }
}
