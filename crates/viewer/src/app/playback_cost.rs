use super::*;
use crate::playback_cost::{distribution, paced};
use job_priority::SpawnCarried as _;
use replay_frame::cpu;
use serde_json::{Value, json};
use std::{
    io,
    sync::atomic::{AtomicBool, Ordering},
};

mod extra_maps;
mod lean;
pub(crate) mod pictures;
pub(crate) mod toggle;
pub(crate) mod viewpoint;

#[derive(Clone)]
pub(crate) struct Schedule {
    pub step: u32,
    pub pan: Option<f32>,
    pub pngs: Vec<u32>,
    pub zoom: Option<Zoom>,
    pub paused: bool,
    pub refresh_hz: f64,
    pub complete_models: bool,
    pub paced: Option<f64>,
    pub seeks: u32,
    pub seek_back: Option<i32>,
    pub layers: Option<Vec<String>>,
    pub flight: Option<crate::playback_cost::flight::Flight>,
    pub windowed: bool,
    pub campaign_tab: bool,
    pub count_pictures: Option<pictures::Counting>,
    pub png_frames: Option<Presented>,
    pub vsync: Option<bool>,
    pub lean: bool,
}

#[derive(Clone, Copy)]
pub(crate) struct Presented {
    pub first: u64,
    pub frames: u64,
}

impl Presented {
    pub(crate) fn parse(value: &str) -> io::Result<Self> {
        value
            .split_once(':')
            .and_then(|(first, frames)| {
                Some(Self { first: first.trim().parse().ok()?, frames: frames.trim().parse().ok()? })
            })
            .filter(|presented| presented.frames > 0)
            .ok_or_else(|| {
                io::Error::new(
                    io::ErrorKind::InvalidInput,
                    "--png-frames needs <first frame saved>:<how many consecutive frames>",
                )
            })
    }

    pub(crate) fn saves(self, index: u64) -> bool {
        (self.first..self.first.saturating_add(self.frames)).contains(&index)
    }
}

const SEEK_BACK_SECONDS: f64 = 2.0;

fn camera_record(placement: Option<Placement>) -> Value {
    match placement {
        None => Value::Null,
        Some(Placement { center, height, pose: Pose::Map }) => json!({"pose": "map", "center": center, "height": height}),
        Some(Placement { center, height, pose: Pose::Free(free) }) => json!({
            "pose": "free", "eye": [center[0], height, center[1]],
            "yaw_deg": free.yaw.to_degrees(), "pitch_deg": free.pitch.to_degrees(),
            "fov_y_deg": free.lens.fov_y.to_degrees(), "far": free.lens.far,
        }),
    }
}

fn pinned_camera(viewer: &Viewer) -> Option<Placement> {
    if viewer.walk.placed() {
        return viewer.walk.eye();
    }
    viewer.capture.as_ref().and_then(|capture| capture.pinned).and_then(|(_, placement)| placement).or(viewer.camera)
}

fn view_record() -> Value {
    Value::Object(
        viewer_runtime::view_record::take()
            .into_iter()
            .map(|(section, counts)| {
                let counts: serde_json::Map<String, Value> =
                    counts.into_iter().map(|(name, n)| (name.to_owned(), json!(n))).collect();
                (section.to_owned(), Value::Object(counts))
            })
            .collect(),
    )
}


#[derive(Clone, Copy)]
pub(crate) struct Zoom {
    pub lines: f32,
    pub every: u32,
}

#[derive(Clone, Copy, Default)]
struct Memory {
    working_set_peak: u64,
    private_peak: u64,
    commit_peak: u64,
}

fn memory() -> Option<Memory> {
    #[cfg(windows)]
    {
        #[repr(C)]
        #[derive(Default)]
        struct Counters {
            size: u32,
            faults: u32,
            peak_working: usize,
            working: usize,
            peak_paged: usize,
            paged: usize,
            peak_nonpaged: usize,
            nonpaged: usize,
            pagefile: usize,
            peak_pagefile: usize,
            private: usize,
        }
        #[link(name = "psapi")]
        unsafe extern "system" {
            fn GetProcessMemoryInfo(
                process: windows_sys::Win32::Foundation::HANDLE,
                counters: *mut std::ffi::c_void,
                size: u32,
            ) -> i32;
        }
        let mut counters = Counters {
            size: std::mem::size_of::<Counters>() as u32,
            ..Default::default()
        };
        let ok = unsafe {
            GetProcessMemoryInfo(
                windows_sys::Win32::System::Threading::GetCurrentProcess(),
                (&raw mut counters).cast(),
                std::mem::size_of::<Counters>() as u32,
            )
        };
        (ok != 0).then_some(Memory {
            working_set_peak: counters.peak_working as u64,
            private_peak: counters.private as u64,
            commit_peak: counters.peak_pagefile as u64,
        })
    }
    #[cfg(not(windows))]
    None
}

pub(crate) struct Monitor {
    stop: Arc<AtomicBool>,
    thread: Option<std::thread::JoinHandle<Option<Memory>>>,
}
impl Monitor {
    pub(crate) fn start() -> io::Result<Self> {
        let stop = Arc::new(AtomicBool::new(false));
        let stopped = Arc::clone(&stop);
        let thread = std::thread::Builder::new()
            .name("playback cost memory".into())
            .spawn_carried(move || {
                let _cost = cpu::scope("viewer", Some("other"));
                let mut peak = memory()?;
                loop {
                    let now = memory()?;
                    peak.working_set_peak = peak.working_set_peak.max(now.working_set_peak);
                    peak.private_peak = peak.private_peak.max(now.private_peak);
                    peak.commit_peak = peak.commit_peak.max(now.commit_peak);
                    if stopped.load(Ordering::Acquire) {
                        break;
                    }
                    std::thread::sleep(Duration::from_millis(10));
                }
                Some(peak)
            })?;
        Ok(Self {
            stop,
            thread: Some(thread),
        })
    }
    fn finish(&mut self) -> Option<Memory> {
        self.stop.store(true, Ordering::Release);
        self.thread.take()?.join().ok().flatten()
    }
}
impl Drop for Monitor {
    fn drop(&mut self) {
        self.finish();
    }
}

fn process_cycles() -> Option<u64> {
    #[cfg(windows)]
    {
        use windows_sys::Win32::System::Threading::GetCurrentProcess;
        #[link(name = "kernel32")]
        unsafe extern "system" {
            fn QueryProcessCycleTime(
                process: windows_sys::Win32::Foundation::HANDLE,
                cycles: *mut u64,
            ) -> i32;
        }
        let mut cycles = 0;
        (unsafe { QueryProcessCycleTime(GetCurrentProcess(), &raw mut cycles) } != 0).then_some(cycles)
    }
    #[cfg(not(windows))]
    None
}

fn copy_of_output(
    device: &wgpu::Device,
    queue: &wgpu::Queue,
    targets: &viewer_gpu::gpu_contract::Targets,
) -> wgpu::Texture {
    let output = targets.output();
    let kept = device.create_texture(&wgpu::TextureDescriptor {
        label: Some("saved display frame"),
        size: output.size(),
        mip_level_count: 1,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format: output.format(),
        usage: wgpu::TextureUsages::COPY_DST | wgpu::TextureUsages::COPY_SRC,
        view_formats: &[],
    });
    let mut encoder = device.create_command_encoder(&wgpu::CommandEncoderDescriptor { label: Some("save display frame") });
    encoder.copy_texture_to_texture(output.as_image_copy(), kept.as_image_copy(), output.size());
    queue.submit([encoder.finish()]);
    kept
}

fn drain(state: &eframe::egui_wgpu::RenderState) -> io::Result<()> {
    state
        .device
        .poll(wgpu::PollType::Wait {
            submission_index: None,
            timeout: None,
        })
        .map(|_| ())
        .map_err(io::Error::other)
}

fn wait_until(due: Instant) -> Option<Instant> {
    let mut woke = None;
    while let Some(left) = due.checked_duration_since(Instant::now()) {
        if left > Duration::from_millis(1) {
            precise_wait(left - Duration::from_millis(1));
            woke = Some(Instant::now());
        } else {
            std::hint::spin_loop();
        }
    }
    woke
}

fn precise_wait(left: Duration) {
    #[cfg(windows)]
    {
        use std::ffi::c_void;
        const HIGH_RESOLUTION: u32 = 0x2;
        const TIMER_ALL_ACCESS: u32 = 0x1F_0003;
        const INFINITE: u32 = 0xFFFF_FFFF;
        unsafe extern "system" {
            fn CreateWaitableTimerExW(attributes: *mut c_void, name: *const u16, flags: u32, access: u32) -> *mut c_void;
            fn SetWaitableTimer(timer: *mut c_void, due: *const i64, period: i32, routine: *mut c_void, argument: *mut c_void, resume: i32) -> i32;
            fn WaitForSingleObject(handle: *mut c_void, milliseconds: u32) -> u32;
        }
        thread_local! {
            static TIMER: *mut c_void = {
                // SAFETY: a nameless, uninherited timer with no attributes; the handle is kept for
                // this thread's life and only ever set and waited on.
                let timer = unsafe { CreateWaitableTimerExW(std::ptr::null_mut(), std::ptr::null(), HIGH_RESOLUTION, TIMER_ALL_ACCESS) };
                timer
            };
        }
        let waited = TIMER.with(|&timer| {
            if timer.is_null() {
                return false;
            }
            let due = -((left.as_nanos() / 100).min(i64::MAX as u128) as i64);
            unsafe {
                if SetWaitableTimer(timer, &raw const due, 0, std::ptr::null_mut(), std::ptr::null_mut(), 0) == 0 {
                    return false;
                }
                WaitForSingleObject(timer, INFINITE);
            }
            true
        });
        if waited {
            return;
        }
    }
    std::thread::sleep(left);
}

pub(crate) fn hold_at_step(entry: &crate::playhead_host::Entry, start: replay_data::Hour, speed: f64) {
    entry.anchor(start, true, speed, false);
    entry.anchor(start, false, speed, false);
}

fn warm_up_state(side: &crate::playhead_host::MapSide) -> String {
    let on = side.on_screen();
    let shown: Vec<_> = side.layer_states().into_iter().zip(&side.last_frame().layers).map(|((name, drawn, upload_left), part)| (name, drawn, part.shown, upload_left)).collect();
    format!("the head at {:?}, composing {:?}, the layer furthest behind {:?}; name, drawn, shown, upload left: {shown:?}", on.as_ref().map(|on| on.head), on.as_ref().map(|on| on.composing), on.as_ref().and_then(|on| on.furthest))
}

