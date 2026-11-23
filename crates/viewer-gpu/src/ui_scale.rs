
use job_priority::SpawnCarried as _;
use std::sync::{
    Arc, Mutex, OnceLock,
    atomic::{AtomicBool, AtomicU64, Ordering},
};

use dashboard_core::protocol::MAP_UI_SCALE_RANGE;
use content_reader_settings::{settings::Desktop, settings::Display};

#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub enum Setting {
    #[default]
    MatchGame,
    Manual(f32),
}

pub const PRESENTATION_FILTER: eframe::wgpu::FilterMode = eframe::wgpu::FilterMode::Linear;

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Raster {
    pub ui_scale: f32,
    pub stretch: f32,
}

impl Raster {
    pub fn direct(ui_scale: f32) -> Self {
        Self {
            ui_scale,
            stretch: 1.0,
        }
    }

    pub fn screen_scale(self) -> f32 {
        self.ui_scale * self.stretch
    }

    pub fn target(self, physical: [u32; 2]) -> [u32; 2] {
        physical.map(|extent| (extent as f32 / self.stretch).round().max(1.0) as u32)
    }
}

impl Setting {
    pub fn resolve_raster(
        self,
        display: Result<&Display, &str>,
        monitors: Result<&[Result<Desktop, String>], &str>,
    ) -> (Raster, Option<String>) {
        let (screen_scale, note) = self.resolve(display, monitors);
        let raster = match (self, display, monitors, &note) {
            (Self::MatchGame, Ok(display), Ok(monitors), None) => {
                let frame = display.frame_result(monitors).expect("validated display frame");
                let gui_scale = display
                    .effective_gui_scale_result(monitors)
                    .expect("validated display gui scale");
                Raster {
                    ui_scale: display.interface_to_frame(frame, gui_scale),
                    stretch: display
                        .stretch_result(monitors)
                        .expect("validated display stretch"),
                }
            }
            _ => Raster::direct(screen_scale),
        };
        (raster, note)
    }
    pub fn from_stored(scale: Option<f32>) -> Self {
        match scale {
            Some(scale) if valid(scale) => Self::Manual(scale),
            _ => Self::MatchGame,
        }
    }

    pub fn resolve(
        self,
        display: Result<&Display, &str>,
        monitors: Result<&[Result<Desktop, String>], &str>,
    ) -> (f32, Option<String>) {
        let fallback = |why: String| {
            (
                1.0,
                Some(format!(
                    "Map UI scale: {why}; drawing the map interface at 1.0."
                )),
            )
        };
        let display = match (self, display) {
            (Self::Manual(scale), _) => return (scale, None),
            (Self::MatchGame, Ok(display)) => display,
            (Self::MatchGame, Err(error)) => {
                return fallback(format!(
                    "the game's display settings are unavailable ({error})"
                ));
            }
        };
        let monitors = match monitors {
            Ok(monitors) => monitors,
            Err(error) => return fallback(format!("the monitors are unavailable ({error})")),
        };
        match display.screen_scale_result(monitors) {
            Ok(scale) if valid(scale) => (scale, None),
            Ok(scale) => fallback(format!(
                "the game's scale works out to {scale}, outside the viewer's {:.1}-{:.1}",
                MAP_UI_SCALE_RANGE.start(),
                MAP_UI_SCALE_RANGE.end()
            )),
            Err(why) => fallback(why),
        }
    }
}

struct MonitorState {
    generation: AtomicU64,
    monitors: Mutex<Arc<Result<Vec<Result<Desktop, String>>, String>>>,
    reading: AtomicBool,
    again: AtomicBool,
}

fn monitor_state() -> &'static MonitorState {
    static STATE: OnceLock<MonitorState> = OnceLock::new();
    STATE.get_or_init(|| MonitorState {
        generation: AtomicU64::new(0),
        monitors: Mutex::new(Arc::new(Err("the monitors have not been read yet".into()))),
        reading: AtomicBool::new(false),
        again: AtomicBool::new(false),
    })
}

pub fn refresh_monitors(wake: impl Fn() + Send + 'static) {
    let state = monitor_state();
    state.again.store(true, Ordering::Release);
    if state.reading.swap(true, Ordering::AcqRel) {
        return;
    }
    let spawned = std::thread::Builder::new()
        .name("monitors".into())
        .spawn_carried(move || {
            let state = monitor_state();
            loop {
                while state.again.swap(false, Ordering::AcqRel) {
                    let read =
                        content_reader_settings::settings::desktops().map_err(|error| error.to_string());
                    *state
                        .monitors
                        .lock()
                        .unwrap_or_else(|error| error.into_inner()) = Arc::new(read);
                    state.generation.fetch_add(1, Ordering::AcqRel);
                }
                state.reading.store(false, Ordering::Release);
                if !state.again.load(Ordering::Acquire)
                    || state.reading.swap(true, Ordering::AcqRel)
                {
                    break;
                }
            }
            wake();
        });
    if spawned.is_err() {
        state.reading.store(false, Ordering::Release);
    }
}

pub fn monitors_generation() -> u64 {
    monitor_state().generation.load(Ordering::Acquire)
}

pub fn monitors() -> Arc<Result<Vec<Result<Desktop, String>>, String>> {
    Arc::clone(
        &monitor_state()
            .monitors
            .lock()
            .unwrap_or_else(|error| error.into_inner()),
    )
}

pub fn valid(scale: f32) -> bool {
    scale.is_finite() && MAP_UI_SCALE_RANGE.contains(&scale)
}

pub fn to_interface(point: [f32; 2], scale: f32) -> [f32; 2] {
    [point[0] / scale, point[1] / scale]
}

pub fn project(
    camera: &super::camera::Camera,
    world: [f32; 3],
    viewport: [u32; 2],
    scale: f32,
) -> Option<[f32; 2]> {
    Some(to_interface(
        camera.project_wrapped(world, viewport)?,
        scale,
    ))
}

pub fn points(interface: f32, ui_scale: f32, pixels_per_point: f32) -> f32 {
    interface * ui_scale / pixels_per_point
}

pub fn extent(viewport: [u32; 2], scale: f32) -> [f32; 2] {
    to_interface([viewport[0] as f32, viewport[1] as f32], scale)
}

#[cfg(test)]
mod tests;

pub fn device_corners(rect: [f32; 4], ui_scale: f32) -> [f32; 4] {
    let x0 = rect[0].round();
    let y0 = rect[1].round();
    let corners = [x0, y0, x0 + rect[2].round(), y0 + rect[3].round()];
    corners.map(|value| (value * ui_scale).round())
}
