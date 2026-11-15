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
pub(crate) mod toggle;

/// Which hours the frames show: every `step`th hour, one hour under a camera moving `pan`
/// map units a frame, or, `paced`, whatever the builders finished under a head running free at that
/// many hours a second while frames go out on the refresh clock.
#[derive(Clone)]
pub(crate) struct Schedule {
    pub step: u32,
    pub pan: Option<f32>,
    /// Frames whose scene is also written to `<out>/frame-<index>.png`, after their timing.
    pub pngs: Vec<u32>,
    /// Wheel lines turned at once, every so many frames, over the map's centre: the window's own
    /// camera takes them through the game's law, reversing at its limits.
    pub zoom: Option<Zoom>,
    /// The hour is held, as when playback is paused.
    pub paused: bool,
    /// The display refresh a dropped frame is counted against.
    pub refresh_hz: f64,
    pub complete_models: bool,
    pub paced: Option<f64>,
    /// Paced, with the map's layers on: seeks while playing, spread over the run.
    pub seeks: u32,
    /// The map drawn from the recording's playhead; without it, today's pipeline alone.
    pub playhead: bool,
    /// Today's hour pipeline stops once its first hour is on screen; only the layers move.
    pub layers_only: bool,
    /// The layers the window registers, by name; every layer when none are named.
    pub layers: Option<Vec<String>>,
    /// Paced: every Nth overlay the tables' worker lays out is compared with the same hour laid
    /// out in place ([`super::laid_proof`]); the run fails on any difference.
    pub laid_proof: Option<u32>,
    /// Paced: the camera flies this path ([`crate::playback_cost::flight`]).
    pub flight: Option<crate::playback_cost::flight::Flight>,
}

/// A frame's camera in its record: the game's look-at camera by its centre and height, a free eye
/// by its place, heading, pitch and lens.
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

/// The camera a frame drew with: the harness's pinned one (`--camera`, `--free-camera`, a pan or a
/// flight), else the window's own.
fn pinned_camera(viewer: &Viewer) -> Option<Placement> {
    viewer.capture.as_ref().and_then(|capture| capture.pinned).and_then(|(_, placement)| placement).or(viewer.camera)
}

/// The frame's flight record ([`viewer_foundation::view_record`]): each part's section and counts.
fn view_record() -> Value {
    Value::Object(
        viewer_foundation::view_record::take()
            .into_iter()
            .map(|(section, counts)| {
                let counts: serde_json::Map<String, Value> =
                    counts.into_iter().map(|(name, n)| (name.to_owned(), json!(n))).collect();
                (section.to_owned(), Value::Object(counts))
            })
            .collect(),
    )
}

/// The parts today's builder draws that a layers-only run hides by the map's own switches, and
/// those with no switch, which stay at the first hour.

#[derive(Clone, Copy)]
pub(crate) struct Zoom {
    pub lines: f32,
    pub every: u32,
}

/// The window thread's charged time by label, and every lap note, for one frame's difference:
/// its CPU, and the wall time it was on top (none for a lap note).
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
                (&mut counters as *mut Counters).cast(),
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
        (unsafe { QueryProcessCycleTime(GetCurrentProcess(), &mut cycles) } != 0).then_some(cycles)
    }
    #[cfg(not(windows))]
    None
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

/// Sleeps to a millisecond short of `due`, then spins; returns when the last sleep returned.
fn wait_until(due: Instant) -> Option<Instant> {
    let mut woke = None;
    while let Some(left) = due.checked_duration_since(Instant::now()) {
        if left > Duration::from_millis(1) {
            std::thread::sleep(left - Duration::from_millis(1));
            woke = Some(Instant::now());
        } else {
            std::hint::spin_loop();
        }
    }
    woke
}

/// A stepped run's clock: a step of the run's own length, held, so that a table a saved frame
/// learns joins at the frame's hour; playing, it would join past the newest step built, which the
/// frame's rows reach only at a later frame.
pub(crate) fn hold_at_step(entry: &crate::playhead_host::Entry, start: replay_data::Hour, speed: f64) {
    entry.anchor(start, true, speed, false);
    entry.anchor(start, false, speed, false);
}

/// Why the map has no drawn hour: the layer furthest behind, and each layer's name, whether it is
/// drawn, the hour it shows, what it carried and whether its upload was left.
fn warm_up_state(side: &crate::playhead_host::MapSide) -> String {
    let on = side.on_screen();
    let shown: Vec<_> = side.layer_states().into_iter().zip(&side.last_frame().layers).map(|((name, drawn, carried, upload_left), part)| (name, drawn, part.shown, carried, upload_left)).collect();
    format!("the head at {:?}, composing {:?}, the layer furthest behind {:?}; name, drawn, shown, carried, upload left: {shown:?}", on.as_ref().map(|on| on.head), on.as_ref().map(|on| on.composing), on.as_ref().and_then(|on| on.furthest))
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
) -> Result<(), String> {
    let mut result = Ok(());
    let _ = ctx.run_ui(
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
            result = viewer.map_with_state(ui, state, document);
        },
    );
    result
}

/// A `dump` line on standard input prints the hour coordinator's state.
fn watch_dump(reader: &mut Option<std::thread::JoinHandle<()>>, viewer: &Viewer) -> io::Result<()> {
    if reader.is_none()
        && let Some(hours) = &viewer.hours
    {
        let dump = hours.state_dump();
        *reader = Some(
            std::thread::Builder::new()
                .name("playback cost dump".into())
                .spawn_carried(move || {
                    for line in io::stdin().lines() {
                        match line {
                            Ok(line) if line.trim() == "dump" => dump.request(),
                            Ok(_) => {}
                            Err(_) => return,
                        }
                    }
                })?,
        );
    }
    Ok(())
}

fn gpu_counts() -> serde_json::Map<String, Value> {
    let passes: serde_json::Map<String, Value> = viewer_foundation::profiling::take_pass_counts()
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
            viewer_world::map::models::take_counts()
                .map(|(name, count)| (name.to_string(), json!(count))),
        )
        .chain(std::iter::once(("passes".to_string(), Value::Object(passes))))
        .chain(std::iter::once((
            "refusals".to_string(),
            Value::Object(
                viewer_world::map::models::take_refusals()
                    .into_iter()
                    .map(|(name, count)| (name, json!(count)))
                    .collect(),
            ),
        )))
        .collect()
}