#[expect(clippy::too_many_arguments, reason = "one window frame's whole input")]
fn frame_ui(
    ctx: &egui::Context,
    pixels: [u32; 2],
    time: f64,
    focused: bool,
    events: Vec<egui::Event>,
    viewer: &mut Viewer,
    state: &eframe::egui_wgpu::RenderState,
    document: &mut Document,
) -> Result<egui::FullOutput, String> {
    let mut result = Ok(());
    let output = ctx.run_ui(
        egui::RawInput {
            screen_rect: Some(egui::Rect::from_min_size(
                egui::Pos2::ZERO,
                egui::vec2(pixels[0] as f32, pixels[1] as f32),
            )),
            time: Some(time),
            focused,
            events,
            ..Default::default()
        },
        |ui| {
            result = if viewer.campaign_tab {
                let drawn = viewer.window_frame(ui, Some(state), document);
                match &viewer.error {
                    Some(error) if !drawn => Err(error.clone()),
                    _ => Ok(()),
                }
            } else {
                viewer.map_with_state(ui, state, document, None)
            };
        },
    );
    result.map(|()| output)
}

fn absorb_textures(
    surface: Option<&mut crate::renderer_host::WindowSurface>,
    state: &eframe::egui_wgpu::RenderState,
    output: egui::FullOutput,
) -> io::Result<()> {
    let Some(surface) = surface else {
        return Ok(());
    };
    let mut textures = output.textures_delta;
    surface
        .paint(
            state,
            state.surface_config,
            None,
            output.pixels_per_point,
            [0.0; 4],
            &[],
            &mut textures,
        )
        .map(|_| ())
        .map_err(io::Error::other)
}

fn gpu_counts() -> serde_json::Map<String, Value> {
    let passes: serde_json::Map<String, Value> = viewer_runtime::profiling::take_pass_counts()
        .into_iter()
        .map(|(lap, counts)| {
            let counts: serde_json::Map<String, Value> =
                counts.into_iter().map(|(name, n)| (name.to_string(), json!(n))).collect();
            (lap.to_string(), Value::Object(counts))
        })
        .collect();
    shader_pipeline::gpu::stats::NAMES
        .iter()
        .zip(shader_pipeline::gpu::stats::take())
        .map(|(name, count)| (name.to_string(), json!(count)))
        .chain(
            viewer_models::take_counts()
                .map(|(name, count)| (name.to_string(), json!(count))),
        )
        .chain(std::iter::once(("passes".to_string(), Value::Object(passes))))
        .chain(std::iter::once((
            "refusals".to_string(),
            Value::Object(
                viewer_models::take_refusals()
                    .into_iter()
                    .map(|(name, count)| (name, json!(count)))
                    .collect(),
            ),
        )))
        .collect()
}

struct Records {
    records: Option<std::sync::mpsc::Sender<Value>>,
    writer: Option<std::thread::JoinHandle<io::Result<()>>>,
}

impl Records {
    fn open(path: &std::path::Path) -> io::Result<Self> {
        let file = std::fs::File::create_new(path)?;
        let (sender, records) = std::sync::mpsc::channel::<Value>();
        let writer = std::thread::Builder::new()
            .name("harness records".to_owned())
            .spawn(move || {
                let mut file = io::BufWriter::new(file);
                for record in records {
                    let _cost = cpu::scope("frame.record", Some("other"));
                    serde_json::to_writer(&mut file, &record)?;
                    io::Write::write_all(&mut file, b"\n")?;
                }
                io::Write::flush(&mut file)
            })?;
        Ok(Self {
            records: Some(sender),
            writer: Some(writer),
        })
    }

    fn write(&mut self, record: Value) -> io::Result<()> {
        if let Some(records) = &self.records
            && records.send(record).is_ok()
        {
            return Ok(());
        }
        self.finish()?;
        Err(io::Error::other("the run's record writer stopped"))
    }

    fn finish(&mut self) -> io::Result<()> {
        drop(self.records.take());
        match self.writer.take() {
            Some(writer) => writer
                .join()
                .map_err(|_| io::Error::other("the run's record writer panicked"))?,
            None => Ok(()),
        }
    }
}

impl Drop for Records {
    fn drop(&mut self) {
        let _ = self.finish();
    }
}

fn frame_scopes(
    before: &mut std::collections::HashMap<String, [Option<u64>; 3]>,
) -> (serde_json::Map<String, Value>, serde_json::Map<String, Value>, serde_json::Map<String, Value>) {
    let after = cpu::own_totals();
    let (mut scopes, mut walls, mut entered) = (serde_json::Map::new(), serde_json::Map::new(), serde_json::Map::new());
    for (label, [ns, wall, calls]) in &after {
        let [was_ns, was_wall, was_calls] = before.get(label).copied().unwrap_or([None, None, None]);
        let calls = calls.unwrap_or(0).saturating_sub(was_calls.unwrap_or(0));
        if calls > 0 {
            entered.insert(label.clone(), json!(calls));
        }
        let spent = ns.unwrap_or(0).saturating_sub(was_ns.unwrap_or(0));
        if spent > 0 {
            scopes.insert(label.clone(), json!(spent / 1000));
        }
        if let Some(wall) = wall {
            let waited = wall.saturating_sub(was_wall.unwrap_or(0));
            if waited > 0 {
                walls.insert(label.clone(), json!(waited / 1000));
            }
        }
    }
    *before = after;
    (scopes, walls, entered)
}

#[derive(Default)]
struct GpuPeaks {
    allocated: Option<u64>,
    reserved: Option<u64>,
}
impl GpuPeaks {
    fn sample(&mut self, state: &eframe::egui_wgpu::RenderState) {
        if let Some(report) = state.device.generate_allocator_report() {
            self.allocated = Some(
                self.allocated
                    .unwrap_or(0)
                    .max(report.total_allocated_bytes),
            );
            self.reserved = Some(self.reserved.unwrap_or(0).max(report.total_reserved_bytes));
        }
    }
}

fn cycles_us(before: Option<u64>, hz: Option<f64>) -> Option<f64> {
    before
        .zip(process_cycles())
        .and_then(|(a, b)| b.checked_sub(a))
        .zip(hz)
        .map(|(cycles, hz)| cycles as f64 / hz * 1e6)
}

fn thread_us(before: Option<Duration>) -> Option<f64> {
    before
        .zip(viewer_runtime::profiling::current_thread_cpu_time())
        .and_then(|(a, b)| b.checked_sub(a))
        .map(|used| used.as_secs_f64() * 1e6)
}

fn window_state(
    window: &crate::host::Window,
) -> io::Result<(
    eframe::egui_wgpu::RenderState,
    crate::renderer_host::WindowSurface,
)> {
    let (state, surface) = crate::host::driver::map_device(Some(window)).map_err(io::Error::other)?;
    let surface = surface.ok_or_else(|| io::Error::other("The map’s window gave no surface"))?;
    crate::renderer_host::bind(&state);
    Ok((state, crate::renderer_host::WindowSurface::new(surface)))
}

#[expect(clippy::await_holding_lock, reason = "block_on polls this future to completion on this thread, and wgpu's native adapter and device futures resolve without another thread taking the lock, which must span device creation")]
fn render_state() -> io::Result<eframe::egui_wgpu::RenderState> {
    pollster::block_on(async {
        let lifecycle = shader_pipeline::gpu::device_lifecycle();
        let instance = shader_pipeline::gpu::instance();
        let available_adapters = instance.enumerate_adapters(wgpu::Backends::all()).await;
        let adapter =
            viewer_gpu::gpu_contract::select_adapter(&available_adapters, None)?;
        let (device, queue) = adapter
            .request_device(&viewer_gpu::gpu_contract::map_device_descriptor(&adapter))
            .await
            .map_err(io::Error::other)?;
        drop(lifecycle);
        let target_format = viewer_gpu::gpu_contract::OUTPUT_FORMAT;
        let renderer = eframe::egui_wgpu::Renderer::new(&device, target_format, Default::default());
        Ok::<_, io::Error>(eframe::egui_wgpu::RenderState {
            instance,
            adapter,
            available_adapters,
            device,
            queue,
            target_format,
            renderer: Arc::new(egui::mutex::RwLock::new(renderer)),
            surface_config: eframe::egui_wgpu::WgpuConfiguration::default().surface,
        })
    })
}

