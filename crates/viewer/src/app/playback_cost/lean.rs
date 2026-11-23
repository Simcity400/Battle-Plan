use super::*;

pub(super) struct Run<'a> {
    pub ctx: &'a egui::Context,
    pub viewer: &'a mut Viewer,
    pub document: &'a mut Document,
    pub state: &'a eframe::egui_wgpu::RenderState,
    pub surface: &'a mut crate::renderer_host::WindowSurface,
    pub window: &'a crate::host::Window,
    pub schedule: &'a Schedule,
    pub pixels: [u32; 2],
    pub branch: replay_data::BranchId,
    pub start: i32,
    pub end: i32,
    pub last: i32,
    pub hours: u32,
}

#[derive(Clone, Copy)]
struct Started {
    us: f64,
    cpu_us: f64,
    leg: Option<&'static str>,
    drawn: Option<i32>,
}

const SETTLE_US: f64 = 2e6;

const STEADY_LEGS: [&str; 3] = ["tilted", "pan", "turn"];

pub(super) fn run(run: Run<'_>) -> io::Result<Value> {
    let Run { ctx, viewer, document, state, surface, window, schedule, pixels, branch, start, end, last, hours } = run;
    let speed = schedule.paced.expect("a lean run is paced");
    let map_width = MapSize::of(&document.map).x();
    let mut frames = Vec::<Started>::with_capacity(1 << 16);
    drain(state)?;
    if let Some(side) = &viewer.playhead_map {
        side.anchor(start, !schedule.paused, speed, false);
    }
    let mut limited = viewer_runtime::frame_limit::Grid::default();
    let mut drawn_before = None::<i32>;
    let t0 = Instant::now();
    for index in 0u64.. {
        let waiting = limited.until(Instant::now());
        if !waiting.is_zero() {
            std::thread::sleep(waiting);
        }
        let now = Instant::now();
        limited.take(now);
        let start_us = now.duration_since(t0).as_secs_f64() * 1e6;
        let cpu_us = viewer_runtime::profiling::current_thread_cpu_time().map_or(f64::NAN, |cpu| cpu.as_secs_f64() * 1e6);
        let head = match &viewer.playhead_map {
            _ if schedule.paused => f64::from(start),
            Some(side) => f64::from(side.head(now)),
            None => paced::head(start, speed, start_us / 1e6, last),
        };
        if (schedule.paused && index >= u64::from(hours)) || (!schedule.paused && head >= f64::from(end)) {
            break;
        }
        let flying = schedule.flight.map(|flight| flight.at(start_us / 1e6, map_width));
        if let Some((_, placement)) = flying {
            viewer.capture.as_mut().unwrap().pinned = Some((pixels, Some(placement)));
        }
        let (hour, fraction) = paced::split(head);
        viewer.playhead = Some(Playhead { branch, hour, fraction });
        viewer.capture.as_mut().unwrap().time = Some((start_us / 1e6) as f32);
        let output = frame_ui(ctx, pixels, start_us / 1e6, false, Vec::new(), viewer, state, document).map_err(io::Error::other)?;
        if !present(surface, window, state, viewer, ctx, output, pixels)?.presented {
            return Err(io::Error::other(format!(
                "Frame {index} was not presented (the window hidden, occluded or its swap chain lost): a lean run times presented frames only"
            )));
        }
        let drawn = viewer.rendered.filter(|rendered| viewer.drew_frame && rendered.branch == branch).map(|rendered| rendered.hour);
        if drawn.is_none()
            && let Some(previous) = drawn_before
        {
            return Err(io::Error::other(format!("Frame {index} drew no map after hour {previous} was on screen")));
        }
        drawn_before = drawn.or(drawn_before);
        frames.push(Started { us: start_us, cpu_us, leg: flying.map(|(leg, _)| leg), drawn });
        if index % 600 == 0 {
            println!("Lean: frame {index}, head {head:.1}, drawn {drawn:?}, to {end}");
        }
    }
    let wall_seconds = t0.elapsed().as_secs_f64();
    drain(state)?;
    let mut report = summary(&frames, schedule.flight.is_some());
    report["wall_seconds"] = json!(wall_seconds);
    report["viewport"] = json!(pixels);
    report["present_mode"] = json!(format!("{:?}", state.surface_config.present_mode));
    report["attached_display_hz"] = json!(content_reader_settings::settings::display_refresh_hz(Some(window.hwnd())));
    report["adapter"] = json!(state.adapter.get_info().name);
    Ok(report)
}