/// The window thread's labels and every note, by "family/category": CPU and wall nanoseconds and
/// entries, as the snapshot's main-role rows and notes give them, read without the whole snapshot.
fn own_labels() -> std::collections::HashMap<String, [Option<u64>; 3]> {
    cpu::own_labels()
        .into_iter()
        .map(|label| (format!("{}/{}", label.family, label.category), [Some(label.ns), label.wall_ns, Some(label.calls)]))
        .chain(cpu::notes().into_iter().map(|(category, family, ns, calls)| (format!("{family}/{category}"), [Some(ns), None, Some(calls)])))
        .collect()
}

/// The window thread's labels charged since `before`, which becomes now: each label's CPU, its
/// wall time on top of the thread's labels (the difference is the thread off its CPU there), and
/// how many times it was entered (a label reads the thread's cycle time twice each time; a note,
/// a span's or a lap's, twice too, and notes are every thread's).
fn frame_scopes(
    before: &mut std::collections::HashMap<String, [Option<u64>; 3]>,
) -> (serde_json::Map<String, Value>, serde_json::Map<String, Value>, serde_json::Map<String, Value>) {
    let after = own_labels();
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
        .zip(viewer_foundation::profiling::current_thread_cpu_time())
        .and_then(|(a, b)| b.checked_sub(a))
        .map(|used| used.as_secs_f64() * 1e6)
}