#[expect(clippy::too_many_arguments, reason = "one run's inputs, as the harness parsed them")]
pub(crate) fn measure(
    config: Config,
    mut options: Options,
    mut document: Document,
    hours: u32,
    schedule: Schedule,
    mut monitor: Monitor,
    output: &std::path::Path,
    window: crate::playback_cost::Window,
) -> io::Result<Value> {
    viewer_runtime::worker_priority::frame_priority();
    if !schedule.lean {
        viewer_layer_core::layers::count_in_view(true);
        viewer_runtime::view_record::enable();
    }
    let (window, inputs) = match window {
        crate::playback_cost::Window::Own(window, inputs) => (Some(window), Some(inputs)),
        crate::playback_cost::Window::None => (None, None),
    };
    let _inputs = inputs;
    let (state, mut surface) = match &window {
        Some(window) => {
            let (mut state, surface) = window_state(window)?;
            if let Some(vsync) = schedule.vsync {
                state.surface_config.present_mode =
                    if vsync { wgpu::PresentMode::AutoVsync } else { wgpu::PresentMode::AutoNoVsync };
            }
            (state, Some(surface))
        }
        None => (render_state()?, None),
    };
    let info = state.adapter.get_info();
    document.icons.lazy.set_background(schedule.paced.is_some());
    if !document.map_modes.knows(&options.map_mode) {
        return Err(io::Error::other("Unknown --map-mode"));
    }
    let branch = options.branch.unwrap_or(document.playback.branch());
    let start = options.hour.unwrap();
    let pixels = options.size;
    let camera = options.camera.map_or_else(
        || Camera::fit(MapSize::of(&document.map), document.map_defines, pixels),
        |placement| placement.on(MapSize::of(&document.map), document.map_defines),
    );
    camera.validate()?;
    options.window_dump = false;
    options.headless_dump = false;
    let mut viewer = Viewer::create(
        config,
        options.clone(),
        None,
        Arc::new(Mutex::new(None)),
        None,
        Some((state.device.clone(), state.queue.clone())),
    );
    viewer.campaign_tab = schedule.campaign_tab;
    viewer.graphics.full_resolution = document.game_full_resolution;
    viewer.take_game_options(&document.game_options);
    viewer.scene = Some(load_scene(
        &state.device,
        &state.queue,
        &document,
        viewer.graphics,
        &Default::default(),
    )?);
    viewer.scene.as_mut().unwrap().set_graphics(viewer.graphics);
    if let Some(names) = &schedule.layers {
        crate::playhead_host::register_only(names.clone());
    }
    let at = crate::playhead_host::At { branch, hour: start, playing: false, speed: schedule.paced.unwrap_or(0.0) };
    let side = viewer_playback::map_open::for_document_at(&document, Some(at)).map_err(io::Error::other)?;
    viewer.playhead_map = Some(viewer_playback::side::SideThread::new(side, Arc::new(|| {})).map_err(io::Error::other)?);
    if let Some(side) = &viewer.playhead_map {
        document.playback.read_rosters_of(side.entry());
    }
    viewer.playhead_opened = true;
    viewer.dashboard_map = extra_maps::as_dashboard_map();
    let mut extras = extra_maps::Extras::open(&document, viewer.playhead_map.as_ref(), schedule.paced.unwrap_or(1440.0))?;
    let hovered = super::hover_rows::Family::from_env()?;
    if hovered.is_some() {
        viewer.hover_rows.time();
    }
    let scene = viewer.scene.as_mut().unwrap();
    scene.settle_builds(schedule.complete_models);
    scene.settle_poses(schedule.complete_models);
    viewer.playing = !schedule.paused;
    viewer.speed = schedule.paced.unwrap_or(1440.0);
    viewer.animation_seek = true;
    viewer.camera_step = Some(1.0 / schedule.refresh_hz as f32);
    viewer.last_view = Some(crate::app::View {
        camera,
        viewport: pixels,
        ui_scale: options.ui_scale.unwrap_or(1.0),
    });
    viewer.capture = Some(Capture {
        path: PathBuf::new(),
        remaining: u32::MAX,
        pinned: schedule.zoom.is_none().then_some((pixels, options.camera)),
        exit: false,
        time: Some(0.0),
        step: 0.0,
        frames: 1,
        index: 0,
        paced: schedule.paced.is_some(),
        export: None,
        video: None,
    });
    let ctx = egui::Context::default();
    ctx.set_theme(egui::Theme::Dark);
    viewer_runtime::profiling::cpu_clock_hz();
    drain(&state)?;
    viewer.playhead = Some(Playhead { branch, hour: start, fraction: 0.0 });
    let warming = Instant::now();
    viewer.warming = true;
    while !viewer.picture_ready() {
        let output = frame_ui(&ctx, pixels, 0.0, false, Vec::new(), &mut viewer, &state, &mut document).map_err(io::Error::other)?;
        absorb_textures(surface.as_mut(), &state, output)?;
        drain(&state)?;
        if let Some(failure) = viewer.playhead_map.as_ref().and_then(|side| side.failure()) {
            return Err(io::Error::other(format!("The map's layers drew no first picture: {failure}")));
        }
        if warming.elapsed() > Duration::from_secs(300) {
            return Err(io::Error::other(format!("The map's layers drew no first picture in five minutes: {}", viewer.playhead_map.as_ref().map_or_else(String::new, |side| side.with_side(warm_up_state)))));
        }
    }
    viewer.warming = false;
    if schedule.lean {
        let (Some(surface), Some(window)) = (surface.as_mut(), window.as_ref()) else {
            return Err(io::Error::other("A lean run presents through its window"));
        };
        let end = options.playback_cost_end.expect("the harness sets the run's end");
        let last = document.playback.range(branch).map_or(end, |(_, last)| last);
        let mut report = lean::run(lean::Run {
            ctx: &ctx, viewer: &mut viewer, document: &mut document, state: &state, surface, window,
            schedule: &schedule, pixels, branch, start, end, last, hours,
        })?;
        report["memory"] = json!(monitor.finish().map(|memory| memory.working_set_peak));
        return Ok(report);
    }
    let mut pictures = None::<pictures::Pictures>;
    drop(cpu::scope("viewer", Some("other")));
    let before = cpu::snapshot();
    let process_start =
        cpu::process_cpu_ns().ok_or_else(|| io::Error::other("Process CPU unavailable"))?;
    let cycle_start = process_cycles();
    let cycle_hz = before["tscHz"].as_f64().filter(|hz| *hz > 0.0);
    let wall = Instant::now();
    let mut frame_cpu = Vec::new();
    let mut frame_thread_cpu = Vec::new();
    let mut frame_wall = Vec::new();
    let mut gpu = GpuPeaks::default();
    let mut warning_frames = 0u32;
    let mut frames = Records::open(&output.join("frames.jsonl"))?;
    let mut labels_before = cpu::own_totals();
    let mut reads_before = cpu::cycle_reads();
    shader_pipeline::gpu::stats::mark_window_thread();
    shader_pipeline::gpu::stats::take();
    viewer_runtime::view_record::enable();
    viewer_runtime::view_record::take();
    let map_width = MapSize::of(&document.map).x();
    if schedule.zoom.is_some() {
        viewer.camera = Some(camera.placement());
        viewer.camera_manual = true;
    }
    let mut zoom_in = true;
    let mut burst_height = None::<f32>;
    let center = egui::pos2(pixels[0] as f32 * 0.5, pixels[1] as f32 * 0.5);
    let mut paced_frames = Vec::new();
    let mut layered = Layered::default();
    if let Some(side) = &viewer.playhead_map {
        let rate = crate::playhead_host::settings().rate;
        match schedule.paced {
            Some(speed) => side.anchor(start, !schedule.paused, speed, false),
            None => hold_at_step(side.entry(), start, f64::from(schedule.step.max(1)) * rate),
        }
    }
    let mut sought = 0u32;
    let last = document.playback.range(branch).map_or(options.playback_cost_end.expect("the harness sets the run's end"), |(_, last)| last);
    let mut sought_back = None::<f64>;
    let mut seed = 0x2545_f491_4f6c_dd1d_u64;
    let mut paced_end_us = 0.0;
    let mut harness_wait_us = 0.0;
    if let Some(speed) = schedule.paced {
        let end = options
            .playback_cost_end
            .expect("the harness sets the run's end");
        let mut drawn = None::<i32>;
        let mut previous_start_us = None::<f64>;
        let mut reached_end = false;
        let seek_every = (f64::from(end - start) / speed * schedule.refresh_hz / f64::from(schedule.seeks + 1)).max(15.0) as u64;
        let t0 = Instant::now();
        let saved_to = output;
        let mut saved_frames: Vec<(u64, wgpu::Texture)> = Vec::new();
        for index in 0u64.. {
            let due_us = paced::due_us(index, schedule.refresh_hz);
            let reached_us = t0.elapsed().as_secs_f64() * 1e6;
            let late = paced::late_start(index, due_us, reached_us);
            let mut woke_us = None;
            if !late && index > 0 {
                let waiting = viewer_runtime::profiling::current_thread_cpu_time();
                woke_us = wait_until(crate::playback_cost::frame_due(t0, due_us / 1e6)?)
                    .map(|woke| woke.duration_since(t0).as_secs_f64() * 1e6);
                harness_wait_us += thread_us(waiting).unwrap_or(0.0);
            }
            let frame_start = if index == 0 { t0 } else { Instant::now() };
            let start_us = frame_start.duration_since(t0).as_secs_f64() * 1e6;
            let interval_us = previous_start_us.replace(start_us).map(|before| start_us - before);
            let harness_cost = cpu::scope("frame.harness", Some("other"));
            if let Some(back) = schedule.seek_back
                && sought_back.is_none()
                && reached_end
            {
                viewer.anchor_playback(Playhead { branch, hour: back, fraction: 0.0 }, true, speed, true, None, Instant::now(), last);
                layered.seek(index, back, start_us);
                sought_back = Some(paced::seek_back_end(back, speed, SEEK_BACK_SECONDS, last));
            }
            let head = match &viewer.playhead_map {
                _ if schedule.paused => f64::from(start),
                Some(side) => f64::from(side.head(Instant::now())),
                None => extras.head(Instant::now()).map_or_else(|| paced::head(start, speed, start_us / 1e6, last), f64::from),
            };
            if let Some(pan) = schedule.pan {
                let mut placement = camera.placement();
                placement.center[0] = (placement.center[0] + pan * index as f32).rem_euclid(map_width);
                viewer.capture.as_mut().unwrap().pinned = Some((pixels, Some(placement)));
            }
            let flying = schedule.flight.map(|flight| flight.at(due_us / 1e6, map_width));
            if let Some((_, placement)) = flying {
                viewer.capture.as_mut().unwrap().pinned = Some((pixels, Some(placement)));
            }
            if let Some(side) = &viewer.playhead_map
                && schedule.seeks > 0
                && sought < schedule.seeks
                && index > 0
                && index % seek_every == 0
                && layered.ready_to_seek()
            {
                seed = seed.wrapping_mul(6_364_136_223_846_793_005).wrapping_add(1_442_695_040_888_963_407);
                let span = (end - start).max(1) as u64 / 2;
                let target = start + ((seed >> 33) % span) as i32;
                side.anchor(target, true, speed, true);
                layered.seek(index, target, start_us);
                sought += 1;
            }
            let ends = sought_back.unwrap_or(f64::from(end));
            if schedule.seek_back.is_some() && sought_back.is_none() && head >= ends {
                reached_end = true;
            } else if (schedule.paused && index >= u64::from(hours)) || (!schedule.paused && head >= ends) {
                paced_end_us = start_us;
                break;
            }
            let (hour, fraction) = paced::split(head);
            viewer.playhead = Some(Playhead {
                branch,
                hour,
                fraction,
            });
            viewer.capture.as_mut().unwrap().time = Some((start_us / 1e6) as f32);
            if extras.main_hidden(index) {
                viewer.close_playhead();
                let extra = extras.frame(&document, None, index, crate::playhead_host::Head::Paced(hour));
                frames.write(json!({"frame": index, "hour": hour, "head_hour": head, "covered": true, "extra": extra}))?;
                continue;
            }
            if let Some(family) = hovered
                && viewer.options.profile_pointer.is_none()
            {
                viewer.options.profile_pointer = viewer.hover_target(family, pixels);
            }
            let process_before = process_cycles();
            crate::perf_log::frame_start();
            let perf_frame = crate::perf_log::frame_id();
            drop(harness_cost);
            let thread_before = viewer_runtime::profiling::current_thread_cpu_time();
            let product_start = Instant::now();
            let product_cost = cpu::scope("frame.product", Some("other"));
            let result = frame_ui(
                &ctx,
                pixels,
                start_us / 1e6,
                false,
                Vec::new(),
                &mut viewer,
                &state,
                &mut document,
            );
            let submitted = Instant::now();
            let thread = thread_us(thread_before);
            drop(product_cost);
            let other_cost = cpu::scope("frame.harness", Some("other"));
            let output = result.map_err(io::Error::other)?;
            let extra = extras.frame(&document, viewer.playhead_map.as_ref(), index, crate::playhead_host::Head::Paced(hour));
            let acquire_us = match (&mut surface, &window) {
                (Some(surface), Some(window)) => {
                    let _present = cpu::scope("frame.present", Some("other"));
                    let acquire = lean::present(surface, window, &state, &viewer, &ctx, output, pixels)?.acquire;
                    Some(f64::from(acquire) * 1e6)
                }
                (surface, _) => {
                    absorb_textures(surface.as_mut(), &state, output)?;
                    drain(&state)?;
                    None
                }
            };
            let drained = Instant::now();
            if let Some(counting) = schedule.count_pictures
                && counting.counts(index)
                && let Some(targets) = viewer
                    .scene
                    .as_ref()
                    .and_then(viewer_world::scene::Scene::targets)
            {
                let pictures = match &mut pictures {
                    Some(pictures) => pictures,
                    none => none.insert(pictures::Pictures::new(
                        &state.device,
                        &state.queue,
                        targets,
                        counting,
                    )?),
                };
                pictures.count(&state.device, &state.queue, targets, index);
            }
            if schedule.png_frames.is_some_and(|presented| presented.saves(index))
                && let Some(targets) = viewer.scene.as_ref().and_then(viewer_world::scene::Scene::targets)
            {
                saved_frames.push((index, copy_of_output(&state.device, &state.queue, targets)));
            }
            crate::perf_log::gpu_times(
                viewer
                    .scene
                    .as_mut()
                    .unwrap()
                    .gpu_frame_times(&state.device),
            );
            crate::perf_log::frame_end();
            let wall_us = drained.duration_since(frame_start).as_secs_f64() * 1e6;
            frame_wall.push(wall_us);
            frame_cpu.extend(cycles_us(process_before, cycle_hz));
            frame_thread_cpu.extend(thread);
            gpu.sample(&state);
            warning_frames += u32::from(!viewer.map_warnings.is_empty());
            drop(other_cost);
            let now_drawn = viewer
                .rendered
                .filter(|rendered| viewer.drew_frame && rendered.branch == branch)
                .map(|rendered| rendered.hour);
            if now_drawn.is_none()
                && let Some(previous) = drawn
            {
                return Err(io::Error::other(format!(
                    "Frame {index} drew nothing after hour {previous} was on screen"
                )));
            }
            layered.overlay = viewer.overlay_hour().or(layered.overlay);
            let oldest = viewer.playhead_map.as_ref().and_then(|side| side.last_frame().oldest);
            let made = viewer.playhead_map.as_ref().map(|side| side.last_frame().made);
            let frame = paced::Frame {
                due_us,
                start_us,
                work_us: submitted.duration_since(product_start).as_secs_f64() * 1e6,
                gpu_wait_us: drained.duration_since(submitted).as_secs_f64() * 1e6,
                interval_us,
                acquire_us,
                thread_us: thread,
                head_hour: head,
                drawn_hour: now_drawn,
                oldest_hour: now_drawn.map(|drawn| oldest.unwrap_or(drawn)),
                overlay_hour: layered.overlay,
                made,
                late_start: late,
                new_hour: paced::new_hour(drawn, now_drawn),
            };
            drawn = now_drawn.or(drawn);
            let layers = viewer.playhead_map.as_ref().map(|side| {
                let (models, meshes) = side
                    .try_with_side(|side| {
                        if index % 30 == 0 {
                            layered.sample_held(side);
                        }
                        let units = viewer_layers_forces::units::of(side);
                        (units.and_then(viewer_layers_forces::units::Units::take_model_counts), units.and_then(viewer_layers_forces::units::Units::take_mesh_counts))
                    })
                    .unwrap_or_default();
                let phases = side.installed().find(|(name, ..)| *name == "political").map(|(_, _, wrote)| wrote.phases);
                let installs: Vec<Value> = side
                    .installed()
                    .map(|(name, shown, wrote)| json!({"name": name, "shown": shown, "us": wrote.ns / 1000, "bytes": wrote.bytes, "writes": wrote.writes}))
                    .collect();
                layered.political = side.names().iter().position(|&name| name == "political");
                let mut record = layered.frame(side.last_frame(), head, cycle_hz, (index, start_us + wall_us), side.entry().playhead().newest(), phases);
                record["side_missed"] = json!(side.products().missed);
                record["installs"] = json!(installs);
                let (kept, _, counters) = side.entry().playhead().held();
                record["ring_steps"] = json!(kept.ring_steps);
                record["room_held_ms"] = json!(counters.room_held_ns as f64 / 1e6);
                record["join_held_ms"] = json!(counters.join_held_ns as f64 / 1e6);
                if let Some(models) = models {
                    let parts: serde_json::Map<String, Value> = models.parts.iter().map(|&(name, ns)| (name.to_owned(), json!(ns / 1000))).collect();
                    record["unit_models"] = json!({"jobs": models.jobs, "busy_us": models.busy_ns / 1000, "parts_us": parts});
                }
                if let Some(meshes) = meshes {
                    let parts: serde_json::Map<String, Value> = meshes.parts.iter().map(|&(name, ns)| (name.to_owned(), json!(ns / 1000))).collect();
                    record["unit_meshes"] = json!({"builds": meshes.builds, "products": meshes.products, "chosen": meshes.chosen, "selections": meshes.selections, "looked_up": meshes.looked_up, "entering": meshes.entering, "leaving": meshes.leaving, "waiting": meshes.waiting, "keys": meshes.keys, "keyed_rows": meshes.keyed_rows, "camouflages": meshes.camouflages, "contexts": meshes.contexts, "busy_us": meshes.busy_ns / 1000, "parts_us": parts});
                }
                record
            });
            let counts = gpu_counts();
            let (scopes, scope_walls, scope_calls) = frame_scopes(&mut labels_before);
            let cycle_reads = cpu::cycle_reads() - std::mem::replace(&mut reads_before, cpu::cycle_reads());
            let mut icon_pools = serde_json::Map::new();
            let mut icon_pools_why = serde_json::Map::new();
            for (name, counted, whys) in viewer_native_gui::icon_windows::take_counts() {
                if !whys.is_empty() {
                    let whys: serde_json::Map<String, Value> = whys.into_iter().map(|(why, count)| (why.to_owned(), json!(count))).collect();
                    icon_pools_why.insert(name.clone(), Value::Object(whys));
                }
                icon_pools.insert(name, json!(counted));
            }
            let overlay_redraws: serde_json::Map<String, Value> = viewer_runtime::redraws::take()
                .into_iter()
                .map(|(name, count)| (name.to_owned(), json!(count)))
                .collect();
            frames.write(json!({"frame":index,"perf_frame":perf_frame,"hour":hour,"thread_us":thread,
                    "icon_pools":icon_pools,"icon_pools_why":icon_pools_why,"overlay_redraws":overlay_redraws,
                    "wall_us":wall_us,
                    "camera_height":viewer.camera.map(|placement| placement.height),
                    "camera":camera_record(pinned_camera(&viewer)),
                    "flight":flying.map(|(leg, _)| leg),"view":view_record(),
                    "gpu":counts,"scopes_us":scopes,"scopes_wall_us":scope_walls,"scopes_calls":scope_calls,"cycle_reads":cycle_reads,
                    "due_us":due_us,"start_us":start_us,"work_us":frame.work_us,
                    "interval_us":interval_us,"acquire_us":acquire_us,
                    "gpu_wait_us":frame.gpu_wait_us,"head_hour":head,"drawn_hour":now_drawn,
                    "lag_hours":frame.lag_hours(),"head_behind_the_clock_hours":frame.clock_lag_hours(),"oldest_hour":frame.oldest_hour,"overlay_hour":frame.overlay_hour,"overlay_behind_hours":frame.overlay_behind_hours(),"new_hour":frame.new_hour,"late_start":late,
                    "reached_us":reached_us,"woke_us":woke_us,"layers":layers,"extra":extra,
                    "hover":viewer.hover_rows.take_timed()}))?;
            if index % 600 == 0 {
                println!(
                    "Playback cost: frame {index}, head {head:.1}, drawn {now_drawn:?}, to {end}"
                );
            }
            paced_frames.push(frame);
        }
        for (index, kept) in saved_frames {
            crate::render::write_texture_png(&kept, &state.device, &state.queue, &saved_to.join(format!("frame-{index}.png")))?;
        }
    }
    let stepped = if schedule.paced.is_some() { 0 } else { hours + u32::from(schedule.seek_back.is_some()) };
    for index in 0..stepped {
        let offset = if schedule.pan.is_some() || schedule.paused {
            0
        } else {
            i64::from(index) * i64::from(schedule.step)
        };
        let hour = match schedule.seek_back.filter(|_| index == hours) {
            Some(back) => {
                let speed = f64::from(schedule.step.max(1)) * crate::playhead_host::settings().rate;
                viewer.anchor_playback(Playhead { branch, hour: back, fraction: 0.0 }, false, speed, true, None, Instant::now(), last);
                back
            }
            None => i32::try_from(i64::from(start) + offset).map_err(io::Error::other)?,
        };
        if let Some(pan) = schedule.pan {
            let mut placement = camera.placement();
            placement.center[0] = (placement.center[0] + pan * index as f32).rem_euclid(map_width);
            viewer.capture.as_mut().unwrap().pinned = Some((pixels, Some(placement)));
        }
        viewer.playhead = Some(Playhead {
            branch,
            hour,
            fraction: 0.0,
        });
        viewer.capture.as_mut().unwrap().time = Some(index as f32 / schedule.refresh_hz as f32);
        let process_before = process_cycles();
        let thread_before = viewer_runtime::profiling::current_thread_cpu_time();
        let frame_start = Instant::now();
        crate::perf_log::frame_start();
        let perf_frame = crate::perf_log::frame_id();
        let _other_cost = cpu::scope("frame.harness", Some("other"));
        let mut events = Vec::new();
        if let Some(zoom) = schedule.zoom {
            events.push(egui::Event::PointerMoved(center));
            if index % zoom.every == 0 {
                let height = viewer.camera.map(|placement| placement.height);
                if burst_height.is_some() && burst_height == height {
                    zoom_in = !zoom_in;
                }
                burst_height = height;
                events.push(egui::Event::MouseWheel {
                    unit: egui::MouseWheelUnit::Line,
                    delta: egui::vec2(0.0, if zoom_in { zoom.lines } else { -zoom.lines }),
                    phase: egui::TouchPhase::Move,
                    modifiers: egui::Modifiers::NONE,
                });
            }
        }
        let result = frame_ui(
            &ctx,
            pixels,
            f64::from(index) / schedule.refresh_hz,
            schedule.zoom.is_some(),
            events,
            &mut viewer,
            &state,
            &mut document,
        );
        absorb_textures(surface.as_mut(), &state, result.map_err(io::Error::other)?)?;
        let extra = extras.frame(&document, viewer.playhead_map.as_ref(), u64::from(index), crate::playhead_host::Head::Stepped(hour));
        if !viewer.drew_frame || viewer.hour_on_screen(&document) != (branch, hour) {
            let map = viewer.playhead_map.as_ref().map_or_else(|| "no playhead map".to_owned(), |side| side.with_side(crate::playhead_host::MapSide::shown_states));
            return Err(io::Error::other(format!(
                "Hour {hour} was not submitted; no missing hour is counted as played (drew a frame {}, on screen {:?}; {map})",
                viewer.drew_frame,
                viewer.hour_on_screen(&document),
            )));
        }
        drain(&state)?;
        crate::perf_log::gpu_times(
            viewer
                .scene
                .as_mut()
                .unwrap()
                .gpu_frame_times(&state.device),
        );
        crate::perf_log::frame_end();
        frame_wall.push(frame_start.elapsed().as_secs_f64() * 1e6);
        frame_cpu.extend(cycles_us(process_before, cycle_hz));
        frame_thread_cpu.extend(thread_us(thread_before));
        gpu.sample(&state);
        warning_frames += u32::from(!viewer.map_warnings.is_empty());
        drop(_other_cost);
        if let Some(side) = &viewer.playhead_map
            && side.last_frame().drawn != Some(hour)
        {
            return Err(io::Error::other(format!("Hour {hour} was not drawn by the map's layers (they show {:?}; name, drawn, carried, upload left: {:?})", side.last_frame().drawn, side.with_side(crate::playhead_host::MapSide::layer_states))));
        }
        if schedule.pngs.contains(&index) {
            crate::render::write_png(
                viewer.scene.as_ref().expect("drawn above"),
                &state.device,
                &state.queue,
                &output.join(format!("frame-{index}.png")),
            )?;
        }
        let counts = gpu_counts();
            let (scopes, scope_walls, scope_calls) = frame_scopes(&mut labels_before);
            let cycle_reads = cpu::cycle_reads() - std::mem::replace(&mut reads_before, cpu::cycle_reads());
        frames.write(json!({"frame":index,"perf_frame":perf_frame,"hour":hour,"thread_us":frame_thread_cpu.last(),
                "wall_us":frame_wall.last(),
                "camera_height":viewer.camera.map(|placement| placement.height),
                "camera":camera_record(pinned_camera(&viewer)),"view":view_record(),
                "gpu":counts,
                "scopes_us":scopes,"scopes_wall_us":scope_walls,"scopes_calls":scope_calls,"cycle_reads":cycle_reads,"extra":extra}))?;
        if index % 100 == 0 {
            println!("Playback cost: {}/{} hours, at {}", index + 1, hours, hour);
        }
    }
    drain(&state)?;
    frames.finish()?;
    let counted = match pictures {
        Some(pictures) => Some(pictures.finish(&state.device, &state.queue)?),
        None => None,
    };
    let seconds = wall.elapsed().as_secs_f64();
    let cycle_ns = cycle_start
        .zip(process_cycles())
        .and_then(|(a, b)| b.checked_sub(a))
        .zip(cycle_hz)
        .map(|(cycles, hz)| (cycles as f64 / hz * 1e9).round() as u64);
    let process_ns = cpu::process_cpu_ns()
        .and_then(|end| end.checked_sub(process_start))
        .ok_or_else(|| io::Error::other("Process CPU unavailable"))?;
    let after = cpu::snapshot();
    let memory = monitor.finish();
    let mut report = crate::playback_cost::accounting(&before, &after, process_ns, cycle_ns, hours);
    report["schema"] = json!(1);
    report["pictures"] = json!(counted);
    if schedule.count_pictures.is_some() {
        let ground = [camera.center[0], 0.0, camera.center[1]];
        let along = |axis: usize, by: f32| {
            let mut moved = ground;
            moved[axis] += by;
            camera
                .project(moved, pixels)
                .zip(camera.project(ground, pixels))
                .map(|([x, y], [x0, y0])| ((x - x0).hypot(y - y0) / by.abs()) as f64)
        };
        report["pixels_a_world_unit"] = json!({
            "east": along(0, 1.0), "north": along(2, 1.0),
            "scope": "Pixels a world unit at the camera's own centre on the ground plane, by the frame's own projection: a mover's glide in world units times this is what it moved on screen there. The view's edges differ under the game's pitched camera.",
        });
    }
    report["cycle_read_ns"] = after["cycleReadNs"].clone();
    report["model_build_mode"] = json!(if schedule.complete_models {
        "complete"
    } else {
        "live"
    });
    report["png_frames"] = json!(schedule.pngs);
    if let Some(presented) = schedule.png_frames {
        report["presented_png_frames"] = json!((presented.first..presented.first + presented.frames).collect::<Vec<u64>>());
    }
    report["seek_back"] = json!(schedule.seek_back);
    report["extra_maps"] = extras.summary();
    report["hour_step"] =
        json!((schedule.pan.is_none() && schedule.paced.is_none()).then_some(schedule.step));
    report["pan_units_per_frame"] = json!(schedule.pan);
    report["walk"] = json!(options.walk.as_ref().map(|walk| json!({
        "start": walk.at, "facing_deg": walk.facing.to_degrees(), "scale": walk.scale, "flying": walk.flying,
        "keys": walk.held.iter().map(|key| format!("{key:?}")).collect::<Vec<_>>(),
        "ctrl": walk.modifiers.ctrl, "shift": walk.modifiers.shift,
        "scope": "The viewing country's infantry figure walks from `start` on the capture clock, holding `keys` every frame; each frame's record names the eye behind it (`camera`) and every part's counts for that view (`view`).",
    })));
    report["flight"] = json!(schedule.flight.map(|flight| json!({
        "start": flight.start, "height": flight.height, "climb_to": crate::playback_cost::flight::CLIMB_TO,
        "legs": crate::playback_cost::flight::LEGS.iter().map(|(name, seconds)| json!({"leg": name, "seconds": seconds})).collect::<Vec<_>>(),
        "scope": "Each frame's camera is the path at its due time (frame index over the refresh rate), looping; a frame's record names its leg (`flight`), its camera (`camera`) and every part's counts for that view (`view`).",
    })));
    report["branch"] = json!(branch);
    report["from_hour"] = json!(start);
    report["game_hours"] = json!(hours);
    report["wall_seconds"] = json!(seconds);
    report["game_hours_per_wall_second"] = json!(f64::from(hours) / seconds);
    report["process_cpu_us_per_game_hour"] = json!(process_ns as f64 / 1000.0 / f64::from(hours));
    let frame_count = frame_wall.len();
    report["frame_cpu_us"] = if frame_cpu.len() == frame_count {
        distribution(frame_cpu)
    } else {
        distribution(Vec::new())
    };
    report["frame_cpu_scope"] = json!(
        "Whole-process QueryProcessCycleTime, converted with the same calibrated TSC rate as exclusive thread scopes; includes unregistered process threads. No scaling to force agreement with GetProcessTimes."
    );
    let budget = 1e6 / schedule.refresh_hz;
    let dropped = |values: &[f64]| values.iter().filter(|us| **us > budget).count();
    report["dropped_frames"] = json!({"refresh_hz":schedule.refresh_hz,"budget_us":budget,
        "window_thread":dropped(&frame_thread_cpu),"wall":dropped(&frame_wall),
        "scope":"frames whose window-thread time, or whose wall time with the harness's wait for the GPU, exceeds one refresh; frame 0 included"});
    report["frame_thread_cpu_us"] = if frame_thread_cpu.len() == frame_count {
        distribution(frame_thread_cpu)
    } else {
        distribution(Vec::new())
    };
    report["frame_wall_us"] = distribution(frame_wall);
    report["memory"] = json!({"working_set_peak_bytes":memory.map(|m| m.working_set_peak),
        "private_peak_bytes":memory.map(|m| m.private_peak),"commit_peak_bytes":memory.map(|m| m.commit_peak),
        "scope":"OS working-set/commit lifetime peaks include startup; private sampled every 10 ms from content setup through final drain"});
    report["gpu"] = json!({"adapter":info.name,"backend":format!("{:?}",info.backend),
        "allocated_peak_bytes":gpu.allocated,"reserved_peak_bytes":gpu.reserved,"dedicated_resident_peak_bytes":null,
        "scope":"wgpu allocator report sampled after every submitted frame, not adapter-wide residency; unsupported reports are null"});
    report["viewport"] = json!(pixels);
    report["presentation"] = json!({
        "surface": if schedule.windowed { json!("the window's own swap chain, present mode as the map's") } else { Value::Null },
        "vertical_sync": false,
        "present_mode": schedule.windowed.then(|| format!("{:?}", state.surface_config.present_mode)),
        "desired_maximum_frame_latency": schedule.windowed.then_some(state.surface_config.desired_maximum_frame_latency),
        "attached_display_hz": content_reader_settings::settings::display_refresh_hz(
            window.as_ref().map(|window| window.hwnd()),
        ),
        "frame_limit_hz": viewer_runtime::frame_limit::limit(),
        "scope": "The harness renders to offscreen textures with no surface, so no frame waits for vertical sync or a swap chain. gpu_wait_us is the harness's own drain after each submission, which a window does not pay: there the next frame's CPU work overlaps the GPU's, and a frame waits only when the GPU is a whole frame behind (the swap chain's queue is full). A frame's GPU work by pass is each frame's view record `gpu_passes` (microseconds, read back frames later: `frames_ago`), or `not_recorded` on an adapter without timestamps.",
    });
    report["camera"] =
        json!({"center":camera.center,"height":camera.height,"pose":format!("{:?}",camera.pose)});
    report["map_mode"] = json!(options.map_mode.token());
    report["perspective"] = json!(format!("{:?}", options.perspective));
    report["last_frame_warnings"] = json!(options.diagnostics.then(|| std::mem::take(&mut viewer.map_warnings)));
    report["frames_with_warnings"] = json!(options.diagnostics.then_some(warning_frames));
    report["diagnostic_warnings_collected"] = json!(options.diagnostics);
    report["schedule"] = json!(if schedule.paced.is_some() {
        "Frame k starts at k/refresh_hz s, or at once when the frame before ran past it; the head runs free from --from-hour at --paced hours a second and the run ends when it reaches --from-hour + --hours. Camera and presentation clocks are the wall clock. Each frame draws the newest hour the map's layers have composed at or before the head, as fast play in the window does: nothing waits for an hour or its icons (loaded in the background). Same playhead, projections, SceneInput and Scene::render as viewing. GPU drained each frame after submission, timed apart. CPU/wall includes pipeline startup and final GPU drain; static content/device setup is separate; game hours are hours of head travel."
    } else {
        "One frame per requested game hour; camera and presentation clocks advance 1/refresh_hz s per frame. Same playhead, projections, SceneInput and Scene::render as viewing. GPU drained each frame; no vsync, window, OS input, audio-device output or synthetic codec work. CPU/wall includes the layers' first compose and the final GPU drain; static content/device setup is separate."
    });
    if let Some(speed) = schedule.paced {
        let mut paced = paced::summary(&paced_frames, schedule.refresh_hz, paced_end_us, speed);
        paced["refresh_hz"] = json!(schedule.refresh_hz);
        paced["head_hours"] = json!(hours);
        paced["process_cpu_us_per_head_hour"] =
            json!(process_ns as f64 / 1000.0 / f64::from(hours));
        paced["harness_wait_thread_cpu_us"] = json!(harness_wait_us);
        paced["scope"] = json!(
            "Distributions and counts over the frames after warm-up (the frames before the first drawn hour). work_us: the product's frame alone, from the call of the frame's UI pass to its return, which submits its commands; the harness's own work around it (this loop's bookkeeping, the records it writes, its drain of the GPU) is charged to frame.harness and frame.record and is in neither work_us nor thread_us. gpu_wait_us: the drain after the frame; thread_us: the window thread's CPU over the product's frame. late_frames: work over one refresh interval; a late start is unexplained when the frame before finished its work and GPU wait by this frame's due time. drawn_hour is the hour the frame drew everything at, its own head (`viewer_native_gui::frame_head`), which follows the clock and is steered to stay inside the hours published; lag_hours is head_hour minus the oldest hour the map's layers show, the overlay no part of it, so it measures the products' lateness and not the head's. overlay_hour is the hour the overlay drawn stands at and overlay_behind_hours how far that is behind the hour the layers drew; overlay_behind_hours and overlay_behind_ms (those hours at the run's speed) are their distributions, and overlay_new_hour_frames the frames in which the overlay moved. Process CPU includes the harness's wait to each due time (its window-thread CPU is harness_wait_thread_cpu_us). In frames.jsonl, reached_us is when the harness came to the frame after the one before it (late when past due_us), and woke_us when its sleep returned, a millisecond short of due, before it spins (null without a sleep). scopes_us is each window-thread label's own CPU this frame and scopes_wall_us its wall time, charged the same way: a label's wall over its CPU is time off a core inside it. In the layers record, take_ms and take_cpu_ms are the frame's taking of the stepper's steps from the ring they share, side_missed names what the side's thread was doing when this frame could not take the side at all (null when it took it), and first and applied are the first and last hour the publication drawn carries. Per publication (deduplicated by made): stage_ms and stage_cpu_ms are the side's step stage by stage, in wall milliseconds and in the side thread's own CPU (a stage whose wall stands over its CPU waited rather than worked), side_cpu_ms that CPU over the whole step, step_cpu_ms the stepper thread's own CPU in the step builds of step_ms, and waited_ms how long past its due time the publication started with, of that, what it waited for an hour the one before did not show and for the frame to release the side."
        );
        report["paced"] = paced;
        if let Some(side) = &viewer.playhead_map {
            report["layers"] = layered.summary(side.names());
            let costs = layer_costs(&report["layers"]);
            for part in report["parts"].as_array_mut().into_iter().flatten() {
                let replacing = part["layers"].as_array().cloned().unwrap_or_default();
                part["layer_cost_a_frame"] = Value::Array(replacing.iter().filter_map(|name| costs.get(name.as_str()?).cloned()).collect());
            }
            side.with_side(|side| {
                report["layers"]["standing"] = side.standing();
                report["view_store"] = side.views().record(distribution);
                report["intel_icons"] = viewer_layers_forces::intel_icons::summary(side);
                report["names"] = viewer_layers_political::names::summary(side);
                report["counters"] = viewer_layers_counters::counters::fed::summary(side);
                report["aircraft"] = viewer_layers_forces::aircraft::summary(side);
                report["buildings"] = viewer_layers_infrastructure::buildings::summary(side);
                report["snow"] = viewer_layers_infrastructure::snow::summary(side);
                report["trains"] = viewer_layers_infrastructure::trains::summary(side);
            });
            report["scope"] = layered.scope_summary(side.names());
            let (checked, differing) = viewer_runtime::install_check::counted();
            report["install_checks"] = json!({"checks": checked, "differing": differing, "on": viewer_runtime::install_check::on()});
            let (checked, differing, carried, longest) = viewer_runtime::frame_budget::check::counted();
            let (judged, refused) = viewer_runtime::publication_check::counted();
            report["publication_checks"] = json!({"checks": judged, "differing": refused, "on": viewer_runtime::publication_check::on()});
            let [looked, compared, nothing_new, art_moved, rows_missed, layers_moved, no_overlay] = crate::app::overlay_checks::counted();
            report["overlay_checks"] = json!({"looked": looked, "checks": compared, "not_compared": {"nothing_new": nothing_new,
                "the_art_moved": art_moved, "the_rows_held_no_table_it_read": rows_missed,
                "a_layers_product_moved": layers_moved, "no_overlay_yet": no_overlay}});
            let (hours_judged, hours_differing) = viewer_runtime::hour_check::counted();
            report["hour_checks"] = json!({"checks": hours_judged, "differing": hours_differing, "on": viewer_runtime::hour_check::on()});
            report["budget_checks"] = json!({"checks": checked, "differing": differing, "frames_carrying": carried, "longest_carry_frames": longest,
                "on": viewer_runtime::frame_budget::check::on()});
        }
    }
    if let Some(side) = &viewer.playhead_map {
        let (now, peak, counters) = side.entry().playhead().held();
        let (host, readers) = side.with_side(|side| (side.held(), side.readers()));
        report["stepper"] = json!({
            "steps": counters.steps, "stalls": counters.stalls, "stalled_steps": counters.stalled_steps, "piece_waits": counters.piece_waits,
            "pieces_crossed": counters.pieces_crossed, "crossed_tables": counters.crossed_tables, "crossed_resets": counters.crossed_resets,
            "crossed_rows": counters.crossed_rows, "crossed_cells": counters.crossed_cells,
            "cross_ms": counters.cross_ns as f64 / 1e6, "cross_most_ms": counters.cross_most_ns as f64 / 1e6,
            "seeks": counters.seeks, "joins": counters.joins, "join_holds": counters.join_holds, "catch_ups": counters.catch_ups, "split_steps": counters.split_steps,
            "landings": counters.landings, "landed_behind": counters.landed_behind, "landing_rebuild_ms": counters.landing_rebuild_ns as f64 / 1e6, "caught_up": counters.caught_up, "resteps": counters.resteps,
            "behind_the_head_ms": {"room": counters.room_held_ns as f64 / 1e6, "joins": counters.join_held_ns as f64 / 1e6, "scope": "Time the stepper was behind the head and built nothing: for want of ring room (the ring's size of steps no consumer had taken), and while a join it had reached waited for its lane."},
            "idle_ms": {"ring_full_ahead": counters.idle_ns[0] as f64 / 1e6, "ring_full_behind": counters.idle_ns[1] as f64 / 1e6, "fetch": counters.idle_ns[2] as f64 / 1e6, "end": counters.idle_ns[3] as f64 / 1e6, "other": counters.idle_ns[4] as f64 / 1e6, "scope": "Time the stepper waited with nothing to build, by why its last build stopped: the ring full ahead of the head (waiting for the frames to take its steps), the ring full behind it, a lane's part not fetched yet, the recording's end, anything else."},
            "playhead_bytes": {"now": now.total, "peak": peak.total, "ring_peak": peak.ring_bytes, "states_peak": peak.state_bytes, "preparing_peak": peak.preparing},
            "host_bytes": {"mirrors": host.mirrors.iter().map(|(_, bytes)| bytes).sum::<usize>(), "layers": host.layers, "pending_steps": host.pending, "states": host.states, "playhead": host.playhead, "total": host.total()},
            "mirrors": readers.into_iter().map(|(table, columns, keyed, layers)| json!({"table": format!("{table:?}"), "columns": columns, "keyed": keyed, "layers": layers, "bytes": host.mirrors.iter().find(|(held, _)| *held == table).map(|(_, bytes)| *bytes)})).collect::<Vec<_>>(),
            "piece_checks": {"tables": crate::playhead_host::piece::counted().0, "differing": crate::playhead_host::piece::counted().1, "on": crate::playhead_host::piece::on()},
            "scope": "The playhead's counters since it opened (steps built, table steps stalled at a part not fetched and the steps holding any, seeks, joins, and the landings: consumers the ring parked for holding more steps than the lanes' cells, each landed with every table whole, which the map answers by composing every layer), the piece boundaries crossed as a step with the tables they carried over, the tables reset in them because no change set carries a log over a session's handover, and the rows and cells the crossings named; the judge of those crossings (BATTLEPLAN_VIEWER_CHECK_PIECE) compares every table's mirror after one with the mirror a rebuild of the new piece's lane makes; its held bytes now and at their peaks, and what the map's side holds at the end. Per frame, the steps' build time is layers.step_ms and frames.jsonl's layers.step_ms.",
        });
    }
    {
        use viewer_runtime::worker_priority as priority;
        report["threads"] = json!({
            "entered": priority::entered().into_iter().map(|entered| json!({"name": entered.name, "class": entered.class.name(), "priority": entered.priority, "threads": entered.threads})).collect::<Vec<_>>(),
            "scope": "Every thread of ours that named its class at its start (worker_priority::enter): its name, class, the priority read back after it was set, and how many threads of that name entered. The pipeline builds and the driver's threads enter no class and run at normal.",
        });
    }
    {
        let (calls, overlapped, contended) = engine_heap::large_blocks();
        report["large_blocks"] = json!({
            "calls": calls, "overlapped": overlapped, "contended": contended,
            "scope": "Blocks past 64 KiB, which have pages of their own: the calls made, those that ran while another was running (what one list behind one lock would have made wait), and those that found their size class held and took fresh pages, or released at once, instead of waiting. No call waits.",
        });
    }
    if let Some(uploads) = viewer.playhead_map.as_ref().and_then(|side| side.installed().find(|(name, ..)| *name == "political").map(|(_, _, wrote)| wrote.writes)) {
        let [whole, country, naval] = uploads;
        report["political_uploads"] = json!({"whole": whole, "country_by_change": country, "naval_by_change": naval});
    }
    Ok(report)
}

