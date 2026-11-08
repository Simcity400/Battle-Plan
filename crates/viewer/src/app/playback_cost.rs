use super::*;
use job_priority::SpawnCarried as _;
use replay_frame::cpu;
use serde_json::{Value, json};
use std::{
    io,
    sync::atomic::{AtomicBool, Ordering},
};

/// Which hours the frames show: every `step`th hour, or one hour under a camera moving `pan`
/// map units a frame.
#[derive(Clone)]
pub(crate) struct Schedule {
    pub step: u32,
    pub pan: Option<f32>,
    /// Frames whose scene is also written to `<out>/frame-<index>.png`, after their timing.
    pub pngs: Vec<u32>,
}

/// The window thread's charged time by label, and every lap note, for one frame's difference.
fn main_labels(snapshot: &Value) -> std::collections::HashMap<String, u64> {
    snapshot["labels"]
        .as_array()
        .into_iter()
        .flatten()
        .filter(|row| row["role"] == "main")
        .chain(snapshot["notes"].as_array().into_iter().flatten())
        .map(|row| {
            (
                format!(
                    "{}/{}",
                    row["family"].as_str().unwrap_or("-"),
                    row["category"].as_str().unwrap_or("-")
                ),
                row["ns"].as_u64().unwrap_or(0),
            )
        })
        .collect()
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

fn distribution(mut values: Vec<f64>) -> Value {
    if values.is_empty() {
        return json!({"p50":null,"p95":null,"p99":null,"max":null});
    }
    values.sort_by(f64::total_cmp);
    let rank = |p: f64| ((values.len() as f64 * p).ceil() as usize).saturating_sub(1);
    json!({"p50":values[rank(0.5)],"p95":values[rank(0.95)],"p99":values[rank(0.99)],"max":values.last()})
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
    let state = pollster::block_on(async {
        let instance = wgpu::Instance::new(wgpu::InstanceDescriptor::new_without_display_handle());
        let available_adapters = instance.enumerate_adapters(wgpu::Backends::all()).await;
        let adapter =
            viewer_foundation::map::gpu_contract::select_adapter(&available_adapters, None)?;
        let (device, queue) = adapter
            .request_device(&viewer_foundation::map::gpu_contract::map_device_descriptor(&adapter))
            .await
            .map_err(io::Error::other)?;
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
    })?;
    let info = state.adapter.get_info();
    document.icons.lazy.set_background(false);
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
    viewer.scene = Some(load_scene(
        &state.device,
        &state.queue,
        &document,
        viewer.graphics,
        &Default::default(),
    )?);
    viewer.scene.as_mut().unwrap().set_graphics(viewer.graphics);
    // Frames written as PNGs are captures, so their models draw complete.
    viewer
        .scene
        .as_mut()
        .unwrap()
        .settle_builds(!schedule.pngs.is_empty());
    viewer.playing = schedule.pan.is_none();
    viewer.speed = 1440.0;
    viewer.animation_seek = true;
    viewer.last_view = Some(hours::View {
        camera,
        viewport: pixels,
        ui_scale: options.ui_scale.unwrap_or(1.0),
    });
    viewer.capture = Some(Capture {
        path: PathBuf::new(),
        remaining: u32::MAX,
        pinned: Some((pixels, options.camera)),
        exit: false,
        time: Some(0.0),
        step: 0.0,
        frames: 1,
        index: 0,
    });
    let ctx = egui::Context::default();
    ctx.set_theme(egui::Theme::Dark);
    viewer_foundation::profiling::cpu_clock_hz();
    state
        .device
        .poll(wgpu::PollType::Wait {
            submission_index: None,
            timeout: None,
        })
        .map_err(io::Error::other)?;
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
    let mut gpu_allocated = None::<u64>;
    let mut gpu_reserved = None::<u64>;
    let mut warning_frames = 0u32;
    // A `dump` line on standard input prints the hour coordinator's state.
    let mut dump_reader = None;
    let mut frames = io::BufWriter::new(std::fs::File::create_new(output.join("frames.jsonl"))?);
    let mut labels_before = main_labels(&cpu::snapshot());
    shader_pipeline::gpu::stats::mark_window_thread();
    shader_pipeline::gpu::stats::take();
    let map_width = MapSize::of(&document.map).x();
    for index in 0..hours {
        let offset = match schedule.pan {
            Some(_) => 0,
            None => i64::from(index) * i64::from(schedule.step),
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
        viewer.capture.as_mut().unwrap().time = Some(index as f32 / 1440.0);
        let process_before = process_cycles();
        let thread_before = viewer_foundation::profiling::current_thread_cpu_time();
        let frame_start = Instant::now();
        // Each frame is a window frame of the perf map log, when it is written.
        crate::perf_log::frame_start();
        let mut result = Ok(());
        let _other_cost = cpu::scope("frame.harness", Some("other"));
        let _ = ctx.run_ui(
            egui::RawInput {
                screen_rect: Some(egui::Rect::from_min_size(
                    egui::Pos2::ZERO,
                    egui::vec2(pixels[0] as f32, pixels[1] as f32),
                )),
                time: Some(f64::from(index) / 1440.0),
                focused: false,
                ..Default::default()
            },
            |ui| {
                result = viewer.map_with_state(ui, &state, &mut document);
            },
        );
        if dump_reader.is_none()
            && let Some(hours) = &viewer.hours
        {
            let dump = hours.state_dump();
            dump_reader = Some(
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
        result.map_err(io::Error::other)?;
        if !viewer.drew_frame
            || (document.playback.branch(), document.playback.hour()) != (branch, hour)
        {
            return Err(io::Error::other(format!(
                "Hour {hour} was not submitted; no missing hour is counted as played"
            )));
        }
        state
            .device
            .poll(wgpu::PollType::Wait {
                submission_index: None,
                timeout: None,
            })
            .map_err(io::Error::other)?;
        crate::perf_log::frame_end();
        frame_wall.push(frame_start.elapsed().as_secs_f64() * 1e6);
        if let Some(ns) = process_before
            .zip(process_cycles())
            .and_then(|(a, b)| b.checked_sub(a))
            .zip(cycle_hz)
        {
            frame_cpu.push(ns.0 as f64 / ns.1 * 1e6);
        }
        if let Some(used) = thread_before
            .zip(viewer_foundation::profiling::current_thread_cpu_time())
            .and_then(|(a, b)| b.checked_sub(a))
        {
            frame_thread_cpu.push(used.as_secs_f64() * 1e6);
        }
        if let Some(report) = state.device.generate_allocator_report() {
            gpu_allocated = Some(gpu_allocated.unwrap_or(0).max(report.total_allocated_bytes));
            gpu_reserved = Some(gpu_reserved.unwrap_or(0).max(report.total_reserved_bytes));
        }
        warning_frames += u32::from(!viewer.map_warnings.is_empty());
        // The frame's own bookkeeping below is charged to no frame scope.
        drop(_other_cost);
        if schedule.pngs.contains(&index) {
            crate::render::write_png(
                viewer.scene.as_ref().expect("drawn above"),
                &state.device,
                &state.queue,
                &output.join(format!("frame-{index}.png")),
            )?;
        }
        let counts = shader_pipeline::gpu::stats::take();
        let labels_after = main_labels(&cpu::snapshot());
        let mut scopes = serde_json::Map::new();
        for (label, ns) in &labels_after {
            let spent = ns.saturating_sub(labels_before.get(label).copied().unwrap_or(0));
            if spent > 0 {
                scopes.insert(label.clone(), json!(spent / 1000));
            }
        }
        labels_before = labels_after;
        serde_json::to_writer(
            &mut frames,
            &json!({"frame":index,"hour":hour,"thread_us":frame_thread_cpu.last(),
                "wall_us":frame_wall.last(),
                "gpu":shader_pipeline::gpu::stats::NAMES.iter().zip(counts)
                    .map(|(name, count)| (name.to_string(), json!(count)))
                    .collect::<serde_json::Map<_, _>>(),
                "scopes_us":scopes}),
        )?;
        io::Write::write_all(
            &mut frames,
            b"
",
        )?;
        if index % 100 == 0 {
            println!("Playback cost: {}/{} hours, at {}", index + 1, hours, hour);
        }
    }
    state
        .device
        .poll(wgpu::PollType::Wait {
            submission_index: None,
            timeout: None,
        })
        .map_err(io::Error::other)?;
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
    report["hour_step"] = json!(schedule.pan.is_none().then_some(schedule.step));
    report["pan_units_per_frame"] = json!(schedule.pan);
    report["branch"] = json!(branch);
    report["from_hour"] = json!(start);
    report["game_hours"] = json!(hours);
    report["wall_seconds"] = json!(seconds);
    report["game_hours_per_wall_second"] = json!(f64::from(hours) / seconds);
    report["process_cpu_us_per_game_hour"] = json!(process_ns as f64 / 1000.0 / f64::from(hours));
    report["frame_cpu_us"] = if frame_cpu.len() == hours as usize {
        distribution(frame_cpu)
    } else {
        distribution(Vec::new())
    };
    report["frame_cpu_scope"] = json!(
        "Whole-process QueryProcessCycleTime, converted with the same calibrated TSC rate as exclusive thread scopes; includes unregistered process threads. No scaling to force agreement with GetProcessTimes."
    );
    report["frame_thread_cpu_us"] = if frame_thread_cpu.len() == hours as usize {
        distribution(frame_thread_cpu)
    } else {
        distribution(Vec::new())
    };
    report["frame_wall_us"] = distribution(frame_wall);
    report["memory"] = json!({"working_set_peak_bytes":memory.map(|m| m.working_set_peak),
        "private_peak_bytes":memory.map(|m| m.private_peak),"commit_peak_bytes":memory.map(|m| m.commit_peak),
        "scope":"OS working-set/commit lifetime peaks include startup; private sampled every 10 ms from content setup through final drain"});
    report["gpu"] = json!({"adapter":info.name,"backend":format!("{:?}",info.backend),
        "allocated_peak_bytes":gpu_allocated,"reserved_peak_bytes":gpu_reserved,"dedicated_resident_peak_bytes":null,
        "scope":"wgpu allocator report sampled after every submitted frame, not adapter-wide residency; unsupported reports are null"});
    report["viewport"] = json!(pixels);
    report["camera"] =
        json!({"center":camera.center,"height":camera.height,"pose":format!("{:?}",camera.pose)});
    report["map_mode"] = json!(options.map_mode.token());
    report["perspective"] = json!(format!("{:?}", options.perspective));
    report["last_frame_warnings"] = json!(options.diagnostics.then_some(viewer.map_warnings));
    report["frames_with_warnings"] = json!(options.diagnostics.then_some(warning_frames));
    report["diagnostic_warnings_collected"] = json!(options.diagnostics);
    report["schedule"] = json!(
        "One frame per consecutive game hour; clocks advance 1/1440 s per hour. Same hour coordinator, projections, SceneInput and Scene::render as viewing. GPU drained each frame; no vsync, window, OS input, PNG readback, audio-device output or synthetic codec work. CPU/wall includes first-hour pipeline startup and final GPU drain; static content/device setup is separate."
    );
    Ok(report)
}