/// The map's device and the window's render state, without a window.
fn render_state() -> io::Result<eframe::egui_wgpu::RenderState> {
    pollster::block_on(async {
        let lifecycle = shader_pipeline::gpu::device_lifecycle();
        let instance = shader_pipeline::gpu::instance();
        let available_adapters = instance.enumerate_adapters(wgpu::Backends::all()).await;
        let adapter =
            viewer_foundation::map::gpu_contract::select_adapter(&available_adapters, None)?;
        let (device, queue) = adapter
            .request_device(&viewer_foundation::map::gpu_contract::map_device_descriptor(&adapter))
            .await
            .map_err(io::Error::other)?;
        drop(lifecycle);
        let target_format = viewer_foundation::map::gpu_contract::OUTPUT_FORMAT;
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

pub(crate) fn measure(
    config: Config,
    mut options: Options,
    mut document: Document,
    hours: u32,
    schedule: Schedule,
    mut monitor: Monitor,
    output: &std::path::Path,
) -> io::Result<Value> {
    // This thread draws the harness's frames, as a map thread draws the window's own.
    viewer_foundation::worker_priority::frame_priority();
    crate::layers::count_in_view(true);
    // Before the device, so it is asked for the per-pass timestamps the record keeps.
    viewer_foundation::view_record::enable();
    let state = render_state()?;
    let info = state.adapter.get_info();
    // A paced run loads icons in the background as the window does; a stepped one settles them.
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
    viewer.graphics.full_resolution = document.game_full_resolution;
    viewer.take_game_options(&document.game_options);
    viewer.layers_only = schedule.layers_only;
    viewer.scene = Some(load_scene(
        &state.device,
        &state.queue,
        &document,
        viewer.graphics,
        &Default::default(),
    )?);
    viewer.scene.as_mut().unwrap().set_graphics(viewer.graphics);
    // The map draws its layers from the recording's playhead, sought here, unless `--no-playhead`.
    if let Some(names) = &schedule.layers {
        crate::playhead_host::register_only(names.clone());
    }
    // Held paused at the paced run's speed: the steps built ahead of the head are those it will play.
    let at = crate::playhead_host::At { branch, hour: start, playing: false, speed: schedule.paced.unwrap_or(0.0) };
    viewer.playhead_map = if schedule.playhead { crate::playhead_host::for_document_at(&document, Some(at)) } else { None };
    if schedule.layers_only && viewer.playhead_map.is_none() {
        return Err(io::Error::other("--layers-only draws the map's layers alone, and the map's playhead is refused (printed above)"));
    }
    if let Some(side) = &viewer.playhead_map {
        document.playback.read_rosters_of(side.entry());
    }
    viewer.playhead_opened = true;
    viewer.dashboard_map = extra_maps::as_dashboard_map();
    let mut extras = extra_maps::Extras::open(&document, viewer.playhead_map.as_ref(), schedule.paced.unwrap_or(1440.0))?;
    // `BATTLE_PLAN_HOVER_FAMILY`: the pointer held on that hover family's first presented target, its figures timed.
    let hovered = super::hover_rows::Family::from_env()?;
    if hovered.is_some() {
        viewer.hover_rows.time();
    }
    viewer
        .scene
        .as_mut()
        .unwrap()
        .settle_builds(schedule.complete_models);
    viewer.playing = schedule.pan.is_none() && !schedule.paused;
    viewer.speed = schedule.paced.unwrap_or(1440.0);
    viewer.animation_seek = true;
    viewer.laid_proof = schedule.laid_proof.map(super::laid_proof::LaidProof::new);
    viewer.camera_step = Some(1.0 / schedule.refresh_hz as f32);
    viewer.last_view = Some(hours::View {
        camera,
        viewport: pixels,
        ui_scale: options.ui_scale.unwrap_or(1.0),
    });
    viewer.capture = Some(Capture {
        path: PathBuf::new(),
        remaining: u32::MAX,
        // A zoom run's camera is the window's own, moved only by its wheel.
        pinned: schedule.zoom.is_none().then_some((pixels, options.camera)),
        exit: false,
        time: Some(0.0),
        step: 0.0,
        frames: 1,
        index: 0,
        paced: schedule.paced.is_some(),
    });
    let ctx = egui::Context::default();
    ctx.set_theme(egui::Theme::Dark);
    viewer_foundation::profiling::cpu_clock_hz();
    drain(&state)?;
    drop(cpu::scope("viewer", Some("other")));
    let before = cpu::snapshot();
    let process_start =
        cpu::process_cpu_ns().ok_or_else(|| io::Error::other("Process CPU unavailable"))?;
    let cycle_start = process_cycles();
    let counts_before = crate::hours::part_counts();
    let cycle_hz = before["tscHz"].as_f64().filter(|hz| *hz > 0.0);
    let wall = Instant::now();
    let mut frame_cpu = Vec::new();
    let mut frame_thread_cpu = Vec::new();
    let mut frame_wall = Vec::new();
    let mut gpu = GpuPeaks::default();
    let mut warning_frames = 0u32;
    let mut dump_reader = None;
    let mut frames = io::BufWriter::new(std::fs::File::create_new(output.join("frames.jsonl"))?);
    let mut labels_before = own_labels();
    let mut reads_before = cpu::cycle_reads();
    shader_pipeline::gpu::stats::mark_window_thread();
    shader_pipeline::gpu::stats::take();
    viewer_foundation::view_record::enable();
    viewer_foundation::view_record::take();
    let map_width = MapSize::of(&document.map).x();
    if schedule.zoom.is_some() {
        viewer.camera = Some(camera.placement());
        viewer.camera_manual = true;
    }
    // Zoom in first; a burst that moved the camera no farther than the last reverses it.
    let mut zoom_in = true;
    let mut burst_height = None::<f32>;
    let center = egui::pos2(pixels[0] as f32 * 0.5, pixels[1] as f32 * 0.5);
    let mut paced_frames = Vec::new();
    let mut layered = Layered::default();
    // The map's layers run on the playhead's clock, as the window's do through its anchors: held at
    // a seek's hour until its reset is shown, and from that frame on at the run's speed.
    // A stepped run plays at its own step, so a step ends on each frame's hour but after a block
    // end, where the held frame is sought.
    // A layers-only run never starts today's pipeline: before the clock starts, the map's
    // layers compose and upload their first picture at the run's first hour.
    if schedule.layers_only {
        viewer.playhead = Some(Playhead { branch, hour: start, fraction: 0.0 });
        let warming = Instant::now();
        viewer.warming = true;
        while !viewer.picture_ready() {
            frame_ui(&ctx, pixels, 0.0, false, Vec::new(), &mut viewer, &state, &mut document).map_err(io::Error::other)?;
            drain(&state)?;
            if let Some(failure) = viewer.playhead_map.as_ref().and_then(|side| side.failure()) {
                return Err(io::Error::other(format!("The map's layers drew no first picture: {failure}")));
            }
            if warming.elapsed() > Duration::from_secs(300) {
                return Err(io::Error::other(format!("The map's layers drew no first picture in five minutes: {}", viewer.playhead_map.as_ref().map_or_else(String::new, warm_up_state))));
            }
        }
        viewer.warming = false;
    }
    if let Some(side) = &viewer.playhead_map {
        let rate = crate::playhead_host::settings().rate;
        match schedule.paced {
            Some(speed) => side.anchor(start, !schedule.paused, speed, false),
            None => hold_at_step(side.entry(), start, f64::from(schedule.step.max(1)) * rate),
        }
    }
    let mut sought = 0u32;
    let mut seed = 0x2545_f491_4f6c_dd1d_u64;
    let mut paced_end_us = 0.0;
    let mut harness_wait_us = 0.0;
    if let Some(speed) = schedule.paced {
        let end = options
            .playback_cost_end
            .expect("the harness sets the run's end");
        let last = document
            .playback
            .range(branch)
            .map_or(end, |(_, last)| last);
        let mut drawn = None::<i32>;
        // The seeks spread over the frames the run would take unsought, at least 15 frames apart.
        let seek_every = (f64::from(end - start) / speed * schedule.refresh_hz / f64::from(schedule.seeks + 1)).max(15.0) as u64;
        let t0 = Instant::now();
        for index in 0u64.. {
            let due_us = paced::due_us(index, schedule.refresh_hz);
            let reached_us = t0.elapsed().as_secs_f64() * 1e6;
            let late = paced::late_start(index, due_us, reached_us);
            let mut woke_us = None;
            if !late && index > 0 {
                let waiting = viewer_foundation::profiling::current_thread_cpu_time();
                woke_us = wait_until(t0 + Duration::from_secs_f64(due_us / 1e6))
                    .map(|woke| woke.duration_since(t0).as_secs_f64() * 1e6);
                harness_wait_us += thread_us(waiting).unwrap_or(0.0);
            }
            let frame_start = if index == 0 { t0 } else { Instant::now() };
            let start_us = frame_start.duration_since(t0).as_secs_f64() * 1e6;
            // Held (`--paused`), the hour stays and the run is `--hours` frames, the camera panning.
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
            if (schedule.paused && index >= u64::from(hours)) || (!schedule.paused && head >= f64::from(end)) {
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
                // The map covered: not drawn, it lets its reader go; the other maps draw on.
                let _cost = cpu::scope("frame.harness", Some("other"));
                viewer.close_playhead();
                let extra = extras.frame(&document, None, index, crate::playhead_host::Head::Paced(hour));
                serde_json::to_writer(&mut frames, &json!({"frame": index, "hour": hour, "head_hour": head, "covered": true, "extra": extra}))?;
                io::Write::write_all(&mut frames, b"\n")?;
                continue;
            }
            if let Some(family) = hovered
                && viewer.options.profile_pointer.is_none()
            {
                viewer.options.profile_pointer = viewer.hover_target(family, pixels);
            }
            let process_before = process_cycles();
            let thread_before = viewer_foundation::profiling::current_thread_cpu_time();
            crate::perf_log::frame_start();
            let perf_frame = crate::perf_log::frame_id();
            let other_cost = cpu::scope("frame.harness", Some("other"));
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
            watch_dump(&mut dump_reader, &viewer)?;
            result.map_err(io::Error::other)?;
            let extra = extras.frame(&document, viewer.playhead_map.as_ref(), index, crate::playhead_host::Head::Paced(hour));
            drain(&state)?;
            let drained = Instant::now();
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
            let thread = thread_us(thread_before);
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
            // The overlay counts from the run's start until its first layout: none of it is on screen.
            layered.overlay = viewer.overlay_hour().or(layered.overlay).or(Some(start));
            let oldest = viewer.playhead_map.as_ref().map(|side| (side.last_frame().oldest, layered.overlay));
            let frame = paced::Frame {
                due_us,
                start_us,
                work_us: submitted.duration_since(frame_start).as_secs_f64() * 1e6,
                gpu_wait_us: drained.duration_since(submitted).as_secs_f64() * 1e6,
                thread_us: thread,
                head_hour: head,
                drawn_hour: now_drawn,
                oldest_hour: now_drawn.map(|drawn| oldest.map_or(drawn, |(layers, overlay)| [Some(drawn), layers, overlay].into_iter().flatten().min().unwrap_or(drawn))),
                late_start: late,
                new_hour: paced::new_hour(drawn, now_drawn),
            };
            drawn = now_drawn.or(drawn);
            let layers = viewer.playhead_map.as_ref().map(|side| {
                if index % 30 == 0 {
                    layered.sample_held(side);
                }
                let phases = side.parts().and_then(|(_, layers)| layers.iter().find_map(|layer| layer.any().downcast_ref::<crate::layers::political::Political>())).map(|political| political.phases);
                layered.political = side.names().iter().position(|&name| name == "political");
                let mut record = layered.frame(side.last_frame(), head, cycle_hz, (index, start_us + wall_us), side.entry().playhead().newest(), phases);
                // The playhead's own state, as last summed: steps kept, and how long its stepper sat behind the head.
                let (kept, _, counters) = side.entry().playhead().held();
                record["ring_steps"] = json!(kept.ring_steps);
                record["room_held_ms"] = json!(counters.room_held_ns as f64 / 1e6);
                record["join_held_ms"] = json!(counters.join_held_ns as f64 / 1e6);
                // The land meshes' worker since the last frame: builds started, products taken, stacks chosen again, selections, camouflages and country contexts made, and its time.
                // The models' worker (ships, raid units, convoys) since the last frame: jobs answered and its time.
                if let Some(models) = crate::layers::units::of(side).and_then(crate::layers::units::Units::take_model_counts) {
                    let parts: serde_json::Map<String, Value> = models.parts.iter().map(|&(name, ns)| (name.to_owned(), json!(ns / 1000))).collect();
                    record["unit_models"] = json!({"jobs": models.jobs, "busy_us": models.busy_ns / 1000, "parts_us": parts});
                }
                if let Some(meshes) = crate::layers::units::of(side).and_then(crate::layers::units::Units::take_mesh_counts) {
                    let parts: serde_json::Map<String, Value> = meshes.parts.iter().map(|&(name, ns)| (name.to_owned(), json!(ns / 1000))).collect();
                    record["unit_meshes"] = json!({"builds": meshes.builds, "products": meshes.products, "chosen": meshes.chosen, "selections": meshes.selections, "looked_up": meshes.looked_up, "entering": meshes.entering, "leaving": meshes.leaving, "waiting": meshes.waiting, "keys": meshes.keys, "keyed_rows": meshes.keyed_rows, "camouflages": meshes.camouflages, "contexts": meshes.contexts, "busy_us": meshes.busy_ns / 1000, "parts_us": parts});
                }
                record
            });
            let record_cost = cpu::scope("frame.record", Some("other"));
            let counts = gpu_counts();
            let (scopes, scope_walls, scope_calls) = frame_scopes(&mut labels_before);
            let cycle_reads = cpu::cycle_reads() - std::mem::replace(&mut reads_before, cpu::cycle_reads());
            // Per map-icon pool presented this frame: [presented, stood still, kept every window,
            // windows laid out, windows kept]. And, per pool, why the frames that laid a window
            // out again did so, by the first change met.
            let mut icon_pools = serde_json::Map::new();
            let mut icon_pools_why = serde_json::Map::new();
            for (name, counted, whys) in viewer_foundation::map::icon_windows::take_counts() {
                if !whys.is_empty() {
                    let whys: serde_json::Map<String, Value> = whys.into_iter().map(|(why, count)| (why.to_owned(), json!(count))).collect();
                    icon_pools_why.insert(name.clone(), Value::Object(whys));
                }
                icon_pools.insert(name, json!(counted));
            }
            // The overlay this frame: what asked it to lay out or draw again (asked.*), what it
            // made (made.*), what it waited for or missed, and what it answered from what it kept.
            let mut overlay_redraws: serde_json::Map<String, Value> = viewer_foundation::map::redraws::take()
                .into_iter()
                .map(|(name, count)| (name.to_owned(), json!(count)))
                .collect();
            // The tables the overlay waits for, by name, in the frame the list changes.
            if let Some(waiting) = crate::app::head_rows::take_waiting() {
                overlay_redraws.insert("waiting.tables.named".to_owned(), json!(waiting.iter().map(|table| format!("{table:?}")).collect::<Vec<_>>()));
            }
            // The rows worker this frame: how far behind the head, its applies and the rows it made.
            let rows_worker = crate::playhead_host::rows::counts::take(viewer.playhead_map.as_ref().and_then(|side| side.entry().map_head()));
            serde_json::to_writer(
                &mut frames,
                &json!({"frame":index,"perf_frame":perf_frame,"hour":hour,"thread_us":thread,
                    "icon_pools":icon_pools,"icon_pools_why":icon_pools_why,"overlay_redraws":overlay_redraws,"rows_worker":rows_worker,
                    "wall_us":wall_us,
                    "camera_height":viewer.camera.map(|placement| placement.height),
                    "camera":camera_record(pinned_camera(&viewer)),
                    "flight":flying.map(|(leg, _)| leg),"view":view_record(),
                    "gpu":counts,"scopes_us":scopes,"scopes_wall_us":scope_walls,"scopes_calls":scope_calls,"cycle_reads":cycle_reads,
                    "due_us":due_us,"start_us":start_us,"work_us":frame.work_us,
                    "gpu_wait_us":frame.gpu_wait_us,"head_hour":head,"drawn_hour":now_drawn,
                    "lag_hours":frame.lag_hours(),"lag_hours_without_cadence_layers_or_overlay":frame.clock_lag_hours(),"oldest_hour":frame.oldest_hour,"new_hour":frame.new_hour,"late_start":late,
                    "reached_us":reached_us,"woke_us":woke_us,"layers":layers,"extra":extra,
                    "hover":viewer.hover_rows.take_timed()}),
            )?;
            io::Write::write_all(&mut frames, b"\n")?;
            io::Write::flush(&mut frames)?;
            drop(record_cost);
            if index % 600 == 0 {
                println!(
                    "Playback cost: frame {index}, head {head:.1}, drawn {now_drawn:?}, to {end}"
                );
            }
            paced_frames.push(frame);
        }
    }
    let stepped = if schedule.paced.is_some() { 0 } else { hours };
    for index in 0..stepped {
        let offset = if schedule.pan.is_some() || schedule.paused {
            0
        } else {
            i64::from(index) * i64::from(schedule.step)
        };
        let hour = i32::try_from(i64::from(start) + offset).map_err(io::Error::other)?;
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
        let thread_before = viewer_foundation::profiling::current_thread_cpu_time();
        let frame_start = Instant::now();
        // Each frame is a window frame of the perf map log, when it is written.
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
        watch_dump(&mut dump_reader, &viewer)?;
        result.map_err(io::Error::other)?;
        let extra = extras.frame(&document, viewer.playhead_map.as_ref(), u64::from(index), crate::playhead_host::Head::Stepped(hour));
        if !viewer.drew_frame || viewer.hour_on_screen(&document) != (branch, hour) {
            let map = viewer.playhead_map.as_ref().map_or_else(|| "no playhead map".to_owned(), crate::playhead_host::MapSide::shown_states);
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
        // The frame's own bookkeeping below is charged to no frame scope.
        drop(_other_cost);
        if let Some(side) = &viewer.playhead_map
            && side.last_frame().drawn != Some(hour)
        {
            return Err(io::Error::other(format!("Hour {hour} was not drawn by the map's layers (they show {:?}; name, drawn, carried, upload left: {:?})", side.last_frame().drawn, side.layer_states())));
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
        serde_json::to_writer(
            &mut frames,
            &json!({"frame":index,"perf_frame":perf_frame,"hour":hour,"thread_us":frame_thread_cpu.last(),
                "wall_us":frame_wall.last(),
                "camera_height":viewer.camera.map(|placement| placement.height),
                "camera":camera_record(pinned_camera(&viewer)),"view":view_record(),
                "gpu":counts,
                "scopes_us":scopes,"scopes_wall_us":scope_walls,"scopes_calls":scope_calls,"cycle_reads":cycle_reads,"extra":extra}),
        )?;
        io::Write::write_all(&mut frames, b"\n")?;
        io::Write::flush(&mut frames)?;
        if index % 100 == 0 {
            println!("Playback cost: {}/{} hours, at {}", index + 1, hours, hour);
        }
    }
    drain(&state)?;
    io::Write::flush(&mut frames)?;
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
    // One read of a thread's cycle time, in wall nanoseconds: a frame's `cycle_reads` times this is
    // what the accounting itself cost that frame.
    report["cycle_read_ns"] = after["cycleReadNs"].clone();
    report["model_build_mode"] = json!(if schedule.complete_models {
        "complete"
    } else {
        "live"
    });
    report["png_frames"] = json!(schedule.pngs);
    report["playhead"] = json!(viewer.playhead_map.is_some());
    report["today_started"] = json!(viewer.hours.is_some());
    report["layers_only"] = json!({"on": schedule.layers_only, "today_started": viewer.hours.is_some()});
    report["extra_maps"] = extras.summary();
    report["parts"] = parts(&report, viewer.playhead_map.as_ref(), &counts_before);
    report["hour_step"] =
        json!((schedule.pan.is_none() && schedule.paced.is_none()).then_some(schedule.step));
    report["pan_units_per_frame"] = json!(schedule.pan);
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
        "surface": null,
        "vertical_sync": false,
        "scope": "The harness renders to offscreen textures with no surface, so no frame waits for vertical sync or a swap chain. gpu_wait_us is the harness's own drain after each submission, which a window does not pay: there the next frame's CPU work overlaps the GPU's, and a frame waits only when the GPU is a whole frame behind (the swap chain's queue is full). A frame's GPU work by pass is each frame's view record `gpu_passes` (microseconds, read back frames later: `frames_ago`), or `not_recorded` on an adapter without timestamps.",
    });
    report["camera"] =
        json!({"center":camera.center,"height":camera.height,"pose":format!("{:?}",camera.pose)});
    report["map_mode"] = json!(options.map_mode.token());
    report["perspective"] = json!(format!("{:?}", options.perspective));
    report["last_frame_warnings"] = json!(options.diagnostics.then_some(viewer.map_warnings));
    report["frames_with_warnings"] = json!(options.diagnostics.then_some(warning_frames));
    report["diagnostic_warnings_collected"] = json!(options.diagnostics);
    report["schedule"] = json!(if schedule.paced.is_some() {
        "Frame k starts at k/refresh_hz s, or at once when the frame before ran past it; the head runs free from --from-hour at --paced hours a second and the run ends when it reaches --from-hour + --hours. Camera and presentation clocks are the wall clock. Each frame draws the newest hour the builders finished at or before the head, as fast play in the window does: nothing waits for an hour, its icons (loaded in the background) or its raid cycle. Same hour coordinator, projections, SceneInput and Scene::render as viewing. GPU drained each frame after submission, timed apart. CPU/wall includes pipeline startup and final GPU drain; static content/device setup is separate; game hours are hours of head travel."
    } else {
        "One frame per requested game hour; camera and presentation clocks advance 1/refresh_hz s per frame. Same hour coordinator, projections, SceneInput and Scene::render as viewing. GPU drained each frame; no vsync, window, OS input, audio-device output or synthetic codec work. CPU/wall includes first-hour pipeline startup and final GPU drain; static content/device setup is separate."
    });
    if let Some(speed) = schedule.paced {
        let mut paced = paced::summary(&paced_frames, schedule.refresh_hz, paced_end_us);
        paced["speed_hours_per_second"] = json!(speed);
        paced["refresh_hz"] = json!(schedule.refresh_hz);
        paced["page_grid"] = json!(hours::page_grid());
        paced["head_hours"] = json!(hours);
        paced["process_cpu_us_per_head_hour"] =
            json!(process_ns as f64 / 1000.0 / f64::from(hours));
        paced["harness_wait_thread_cpu_us"] = json!(harness_wait_us);
        paced["scope"] = json!(
            "Distributions and counts over the frames after warm-up (the frames before the first drawn hour). work_us: frame start to the return of the frame's UI pass, which submits its commands; gpu_wait_us: the drain after it; thread_us: the window thread's CPU over both. late_frames: work over one refresh interval; a late start is unexplained when the frame before finished its work and GPU wait by this frame's due time. drawn_hour is the hour presented and drawn this frame; lag_hours is head_hour minus it. Process CPU includes the harness's wait to each due time (its window-thread CPU is harness_wait_thread_cpu_us). In frames.jsonl, reached_us is when the harness came to the frame after the one before it (late when past due_us), and woke_us when its sleep returned, a millisecond short of due, before it spins (null without a sleep). scopes_us is each window-thread label's own CPU this frame and scopes_wall_us its wall time, charged the same way: a label's wall over its CPU is time off a core inside it. In the layers record, take_ms and take_cpu_ms are the frame's taking of the stepper's steps from the ring they share."
        );
        report["paced"] = paced;
        if let Some(side) = &viewer.playhead_map {
            report["layers"] = layered.summary(side.names());
            let costs = layer_costs(&report["layers"]);
            for part in report["parts"].as_array_mut().into_iter().flatten() {
                let replacing = part["layers"].as_array().cloned().unwrap_or_default();
                part["layer_cost_a_frame"] = Value::Array(replacing.iter().filter_map(|name| costs.get(name.as_str()?).cloned()).collect());
            }
            report["intel_icons"] = crate::layers::intel_icons::summary(side);
            report["names"] = crate::layers::names::summary(side);
            report["counters"] = crate::layers::counters::fed::summary(side);
            report["aircraft"] = crate::layers::aircraft::summary(side);
            report["buildings"] = crate::layers::buildings::summary(side);
            report["snow"] = crate::layers::snow::summary(side);
            report["trains"] = crate::layers::trains::summary(side);
            report["scope"] = layered.scope_summary(side.names());
        }
    }
    if let Some(side) = &viewer.playhead_map {
        let (now, peak, counters) = side.entry().playhead().held();
        let host = side.held();
        report["stepper"] = json!({
            "steps": counters.steps, "stalls": counters.stalls, "stalled_steps": counters.stalled_steps, "piece_waits": counters.piece_waits,
            "seeks": counters.seeks, "joins": counters.joins, "join_holds": counters.join_holds, "catch_ups": counters.catch_ups, "split_steps": counters.split_steps,
            "behind_the_head_ms": {"room": counters.room_held_ns as f64 / 1e6, "joins": counters.join_held_ns as f64 / 1e6, "scope": "Time the stepper was behind the head and built nothing: for want of ring room (the ring's size of steps no consumer had taken), and while a join it had reached waited for its lane."},
            "idle_ms": {"ring_full_ahead": counters.idle_ns[0] as f64 / 1e6, "ring_full_behind": counters.idle_ns[1] as f64 / 1e6, "fetch": counters.idle_ns[2] as f64 / 1e6, "end": counters.idle_ns[3] as f64 / 1e6, "other": counters.idle_ns[4] as f64 / 1e6, "scope": "Time the stepper waited with nothing to build, by why its last build stopped: the ring full ahead of the head (waiting for the frames to take its steps), the ring full behind it, a lane's part not fetched yet, the recording's end, anything else."},
            "playhead_bytes": {"now": now.total, "peak": peak.total, "ring_peak": peak.ring_bytes, "states_peak": peak.state_bytes, "preparing_peak": peak.preparing},
            "host_bytes": {"mirrors": host.mirrors.iter().map(|(_, bytes)| bytes).sum::<usize>(), "layers": host.layers, "pending_steps": host.pending, "states": host.states, "playhead": host.playhead, "total": host.total()},
            "mirrors": side.readers().into_iter().map(|(table, columns, keyed, layers)| json!({"table": format!("{table:?}"), "columns": columns, "keyed": keyed, "layers": layers, "bytes": host.mirrors.iter().find(|(held, _)| *held == table).map(|(_, bytes)| *bytes)})).collect::<Vec<_>>(),
            "scope": "The playhead's counters since it opened (steps built, table steps stalled at a part not fetched and the steps holding any, seeks, joins), its held bytes now and at their peaks, and what the map's side holds at the end. Per frame, the steps' build time is layers.step_ms and frames.jsonl's layers.step_ms.",
        });
    }
    report["rows_worker"] = crate::playhead_host::rows::counts::totals();
    {
        use viewer_foundation::worker_priority as priority;
        report["threads"] = json!({
            "entered": priority::entered().into_iter().map(|entered| json!({"name": entered.name, "class": entered.class.name(), "priority": entered.priority, "threads": entered.threads})).collect::<Vec<_>>(),
            "scope": "Every thread of ours that named its class at its start (worker_priority::enter): its name, class, the priority read back after it was set, and how many threads of that name entered. The playhead's own threads, the pipeline builds and the driver's threads enter no class and run at normal.",
        });
    }
    if let Some(proof) = &viewer.laid_proof {
        let (tally, failure) = proof.finish();
        std::fs::write(output.join("laid-proof.json"), serde_json::to_vec_pretty(&tally)?)?;
        report["laid_proof"] = tally;
        if let Some(failure) = failure {
            return Err(io::Error::other(failure));
        }
    }
    if let Some(political) = viewer.playhead_map.as_ref().and_then(|side| side.parts()).and_then(|(_, layers)| layers.iter().find_map(|layer| layer.any().downcast_ref::<crate::layers::political::Political>())) {
        let [whole, country, naval] = political.uploads;
        report["political_uploads"] = json!({"whole": whole, "country_by_change": country, "naval_by_change": naval});
    }
    Ok(report)
}

/// The map's layers frame by frame in a paced run: each layer's apply, settle and upload, its own
/// cycles, carried entities, and the content lag of what the layers show.
#[derive(Default)]
struct Layered {
    /// Per frame after the first reset is shown: per layer its part, the lag in hours with every
    /// layer and the overlay counted, and the lag of the window clock's drawn hour alone.
    frames: Vec<(Vec<Part>, Option<f64>, Option<f64>)>,
    /// The hour of the rows the overlay on screen was laid out from, as last known; none before
    /// its first layout.
    overlay: Option<i32>,
    mirror_ms: Vec<f64>,
    /// The mirrors' apply by table index, summed over the frames of `mirror_ms`: wall nanoseconds
    /// and the frame thread's own cycles.
    mirror_tables: Vec<(u64, u64)>,
    /// Per frame of `mirror_ms`: the mirrors' apply in the thread's own milliseconds.
    mirror_cpu_ms: Vec<f64>,
    /// Per frame: the build time of the steps it took, on the playhead's threads.
    step_ms: Vec<f64>,
    /// Per frame: the political upload's phases (naval, country fill, plans built, plans written).
    phases: Vec<[f64; 4]>,
    /// The host's mirrors, layers and whole side, held bytes at their peaks.
    held_peak: [usize; 3],
    /// Frames from the run's start until the first reset is swapped in, and those composing since.
    before_shown: u32,
    /// Seeks while playing, each with the marks of its way to the screen.
    seeks: Vec<Sought>,
    /// The political layer's place, which says when a seek's first content is on screen.
    political: Option<usize>,
    /// Per frame: the head, the newest step built, the newest applied, the hour shown whole.
    trace: Vec<(f64, Option<i32>, Option<i32>, Option<i32>)>,
    /// Per frame of `frames`: outside every seek's window (from the seek to its lag within a step).
    outside: Vec<bool>,
    composing: u32,
    /// Per layer, over the frames drawn: entities settled; of those in frames that counted the
    /// view, the entities and those in view; and the frames that settled any.
    scope: Vec<[u64; 5]>,
}

/// One layer's frame: apply, settle and upload, each in wall milliseconds and in the window
/// thread's own milliseconds (none without a cycle rate), and what it carried.
#[derive(Clone, Copy)]
struct Part {
    /// The hours its published product is behind the head.
    lag: Option<f64>,
    wall: [f64; 3],
    cpu: [Option<f64>; 3],
    carried: usize,
    upload_carried: bool,
    /// GPU writes and their bytes in its upload.
    uploads: (u64, u64),
    /// Frames since the oldest change it carries was marked.
    late: u64,
    /// Its settle or upload alone took more than the whole share it was given.
    overran: bool,
    /// Fed the steps ahead, in a frame that applied a step: its product of the applied hour was
    /// ready; and the hand-over's milliseconds on the frame thread.
    ready_at_hour: Option<bool>,
    ahead_ms: f64,
}

const SCOPES: [&str; 3] = ["apply", "settle", "upload"];

/// A seek while playing: the frame that asked it, the hour, and the frames that mark its way to
/// the screen, each with the milliseconds from the start of the asking frame to the mark frame's end.
#[derive(Default)]
struct Sought {
    frame: u64,
    hour: i32,
    asked_us: f64,
    /// The host took the seek's reset step and began composing.
    held: Option<(u64, f64)>,
    /// The compose came back and was swapped in (`frames_to_shown`).
    composed: Option<(u64, f64)>,
    /// The political layer shows its picture made since the swap: the first content.
    first: Option<(u64, f64)>,
    /// Every drawn layer does: all content.
    all: Option<(u64, f64)>,
    /// The first frame after the swap whose lag was within a step.
    within: Option<u64>,
    /// Composes a newer seek discarded meanwhile.
    superseded: u32,
    /// Each layer's compose on the worker, from the swap, and what the playhead's seek took.
    compose_ns: Vec<u64>,
    seeking: Option<playhead::Seeking>,
}

impl Sought {
    fn mark(&self, mark: Option<(u64, f64)>) -> Value {
        mark.map_or(Value::Null, |(at, ms)| json!({"frames": at - self.frame, "ms": ms}))
    }
}

impl Layered {
    /// No seek is waiting for its picture or its lag.
    fn settled(&self) -> bool {
        self.seeks.last().is_none_or(|seek| seek.within.is_some())
    }

    /// No seek is waiting for all its content: the next may be asked. The lag within a step is
    /// not waited for (a worker layer a step behind keeps it at 25 hours in some stretches).
    fn ready_to_seek(&self) -> bool {
        self.seeks.last().is_none_or(|seek| seek.all.is_some())
    }

    fn seek(&mut self, index: u64, hour: i32, asked_us: f64) {
        self.seeks.push(Sought { frame: index, hour, asked_us, ..Sought::default() });
    }

    /// The mirrors' apply by table, dearest first: the mean a frame of `mirror_ms`.
    fn mirror_by_table(&self) -> Vec<Value> {
        let mut tables: Vec<(usize, u64)> = self.mirror_tables.iter().map(|&(ns, _)| ns).enumerate().filter(|(_, ns)| *ns > 0).collect();
        tables.sort_by_key(|&(_, ns)| std::cmp::Reverse(ns));
        tables.into_iter().map(|(table, ns)| json!({"table": format!("{:?}", replay_data::rows::TableId::ALL[table]), "mean_ms": ns as f64 / 1e6 / self.mirror_ms.len().max(1) as f64})).collect()
    }

    /// The host's held bytes now, kept at their peaks.
    fn sample_held(&mut self, side: &crate::playhead_host::MapSide) {
        let held = side.held();
        let mirrors: usize = held.mirrors.iter().map(|(_, bytes)| bytes).sum();
        let layers: usize = held.layers.iter().map(|(_, bytes)| bytes).sum();
        self.held_peak = [self.held_peak[0].max(mirrors), self.held_peak[1].max(layers), self.held_peak[2].max(held.total())];
    }

    /// One frame's record: frame `index`, which ended `end_us` after the run's start.
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
                    wall: [part.apply_ns, part.settle_ns, part.upload_ns].map(|ns| ns as f64 / 1e6),
                    cpu: [part.apply_cycles, part.settle_cycles, part.upload_cycles].map(cpu),
                    carried: part.carried,
                    upload_carried: part.upload_carried,
                    uploads: (part.uploads, part.upload_bytes),
                    late: part.late,
                    overran: part.overran,
                    ready_at_hour: part.ready_at_hour,
                    ahead_ms: part.ahead_ns as f64 / 1e6,
                }
            })
            .collect();
        let clock = frame.drawn.map(|drawn| head - f64::from(drawn));
        let oldest = frame.drawn.and(frame.oldest.or(frame.drawn)).map(|oldest| self.overlay.map_or(oldest, |overlay| overlay.min(oldest)));
        let lag = oldest.map(|oldest| head - f64::from(oldest));
        self.composing += u32::from(frame.composing);
        self.step_ms.push(frame.step_ns as f64 / 1e6);
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
            }
        }
        let named = |table: usize| format!("{:?}", replay_data::rows::TableId::ALL[table]);
        let mirror_tables: serde_json::Map<String, Value> = frame.mirror_tables.iter().enumerate().filter(|(_, (ns, _))| *ns > 0).map(|(table, &(ns, _))| (named(table), json!(ns as f64 / 1e6))).collect();
        let mirror_tables_cpu: serde_json::Map<String, Value> = hz.map(|hz| frame.mirror_tables.iter().enumerate().filter(|(_, (_, cycles))| *cycles > 0).map(|(table, &(_, cycles))| (named(table), json!(cycles as f64 / hz * 1e3))).collect()).unwrap_or_default();
        let mut record = json!({"head": head, "newest": newest, "slice_ms": frame.slice_ns as f64 / 1e6, "upload_share_ms": frame.upload_share_ns as f64 / 1e6, "overran": frame.overran, "late": frame.layers.iter().map(|part| part.late).collect::<Vec<_>>(),
            "political_upload_phases_ms": phases.map(|phases| phases.map(|ns| ns as f64 / 1e6)),
            "steps": frame.steps, "step_ms": frame.step_ns as f64 / 1e6, "applied": frame.applied, "drawn": frame.drawn, "oldest": oldest, "overlay_hour": self.overlay, "lag_hours": lag, "lag_hours_without_cadence_layers_or_overlay": clock, "mirror_ms": frame.mirror_ns as f64 / 1e6, "mirror_cpu_ms": hz.map(|hz| frame.mirror_cycles as f64 / hz * 1e3), "take_ms": frame.take_ns as f64 / 1e6, "take_cpu_ms": hz.map(|hz| frame.take_cycles as f64 / hz * 1e3), "mirror_tables_ms": mirror_tables, "mirror_tables_cpu_ms": mirror_tables_cpu, "cells": frame.cells, "rows": frame.rows,
            "wall_ms": parts.iter().map(|part| part.wall).collect::<Vec<_>>(), "cpu_ms": parts.iter().map(|part| part.cpu).collect::<Vec<_>>(),
            "carried": parts.iter().map(|part| part.carried).collect::<Vec<_>>(), "upload_carried": parts.iter().map(|part| part.upload_carried).collect::<Vec<_>>(),
            "ready": frame.layers.iter().map(|part| part.ready).collect::<Vec<_>>(),
            "shown": frame.layers.iter().map(|part| part.shown).collect::<Vec<_>>(), "layer_lag_hours": parts.iter().map(|part| part.lag).collect::<Vec<_>>(),
            "ready_at_hour": parts.iter().map(|part| part.ready_at_hour).collect::<Vec<_>>(), "ahead_ms": parts.iter().map(|part| part.ahead_ms).collect::<Vec<_>>(),
            "composing": frame.composing, "resets": frame.resets, "swapped": frame.swapped, "superseded": frame.superseded,
            "reset_shown": frame.reset_shown, "layer_reset_shown": frame.layers.iter().map(|part| part.reset_shown).collect::<Vec<_>>(),
            "scope": frame.layers.iter().map(|part| part.scope.map(|scope| json!([scope.settled, scope.in_view]))).collect::<Vec<_>>()});
        record["uploads"] = json!(parts.iter().map(|part| part.uploads.0).collect::<Vec<_>>());
        record["upload_bytes"] = json!(parts.iter().map(|part| part.uploads.1).collect::<Vec<_>>());
        record
    }

    /// Per layer: the entities its settles made, and the share of them the view showed (for sounds,
    /// within hearing), from the frames that counted the view.
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
                    "carried": distribution(self.frames.iter().map(|(parts, ..)| parts[at].carried as f64).collect()),
                    "frames_upload_carried": self.frames.iter().filter(|(parts, ..)| parts[at].upload_carried).count(),
                    "frames_overran": self.frames.iter().filter(|(parts, ..)| parts[at].overran).count(),
                    "late_frames": distribution(self.frames.iter().map(|(parts, ..)| parts[at].late as f64).collect()),
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
            "lag_hours_without_cadence_layers_or_overlay": distribution(self.frames.iter().filter_map(|(.., clock)| *clock).collect()),
            "mirror_ms": distribution(self.mirror_ms.clone()), "mirror_cpu_ms": distribution(self.mirror_cpu_ms.clone()), "step_ms": distribution(self.step_ms.clone()), "layers": layers,
            "mirror_by_table": {
                "tables": self.mirror_by_table(),
                "scope": "The mirrors' apply by table, the mean over the frames of mirror_ms (frames applying nothing count as zero).",
            },
            "political_upload_phases_ms": phases,
            "held_peak_bytes": {"mirrors": self.held_peak[0], "layers": self.held_peak[1], "side": self.held_peak[2], "scope": "sampled every 30 frames"},
            "scope": "Per frame after the layers first showed the sought hour: each layer's apply, settle and upload on the window thread, apart and summed (wall, and the thread's own cycles as milliseconds), entities carried over the budget, and the content lag: the harness head minus the oldest hour on screen, every drawn layer counted (those whose product is made from rows at the head on a worker at the hour those rows stand at) and the overlay (the hour of the rows it was laid out from, overlay_hour). lag_hours_without_cadence_layers_or_overlay is the window clock's drawn hour alone, which leaves both out.",
        })
    }
}

