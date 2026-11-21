use crate::{tray::TRAY_ID, worker::Recorder};
use serde::Serialize;
use std::{
    path::PathBuf,
    sync::{Arc, Mutex},
    time::{Duration, Instant},
};
use tauri::{
    AppHandle, Emitter, Manager, Monitor, PhysicalPosition, PhysicalSize, Rect, WebviewUrl,
    WebviewWindow, WebviewWindowBuilder, WindowEvent,
};

pub const LABEL: &str = "panel";
pub const ASK_STOP_EVENT: &str = "recorder-ask-stop";
const WIDTH: f64 = 360.0;
const MARGIN: f64 = 12.0;
const REOPEN_GUARD: Duration = Duration::from_millis(400);
const REPLAY_IDENTIFIER: &str = "local.hoi4.replay";

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Opening {
    Panel,
    QuitPrompt,
    DiscardPrompt,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize)]
pub struct Ask {
    pub quitting: bool,
    pub discarding: bool,
}

impl Ask {
    fn of(opening: Opening) -> Self {
        Self {
            quitting: opening == Opening::QuitPrompt,
            discarding: opening == Opening::DiscardPrompt,
        }
    }

    fn join(&mut self, other: Self) {
        self.quitting |= other.quitting;
        self.discarding |= other.discarding;
    }
}

#[derive(Default)]
struct State {
    anchor: Option<Rect>,
    closed_at: Option<Instant>,
    ask_stop: bool,
    ask: Ask,
    compact: bool,
    moved: Option<PhysicalPosition<i32>>,
    placed: Option<PhysicalPosition<i32>>,
    dialog: bool,
    focused: bool,
    shown: bool,
    opening: bool,
    recordings_panel: Option<String>,
}

#[derive(Default)]
pub struct Panel(Mutex<State>);

fn state(app: &AppHandle) -> std::sync::MutexGuard<'_, State> {
    app.state::<Panel>()
        .inner()
        .0
        .lock()
        .unwrap_or_else(|error| error.into_inner())
}

#[derive(Debug, PartialEq, Eq)]
enum Click {
    Close,
    Open,
    Ignore,
}

fn click(exists: bool, opening: bool, closed_recently: bool) -> Click {
    if exists {
        Click::Close
    } else if opening || closed_recently {
        Click::Ignore
    } else {
        Click::Open
    }
}

fn closes_on_focus_loss(shown: bool, focused: bool, dialog: bool) -> bool {
    shown && focused && !dialog
}

pub fn toggle(app: &AppHandle, anchor: Rect) {
    let window = app.get_webview_window(LABEL);
    let (opening, closed_recently) = {
        let state = state(app);
        (
            state.opening,
            state
                .closed_at
                .is_some_and(|closed| closed.elapsed() < REOPEN_GUARD),
        )
    };
    match click(window.is_some(), opening, closed_recently) {
        Click::Close => {
            if let Some(window) = window {
                let _ = window.destroy();
            }
        }
        Click::Open => open(app, Some(anchor), Opening::Panel),
        Click::Ignore => {}
    }
}

pub fn open(app: &AppHandle, anchor: Option<Rect>, opening: Opening) {
    let ask_stop = opening != Opening::Panel;
    let ask = Ask::of(opening);
    if let Some(window) = app.get_webview_window(LABEL) {
        if ask_stop {
            {
                let mut state = state(app);
                state.ask_stop = true;
                state.ask.join(ask);
            }
            let _ = window.emit(ASK_STOP_EVENT, ask);
        }
        let _ = window.set_focus();
        return;
    }
    let anchor = anchor.or_else(|| {
        app.tray_by_id(TRAY_ID)
            .and_then(|tray| tray.rect().ok().flatten())
    });
    {
        let mut state = state(app);
        if state.opening {
            state.ask_stop |= ask_stop;
            state.ask.join(ask);
            return;
        }
        *state = State {
            anchor,
            ask_stop,
            ask,
            compact: ask_stop,
            moved: state.moved,
            opening: true,
            ..State::default()
        };
    }
    let handle = app.clone();
    let spawned = std::thread::Builder::new()
        .name("recorder-panel".into())
        .spawn(move || {
            if let Err(error) = build(&handle) {
                eprintln!("The recorder panel could not open: {error}");
            }
            state(&handle).opening = false;
        });
    if let Err(error) = spawned {
        eprintln!("The recorder panel could not open: {error}");
        state(app).opening = false;
    }
}

