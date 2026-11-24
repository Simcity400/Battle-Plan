use crate::{tray::TRAY_ID, worker::Recorder};
use serde::Serialize;
use std::{
    path::PathBuf,
    sync::{Arc, Mutex},
    time::{Duration, Instant},
};
use tauri::{
    AppHandle, Manager, Monitor, PhysicalPosition, PhysicalSize, Rect, WebviewUrl,
    WebviewWindow, WebviewWindowBuilder, WindowEvent,
};

pub const LABEL: &str = "panel";
const WIDTH: f64 = 360.0;
const OPENING_HEIGHT: f64 = 120.0;
const MARGIN: f64 = 12.0;
const REOPEN_GUARD: Duration = Duration::from_millis(400);
const REPLAY_IDENTIFIER: &str = "local.battleplan.replay";
pub const CLOSED: &str = "This recorder panel has closed.";
const OPENING: &str = "This recorder panel is still opening.";

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Prompt {
    pub id: u64,
    pub quitting: bool,
    pub discarding: bool,
    pub session_key: u64,
}

#[derive(Default)]
pub struct Prompts {
    asked: Vec<Prompt>,
    next: u64,
}

impl Prompts {
    pub fn ask(&mut self, session_key: u64, quitting: bool) -> Prompt {
        let prompt = Prompt { id: self.next, quitting, discarding: !quitting, session_key };
        self.next += 1;
        self.asked.push(prompt);
        prompt
    }

    pub fn pending(&self, session_key: Option<u64>) -> Option<Prompt> {
        self.asked.iter().copied().filter(|prompt| Some(prompt.session_key) == session_key).reduce(|mut joined, prompt| {
            joined.quitting |= prompt.quitting;
            joined.discarding |= prompt.discarding;
            joined.id = joined.id.max(prompt.id);
            joined
        })
    }

    pub fn retire(&mut self, through: u64) {
        self.asked.retain(|prompt| prompt.id > through);
    }

    pub fn clear(&mut self) {
        self.asked.clear();
    }

    pub fn issued(&self) -> u64 {
        self.next
    }

    pub fn follow(&mut self, session_key: u64) -> Option<u64> {
        let quitting = self.asked.iter().any(|prompt| prompt.quitting);
        self.asked.clear();
        quitting.then(|| self.ask(session_key, true).id)
    }
}

#[derive(Default)]
struct State {
    anchor: Option<Rect>,
    closed_at: Option<Instant>,
    compact: bool,
    moved: Option<PhysicalPosition<i32>>,
    placed: Option<PhysicalPosition<i32>>,
    dialog: bool,
    focused: bool,
    shown: bool,
    fitted: bool,
    unseen: bool,
    dismissed: bool,
    opening: bool,
    instance: u64,
    live: Option<(u64, isize)>,
    recordings_panel: Option<String>,
    requests: usize,
    closing: bool,
}

pub struct Request(AppHandle);

pub fn request(app: &AppHandle) -> Request {
    state(app).requests += 1;
    Request(app.clone())
}

impl Drop for Request {
    fn drop(&mut self) {
        let close = state(&self.0).answered();
        if close && let Some(window) = self.0.get_webview_window(LABEL) {
            let _ = window.destroy();
        }
    }
}

fn dismiss_window(app: &AppHandle, window: &tauri::WebviewWindow) {
    let busy = state(app).dismissing();
    let _ = if busy { window.hide() } else { window.destroy() };
}

impl State {
    fn dismissing(&mut self) -> bool {
        self.closing |= self.requests > 0;
        self.closing
    }

    fn answered(&mut self) -> bool {
        self.requests -= 1;
        self.requests == 0 && std::mem::take(&mut self.closing)
    }

    fn live(&self, instance: u64) -> bool {
        self.live.is_some_and(|(live, _)| live == instance)
    }

    fn showed(&mut self, instance: u64) {
        let live = self.live(instance);
        self.shown |= live;
    }

