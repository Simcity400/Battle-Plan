//! The scale of the screen-space map interface: counters, map icons, VP and
//! city labels, combat bubbles, the combat window and the map's tooltips.
//!
//! The game lays out its interface in logical pixels: frame pixels divided by effective
//! `gui_scale`. [`installed_content_settings::settings::Display`] defines settings-file precedence. Windows then
//! stretches the rasterized frame by display scale. [`Setting::MatchGame`] preserves both stages;
//! dumps use the game's frame size and interface scale 1.0.
//!
//! Every screen-space layout runs in interface pixels, the device pixels
//! divided by the frame's GUI scale: projected anchors are divided and the
//! `.gui` sizes stay as authored. The completed framebuffer is presented with
//! linear filtering at the display stretch. Pointer coordinates undo that
//! stretch before camera and interface hit tests.

use job_priority::SpawnCarried as _;
use std::sync::{
    Arc, Mutex, OnceLock,
    atomic::{AtomicBool, AtomicU64, Ordering},
};

use dashboard_core::protocol::MAP_UI_SCALE_RANGE;
use installed_content_settings::{settings::Desktop, settings::Display};

/// Where the interface scale comes from.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub enum Setting {
    /// The game's effective scale times its stretch onto the desktop.
    #[default]
    MatchGame,
    /// A scale the user chose.
    Manual(f32),
}

/// Present the completed image, including its already blended one-texel
/// features. Nearest selection can omit a whole stroke when resized.
pub const PRESENTATION_FILTER: eframe::wgpu::FilterMode = eframe::wgpu::FilterMode::Linear;

/// Keep GUI rasterisation separate from desktop presentation. Project the interface into the native
/// window-size frame, then apply Windows display stretch (`Display::stretch`). Scaling translucent
/// artwork before blending changes one-texel borders and glyph strokes at the same final size.
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
        monitors: Result<&[Desktop], &str>,
    ) -> (Raster, Option<String>) {
        let (screen_scale, note) = self.resolve(display, monitors);
        let raster = match (self, display, monitors, &note) {
            (Self::MatchGame, Ok(display), Ok(monitors), None) => {
                // Resolve each native stage directly, without a multiply/divide
                // round trip through the screen scale.
                let frame = display.frame(monitors).expect("validated display frame");
                Raster {
                    ui_scale: display
                        .interface_to_frame(frame, display.effective_gui_scale(monitors[0])),
                    stretch: display
                        .stretch(monitors)
                        .expect("validated display stretch"),
                }
            }
            _ => Raster::direct(screen_scale),
        };
        (raster, note)
    }
    /// `None` matches the game; a stored value outside the range does too.
    pub fn from_stored(scale: Option<f32>) -> Self {
        match scale {
            Some(scale) if valid(scale) => Self::Manual(scale),
            _ => Self::MatchGame,
        }
    }

    /// The scale to draw at, and a note when the game's could not be matched.
    ///
    /// `display` is the user folder's display settings and `monitors` every
    /// monitor in SDL's display order ([`installed_content_settings::settings::desktops`]).
    pub fn resolve(
        self,
        display: Result<&Display, &str>,
        monitors: Result<&[Desktop], &str>,
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
        match display.screen_scale(monitors) {
            Some(scale) if valid(scale) => (scale, None),
            Some(scale) => fallback(format!(
                "the game's scale works out to {scale}, outside the viewer's {:.1}-{:.1}",
                MAP_UI_SCALE_RANGE.start(),
                MAP_UI_SCALE_RANGE.end()
            )),
            None => fallback("the game's window size is not one these monitors can show".into()),
        }
    }
}

/// The monitors in SDL's display order, read off the UI thread.
///
/// Enumerating every display adapter takes up to about a millisecond, so no
/// frame does it. [`refresh_monitors`] reads them on a worker when a window
/// sees its monitor or display scale change, and bumps
/// [`monitors_generation`] when the new list is in; a window re-resolves its
/// scale only then or when its setting changes.
struct MonitorState {
    generation: AtomicU64,
    monitors: Mutex<Arc<Result<Vec<Desktop>, String>>>,
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

/// Reads the monitors on a worker thread, then calls `wake`. A request made
/// while a read is running makes that read run once more.
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
                        installed_content_settings::settings::desktops().map_err(|error| error.to_string());
                    *state
                        .monitors
                        .lock()
                        .unwrap_or_else(|error| error.into_inner()) = Arc::new(read);
                    state.generation.fetch_add(1, Ordering::AcqRel);
                }
                state.reading.store(false, Ordering::Release);
                // A request that arrived after the last read but before the
                // flag cleared still needs one.
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

/// Bumped each time [`refresh_monitors`] stores a new list; 0 before the first.
pub fn monitors_generation() -> u64 {
    monitor_state().generation.load(Ordering::Acquire)
}

/// The last list [`refresh_monitors`] read.
pub fn monitors() -> Arc<Result<Vec<Desktop>, String>> {
    Arc::clone(
        &monitor_state()
            .monitors
            .lock()
            .unwrap_or_else(|error| error.into_inner()),
    )
}

/// Whether a scale may be drawn at.
pub fn valid(scale: f32) -> bool {
    scale.is_finite() && MAP_UI_SCALE_RANGE.contains(&scale)
}

/// A device-pixel point in interface pixels.
pub fn to_interface(point: [f32; 2], scale: f32) -> [f32; 2] {
    [point[0] / scale, point[1] / scale]
}

/// `world` projected into interface pixels on a viewport of `viewport` device
/// pixels, through the nearest world-wrap copy.
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

/// An interface length in egui points: `ui_scale` device pixels per interface
/// pixel over `pixels_per_point` device pixels per point. egui text and
/// frames sized this way land on the same device pixels as the sprites,
/// whatever the Windows display scale.
pub fn points(interface: f32, ui_scale: f32, pixels_per_point: f32) -> f32 {
    interface * ui_scale / pixels_per_point
}

/// The viewport's size in interface pixels.
pub fn extent(viewport: [u32; 2], scale: f32) -> [f32; 2] {
    to_interface([viewport[0] as f32, viewport[1] as f32], scale)
}

#[cfg(test)]
mod tests;