fn replay_webview_data(app: &AppHandle) -> Option<PathBuf> {
    app.path()
        .local_data_dir()
        .ok()
        .map(|directory| directory.join(REPLAY_IDENTIFIER))
}

fn builder(
    app: &AppHandle,
    shared: Option<PathBuf>,
) -> WebviewWindowBuilder<'_, tauri::Wry, AppHandle> {
    let builder = WebviewWindowBuilder::new(app, LABEL, WebviewUrl::App("recorder.html".into()))
        .title("BattlePlan Recorder")
        .inner_size(WIDTH, 120.0)
        .resizable(false)
        .maximizable(false)
        .minimizable(false)
        .decorations(false)
        .shadow(true)
        .always_on_top(true)
        .skip_taskbar(true)
        .visible(false)
        .focused(true);
    match shared {
        Some(directory) => builder.data_directory(directory),
        None => builder,
    }
}

fn build(app: &AppHandle) -> tauri::Result<()> {
    let window = match builder(app, replay_webview_data(app)).build() {
        Ok(window) => window,
        Err(error) => {
            eprintln!("Opening the panel with the replay app's appearance failed: {error}");
            builder(app, None).build()?
        }
    };
    let handle = app.clone();
    window.on_window_event(move |event| match event {
        WindowEvent::Focused(true) => state(&handle).focused = true,
        WindowEvent::Moved(position) => {
            let mut state = state(&handle);
            if state.shown && !state.compact && state.placed != Some(*position) {
                state.moved = Some(*position);
            }
        }
        WindowEvent::Focused(false) => {
            let close = {
                let mut state = state(&handle);
                let close = closes_on_focus_loss(state.shown, state.focused, state.dialog);
                state.focused = false;
                close
            };
            if close && let Some(window) = handle.get_webview_window(LABEL) {
                let _ = window.destroy();
            }
        }
        WindowEvent::Destroyed => {
            {
                let mut state = state(&handle);
                state.closed_at = Some(Instant::now());
                state.focused = false;
                state.shown = false;
                state.ask_stop = false;
                state.ask = Ask::default();
                if let Some(panel) = state.recordings_panel.take() {
                    handle.state::<crate::recordings::Library>().close(&panel);
                }
            }
            handle.state::<Arc<Recorder>>().panel_closed(&handle);
        }
        _ => {}
    });
    Ok(())
}

pub fn take_opening(app: &AppHandle, panel: &str) -> (bool, bool, Ask) {
    let mut state = state(app);
    if state.closed_at.is_none() && state.recordings_panel.as_deref() != Some(panel) {
        let library = app.state::<crate::recordings::Library>();
        if let Some(previous) = state.recordings_panel.replace(panel.to_owned()) {
            library.close(&previous);
        }
        library.register_panel(panel);
    }
    (
        std::mem::take(&mut state.ask_stop),
        state.compact,
        std::mem::take(&mut state.ask),
    )
}

pub fn fit(app: &AppHandle, window: &WebviewWindow, height: f64) -> tauri::Result<()> {
    let (anchor, moved) = {
        let state = state(app);
        (state.anchor, state.moved.filter(|_| !state.compact))
    };
    let mut screens = Vec::new();
    let primary = app.primary_monitor()?;
    for monitor in primary.iter().chain(&app.available_monitors()?) {
        let screen = screen(monitor);
        if !screens.contains(&screen) {
            screens.push(screen);
        }
    }
    let anchor = anchor.map(|rect| {
        let position = rect.position.to_physical::<f64>(1.0);
        let size = rect.size.to_physical::<f64>(1.0);
        Area {
            x: position.x.round() as i32,
            y: position.y.round() as i32,
            width: size.width.round() as i32,
            height: size.height.round() as i32,
        }
    });
    let at = moved.map(|position| (position.x, position.y));
    match placement(&screens, anchor, at, (WIDTH, height.clamp(40.0, 4000.0))) {
        Some(placed) => {
            let position = PhysicalPosition::new(placed.x, placed.y);
            state(app).placed = Some(position);
            window.set_size(PhysicalSize::new(placed.width, placed.height))?;
            window.set_position(position)?;
        }
        None => window.set_size(tauri::LogicalSize::new(WIDTH, height))?,
    }
    let first = !std::mem::replace(&mut state(app).shown, true);
    if first {
        window.show()?;
        window.set_focus()?;
    }
    Ok(())
}