    fn retire(&mut self, instance: u64) -> bool {
        let live = self.live(instance);
        if live {
            self.live = None;
        }
        live
    }

    fn opening_height(&self, instance: u64) -> Option<f64> {
        (self.live(instance) && !self.fitted).then_some(OPENING_HEIGHT)
    }

    fn instance_at(&self, hwnd: Option<isize>) -> Option<u64> {
        self.live.filter(|&(_, live)| Some(live) == hwnd).map(|(instance, _)| instance)
    }

    fn focus_lost(&mut self, instance: u64) -> bool {
        let live = self.live(instance);
        let close = live && closes_on_focus_loss(self.shown, self.focused, self.dialog);
        self.focused &= !live;
        self.dismissed |= close;
        close
    }

    fn dismiss(&mut self, instance: Option<u64>) {
        self.dismissed |= instance.is_some_and(|instance| self.live(instance));
    }

    fn register(&mut self, hwnd: Option<isize>, panel: &str) -> Result<(bool, Option<String>, bool), &'static str> {
        if self.instance_at(hwnd).is_none() {
            return Err(if self.opening && self.live.is_none() { OPENING } else { CLOSED });
        }
        let registered = self.closed_at.is_none() && self.recordings_panel.as_deref() != Some(panel);
        let previous = if registered { self.recordings_panel.replace(panel.to_owned()) } else { None };
        Ok((self.compact, previous, registered))
    }
}

#[derive(Default)]
pub struct Panel(Mutex<State>);

impl Panel {
    fn lock(&self) -> std::sync::MutexGuard<'_, State> {
        self.0.lock().unwrap_or_else(|error| error.into_inner())
    }

    fn at_window<T>(&self, hwnd: impl FnOnce() -> Option<isize>, read: impl FnOnce(&mut State, Option<isize>) -> T) -> T {
        let hwnd = hwnd();
        read(&mut self.lock(), hwnd)
    }
}

fn state(app: &AppHandle) -> std::sync::MutexGuard<'_, State> {
    app.state::<Panel>().inner().lock()
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

#[derive(Debug, PartialEq, Eq)]
enum Reopen {
    Focus,
    Show,
    Build,
}

fn reopen(exists: bool, visible: bool) -> Reopen {
    match (exists, visible) {
        (true, true) => Reopen::Focus,
        (true, false) => Reopen::Show,
        (false, _) => Reopen::Build,
    }
}

#[derive(Debug, PartialEq, Eq)]
enum Unseen {
    Gone,
    Show,
    Close,
}

fn answered(live: bool, retry: bool) -> Unseen {
    if !live {
        Unseen::Gone
    } else if retry {
        Unseen::Show
    } else {
        Unseen::Close
    }
}

fn closes_on_focus_loss(shown: bool, focused: bool, dialog: bool) -> bool {
    shown && focused && !dialog
}

pub fn toggle(app: &AppHandle, anchor: Rect) {
    let window = app.get_webview_window(LABEL);
    let (opening, closed_recently, closing) = {
        let state = state(app);
        (
            state.opening,
            state
                .closed_at
                .is_some_and(|closed| closed.elapsed() < REOPEN_GUARD),
            state.closing,
        )
    };
    match click(window.is_some() && !closing, opening, closed_recently) {
        Click::Close => {
            if let Some(window) = window {
                let instance = instance_of(app, &window);
                state(app).dismiss(instance);
                dismiss_window(app, &window);
            }
        }
        Click::Open => open(app, Some(anchor), false),
        Click::Ignore => {}
    }
}