#[derive(Default)]
struct Layered {
    frames: Vec<(Vec<Part>, Option<f64>, Option<f64>)>,
    overlay: Option<i32>,
    mirror_ms: Vec<f64>,
    mirror_tables: Vec<(u64, u64)>,
    mirror_split: crate::playhead_host::mirror::Split,
    mirror_cpu_ms: Vec<f64>,
    step_ms: Vec<f64>,
    side_step_ms: Vec<f64>,
    phases: Vec<[f64; 4]>,
    held_peak: [usize; 3],
    before_shown: u32,
    seeks: Vec<Sought>,
    political: Option<usize>,
    trace: Vec<(f64, Option<i32>, Option<i32>, Option<i32>)>,
    outside: Vec<bool>,
    composing: u32,
    scope: Vec<[u64; 5]>,
}

#[derive(Clone, Copy)]
struct Part {
    lag: Option<f64>,
    wall: [f64; 4],
    cpu: [Option<f64>; 4],
    upload_carried: bool,
    uploads: (u64, u64),
    inside: [f64; 2],
    ready_at_hour: Option<bool>,
    ahead_ms: f64,
}

const SCOPES: [&str; 4] = ["apply", "settle", "stage", "upload"];

#[derive(Default)]
struct Sought {
    frame: u64,
    hour: i32,
    asked_us: f64,
    held: Option<(u64, f64)>,
    composed: Option<(u64, f64)>,
    first: Option<(u64, f64)>,
    all: Option<(u64, f64)>,
    within: Option<u64>,
    superseded: u32,
    compose_ns: Vec<u64>,
    seeking: Option<playhead::Seeking>,
}