fn summary(frames: &[Started], flying: bool) -> Value {
    let first = frames.iter().find(|frame| frame.drawn.is_some()).map(|frame| frame.us);
    let settled: Vec<(Option<&str>, f64, f64)> = frames
        .windows(2)
        .filter(|pair| first.is_some_and(|first| pair[0].us >= first + SETTLE_US))
        .map(|pair| (pair[0].leg, pair[1].us - pair[0].us, pair[1].cpu_us - pair[0].cpu_us))
        .collect();
    let steady: Vec<_> = settled
        .iter()
        .filter(|(leg, ..)| !flying || leg.is_some_and(|leg| STEADY_LEGS.contains(&leg)))
        .collect();
    let thread = figures(steady.iter().map(|&&(_, _, cpu)| cpu).filter(|cpu| cpu.is_finite()).collect());
    let steady = figures(steady.iter().map(|&&(_, us, _)| us).collect());
    let legs: serde_json::Map<String, Value> = crate::playback_cost::flight::LEGS
        .iter()
        .filter(|_| flying)
        .map(|&(name, _)| {
            let intervals = settled.iter().filter(|(leg, ..)| *leg == Some(name)).map(|&(_, us, _)| us).collect();
            (name.to_owned(), figures(intervals))
        })
        .collect();
    let mut report = json!({
        "lean": true,
        "frames": frames.len(),
        "legs": legs,
        "frame_thread_cpu": thread,
        "frames_us": frames.iter().map(|frame| frame.us.round()).collect::<Vec<_>>(),
        "drawn_hours": frames.iter().map(|frame| frame.drawn).collect::<Vec<_>>(),
        "scope": "Intervals between consecutive frame starts, each the frame that began it, counted from 2 s after the first drawn frame and, when the camera flies, on the tilted, pan and turn legs that frame stood on (each leg also apart, in legs). The camera flies on the wall clock. p90 and p99 are the interval's percentiles, so their fps is the rate the slower frames reach. Frames are paced as a map paces itself: on the frame limit's grid, waiting blocked, or at once without a limit. The run keeps no view record, CPU accounting or frame records, and no frame drains the GPU: the drains before the first frame and after the last stand outside every interval. A frame that draws no map once one was drawn, or that is not presented, ends the run.",
    });
    for (key, value) in steady.as_object().into_iter().flatten() {
        report[key] = value.clone();
    }
    report
}

fn figures(mut intervals: Vec<f64>) -> Value {
    intervals.sort_by(f64::total_cmp);
    let figure = |quantile: f64| match intervals.len().checked_sub(1) {
        Some(last) => {
            let us = intervals[((last as f64) * quantile).round() as usize];
            json!({"interval_ms": us / 1e3, "fps": 1e6 / us})
        }
        None => Value::Null,
    };
    json!({"intervals_counted": intervals.len(), "median": figure(0.5), "p90": figure(0.9), "p99": figure(0.99)})
}

pub(super) fn present(
    surface: &mut crate::renderer_host::WindowSurface,
    window: &crate::host::Window,
    state: &eframe::egui_wgpu::RenderState,
    viewer: &Viewer,
    ctx: &egui::Context,
    output: egui::FullOutput,
    pixels: [u32; 2],
) -> io::Result<crate::renderer_host::Painted> {
    let clear = <Viewer as eframe::App>::clear_color(viewer, &ctx.style_of(egui::Theme::Dark).visuals);
    let primitives = ctx.tessellate(output.shapes, output.pixels_per_point);
    let mut textures = output.textures_delta;
    surface
        .paint(
            state,
            state.surface_config,
            window.shown().then_some(pixels),
            output.pixels_per_point,
            clear,
            &primitives,
            &mut textures,
        )
        .map_err(io::Error::other)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn frame(ms: f64, leg: &'static str, drawn: bool) -> Started {
        Started { us: ms * 1e3, cpu_us: ms * 5e2, leg: Some(leg), drawn: drawn.then_some(1) }
    }

    #[test]
    fn intervals_count_from_two_seconds_after_the_first_drawn_frame_on_the_steady_legs() {
        let mut frames = vec![frame(0.0, "tilted", false), frame(100.0, "tilted", true)];
        frames.extend((1..=40).map(|at| frame(100.0 + 50.0 * f64::from(at), "tilted", true)));
        let settled = frames.last().unwrap().us / 1e3;
        frames.extend((1..=100).map(|at| frame(settled + 4.0 * f64::from(at), "pan", true)));
        let climbing = frames.last().unwrap().us / 1e3;
        frames.extend((1..=10).map(|at| frame(climbing + 10.0 * f64::from(at), "climb", true)));
        let report = summary(&frames, true);
        assert_eq!(report["intervals_counted"], 101);
        assert!((report["median"]["fps"].as_f64().unwrap() - 250.0).abs() < 1e-6);
        assert!((report["p99"]["interval_ms"].as_f64().unwrap() - 4.0).abs() < 1e-9);
        assert_eq!(report["legs"]["climb"]["intervals_counted"], 9);
        assert!((report["frame_thread_cpu"]["median"]["interval_ms"].as_f64().unwrap() - 2.0).abs() < 1e-9);
        assert!((report["legs"]["climb"]["median"]["fps"].as_f64().unwrap() - 100.0).abs() < 1e-6);
        assert_eq!(summary(&frames, false)["intervals_counted"], 110);
        assert_eq!(summary(&frames[..1], true)["median"], Value::Null, "nothing drawn, nothing counted");
    }

    #[test]
    fn a_slow_last_frame_of_a_leg_counts_on_that_leg() {
        let mut frames = vec![frame(0.0, "tilted", true)];
        frames.extend((1..=20).map(|at| frame(SETTLE_US / 1e3 + 4.0 * f64::from(at), "turn", true)));
        let last = frames.last().unwrap().us / 1e3;
        frames.extend([frame(last + 30.0, "climb", true), frame(last + 34.0, "climb", true)]);
        let report = summary(&frames, true);
        assert_eq!(report["legs"]["turn"]["intervals_counted"], 20);
        assert!((report["legs"]["turn"]["p99"]["interval_ms"].as_f64().unwrap() - 30.0).abs() < 1e-9, "the slow frame stands on the turn");
        assert!((report["p99"]["interval_ms"].as_f64().unwrap() - 30.0).abs() < 1e-9, "and in the steady flight");
        assert_eq!(report["legs"]["climb"]["intervals_counted"], 1);
        assert!((report["legs"]["climb"]["median"]["interval_ms"].as_f64().unwrap() - 4.0).abs() < 1e-9, "the climb holds only its own frame");
    }
}
