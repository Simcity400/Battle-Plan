use super::super::{Capture, Playhead, Viewer};
use super::{drain, frame_ui, memory, render_state, wait_until};
use crate::Options;
use eframe::egui;
use content_reader_layers::content::visual::Choice;
use serde_json::{Value, json};
use std::{
    io,
    path::{Path, PathBuf},
    sync::{Arc, Mutex},
    time::{Duration, Instant},
};

pub(crate) struct Toggle {
    pub sets: [Vec<Choice>; 2],
    pub toggles: u32,
    pub refresh_hz: f64,
    pub settle_frames: u32,
    pub timeout: Duration,
    pub fresh: bool,
}

struct Totals {
    programs: (u64, u64),
    builds: u64,
    process_ns: Option<u64>,
}

impl Totals {
    fn now() -> Self {
        Self {
            programs: shader_pipeline::cache::totals(),
            builds: shader_pipeline::gpu::stats::builds_total(),
            process_ns: replay_frame::cpu::process_cpu_ns(),
        }
    }
}

fn label(set: usize) -> &'static str {
    ["A", "B"][set]
}

fn names(set: &[Choice]) -> Vec<&str> {
    set.iter().map(|choice| choice.name.as_str()).collect()
}

fn ids(set: &[Choice]) -> std::collections::BTreeSet<&str> {
    set.iter().map(|choice| choice.id.as_str()).collect()
}

pub(super) fn viewer(
    config: &dashboard_core::config::Config,
    options: &Options,
    state: &eframe::egui_wgpu::RenderState,
    service: &Arc<dashboard::Service>,
) -> io::Result<Viewer> {
    let mut viewer = Viewer::create(
        config.clone(),
        options.clone(),
        None,
        Arc::new(Mutex::new(None)),
        None,
        Some((state.device.clone(), state.queue.clone())),
    );
    let pool = crate::worker_pool::WorkerPool::new()?;
    viewer.dropper = Some(Arc::clone(pool.dropper()));
    viewer.worker_pool = Some(Arc::clone(&pool));
    viewer.shared_pool = Some(pool);
    viewer.harness_service = Some(Arc::clone(service));
    viewer.playing = false;
    viewer.capture = Some(Capture {
        path: PathBuf::new(),
        remaining: u32::MAX,
        pinned: Some((options.size, options.camera)),
        exit: false,
        time: Some(0.0),
        step: 0.0,
        frames: 1,
        index: 0,
        paced: true,
        export: None,
        video: None,
    });
    Ok(viewer)
}

fn save(key: &str, set: &[Choice]) -> io::Result<()> {
    let mut store = dashboard_core::map_settings::Store::open();
    store.save(
        key.to_owned(),
        dashboard_core::protocol::CampaignMapSettings {
            visual_mods: Some(set.to_vec()),
            ..Default::default()
        },
    )
    .map(drop)
}

pub(super) fn frame(
    viewer: &mut Viewer,
    ctx: &egui::Context,
    state: &eframe::egui_wgpu::RenderState,
    service: &dashboard::Service,
    pixels: [u32; 2],
    seconds: f64,
) -> io::Result<Option<Vec<Choice>>> {
    viewer.follow_campaign_preferences(service, ctx);
    viewer.follow_visual_checks(ctx, None);
    viewer.poll(ctx);
    viewer.follow_shown_maps(ctx, None);
    if let Some(capture) = viewer.capture.as_mut() {
        capture.time = Some(seconds as f32);
    }
    if viewer.scene.is_none() {
        viewer.release_audio();
        viewer.drew_frame = false;
        return Ok(None);
    }
    let Some(mut document) = viewer.document.take() else {
        viewer.release_audio();
        viewer.drew_frame = false;
        return Ok(None);
    };
    let result = frame_ui(ctx, pixels, seconds, false, Vec::new(), viewer, state, &mut document);
    viewer.after_frame_with_visual_mods(&document, result.as_ref().err());
    let mods = document.visual_mods.clone().unwrap_or_default();
    viewer.document = Some(document);
    result.map_err(io::Error::other)?;
    drain(state)?;
    Ok(Some(mods))
}

