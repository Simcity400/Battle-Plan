//! The scale of the screen-space map interface: counters, map icons, VP and
//! city labels, combat bubbles, the combat window and the map's tooltips.
//!
//! The game lays its interface out in logical pixels, the frame's pixels
//! divided by its effective `gui_scale` (`CGraphics` `1422206c0`, see
//! [`installed_content::settings::Display`] for the order `settings.txt` and
//! `pdx_settings.txt` are applied in), and the frame is then shown on its
//! monitor, stretched by Windows' display scale since the game is not DPI
//! aware. The viewer draws at the window's own device pixels, so matching the
//! game on screen takes the product of the two: [`Setting::MatchGame`]. A dump
//! renders at the game's frame size, where the verified comparison pipeline
//! has always used 1.0.
//!
//! Every screen-space layout runs in interface pixels, the device pixels
//! divided by the scale: projected anchors are divided and the `.gui` sizes
//! stay as authored. [`crate::map::sprites::SpritePass::upload`] then multiplies
//! each snapped rectangle back into device pixels, so a quad is drawn at its
//! final size rather than stretched from a smaller image. Hit tests divide the
//! pointer the same way. The world-space names and the 3D map do not scale.

use std::sync::{
    Arc, Mutex, OnceLock,
    atomic::{AtomicBool, AtomicU64, Ordering},
};

use dashboard::protocol::MAP_UI_SCALE_RANGE;
use installed_content::settings::{Desktop, Display};

/// Where the interface scale comes from.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub enum Setting {
    /// The game's effective scale times its stretch onto the desktop.
    #[default]
    MatchGame,
    /// A scale the user chose.
    Manual(f32),
}

impl Setting {
    /// `None` matches the game; a stored value outside the range does too.
    pub fn from_stored(scale: Option<f32>) -> Self {
        match scale {
            Some(scale) if valid(scale) => Self::Manual(scale),
            _ => Self::MatchGame,
        }
    }

    /// The stored form: `None` for [`Setting::MatchGame`].
    pub fn stored(self) -> Option<f32> {
        match self {
            Self::MatchGame => None,
            Self::Manual(scale) => Some(scale),
        }
    }

    /// The scale to draw at, and a note when the game's could not be matched.
    ///
    /// `display` is the user folder's display settings and `monitors` every
    /// monitor in SDL's display order ([`installed_content::settings::desktops`]).
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
        .spawn(move || {
            let state = monitor_state();
            loop {
                while state.again.swap(false, Ordering::AcqRel) {
                    let read = installed_content::settings::desktops().map_err(|error| error.to_string());
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
mod tests {
    use super::*;
    use installed_content::settings::DisplayMode;

    /// The owner's borderless game on a 2560x1440 monitor at 125%.
    const BORDERLESS: Display = Display {
        gui_scale: 1.0,
        clamp_size: Some([2048, 1152]),
        clamp_borderless: true,
        size: Some([2560, 1440]),
        min_gui: [1280, 720],
        mode: DisplayMode::Borderless,
        display_index: 0,
    };
    const MONITOR: Desktop = Desktop {
        physical: [2560, 1440],
        dpi: 120,
    };

    #[test]
    fn matching_the_game_multiplies_its_scale_by_its_stretch() {
        let (scale, note) = Setting::MatchGame.resolve(Ok(&BORDERLESS), Ok(&[MONITOR]));
        assert_eq!((scale, note), (1.25, None));
        // At 100% the same game fills the monitor 1:1.
        let unscaled = Desktop { dpi: 96, ..MONITOR };
        assert_eq!(
            Setting::MatchGame.resolve(Ok(&BORDERLESS), Ok(&[unscaled])),
            (1.0, None)
        );
        let (scale, _) = Setting::MatchGame.resolve(
            Ok(&Display {
                gui_scale: 1.2,
                ..BORDERLESS
            }),
            Ok(&[unscaled]),
        );
        // 2560 frame pixels over 2133 interface pixels, truncated as the game does.
        assert_eq!(scale, 2560.0 / 2133.0);
    }

    #[test]
    fn missing_game_settings_fall_back_to_one_with_a_note() {
        let (scale, note) = Setting::MatchGame.resolve(Err("Missing graphics block"), Ok(&[]));
        assert_eq!(scale, 1.0);
        assert!(note.unwrap().contains("Missing graphics block"));
        let (scale, note) = Setting::MatchGame.resolve(Ok(&BORDERLESS), Err("not DPI aware"));
        assert_eq!(scale, 1.0);
        assert!(note.unwrap().contains("not DPI aware"));
        let (scale, note) = Setting::MatchGame.resolve(Ok(&BORDERLESS), Ok(&[]));
        assert_eq!(scale, 1.0);
        assert!(note.is_some());
    }

    #[test]
    fn a_manual_scale_ignores_the_game_and_invalid_stored_values_match_it() {
        assert_eq!(
            Setting::Manual(1.5).resolve(Err("unused"), Err("unused")),
            (1.5, None)
        );
        assert_eq!(Setting::from_stored(Some(1.5)), Setting::Manual(1.5));
        assert_eq!(Setting::from_stored(None), Setting::MatchGame);
        assert_eq!(Setting::from_stored(Some(0.1)), Setting::MatchGame);
        assert_eq!(Setting::from_stored(Some(f32::NAN)), Setting::MatchGame);
        assert_eq!(Setting::Manual(2.0).stored(), Some(2.0));
    }

    #[test]
    fn a_click_hits_exactly_what_the_scaled_quad_covers() {
        // A combat bubble laid out at (100, 100) 40 x 20 interface pixels.
        let hit = crate::combat::Hit {
            selection: crate::combat::Selection::Land(7),
            rect: [100.0, 100.0, 40.0, 20.0],
        };
        let [x0, y0, x1, y1] = crate::map::sprites::device_corners(hit.rect, 1.25);
        assert_eq!([x0, y0, x1, y1], [125.0, 125.0, 175.0, 150.0]);
        let click = |device: [f32; 2]| hit.contains(to_interface(device, 1.25));
        assert!(click([x0, y0]));
        assert!(click([x1 - 0.5, y1 - 0.5]));
        assert!(!click([x1, y0]));
        assert!(!click([x0, y1]));
        assert!(!click([x0 - 0.5, y0]));
        // Unscaled, the same device point that hits at 1.25 misses the bubble.
        assert!(!hit.contains([170.0, 140.0]));
        assert!(click([170.0, 140.0]));
    }

    #[test]
    fn tooltip_lengths_follow_the_ui_scale_alone_whatever_the_display_scale() {
        // 14 interface pixels are 17.5 device pixels at 1.25 on a 100%, 150%
        // or 200% Windows display, as a 14-pixel sprite would be.
        for pixels_per_point in [1.0, 1.5, 2.0] {
            let device = points(14.0, 1.25, pixels_per_point) * pixels_per_point;
            assert!((device - 17.5).abs() < 1e-4, "{pixels_per_point}");
        }
    }

    #[test]
    fn device_points_divide_into_interface_pixels() {
        assert_eq!(to_interface([1280.0, 720.0], 1.25), [1024.0, 576.0]);
        assert_eq!(extent([2560, 1440], 1.25), [2048.0, 1152.0]);
        assert_eq!(extent([2048, 1152], 1.0), [2048.0, 1152.0]);
    }
}
