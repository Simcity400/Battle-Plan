use crate::hours;

pub fn exact_capture_wait(remaining: Option<u32>, profiling: bool) -> bool {
    remaining.is_some_and(|remaining| !profiling || remaining == 0)
}

pub fn grid_activation(
    switch: bool,
    playing: bool,
    dashboard_map: bool,
    scripted: bool,
    capture: bool,
    exact_wait: bool,
    wall_sweep: bool,
    speed: f64,
) -> Result<u32, &'static str> {
    if !switch && !dashboard_map {
        return Err("switch-off");
    }
    if !playing {
        return Err("paused");
    }
    if scripted {
        return Err("scripted-map-mode");
    }
    if exact_wait || (capture && !wall_sweep) {
        return Err("capture");
    }
    hours::grid_stride(speed).ok_or("below-finest-stride")
}

/// Hours the clock passes while a target is built, from the measured build
/// latency; nothing is assumed before the first hour is measured.
pub fn build_lead(speed: f64, latency_ms: Option<f64>) -> u32 {
    let hours = speed * latency_ms.unwrap_or(0.0) / 1000.0;
    if hours.is_finite() && hours > 0.0 {
        hours.ceil().min(f64::from(u32::MAX)) as u32
    } else {
        0
    }
}

/// The build latency estimate after one more measured hour.
pub fn next_latency(estimate: Option<f64>, wall_ms: f64) -> Option<f64> {
    if !wall_ms.is_finite() || wall_ms <= 0.0 {
        return estimate;
    }
    // A quarter weight follows a change in load within a few hours.
    Some(estimate.map_or(wall_ms, |old| old + (wall_ms - old) * 0.25))
}

/// The political part's limitation lines: with the playhead's map open, the political, naval and
/// mode fills layers' own while the political layer draws (its refusal is the map's own line
/// otherwise); without it, `today`'s.
pub fn political_lines(side: Option<&crate::playhead_host::MapSide>, today: &[String]) -> Vec<String> {
    let Some(side) = side else { return today.to_vec() };
    match side.parts().filter(|_| side.draws("political")) {
        Some((_, layers)) => {
            let political = layers.iter().filter_map(|layer| layer.any().downcast_ref::<crate::layers::political::Political>()).flat_map(|layer| layer.warnings());
            let naval = layers.iter().filter_map(|layer| layer.any().downcast_ref::<crate::layers::naval::Naval>()).flat_map(|layer| layer.warnings());
            let modes = layers.iter().filter_map(|layer| layer.any().downcast_ref::<crate::layers::mode_fills::ModeFills>()).flat_map(|layer| layer.lines());
            political.chain(naval).chain(modes).collect()
        }
        None => Vec::new(),
    }
}