pub fn open(app: &AppHandle, anchor: Option<Rect>, prompted: bool) {
    let window = app.get_webview_window(LABEL);
    let visible = window.as_ref().is_some_and(|window| window.is_visible().unwrap_or(false));
    match (reopen(window.is_some(), visible), window) {
        (Reopen::Focus, Some(window)) => {
            let _ = window.set_focus();
            return;
        }
        (Reopen::Show, Some(window)) => {
            state(app).closing = false;
            if let Some(instance) = instance_of(app, &window)
                && let Err(error) = show(app, &window, instance)
            {
                eprintln!("The recorder panel still couldn’t be shown: {error}");
            }
            return;
        }
        _ => {}
    }
    let anchor = anchor.or_else(|| {
        app.tray_by_id(TRAY_ID)
            .and_then(|tray| tray.rect().ok().flatten())
    });
    let instance = {
        let mut state = state(app);
        if state.opening {
            return;
        }
        *state = State {
            anchor,
            compact: prompted,
            moved: state.moved,
            opening: true,
            instance: state.instance + 1,
            ..State::default()
        };
        state.instance
    };
    let handle = app.clone();
    let spawned = std::thread::Builder::new()
        .name("recorder-panel".into())
        .spawn(move || {
            if let Err(error) = build(&handle, instance) {
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
        .title(app_update::recorder_name())
        .inner_size(WIDTH, OPENING_HEIGHT)
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

fn build(app: &AppHandle, instance: u64) -> tauri::Result<()> {
    let window = match builder(app, replay_webview_data(app)).build() {
        Ok(window) => window,
        Err(error) => {
            eprintln!("Opening the panel with the replay app's appearance failed: {error}");
            builder(app, None).build()?
        }
    };
    let hwnd = match window.hwnd() {
        Ok(hwnd) => hwnd.0 as isize,
        Err(error) => {
            let _ = window.destroy();
            return Err(error);
        }
    };
    {
        let mut state = state(app);
        if state.instance == instance {
            state.live = Some((instance, hwnd));
        }
    }
    let handle = app.clone();
    window.on_window_event(move |event| match event {
        WindowEvent::Focused(true) => {
            let mut state = state(&handle);
            let live = state.live(instance);
            state.focused |= live;
        }
        WindowEvent::Moved(position) => {
            let mut state = state(&handle);
            if state.live(instance) && state.shown && !state.compact && state.placed != Some(*position) {
                state.moved = Some(*position);
            }
        }
        WindowEvent::Focused(false) => {
            let close = state(&handle).focus_lost(instance);
            if close && let Some(window) = handle.get_webview_window(LABEL) {
                dismiss_window(&handle, &window);
            }
        }
        WindowEvent::Destroyed => {
            let (panel, dismissed) = {
                let mut state = state(&handle);
                if !state.retire(instance) {
                    return;
                }
                state.closed_at = Some(Instant::now());
                state.focused = false;
                state.shown = false;
                (state.recordings_panel.take(), std::mem::take(&mut state.dismissed))
            };
            if let Some(panel) = panel {
                handle.state::<crate::recordings::Library>().close(&panel);
            }
            handle.state::<Arc<Recorder>>().panel_closed(&handle, instance, dismissed);
        }
        _ => {}
    });
    let handle = app.clone();
    let opened = window.clone();
    let placed = app.run_on_main_thread(move || {
        let Some(height) = state(&handle).opening_height(instance) else { return };
        if let Err(error) = reveal(false, || place(&handle, &opened, height), || show(&handle, &opened, instance)) {
            eprintln!("The recorder panel couldn’t be placed as it opened: {error}");
        }
    });
    if let Err(error) = placed {
        eprintln!("The recorder panel couldn’t be placed as it opened, so it waits for its page to size it: {error}");
    }
    Ok(())
}

fn hwnd_of(window: &WebviewWindow) -> Option<isize> {
    window.hwnd().ok().map(|hwnd| hwnd.0 as isize)
}

pub fn instance_of(app: &AppHandle, window: &WebviewWindow) -> Option<u64> {
    app.state::<Panel>().at_window(|| hwnd_of(window), |state, hwnd| state.instance_at(hwnd))
}

pub fn opening(app: &AppHandle, window: &WebviewWindow, panel: &str) -> Result<bool, &'static str> {
    let (compact, previous, registered) = app.state::<Panel>().at_window(|| hwnd_of(window), |state, hwnd| state.register(hwnd, panel))?;
    let library = app.state::<crate::recordings::Library>();
    if let Some(previous) = previous {
        library.close(&previous);
    }
    if registered {
        library.register_panel(panel);
    }
    Ok(compact)
}

pub fn fit(app: &AppHandle, window: &WebviewWindow, height: f64) -> Result<(), String> {
    let instance = instance_of(app, window).ok_or(CLOSED)?;
    let shown = {
        let mut state = state(app);
        state.fitted = true;
        state.shown
    };
    reveal(shown, || place(app, window, height), || show(app, window, instance)).map_err(|error| error.to_string())
}

fn reveal<E>(shown: bool, place: impl FnOnce() -> Result<(), E>, show: impl FnOnce() -> Result<(), E>) -> Result<(), E> {
    let placed = place();
    if !shown {
        show()?;
    }
    placed
}

fn show(app: &AppHandle, window: &WebviewWindow, instance: u64) -> tauri::Result<()> {
    if let Err(error) = window.show() {
        unseen(app, window, instance, &error);
        return Err(error);
    }
    state(app).showed(instance);
    window.set_focus()
}

fn unseen(app: &AppHandle, window: &WebviewWindow, instance: u64, error: &tauri::Error) {
    use windows_sys::Win32::UI::WindowsAndMessaging::{IDRETRY, MB_ICONWARNING, MB_RETRYCANCEL, MessageBoxW};
    {
        let mut state = state(app);
        if !state.live(instance) || std::mem::replace(&mut state.unseen, true) {
            return;
        }
    }
    let wide = |text: &str| text.encode_utf16().chain(Some(0)).collect::<Vec<u16>>();
    let text = wide(&format!(
        "BattlePlan Recorder couldn’t show its panel: {error}\n\nChoose Retry to show it again, or Cancel to close it. The tray icon opens it again."
    ));
    let handle = app.clone();
    let window = window.clone();
    let asked = std::thread::Builder::new().name("recorder-panel-unseen".into()).spawn(move || {
        let caption = wide("BattlePlan Recorder");
        let retry = unsafe { MessageBoxW(std::ptr::null_mut(), text.as_ptr(), caption.as_ptr(), MB_RETRYCANCEL | MB_ICONWARNING) } == IDRETRY;
        let answer = {
            let mut state = state(&handle);
            let answer = answered(state.live(instance), retry);
            if answer != Unseen::Gone {
                state.unseen = false;
                state.dismissed |= answer == Unseen::Close;
            }
            answer
        };
        match answer {
            Unseen::Gone => {}
            Unseen::Close => {
                let _ = window.destroy();
            }
            Unseen::Show => {
                if show(&handle, &window, instance).is_err() {
                    eprintln!("The recorder panel still couldn’t be shown.");
                }
            }
        }
    });
    if let Err(spawned) = asked {
        eprintln!("The recorder panel couldn’t be shown, nor could BattlePlan Recorder say so: {spawned}");
        state(app).unseen = false;
    }
}

fn place(app: &AppHandle, window: &WebviewWindow, height: f64) -> tauri::Result<()> {
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
    fn a_prompt_is_shown_for_its_own_session_until_retired_and_a_newer_one_outlives_that() {
        let mut prompts = Prompts::default();
        let discard = prompts.ask(1, false);
        assert_eq!(prompts.pending(Some(1)), Some(discard));
        assert_eq!(prompts.pending(Some(2)), None, "a Discard asked about one session isn’t shown for another");
        assert_eq!(prompts.pending(None), None);
        prompts.retire(discard.id);
        assert_eq!(prompts.pending(Some(1)), None, "a reloaded panel isn’t asked what the user already cancelled");

        let quit = prompts.ask(1, true);
        let discard = prompts.ask(1, false);
        assert_eq!(
            prompts.pending(Some(1)),
            Some(Prompt { id: discard.id, quitting: true, discarding: true, session_key: 1 }),
            "prompts asked together are shown together"
        );
        prompts.retire(quit.id);
        assert_eq!(prompts.pending(Some(1)), Some(discard), "a prompt asked after the one handled stays");
        prompts.retire(discard.id);
        prompts.retire(discard.id);
        assert_eq!(prompts.pending(Some(1)), None, "retiring an id again changes nothing");
    }

    #[test]
    fn a_new_session_drops_the_discard_asked_about_the_old_one_and_asks_the_quit_again() {
        let mut prompts = Prompts::default();
        let quit = prompts.ask(1, true);
        prompts.ask(1, false);
        let asked_again = prompts.follow(2).expect("the Quit is asked again");
        assert_ne!(asked_again, quit.id, "with a fresh id, so the panel’s report of showing it about the old session doesn’t count");
        assert_eq!(prompts.pending(Some(1)), None);
        assert_eq!(prompts.pending(Some(2)), Some(Prompt { id: asked_again, session_key: 2, ..quit }));
    }

    #[test]
    fn a_click_while_the_panel_is_being_created_opens_no_second_window() {
        assert_eq!(click(false, true, false), Click::Ignore);
        assert_eq!(click(false, false, true), Click::Ignore);
        assert_eq!(click(true, false, false), Click::Close);
        assert_eq!(click(false, false, false), Click::Open);
    }

    #[test]
    fn a_panel_dismissed_while_its_save_is_out_hides_can_be_opened_again_and_survives_the_reply() {
        let mut state = State { requests: 1, ..State::default() };
        assert!(state.dismissing(), "a panel with a save out is hidden, not destroyed");
        assert_eq!(click(!state.closing, false, false), Click::Open, "so the tray opens it again instead of closing it twice");
        state.closing = false;
        assert!(!state.answered(), "the reply to a panel opened again destroys nothing");
        state.requests = 1;
        assert!(state.dismissing());
        assert!(state.answered(), "the reply to a dismissed panel closes it");
        assert!(!State::default().dismissing(), "a panel with nothing out closes at once");
    }

    #[test]
    fn a_panel_that_cant_be_sized_is_still_shown_so_its_failure_and_try_again_are_seen() {
        let mut shown = Vec::new();
        assert_eq!(reveal(false, || Err("no size"), || { shown.push("show"); Ok(()) }), Err("no size"));
        assert_eq!(shown, ["show"], "the hidden window is shown though it couldn’t be sized");
        assert_eq!(reveal(false, || Ok(()), || Err("no show")), Err("no show"));
        assert_eq!(reveal(true, || Err("no size"), || unreachable!("a shown panel isn’t shown again")), Err("no size"));
    }

    #[test]
    fn opening_a_panel_whose_window_is_still_hidden_shows_it_without_waiting_for_its_page() {
        assert_eq!(reopen(true, false), Reopen::Show, "a second Quit shows the hidden window, whatever its page has done");
        assert_eq!(reopen(true, true), Reopen::Focus);
        assert_eq!(reopen(false, false), Reopen::Build);
    }

    #[test]
    fn the_answer_to_a_panel_that_couldnt_be_shown_acts_only_on_that_panel_while_it_lives() {
        let mut state = State { instance: 3, live: Some((3, 30)), ..State::default() };
        assert_eq!(answered(state.live(3), false), Unseen::Close);
        assert_eq!(answered(state.live(3), true), Unseen::Show);
        assert!(state.retire(3), "the panel is destroyed while its message is open");
        assert_eq!(answered(state.live(3), true), Unseen::Gone, "Retry doesn’t show a panel that has gone, though none has opened since");
        assert_eq!(answered(state.live(3), false), Unseen::Gone);
        state.showed(3);
        assert!(!state.shown, "a show that reaches the dead window marks nothing shown");
        assert!(!state.retire(3), "its destruction is taken once");

        let mut state = State { instance: 4, live: Some((4, 30)), ..State::default() };
        assert_eq!(answered(state.live(3), true), Unseen::Gone, "nor the panel the tray opened in its place");
        state.showed(3);
        assert!(!state.shown, "so the panel opened in its place inherits nothing");
        assert!(!state.retire(3), "and the old window’s destruction doesn’t close it");
        assert!(state.live(4));
    }

    #[test]
    fn the_page_sizes_its_panel_whichever_runs_first_on_the_main_thread_its_fit_or_the_opening_placement() {
        let opened = |state: &State, height: &mut f64| {
            if let Some(opening) = state.opening_height(1) {
                *height = opening;
            }
        };
        let fitted = |state: &mut State, height: &mut f64| {
            state.fitted = true;
            *height = 300.0;
        };
        let mut state = State { instance: 1, live: Some((1, 10)), ..State::default() };
        let mut height = 0.0;
        fitted(&mut state, &mut height);
        opened(&state, &mut height);
        assert_eq!(height, 300.0, "the opening size never lands over the page’s");

        let mut state = State { instance: 1, live: Some((1, 10)), ..State::default() };
        let mut height = 0.0;
        opened(&state, &mut height);
        assert_eq!(height, OPENING_HEIGHT);
        fitted(&mut state, &mut height);
        assert_eq!(height, 300.0);

        let mut state = State { instance: 1, live: Some((1, 10)), ..State::default() };
        state.retire(1);
        assert_eq!(state.opening_height(1), None, "a panel gone before its placement runs isn’t placed");
    }

    #[test]
    fn focus_lost_before_the_panel_shows_does_not_close_it() {
        assert!(!closes_on_focus_loss(false, true, false));
        assert!(closes_on_focus_loss(true, true, false));
        assert!(!closes_on_focus_loss(true, true, true));
        assert!(!closes_on_focus_loss(true, false, false));
    }

    #[test]
    fn a_click_away_or_the_tray_icon_dismisses_the_panel_so_its_prompt_waits_without_taking_focus_back() {
        let mut state = State { instance: 1, live: Some((1, 10)), shown: true, focused: true, ..State::default() };
        assert!(state.focus_lost(1), "a click away closes the panel");
        assert!(state.dismissed, "and counts as the user dismissing it, so its prompt doesn’t reopen another panel");

        let mut state = State { instance: 2, live: Some((2, 20)), ..State::default() };
        state.dismiss(Some(1));
        assert!(!state.dismissed, "a retired panel’s dismissal says nothing of this one");
        state.dismiss(Some(2));
        assert!(state.dismissed, "the tray icon closing the panel dismisses it too");
    }

    #[test]
    fn the_panels_window_handle_is_read_before_its_state_is_locked() {
        let panel = Panel(Mutex::new(State { instance: 1, live: Some((1, 10)), ..State::default() }));
        let read = || {
            assert!(panel.0.try_lock().is_ok(), "the handle is read on the UI thread, which may itself wait on this lock");
            Some(10)
        };
        assert_eq!(panel.at_window(read, |state, hwnd| state.instance_at(hwnd)), Some(1));
        assert_eq!(panel.at_window(read, |state, hwnd| state.register(hwnd, "a")), Ok((false, None, true)));
    }

    #[test]
    fn a_page_that_asks_for_its_opening_state_before_its_window_is_registered_hears_to_try_again() {
        let mut state = State { instance: 1, opening: true, compact: true, ..State::default() };
        assert_eq!(state.register(Some(10), "a"), Err(OPENING), "no opening succeeds without registering its panel");
        state.live = Some((1, 10));
        assert_eq!(state.register(Some(10), "a"), Ok((true, None, true)));
        assert_eq!(state.register(Some(10), "a"), Ok((true, None, false)), "a second read registers nothing again");
        assert_eq!(state.register(Some(11), "b"), Err(CLOSED), "another window isn’t this panel");
        state.opening = false;
        state.retire(1);
        assert_eq!(state.register(Some(10), "a"), Err(CLOSED));
    }
}