impl Sought {
    fn mark(&self, mark: Option<(u64, f64)>) -> Value {
        mark.map_or(Value::Null, |(at, ms)| json!({"frames": at - self.frame, "ms": ms}))
    }
}

impl Layered {
    fn settled(&self) -> bool {
        self.seeks.last().is_none_or(|seek| seek.within.is_some())
    }

    fn ready_to_seek(&self) -> bool {
        self.seeks.last().is_none_or(|seek| seek.all.is_some())
    }

    fn seek(&mut self, index: u64, hour: i32, asked_us: f64) {
        self.seeks.push(Sought { frame: index, hour, asked_us, ..Sought::default() });
    }

    fn mirror_by_table(&self) -> Vec<Value> {
        let mut tables: Vec<(usize, u64)> = self.mirror_tables.iter().map(|&(ns, _)| ns).enumerate().filter(|(_, ns)| *ns > 0).collect();
        tables.sort_by_key(|&(_, ns)| std::cmp::Reverse(ns));
        tables.into_iter().map(|(table, ns)| json!({"table": format!("{:?}", replay_data::rows::TableId::ALL[table]), "mean_ms": ns as f64 / 1e6 / self.mirror_ms.len().max(1) as f64})).collect()
    }

    fn sample_held(&mut self, side: &crate::playhead_host::MapSide) {
        let held = side.held();
        let mirrors: usize = held.mirrors.iter().map(|(_, bytes)| bytes).sum::<usize>() + held.views.iter().map(|(_, bytes)| bytes).sum::<usize>();
        let layers: usize = held.layers.iter().map(|(_, bytes)| bytes).sum();
        self.held_peak = [self.held_peak[0].max(mirrors), self.held_peak[1].max(layers), self.held_peak[2].max(held.total())];
    }