fn correct(viewer: &Viewer, drawn: Option<&[Choice]>, target: &[Choice]) -> bool {
    drawn.is_some_and(|mods| ids(mods) == ids(target))
        && viewer.drew_frame
        && viewer.playhead.is_some_and(|head| {
            viewer.presentation_ready(
                Playhead {
                    branch: head.branch,
                    hour: head.hour,
                    fraction: 0.0,
                },
                false,
            )
        })
}

fn layers_alone(viewer: &Viewer, drawn: Option<&[Choice]>, target: &[Choice]) -> bool {
    drawn.is_some_and(|mods| ids(mods) == ids(target))
        && viewer.drew_frame
        && viewer.playhead.zip(viewer.playhead_map.as_ref()).is_some_and(|(head, side)| {
            side.on_screen().is_some_and(|on| super::super::layers_show(&on, head.hour, false))
        })
}

fn waiting(viewer: &Viewer) -> String {
    let layers = viewer
        .playhead_map
        .as_ref()
        .map(|side| format!("{:?}", side.with_side(crate::playhead_host::MapSide::layer_states)));
    format!(
        "loading {}, building_scene {}, document {}, scene {}, drew {}, head {:?}, layers (name, drawn, carried, upload left) {layers:?}, error {:?}",
        viewer.loading.is_some(),
        viewer.building_scene(),
        viewer.document.is_some(),
        viewer.scene.is_some(),
        viewer.drew_frame,
        viewer.playhead.map(|head| head.hour),
        viewer.reported_error(),
    )
}

fn layers_seen(viewer: &Viewer) -> Value {
    let Some(side) = viewer.playhead_map.as_ref() else {
        return Value::Null;
    };
    let frame = side.last_frame();
    side.with_side(crate::playhead_host::MapSide::layer_states)
        .iter()
        .zip(&frame.layers)
        .map(|((name, drawn, _), part)| {
            json!({"layer": name, "drawn": drawn, "shown": part.shown, "ready": part.ready, "dirty": part.dirty})
        })
        .collect()
}

