//! The panel window: created when the tray icon is clicked, placed above it
//! within the monitor's work area, and destroyed when it loses focus.
use crate::{tray::TRAY_ID, worker::Recorder};
use std::{
    path::PathBuf,
    sync::{Arc, Mutex},
    time::{Duration, Instant},
};
use tauri::{
    AppHandle, Emitter, Manager, PhysicalPosition, PhysicalSize, Rect, WebviewUrl, WebviewWindow,
    WebviewWindowBuilder, WindowEvent,
};

pub const LABEL: &str = "panel";
pub const ASK_STOP_EVENT: &str = "recorder-ask-stop";
/// The design's panel width in CSS pixels.
const WIDTH: f64 = 360.0;
/// Distance from the taskbar and screen edges, in CSS pixels.
const MARGIN: f64 = 12.0;
/// A tray click that arrives this soon after the panel closed is the click that
/// closed it, not a request to open it again.
const REOPEN_GUARD: Duration = Duration::from_millis(400);
/// The replay app's identifier: its WebView data folder holds the user's
/// appearance, so the panel opens with the same theme.
const REPLAY_IDENTIFIER: &str = "local.hoi4.replay";

#[derive(Default)]
struct State {
    anchor: Option<Rect>,
    closed_at: Option<Instant>,
    ask_stop: bool,
    /// A native dialog owns focus; losing focus to it does not close the panel.
    dialog: bool,
    focused: bool,
    shown: bool,
    /// A window is being created on its own thread.
    opening: bool,
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

/// What a left click on the tray icon does.
#[derive(Debug, PartialEq, Eq)]
enum Click {
    Close,
    Open,
    /// The panel is still being created, or this is the click that just closed it.
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

/// Left click on the tray icon: open the panel, or close it when open.
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
        Click::Open => open(app, Some(anchor), false),
        Click::Ignore => {}
    }
}

/// Open the panel (or focus it), optionally on the Stop prompt. Tray and menu
/// callbacks run on the event loop, where building a webview window would wait
/// on that same loop, so the window is created on its own thread.
pub fn open(app: &AppHandle, anchor: Option<Rect>, ask_stop: bool) {
    if let Some(window) = app.get_webview_window(LABEL) {
        if ask_stop {
            state(app).ask_stop = true;
            let _ = window.emit(ASK_STOP_EVENT, ());
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
            // The window being created will ask for the Stop prompt when it draws.
            state.ask_stop |= ask_stop;
            return;
        }
        *state = State {
            anchor,
            ask_stop,
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
        .title("Battle Plan Recorder")
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
    // Share the replay app's WebView storage for its appearance settings. Should
    // that WebView refuse this window's options, open with the recorder's own.
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
        WindowEvent::Focused(false) => {
            let close = {
                let state = state(&handle);
                state.focused && !state.dialog
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
            }
            handle.state::<Arc<Recorder>>().panel_closed(&handle);
        }
        _ => {}
    });
    Ok(())
}

/// The panel asks this once it has drawn: whether it opens on the Stop prompt.
pub fn take_ask_stop(app: &AppHandle) -> bool {
    std::mem::take(&mut state(app).ask_stop)
}

/// Size the panel to its content, place it above the tray icon and show it.
pub fn fit(app: &AppHandle, window: &WebviewWindow, height: f64) -> tauri::Result<()> {
    let anchor = state(app).anchor;
    place(app, window, anchor, height.clamp(40.0, 2000.0))?;
    let first = !std::mem::replace(&mut state(app).shown, true);
    if first {
        window.show()?;
        window.set_focus()?;
    }
    Ok(())
}

fn place(
    app: &AppHandle,
    window: &WebviewWindow,
    anchor: Option<Rect>,
    height: f64,
) -> tauri::Result<()> {
    // Tray rectangles are physical on Windows; scale 1 only finds the monitor.
    let point = anchor.map(|rect| {
        let position = rect.position.to_physical::<f64>(1.0);
        let size = rect.size.to_physical::<f64>(1.0);
        (position.x + size.width / 2.0, position.y, size.width)
    });
    let monitor = match point {
        Some((x, y, _)) => app.monitor_from_point(x, y)?,
        None => None,
    };
    let monitor = match monitor {
        Some(monitor) => Some(monitor),
        None => app.primary_monitor()?,
    };
    let Some(monitor) = monitor else {
        return window.set_size(tauri::LogicalSize::new(WIDTH, height));
    };
    let scale = monitor.scale_factor();
    let area = monitor.work_area();
    let width = (WIDTH * scale).round() as i32;
    let tall = (height * scale).round() as i32;
    let margin = (MARGIN * scale).round() as i32;
    let left = area.position.x;
    let top = area.position.y;
    let right = left + area.size.width as i32;
    let bottom = top + area.size.height as i32;
    // Without a tray position, use the work area's bottom-right corner.
    let (centre, icon_top) = point
        .map(|(x, y, _)| (x.round() as i32, y.round() as i32))
        .unwrap_or((right, bottom));
    let x = (centre - width / 2).clamp(left + margin, (right - width - margin).max(left + margin));
    let y =
        (icon_top - tall - margin).clamp(top + margin, (bottom - tall - margin).max(top + margin));
    window.set_size(PhysicalSize::new(width.max(1) as u32, tall.max(1) as u32))?;
    window.set_position(PhysicalPosition::new(x, y))
}

/// Run a native dialog without the panel closing when it takes focus.
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
}