    fn frame(&mut self, frame: &crate::playhead_host::Frame, head: f64, hz: Option<f64>, (index, end_us): (u64, f64), newest: Option<i32>, phases: Option<[u64; 4]>) -> Value {
        if let Some(phases) = phases {
            self.phases.push(phases.map(|ns| ns as f64 / 1e6));
        }
        let parts: Vec<Part> = frame
            .layers
            .iter()
            .map(|part| {
                let cpu = |cycles: u64| hz.map(|hz| cycles as f64 / hz * 1e3);
                Part {
                    lag: part.shown.map(|shown| head - f64::from(shown)),
                    wall: [part.apply_ns, part.settle_ns, part.stage_ns, part.upload_ns].map(|ns| ns as f64 / 1e6),
                    cpu: [part.apply_cycles, part.settle_cycles, part.stage_cycles, part.upload_cycles].map(cpu),
                    upload_carried: part.upload_carried,
                    uploads: (part.uploads, part.upload_bytes),
                    inside: [part.write_ns, part.create_ns].map(|ns| ns as f64 / 1e6),
                    ready_at_hour: part.ready_at_hour,
                    ahead_ms: part.ahead_ns as f64 / 1e6,
                }
            })
            .collect();
        let clock = frame.drawn.map(|drawn| head - f64::from(drawn));
        let oldest = frame.drawn.and(frame.oldest.or(frame.drawn));
        let lag = oldest.map(|oldest| head - f64::from(oldest));
        self.composing += u32::from(frame.composing);
        self.step_ms.push(frame.step_ns as f64 / 1e6);
        self.side_step_ms.push(frame.side_ns as f64 / 1e6);
        self.trace.push((head, newest, frame.applied, frame.drawn));
        if let Some(seek) = self.seeks.last_mut() {
            let at = Some((index, (end_us - seek.asked_us) / 1e3));
            seek.superseded += u32::from(frame.superseded);
            if seek.held.is_none() && frame.sought.is_some() {
                (seek.held, seek.seeking) = (at, frame.sought);
            }
            if seek.held.is_some()
                && seek.composed.is_none()
                && let Some(swapped) = &frame.swapped
            {
                (seek.composed, seek.compose_ns) = (at, swapped.clone());
            }
            if seek.composed.is_some() {
                if seek.first.is_none() && self.political.and_then(|political| frame.layers.get(political)).is_some_and(|part| part.reset_shown) {
                    seek.first = at;
                }
                if seek.all.is_none() && frame.reset_shown {
                    seek.all = at;
                }
                if seek.within.is_none() && frame.drawn.is_some_and(|drawn| head - f64::from(drawn) <= 24.0) {
                    seek.within = Some(index);
                }
            }
        }
        match frame.drawn {
            None => self.before_shown += 1,
            Some(_) => {
                if self.scope.len() < frame.layers.len() {
                    self.scope.resize(frame.layers.len(), [0; 5]);
                }
                for (sum, part) in self.scope.iter_mut().zip(&frame.layers) {
                    sum[4] += u64::from(!part.ready);
                    let Some(scope) = part.scope else { continue };
                    sum[0] += scope.settled;
                    if let Some(in_view) = scope.in_view {
                        (sum[1], sum[2]) = (sum[1] + scope.settled, sum[2] + in_view);
                    }
                    sum[3] += u64::from(scope.settled > 0);
                }
                self.outside.push(self.settled());
                self.frames.push((parts.clone(), lag, clock));
                self.mirror_ms.push(frame.mirror_ns as f64 / 1e6);
                if let Some(hz) = hz {
                    self.mirror_cpu_ms.push(frame.mirror_cycles as f64 / hz * 1e3);
                }
                if self.mirror_tables.len() < frame.mirror_tables.len() {
                    self.mirror_tables.resize(frame.mirror_tables.len(), (0, 0));
                }
                self.mirror_tables.iter_mut().zip(&frame.mirror_tables).for_each(|(sum, (ns, cycles))| *sum = (sum.0 + ns, sum.1 + cycles));
                let held = &mut self.mirror_split;
                let split = frame.mirror_split;
                held.awake_ns += split.awake_ns;
                held.awake_sets += split.awake_sets;
                held.asleep_ns += split.asleep_ns;
                held.asleep_sets += split.asleep_sets;
                held.asleep_cells += split.asleep_cells;
                held.asleep_rows += split.asleep_rows;
            }
        }
        let waited = frame.waited;
        let waited_ms: serde_json::Map<String, Value> = [("late", waited.late_ns), ("for_an_hour", waited.for_an_hour_ns), ("for_the_frame", waited.for_the_frame_ns)].into_iter().map(|(name, ns)| (name.to_owned(), json!(ns as f64 / 1e6))).collect();
        let stages = crate::playhead_host::Stages::NAMES;
        let stage_ms: serde_json::Map<String, Value> = stages.iter().zip(frame.stages.ns).map(|(name, ns)| ((*name).to_owned(), json!(ns as f64 / 1e6))).collect();
        let stage_cpu_ms: serde_json::Map<String, Value> = hz.map(|hz| stages.iter().zip(frame.stages.cycles).map(|(name, cycles)| ((*name).to_owned(), json!(cycles as f64 / hz * 1e3))).collect()).unwrap_or_default();
        let named = |table: usize| format!("{:?}", replay_data::rows::TableId::ALL[table]);
        let mirror_tables: serde_json::Map<String, Value> = frame.mirror_tables.iter().enumerate().filter(|(_, (ns, _))| *ns > 0).map(|(table, &(ns, _))| (named(table), json!(ns as f64 / 1e6))).collect();
        let mirror_tables_cpu: serde_json::Map<String, Value> = hz.map(|hz| frame.mirror_tables.iter().enumerate().filter(|(_, (_, cycles))| *cycles > 0).map(|(table, &(_, cycles))| (named(table), json!(cycles as f64 / hz * 1e3))).collect()).unwrap_or_default();
        let mut record = json!({"head": head, "newest": newest, "made": frame.made,
            "political_upload_phases_ms": phases.map(|phases| phases.map(|ns| ns as f64 / 1e6)),
            "steps": frame.steps, "step_ms": frame.step_ns as f64 / 1e6, "side_step_ms": frame.side_ns as f64 / 1e6, "first": frame.first, "applied": frame.applied, "drawn": frame.drawn, "oldest": oldest, "overlay_hour": self.overlay, "lag_hours": lag, "lag_hours_without_cadence_layers": clock, "mirror_ms": frame.mirror_ns as f64 / 1e6, "mirror_cpu_ms": hz.map(|hz| frame.mirror_cycles as f64 / hz * 1e3), "take_ms": frame.take_ns as f64 / 1e6, "take_cpu_ms": hz.map(|hz| frame.take_cycles as f64 / hz * 1e3), "mirror_tables_ms": mirror_tables, "mirror_tables_cpu_ms": mirror_tables_cpu, "cells": frame.cells, "rows": frame.rows,
            "wall_ms": parts.iter().map(|part| part.wall).collect::<Vec<_>>(), "cpu_ms": parts.iter().map(|part| part.cpu).collect::<Vec<_>>(),
            "upload_carried": parts.iter().map(|part| part.upload_carried).collect::<Vec<_>>(),
            "layer_published": frame.layers.iter().map(|part| part.published).collect::<Vec<_>>(),
            "reached_ms": frame.reached_ns as f64 / 1e6,
            "ready": frame.layers.iter().map(|part| part.ready).collect::<Vec<_>>(),
            "shown": frame.layers.iter().map(|part| part.shown).collect::<Vec<_>>(), "layer_lag_hours": parts.iter().map(|part| part.lag).collect::<Vec<_>>(),
            "ready_at_hour": parts.iter().map(|part| part.ready_at_hour).collect::<Vec<_>>(), "ahead_ms": parts.iter().map(|part| part.ahead_ms).collect::<Vec<_>>(),
            "composing": frame.composing, "resets": frame.resets, "swapped": frame.swapped, "superseded": frame.superseded,
            "reset_shown": frame.reset_shown, "layer_reset_shown": frame.layers.iter().map(|part| part.reset_shown).collect::<Vec<_>>(),
            "scope": frame.layers.iter().map(|part| part.scope.map(|scope| json!([scope.settled, scope.in_view]))).collect::<Vec<_>>()});
        record["reset_tables"] = json!(frame.reset_tables.iter().map(|&table| named(table)).collect::<Vec<_>>());
        record["reset_step"] = json!(frame.reset_step);
        record["renumbers"] = json!(frame.renumbers.iter().map(|&(table, rows, moved)| json!([named(table), rows, moved])).collect::<Vec<_>>());
        record["side_cpu_ms"] = json!(hz.map(|hz| frame.side_cycles as f64 / hz * 1e3));
        record["step_cpu_ms"] = json!(hz.map(|hz| frame.step_cycles as f64 / hz * 1e3));
        record["stage_ms"] = Value::Object(stage_ms);
        record["stage_cpu_ms"] = Value::Object(stage_cpu_ms);
        record["waited_ms"] = Value::Object(waited_ms);
        record["uploads"] = json!(parts.iter().map(|part| part.uploads.0).collect::<Vec<_>>());
        record["upload_bytes"] = json!(parts.iter().map(|part| part.uploads.1).collect::<Vec<_>>());
        record["upload_inside_ms"] = json!(parts.iter().map(|part| part.inside).collect::<Vec<_>>());
        record
    }