/// Per part of today's hour: who draws it (the layers that replace it whole, or today's
/// builder), the jobs dispatched for it and the hours its layers drew instead in this run, and
/// the CPU today's builders spent on it a head hour from the exclusive scopes.
fn parts(report: &Value, side: Option<&crate::playhead_host::MapSide>, before: &[(&str, u64, u64)]) -> Value {
    const FAMILIES: [(&str, &[&str]); 8] = [
        ("political", &["territory"]),
        ("names", &["world names", "labels"]),
        ("overlay", &["engagement/strikes/text", "raid responses"]),
        ("rails", &["rails"]),
        ("buildings", &["buildings"]),
        ("movers", &["actors/units", "air/naval source choices"]),
        ("fog", &["fog"]),
        ("snow", &["weather"]),
    ];
    let cost = |family: &str| {
        report["exclusive_cpu"].as_array().into_iter().flatten().find(|row| row["name"] == family).and_then(|row| row["exclusive_cpu_us_per_game_hour"].as_f64())
    };
    let drawn: Vec<&str> = side.map(|side| side.drawn_parts().collect()).unwrap_or_default();
    let after = crate::hours::part_counts();
    Value::Array(
        FAMILIES
            .iter()
            .map(|&(part, families)| {
                let today: f64 = families.iter().filter_map(|family| cost(family)).sum();
                let layers: Vec<&str> = crate::playhead_host::REPLACES.iter().find(|(held, _)| *held == part).map(|(_, layers)| layers.to_vec()).unwrap_or_default();
                let (jobs, layered) = after.iter().zip(before).find(|((name, ..), _)| *name == part).map_or((0, 0), |(&(_, jobs, layered), &(_, jobs_before, layered_before))| (jobs - jobs_before, layered - layered_before));
                json!({
                    "part": part, "drawn_by": if drawn.contains(&part) { "layers" } else { "today's builder" }, "layers": if drawn.contains(&part) { layers } else { Vec::new() },
                    "jobs_dispatched": jobs, "hours_drawn_by_layers": layered, "today_cpu_us_per_head_hour": today, "families": families,
                })
            })
            .collect(),
    )
}

/// Each registered layer's record in the paced summary (its apply, settle and upload a frame), by
/// name.
fn layer_costs(layers: &Value) -> std::collections::BTreeMap<String, Value> {
    layers["layers"].as_array().into_iter().flatten().filter_map(|layer| Some((layer["name"].as_str()?.to_owned(), layer.clone()))).collect()
}

#[cfg(test)]
mod seek_record_tests {
    use super::Layered;
    use crate::playhead_host::{Frame, LayerFrame};

    /// One frame of a political and a snow layer: whether the seek's reset was taken and its
    /// compose swapped in, which layers show their picture made since the swap, and the hours.
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

    /// A backward seek: the old picture's hour is past the hour sought, so no hour tells the marks
    /// apart. The reset taken, the swap, the political layer's new picture and every layer's are
    /// each marked once, at their frame, with the milliseconds since the frame that asked; the
    /// next seek waits for all the content, not for the lag within a step.
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