#[expect(clippy::too_many_arguments, reason = "one open or toggle's whole input")]
fn until_correct(
    viewer: &mut Viewer,
    ctx: &egui::Context,
    state: &eframe::egui_wgpu::RenderState,
    service: &dashboard::Service,
    run: &Toggle,
    target: &[Choice],
    requested: Instant,
    before: Totals,
    output: &Path,
    name: &str,
) -> io::Result<Value> {
    let pixels = viewer.options.size;
    let mut frames = Vec::new();
    let (mut opened, mut installed, mut first_drawn, mut ready) = (None, None, None, None);
    let mut layers_ready = None;
    let mut after_ready = 0u32;
    let mut work_after_ready = Vec::new();
    let mut layers_first = Value::Null;
    for index in 0u64.. {
        let due = crate::playback_cost::frame_due(requested, index as f64 / run.refresh_hz)?;
        wait_until(due);
        let started = Instant::now();
        let at_ms = started.duration_since(requested).as_secs_f64() * 1e3;
        let loading_before = viewer.loading.is_some() || viewer.building_scene();
        let drawn = frame(viewer, ctx, state, service, pixels, at_ms / 1e3)?;
        let work_ms = started.elapsed().as_secs_f64() * 1e3;
        if opened.is_none() && (viewer.loading.is_some() || viewer.building_scene() || loading_before) {
            opened = Some(at_ms);
        }
        let installed_current = viewer.document.as_ref().is_some_and(|document| {
            ids(document.visual_mods.as_deref().unwrap_or_default()) == ids(target)
        });
        let current = drawn.as_deref().is_some_and(|mods| ids(mods) == ids(target));
        if installed.is_none() && opened.is_some() && installed_current {
            installed = Some(at_ms);
        }
        if first_drawn.is_none() && current && viewer.drew_frame {
            first_drawn = Some(at_ms);
        }
        if layers_ready.is_none() && layers_alone(viewer, drawn.as_deref(), target) {
            layers_ready = Some(at_ms);
        }
        let state_name = match (&drawn, viewer.loading.is_some() || viewer.building_scene()) {
            (None, true) => "loading",
            (None, false) => "no document",
            (Some(_), _) if !current => "old look",
            (Some(_), _) if !viewer.drew_frame => "not drawn",
            (Some(_), _) if ready.is_some() => "settling",
            (Some(_), _) if correct(viewer, drawn.as_deref(), target) => "correct",
            (Some(_), _) => "drawn, not settled",
        };
        frames.push(json!({"frame": index, "at_ms": at_ms, "work_ms": work_ms, "state": state_name}));
        if ready.is_none() && state_name == "correct" {
            ready = Some(at_ms);
            layers_first = layers_seen(viewer);
            crate::render::write_png(
                viewer.scene.as_ref().expect("a frame was drawn"),
                &state.device,
                &state.queue,
                &output.join(format!("{name}-first.png")),
            )?;
        } else if ready.is_some() {
            work_after_ready.push(work_ms);
            after_ready += 1;
            if after_ready >= run.settle_frames {
                break;
            }
        }
        if ready.is_none() && started.duration_since(requested) > run.timeout {
            return Err(io::Error::other(format!(
                "{name}: no correct frame in {} s: {}",
                run.timeout.as_secs(),
                waiting(viewer)
            )));
        }
    }
    if viewer.drew_frame {
        crate::render::write_png(
            viewer.scene.as_ref().expect("a frame was drawn"),
            &state.device,
            &state.queue,
            &output.join(format!("{name}-settled.png")),
        )?;
    }
    let (stages, counts) = viewer_runtime::load_record::take();
    let after = Totals::now();
    let gpu: serde_json::Map<String, Value> = shader_pipeline::gpu::stats::NAMES
        .iter()
        .zip(shader_pipeline::gpu::stats::take())
        .filter(|(_, count)| *count > 0)
        .map(|(name, count)| (name.to_string(), json!(count)))
        .collect();
    let memory = memory();
    Ok(json!({
        "name": name,
        "mods": names(target),
        "ms": {
            "load_started": opened,
            "installed": installed,
            "first_drawn": first_drawn,
            "layers_ready": layers_ready,
            "first_correct": ready,
        },
        "stages": stages.iter().map(|stage| json!({
            "name": stage.name, "thread": stage.thread, "start_ms": stage.start_us / 1e3,
            "wall_ms": stage.wall_us / 1e3, "cpu_ms": stage.cpu_us.map(|us| us / 1e3),
        })).collect::<Vec<_>>(),
        "counts": counts.iter().map(|count| json!({"kind": count.kind, "n": count.n, "bytes": count.bytes})).collect::<Vec<_>>(),
        "programs_compiled": after.programs.0 - before.programs.0,
        "programs_from_disk": after.programs.1 - before.programs.1,
        "pipelines_and_programs_built": after.builds - before.builds,
        "gpu": gpu,
        "process_cpu_ms": after.process_ns.zip(before.process_ns).map(|(a, b)| a.saturating_sub(b) as f64 / 1e6),
        "private_bytes_after": memory.map(|memory| memory.private_peak),
        "working_set_peak_bytes": memory.map(|memory| memory.working_set_peak),
        "settle_work_ms": crate::playback_cost::distribution(work_after_ready),
        "frames": frames,
        "layers_first": layers_first,
        "layers_settled": layers_seen(viewer),
        "pictures": {"first": format!("{name}-first.png"), "settled": format!("{name}-settled.png")},
    }))
}

#[expect(clippy::too_many_arguments, reason = "one open's whole input")]
fn open(
    config: &dashboard_core::config::Config,
    options: &Options,
    state: &eframe::egui_wgpu::RenderState,
    run: &Toggle,
    path: &Path,
    set: &[Choice],
    output: &Path,
    name: &str,
) -> io::Result<(Value, Viewer, Arc<dashboard::Service>, egui::Context, String)> {
    let service = dashboard::Service::new(Arc::new(|_| true))?;
    service.configure_content(config, options.allow_unverified)?;
    let revision = dashboard_core::protocol::replay_revision(path)?;
    let source = service
        .open_map(path, Some(&revision))
        .map_err(io::Error::other)?;
    let key = dashboard_core::map_settings::campaign_key(&source.replay());
    save(&key, set)?;
    service.reload_map_settings();
    let mut viewer = viewer(config, options, state, &service)?;
    let ctx = egui::Context::default();
    ctx.set_theme(egui::Theme::Dark);
    let requested = Instant::now();
    viewer_runtime::load_record::begin(requested);
    shader_pipeline::gpu::stats::take();
    let before = Totals::now();
    viewer.open(path.to_path_buf(), &ctx, Some(source.key().revision.clone()));
    let record = until_correct(&mut viewer, &ctx, state, &service, run, set, requested, before, output, name)?;
    Ok((record, viewer, service, ctx, key))
}