    fn scope_summary(&self, names: &[&str]) -> Value {
        let layers: Vec<Value> = names
            .iter()
            .enumerate()
            .map(|(at, name)| {
                let [settled, counted, in_view, frames, not_ready] = self.scope.get(at).copied().unwrap_or_default();
                json!({"name": name, "settled": settled, "frames_settling": frames, "settled_where_the_view_was_counted": counted, "in_view": (counted > 0).then_some(in_view), "share_in_view": (counted > 0).then(|| in_view as f64 / counted as f64), "frames_not_ready": not_ready})
            })
            .collect();
        json!({
            "layers": layers,
            "scope": "Entities each layer's settles made over the frames drawn. A layer that names where its entities stand (provinces, states, or for sounds the battles and fronts within hearing of the eye) counts those the camera admits; one that does not gives the entities left its carry-over each frame it settled, and in_view null. frames_not_ready: frames drawn in which the layer was declared but not composed (a layer declared on approach and not ready in time shows here, drawn late, never with old or zero values).",
        })
    }

    fn summary(&self, names: &[&str]) -> Value {
        let phases: serde_json::Map<String, Value> = ["naval", "country fill", "plans built", "plans written"].iter().enumerate().map(|(at, phase)| (phase.to_string(), distribution(self.phases.iter().map(|phases| phases[at]).collect()))).collect();
        let width = self.frames.first().map_or(0, |(parts, ..)| parts.len());
        let layers: Vec<Value> = (0..width)
            .map(|at| {
                let mut layer = json!({
                    "name": names.get(at),
                    "lag_hours": distribution(self.frames.iter().filter_map(|(parts, ..)| parts[at].lag).collect()),
                    "wall_ms": distribution(self.frames.iter().map(|(parts, ..)| parts[at].wall.iter().sum()).collect()),
                    "cpu_ms": distribution(self.frames.iter().filter_map(|(parts, ..)| parts[at].cpu.iter().copied().sum::<Option<f64>>()).collect()),
                    "frames_upload_carried": self.frames.iter().filter(|(parts, ..)| parts[at].upload_carried).count(),
                });
                let fed: Vec<bool> = self.frames.iter().filter_map(|(parts, ..)| parts[at].ready_at_hour).collect();
                if !fed.is_empty() {
                    layer["ahead"] = json!({
                        "frames_applying": fed.len(),
                        "ready_at_hour": fed.iter().filter(|&&ready| ready).count(),
                        "hand_ms": distribution(self.frames.iter().filter(|(parts, ..)| parts[at].ready_at_hour.is_some()).map(|(parts, ..)| parts[at].ahead_ms).collect()),
                    });
                }
                for (scope, name) in SCOPES.iter().enumerate() {
                    layer[name] = json!({
                        "wall_ms": distribution(self.frames.iter().map(|(parts, ..)| parts[at].wall[scope]).collect()),
                        "cpu_ms": distribution(self.frames.iter().filter_map(|(parts, ..)| parts[at].cpu[scope]).collect()),
                    });
                }
                layer["upload"]["queue_write_ms"] = json!(distribution(self.frames.iter().map(|(parts, ..)| parts[at].inside[0]).collect()));

                layer["upload"]["created_ms"] = json!(distribution(self.frames.iter().map(|(parts, ..)| parts[at].inside[1]).collect()));
                layer["upload"]["writes"] = json!(distribution(self.frames.iter().map(|(parts, ..)| parts[at].uploads.0 as f64).collect()));
                layer["upload"]["written_bytes"] = json!(distribution(self.frames.iter().map(|(parts, ..)| parts[at].uploads.1 as f64).collect()));
                layer
            })
            .collect();
        json!({
            "frames": self.frames.len(), "frames_before_first_shown": self.before_shown, "frames_composing": self.composing,
            "seeks": self.seeks.iter().map(|seek| json!({
                "frame": seek.frame, "hour": seek.hour,
                "frames_to_shown": seek.composed.map(|(at, _)| at - seek.frame), "frames_to_lag_within_a_step": seek.within.map(|at| at - seek.frame),
                "reset_held": seek.mark(seek.held), "compose_done": seek.mark(seek.composed), "first_content": seek.mark(seek.first), "all_content": seek.mark(seek.all),
                "superseded": seek.superseded,
                "compose_ms": names.iter().zip(&seek.compose_ns).map(|(name, ns)| ((*name).to_owned(), json!(*ns as f64 / 1e6))).collect::<serde_json::Map<String, Value>>(),
                "seek": seek.seeking.map(|sought| json!({
                    "ms": sought.ns as f64 / 1e6, "tables": sought.tables, "table_most_ms": sought.table_most_ns as f64 / 1e6,
                    "restore_ms": sought.restore_ns as f64 / 1e6, "restore_most_ms": sought.restore_most_ns as f64 / 1e6,
                    "replay_ms": sought.replay_ns as f64 / 1e6, "replay_most_ms": sought.replay_most_ns as f64 / 1e6,
                    "fetch_ms": sought.fetch_ns as f64 / 1e6, "rebuild_ms": sought.rebuild_ns as f64 / 1e6, "rebuild_most_ms": sought.rebuild_most_ns as f64 / 1e6,
                    "ahead_ms": sought.ahead_ns as f64 / 1e6, "waited_ms": sought.waited_ns as f64 / 1e6, "threads": sought.threads,
                    "work_ms": (sought.restore_ns + sought.replay_ns + sought.fetch_ns + sought.rebuild_ns) as f64 / 1e6,
                })),
            })).collect::<Vec<_>>(),
            "seek_marks_ms": {
                "reset_held": distribution(self.seeks.iter().filter_map(|seek| seek.held.map(|(_, ms)| ms)).collect()),
                "compose_done": distribution(self.seeks.iter().filter_map(|seek| seek.composed.map(|(_, ms)| ms)).collect()),
                "first_content": distribution(self.seeks.iter().filter_map(|seek| seek.first.map(|(_, ms)| ms)).collect()),
                "all_content": distribution(self.seeks.iter().filter_map(|seek| seek.all.map(|(_, ms)| ms)).collect()),
                "scope": "Milliseconds from the start of the frame that asked a seek to the end of the frame of each mark: the host took the seek's reset (reset_held); its compose came back and was swapped in (compose_done); the political layer shows its picture made since the swap (first_content); every drawn layer does (all_content). A seek without a mark is left out of that mark's distribution.",
            },
            "trace": self.trace.iter().map(|&(head, built, applied, drawn)| json!([head, built, applied, drawn])).collect::<Vec<_>>(),
            "lag_hours": distribution(self.frames.iter().filter_map(|(_, lag, _)| *lag).collect()),
            "lag_hours_outside_seeks": distribution(self.frames.iter().zip(&self.outside).filter(|(_, outside)| **outside).filter_map(|((_, lag, _), _)| *lag).collect()),
            "lag_hours_without_cadence_layers": distribution(self.frames.iter().filter_map(|(.., clock)| *clock).collect()),
            "mirror_ms": distribution(self.mirror_ms.clone()), "mirror_cpu_ms": distribution(self.mirror_cpu_ms.clone()), "step_ms": distribution(self.step_ms.clone()), "side_step_ms": distribution(self.side_step_ms.clone()), "layers": layers,
            "step_timeline": crate::playhead_host::timeline::report(),
            "mirror_split": {
                "frames": self.mirror_ms.len(),
                "awake_ms_a_frame": self.mirror_split.awake_ns as f64 / 1e6 / self.mirror_ms.len().max(1) as f64,
                "asleep_ms_a_frame": self.mirror_split.asleep_ns as f64 / 1e6 / self.mirror_ms.len().max(1) as f64,
                "awake_sets_a_frame": self.mirror_split.awake_sets as f64 / self.mirror_ms.len().max(1) as f64,
                "asleep_sets_a_frame": self.mirror_split.asleep_sets as f64 / self.mirror_ms.len().max(1) as f64,
                "asleep_cells_a_frame": self.mirror_split.asleep_cells as f64 / self.mirror_ms.len().max(1) as f64,
                "asleep_rows_a_frame": self.mirror_split.asleep_rows as f64 / self.mirror_ms.len().max(1) as f64,
                "scope": "The side's apply a frame, split by whether a drawn layer or a maker reads the table: asleep is what is applied for nobody.",
            },
            "mirror_by_table": {
                "tables": self.mirror_by_table(),
                "scope": "The mirrors' apply by table, the mean over the frames of mirror_ms (frames applying nothing count as zero).",
            },
            "political_upload_phases_ms": phases,
            "held_peak_bytes": {"mirrors": self.held_peak[0], "layers": self.held_peak[1], "side": self.held_peak[2], "scope": "sampled every 30 frames"},
            "scope": "Per frame after the layers first showed the sought hour: each layer's apply and settle on the map side's thread and its upload in the frame's window, apart and summed (wall, and the thread's own cycles as milliseconds), the entities its settles carried to a later one, and the content lag: the harness head minus the oldest hour the map's layers show, every drawn layer counted (those whose product is made from rows at the head on a worker at the hour those rows stand at). The overlay is no part of it: overlay_hour is the hour the overlay drawn stands at, reported beside the layers' hours. lag_hours_without_cadence_layers is the window clock's drawn hour alone, which leaves the cadence layers out too.",
        })
    }
}