fn screen(monitor: &Monitor) -> Screen {
    let area = |position: PhysicalPosition<i32>, size: PhysicalSize<u32>| Area {
        x: position.x,
        y: position.y,
        width: size.width as i32,
        height: size.height as i32,
    };
    let work = monitor.work_area();
    Screen {
        bounds: area(*monitor.position(), *monitor.size()),
        work: area(work.position, work.size),
        scale: monitor.scale_factor(),
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Area {
    pub x: i32,
    pub y: i32,
    pub width: i32,
    pub height: i32,
}

impl Area {
    fn right(&self) -> i32 {
        self.x + self.width
    }

    fn bottom(&self) -> i32 {
        self.y + self.height
    }

    fn contains(&self, (x, y): (i32, i32)) -> bool {
        (self.x..self.right()).contains(&x) && (self.y..self.bottom()).contains(&y)
    }

    fn distance(&self, (x, y): (i32, i32)) -> i64 {
        let dx = i64::from((self.x - x).max(0).max(x - (self.right() - 1)));
        let dy = i64::from((self.y - y).max(0).max(y - (self.bottom() - 1)));
        dx * dx + dy * dy
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Screen {
    pub bounds: Area,
    pub work: Area,
    pub scale: f64,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Placement {
    pub x: i32,
    pub y: i32,
    pub width: u32,
    pub height: u32,
}

#[derive(Clone, Copy)]
enum Side {
    Bottom,
    Top,
    Left,
    Right,
}

pub fn placement(
    screens: &[Screen],
    anchor: Option<Area>,
    at: Option<(i32, i32)>,
    size: (f64, f64),
) -> Option<Placement> {
    let nearest = |point: (i32, i32)| {
        screens
            .iter()
            .find(|screen| screen.bounds.contains(point))
            .or_else(|| {
                screens
                    .iter()
                    .min_by_key(|screen| screen.bounds.distance(point))
            })
    };
    let point = at.or(anchor.map(|icon| (icon.x + icon.width / 2, icon.y + icon.height / 2)));
    let screen = match point {
        Some(point) => nearest(point)?,
        None => screens.first()?,
    };
    let work = screen.work;
    let margin = (MARGIN * screen.scale).round() as i32;
    let fit =
        |logical: f64, room: i32| ((logical * screen.scale).round() as i32).clamp(1, room.max(1));
    let width = fit(size.0, work.width - 2 * margin);
    let height = fit(size.1, work.height - 2 * margin);
    let clamp =
        |value: i32, low: i32, length: i32, high: i32| value.clamp(low, (high - length).max(low));
    let (x, y) = match (at, anchor) {
        (Some((x, y)), _) => (
            clamp(x, work.x, width, work.right()),
            clamp(y, work.y, height, work.bottom()),
        ),
        (None, Some(icon)) => {
            let (centre_x, centre_y) = (icon.x + icon.width / 2, icon.y + icon.height / 2);
            let bounds = screen.bounds;
            let side = if centre_y >= work.bottom() {
                Side::Bottom
            } else if centre_y < work.y {
                Side::Top
            } else if centre_x < work.x {
                Side::Left
            } else if centre_x >= work.right() {
                Side::Right
            } else {
                [
                    (bounds.bottom() - centre_y, Side::Bottom),
                    (centre_y - bounds.y, Side::Top),
                    (centre_x - bounds.x, Side::Left),
                    (bounds.right() - centre_x, Side::Right),
                ]
                .into_iter()
                .min_by_key(|(distance, _)| *distance)
                .map(|(_, side)| side)
                .unwrap()
            };
            let (low_x, high_x) = (work.x + margin, work.right() - margin);
            let (low_y, high_y) = (work.y + margin, work.bottom() - margin);
            let across_x = clamp(centre_x - width / 2, low_x, width, high_x);
            let across_y = clamp(centre_y - height / 2, low_y, height, high_y);
            match side {
                Side::Bottom => {
                    let y = icon.y.min(work.bottom()) - margin - height;
                    (across_x, clamp(y, low_y, height, high_y))
                }
                Side::Top => {
                    let y = icon.bottom().max(work.y) + margin;
                    (across_x, clamp(y, low_y, height, high_y))
                }
                Side::Left => {
                    let x = icon.right().max(work.x) + margin;
                    (clamp(x, low_x, width, high_x), across_y)
                }
                Side::Right => {
                    let x = icon.x.min(work.right()) - margin - width;
                    (clamp(x, low_x, width, high_x), across_y)
                }
            }
        }
        (None, None) => (
            work.right() - margin - width,
            work.bottom() - margin - height,
        ),
    };
    Some(Placement {
        x,
        y,
        width: width as u32,
        height: height as u32,
    })
}

pub fn with_dialog<T>(app: &AppHandle, dialog: impl FnOnce() -> T) -> T {
    state(app).dialog = true;
    let result = dialog();
    state(app).dialog = false;
    if let Some(window) = app.get_webview_window(LABEL) {
        let _ = window.set_focus();
    }
    result
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_click_while_the_panel_is_being_created_opens_no_second_window() {
        assert_eq!(click(false, true, false), Click::Ignore);
        assert_eq!(click(false, false, true), Click::Ignore);
        assert_eq!(click(true, false, false), Click::Close);
        assert_eq!(click(false, false, false), Click::Open);
    }

    const fn area(x: i32, y: i32, width: i32, height: i32) -> Area {
        Area {
            x,
            y,
            width,
            height,
        }
    }

    fn screen(bounds: Area, work: Area, scale: f64) -> Screen {
        Screen {
            bounds,
            work,
            scale,
        }
    }

    fn inside(placed: Placement, work: Area) -> bool {
        placed.x >= work.x
            && placed.y >= work.y
            && placed.x + placed.width as i32 <= work.right()
            && placed.y + placed.height as i32 <= work.bottom()
    }

    const PANEL: (f64, f64) = (360.0, 200.0);

    #[test]
    fn the_panel_stands_beside_the_tray_icon_away_from_a_taskbar_on_any_edge() {
        let work = area(0, 0, 1920, 1032);
        let monitor = screen(area(0, 0, 1920, 1080), work, 1.0);
        let placed = placement(&[monitor], Some(area(1700, 1036, 32, 40)), None, PANEL).unwrap();
        assert_eq!(
            placed,
            Placement {
                x: 1536,
                y: 820,
                width: 360,
                height: 200
            }
        );
        let work = area(0, 60, 2560, 1380);
        let monitor = screen(area(0, 0, 2560, 1440), work, 1.25);
        let placed = placement(&[monitor], Some(area(2400, 10, 40, 40)), None, PANEL).unwrap();
        assert_eq!(
            placed,
            Placement {
                x: 2095,
                y: 75,
                width: 450,
                height: 250
            }
        );
        assert!(inside(placed, work));
        let work = area(72, 0, 3768, 2160);
        let monitor = screen(area(0, 0, 3840, 2160), work, 1.5);
        let placed = placement(&[monitor], Some(area(16, 2000, 40, 40)), None, PANEL).unwrap();
        assert_eq!(
            placed,
            Placement {
                x: 90,
                y: 1842,
                width: 540,
                height: 300
            }
        );
        assert!(inside(placed, work));
        let work = area(0, 0, 3744, 2160);
        let monitor = screen(area(0, 0, 3840, 2160), work, 2.0);
        let placed = placement(&[monitor], Some(area(3780, 1900, 48, 48)), None, PANEL).unwrap();
        assert_eq!(
            placed,
            Placement {
                x: 3000,
                y: 1724,
                width: 720,
                height: 400
            }
        );
        assert!(inside(placed, work));
    }

    #[test]
    fn an_auto_hidden_taskbar_is_found_at_the_nearest_monitor_edge() {
        let whole = area(0, 0, 1920, 1080);
        let monitor = screen(whole, whole, 1.0);
        let bottom = placement(&[monitor], Some(area(1700, 1040, 32, 40)), None, PANEL).unwrap();
        assert_eq!((bottom.x, bottom.y), (1536, 828));
        let right = placement(&[monitor], Some(area(1880, 900, 40, 32)), None, PANEL).unwrap();
        assert_eq!((right.x, right.y), (1508, 816));
        assert!(inside(bottom, whole) && inside(right, whole));
    }

    #[test]
    fn a_negative_coordinate_secondary_monitor_holds_its_own_panel() {
        let primary = screen(area(0, 0, 1920, 1080), area(0, 0, 1920, 1032), 1.0);
        let work = area(-2560, -200, 2560, 1380);
        let secondary = screen(area(-2560, -200, 2560, 1440), work, 1.25);
        let placed = placement(
            &[primary, secondary],
            Some(area(-300, 1190, 40, 40)),
            None,
            PANEL,
        )
        .unwrap();
        assert_eq!(
            placed,
            Placement {
                x: -505,
                y: 915,
                width: 450,
                height: 250
            }
        );
        assert!(inside(placed, work));
    }

    #[test]
    fn mixed_dpi_sizes_the_panel_for_the_monitor_it_lands_on() {
        let primary_work = area(0, 0, 3840, 2064);
        let primary = screen(area(0, 0, 3840, 2160), primary_work, 2.0);
        let secondary_work = area(3840, 500, 1920, 1032);
        let secondary = screen(area(3840, 500, 1920, 1080), secondary_work, 1.0);
        let screens = [primary, secondary];
        let by_tray = placement(&screens, Some(area(3600, 2080, 64, 64)), None, PANEL).unwrap();
        assert_eq!((by_tray.width, by_tray.height), (720, 400));
        assert!(inside(by_tray, primary_work));
        let dragged = placement(&screens, None, Some((4000, 600)), PANEL).unwrap();
        assert_eq!(
            dragged,
            Placement {
                x: 4000,
                y: 600,
                width: 360,
                height: 200
            }
        );
        let off = placement(&screens, None, Some((5700, 1400)), PANEL).unwrap();
        assert_eq!((off.x, off.y), (5400, 1332));
        assert!(inside(off, secondary_work));
        let gone = placement(&screens, None, Some((9000, 600)), PANEL).unwrap();
        assert!(inside(gone, secondary_work));
        let both = placement(
            &screens,
            Some(area(3600, 2080, 64, 64)),
            Some((4000, 600)),
            PANEL,
        );
        assert_eq!(both, Some(dragged));
    }

    #[test]
    fn a_panel_taller_than_the_screen_at_300_percent_stays_on_it() {
        let work = area(0, 0, 1920, 1032);
        let monitor = screen(area(0, 0, 1920, 1080), work, 3.0);
        let placed = placement(
            &[monitor],
            Some(area(1700, 1036, 32, 40)),
            None,
            (360.0, 500.0),
        )
        .unwrap();
        assert_eq!((placed.width, placed.height), (1080, 960));
        assert!(inside(placed, work));
        let corner = placement(&[monitor], None, None, PANEL).unwrap();
        assert_eq!((corner.x, corner.y), (804, 396));
        assert!(placement(&[], None, None, PANEL).is_none());
    }

    #[test]
    fn focus_lost_before_the_panel_shows_does_not_close_it() {
        assert!(!closes_on_focus_loss(false, true, false));
        assert!(closes_on_focus_loss(true, true, false));
        assert!(!closes_on_focus_loss(true, true, true));
        assert!(!closes_on_focus_loss(true, false, false));
    }
}