pub(crate) fn measure(
    config: dashboard_core::config::Config,
    mut options: Options,
    path: PathBuf,
    run: Toggle,
    output: &Path,
) -> io::Result<Value> {
    viewer_runtime::worker_priority::frame_priority();
    shader_pipeline::gpu::stats::mark_window_thread();
    let state = render_state()?;
    let info = state.adapter.get_info();
    options.window_dump = false;
    options.headless_dump = false;
    let wall = Instant::now();
    let mut records = Vec::new();
    let (record, mut viewer, service, ctx, key) =
        open(&config, &options, &state, &run, &path, &run.sets[0], output, "open-A")?;
    println!("open A: first correct frame at {:?} ms", record["ms"]["first_correct"]);
    records.push(record);
    let mut pairs = Vec::new();
    for toggle in 1..=run.toggles {
        let set = (toggle % 2) as usize;
        let target = &run.sets[set];
        let name = format!("toggle-{toggle}-{}", label(set));
        save(&key, target)?;
        let requested = Instant::now();
        viewer_runtime::load_record::begin(requested);
        shader_pipeline::gpu::stats::take();
        let before = Totals::now();
        viewer.check_visual_mods(true, &ctx);
        let record = until_correct(&mut viewer, &ctx, &state, &service, &run, target, requested, before, output, &name)?;
        println!("{name}: first correct frame at {:?} ms", record["ms"]["first_correct"]);
        records.push(record);
        pairs.push((name, set));
    }
    drop(viewer);
    drop(service);
    let mut fresh = Vec::new();
    if run.fresh && run.toggles > 0 {
        let (record, viewer, service, _, _) =
            open(&config, &options, &state, &run, &path, &run.sets[1], output, "fresh-B")?;
        println!("fresh B: first correct frame at {:?} ms", record["ms"]["first_correct"]);
        fresh.push(record);
        drop(viewer);
        drop(service);
    }
    let compare: Vec<Value> = pairs
        .iter()
        .filter(|(_, set)| *set == 1 && run.fresh)
        .chain(pairs.iter().filter(|(_, set)| *set == 0))
        .map(|(name, set)| {
            let fresh = if *set == 0 { "open-A" } else { "fresh-B" };
            json!({"toggle": format!("{name}-settled.png"), "fresh": format!("{fresh}-settled.png"),
                "toggle_first": format!("{name}-first.png"), "fresh_first": format!("{fresh}-first.png")})
        })
        .collect();
    Ok(json!({
        "schema": 1,
        "mode": "visual-toggle",
        "sets": {"A": names(&run.sets[0]), "B": names(&run.sets[1]),
            "A_ids": run.sets[0].iter().map(|choice| &choice.id).collect::<Vec<_>>(),
            "B_ids": run.sets[1].iter().map(|choice| &choice.id).collect::<Vec<_>>()},
        "toggles": run.toggles,
        "refresh_hz": run.refresh_hz,
        "settle_frames": run.settle_frames,
        "adapter": {"name": info.name, "backend": format!("{:?}", info.backend), "driver": info.driver},
        "viewport": options.size,
        "camera": options.camera.map(|camera| format!("{camera:?}")),
        "wall_seconds": wall.elapsed().as_secs_f64(),
        "records": records,
        "fresh": fresh,
        "compare": compare,
        "scope": "Each record runs from the request (the map settings saved and the host's MapSettingsSaved handled) to the first correct frame: the toggled set's document, drawn at the held hour with every layer settled (presentation_ready). ms.load_started is the frame that reopened the map; installed, the first frame of the new document; first_drawn, its first drawn frame. Stages are inclusive wall and the stage's own thread CPU; a parallel wave's jobs overlap. counts are what the load reused or made; programs_* are DXC compiles and on-disk cache reads; gpu sums the shader pipeline's counters over the record. The open-A record is a fresh open with set A and fresh-B one with set B in a new map of the same process (disk caches warm): compare each toggle's pictures with the fresh open of the same set.",
    }))
}