fn layer_costs(layers: &Value) -> std::collections::BTreeMap<String, Value> {
    layers["layers"].as_array().into_iter().flatten().filter_map(|layer| Some((layer["name"].as_str()?.to_owned(), layer.clone()))).collect()
}

#[cfg(test)]
mod seek_record_tests {
    use super::Layered;
    use crate::playhead_host::{Frame, LayerFrame};

    fn frame(sought: bool, swapped: bool, fresh: [bool; 2], drawn: i32) -> Frame {
        Frame {
            sought: sought.then(playhead::Seeking::default),
            swapped: swapped.then(|| vec![2_000_000, 5_000_000]),
            layers: fresh.iter().map(|&reset_shown| LayerFrame { reset_shown, shown: Some(drawn), ..LayerFrame::default() }).collect(),
            reset_shown: fresh.iter().all(|&fresh| fresh),
            drawn: Some(drawn),
            ..Frame::default()
        }
    }

    #[test]
    fn a_seek_record_marks_the_reset_held_the_compose_done_the_first_and_all_content() {
        let mut layered = Layered { political: Some(0), ..Layered::default() };
        layered.frame(&frame(false, false, [true, true], 900), 900.0, None, (10, 160_000.0), None, None);
        layered.seek(11, 500, 170_000.0);
        assert!(!layered.ready_to_seek());
        layered.frame(&frame(false, false, [true, true], 900), 500.0, None, (11, 180_000.0), None, None);
        layered.frame(&frame(true, false, [true, true], 900), 500.0, None, (12, 200_000.0), None, None);
        layered.frame(&frame(false, false, [true, true], 900), 500.0, None, (13, 220_000.0), None, None);
        layered.frame(&frame(false, true, [false, false], 900), 500.0, None, (14, 240_000.0), None, None);
        layered.frame(&frame(false, false, [true, false], 900), 524.0, None, (15, 260_000.0), None, None);
        assert!(!layered.ready_to_seek(), "the snow layer still shows the old picture");
        layered.frame(&frame(false, false, [true, true], 500), 548.0, None, (16, 280_000.0), None, None);
        assert!(layered.ready_to_seek());
        let summary = layered.summary(&["political", "snow"]);
        let seek = &summary["seeks"][0];
        let mark = |name: &str| (seek[name]["frames"].as_u64(), seek[name]["ms"].as_f64());
        assert_eq!(mark("reset_held"), (Some(1), Some(30.0)));
        assert_eq!(mark("compose_done"), (Some(3), Some(70.0)));
        assert_eq!(mark("first_content"), (Some(4), Some(90.0)));
        assert_eq!(mark("all_content"), (Some(5), Some(110.0)));
        assert_eq!(seek["frames_to_shown"], 3);
        assert_eq!(seek["compose_ms"]["snow"], 5.0);
    }
}
