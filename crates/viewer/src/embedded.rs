//! Control pipe and a per-window Win32 adapter for the desktop host.
//!
//! eframe 0.36 calls `App::logic` when a hidden window requests a repaint. The
//! pipe wakes that hook only for work; the map's UI and GPU path can stay idle.
//! SetParent alone is insufficient: winit still caches top-level window flags
//! and eframe calls set_visible after its first paint. The subclass below keeps
//! only this process's creation-context HWND a child, with host-owned geometry.

use desktop_protocol::{
    EVENT_PREFIX, MAX_MESSAGE_BYTES, NativeCommand, NativeEvent, ViewBounds, ViewerState,
};
use eframe::egui;
use raw_window_handle::{HasWindowHandle as _, RawWindowHandle};
use std::{
    cell::Cell,
    io::{self, BufRead, Read as _, Write as _},
    os::windows::io::{AsRawHandle as _, FromRawHandle as _, OwnedHandle},
    path::{Path, PathBuf},
    ptr,
    sync::{
        Arc, Condvar, Mutex,
        atomic::{AtomicBool, Ordering},
        mpsc,
    },
    thread,
    time::{Duration, Instant},
};
use windows_sys::Win32::{
    Foundation::{GetLastError, HWND, LPARAM, LRESULT, RECT, SetLastError, WAIT_TIMEOUT, WPARAM},
    System::Threading::{
        GetCurrentThreadId, OpenProcess, PROCESS_SYNCHRONIZE, WaitForSingleObject,
    },
    UI::{
        HiDpi::{
            AreDpiAwarenessContextsEqual, DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2,
            GetDpiForWindow, GetWindowDpiAwarenessContext, SetThreadDpiAwarenessContext,
        },
        Input::KeyboardAndMouse::SetFocus,
        Shell::{DefSubclassProc, RemoveWindowSubclass, SetWindowSubclass},
        WindowsAndMessaging::*,
    },
};

const STATE_INTERVAL: Duration = Duration::from_millis(250);
const SUBCLASS_ID: usize = 0x484f4934;

#[derive(Clone, Debug, PartialEq)]
pub struct Seek {
    pub path: PathBuf,
    pub revision: String,
    pub branch: replay::BranchId,
    pub hour: i32,
}

impl Seek {
    pub fn matches(&self, path: &Path, revision: Option<&str>) -> bool {
        self.path.as_path() == path && revision == Some(self.revision.as_str())
    }

    pub fn open_request(&self) -> OpenRequest {
        OpenRequest {
            path: self.path.clone(),
            revision: Some(self.revision.clone()),
        }
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct OpenRequest {
    pub path: PathBuf,
    // Native picker/CLI requests establish their own snapshot at load time.
    pub revision: Option<String>,
}

fn validate_snapshot(path: &str, revision: &str) -> Result<(), String> {
    if path.is_empty() || path.contains('\0') {
        return Err("A replay path must be nonempty and contain no NUL".into());
    }
    if revision.is_empty() || revision.len() > 128 {
        return Err("A replay command must include its recording revision".into());
    }
    Ok(())
}

/// One slot per command kind. A new open invalidates seeks for the old replay;
/// a close is terminal, including if more input has already reached the pipe.
#[derive(Default)]
pub struct Pending {
    pub open: Option<OpenRequest>,
    pub seek: Option<Seek>,
    pub close: bool,
    pub error: Option<String>,
    bounds: Option<ViewBounds>,
    visibility: Option<bool>,
}

impl Pending {
    fn is_empty(&self) -> bool {
        self.open.is_none()
            && self.seek.is_none()
            && !self.close
            && self.bounds.is_none()
            && self.visibility.is_none()
            && self.error.is_none()
    }

    fn push(&mut self, command: NativeCommand) -> Result<(), String> {
        if self.close {
            return Ok(());
        }
        match command {
            NativeCommand::Visibility { visible } => self.visibility = Some(visible),
            NativeCommand::Bounds {
                x,
                y,
                width,
                height,
            } => {
                self.bounds = Some(
                    ViewBounds {
                        x,
                        y,
                        width,
                        height,
                    }
                    .validate()?,
                );
            }
            NativeCommand::Open { path, revision } => {
                validate_snapshot(&path, &revision)?;
                self.open = Some(OpenRequest {
                    path: PathBuf::from(path),
                    revision: Some(revision),
                });
                self.seek = None;
                self.error = None;
            }
            NativeCommand::Seek {
                path,
                revision,
                branch,
                hour,
            } => {
                validate_snapshot(&path, &revision)?;
                if branch.is_empty() || !branch.bytes().all(|byte| byte.is_ascii_digit()) {
                    return Err("A branch must be an unsigned decimal string".into());
                }
                self.seek = Some(Seek {
                    path: PathBuf::from(path),
                    revision,
                    branch: branch
                        .parse()
                        .map_err(|_| "Branch exceeds the replay's u64 id")?,
                    hour,
                });
                self.error = None;
            }
            NativeCommand::Close => {
                *self = Self {
                    close: true,
                    ..Self::default()
                };
            }
        }
        Ok(())
    }
}

/// Read at most one bounded line, even if the peer never supplies a newline.
fn read_command(
    reader: &mut impl BufRead,
    bytes: &mut Vec<u8>,
) -> io::Result<Option<NativeCommand>> {
    bytes.clear();
    let count = reader
        .take(MAX_MESSAGE_BYTES as u64 + 1)
        .read_until(b'\n', bytes)?;
    if count == 0 {
        return Ok(None);
    }
    if count > MAX_MESSAGE_BYTES || bytes.last() != Some(&b'\n') {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "Viewer command is oversized or lacks a newline",
        ));
    }
    serde_json::from_slice(bytes).map(Some).map_err(Into::into)
}

#[derive(Clone, Debug, PartialEq)]
struct ReportedState {
    viewer: ViewerState,
    error: Option<String>,
}

impl ReportedState {
    fn into_event(self) -> NativeEvent {
        NativeEvent::State {
            path: self.viewer.path,
            branch: self.viewer.branch,
            hour: self.viewer.hour,
            revision: self.viewer.revision,
            error: self.error,
        }
    }
}

#[derive(Default)]
struct Outgoing {
    ready: Option<String>,
    state: Option<ReportedState>,
    error: Option<String>,
}

impl Outgoing {
    fn is_empty(&self) -> bool {
        self.ready.is_none() && self.state.is_none() && self.error.is_none()
    }
}

struct Shared {
    pending: Mutex<Pending>,
    outgoing: Mutex<Outgoing>,
    output_ready: Condvar,
    stopped: AtomicBool,
    terminal_error: AtomicBool,
    ctx: egui::Context,
}

impl Shared {
    fn input(&self, command: NativeCommand) {
        let mut pending = self
            .pending
            .lock()
            .unwrap_or_else(|error| error.into_inner());
        let wake = pending.is_empty();
        if let Err(error) = pending.push(command) {
            pending.error = Some(error);
        }
        drop(pending);
        if wake {
            self.ctx.request_repaint();
        }
    }

    fn error(&self, message: impl Into<String>) {
        let message = bounded_message(&message.into());
        self.terminal_error.store(true, Ordering::Release);
        let mut outgoing = self
            .outgoing
            .lock()
            .unwrap_or_else(|error| error.into_inner());
        outgoing.state = None;
        outgoing.error = Some(message);
        drop(outgoing);
        self.output_ready.notify_one();
    }
}

fn bounded_message(message: &str) -> String {
    // Errors may include paths or peer input. Leave space for the other
    // fields and JSON escapes without splitting a UTF-8 character.
    let mut end = message.len().min(4096);
    while !message.is_char_boundary(end) {
        end -= 1;
    }
    message[..end].to_owned()
}

fn write_event(event: NativeEvent) -> io::Result<()> {
    let json = serde_json::to_string(&event)?;
    if json.len() + EVENT_PREFIX.len() + 1 > MAX_MESSAGE_BYTES {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "Viewer event exceeds the pipe's line budget",
        ));
    }
    let mut stdout = io::stdout().lock();
    writeln!(stdout, "{EVENT_PREFIX}{json}")?;
    stdout.flush()
}

/// Startup failures happen before the asynchronous writer exists.
pub fn startup_error(message: String) {
    if std::env::var_os("BATTLE_PLAN_VIEWER_PARENT").or_else(|| std::env::var_os("HOI4_VIEWER_PARENT")).is_some() {
        let _ = write_event(NativeEvent::Error {
            message: bounded_message(&message),
        });
    }
}

pub struct Parent {
    hwnd: usize,
    pid: u32,
    process: OwnedHandle,
    previous_dpi: usize,
}

impl Parent {
    pub fn from_environment() -> io::Result<Option<Self>> {
        let Some(value) = std::env::var_os("BATTLE_PLAN_VIEWER_PARENT").or_else(|| std::env::var_os("HOI4_VIEWER_PARENT")) else {
            return Ok(None);
        };
        let hwnd = value
            .to_str()
            .filter(|value| !value.is_empty() && value.bytes().all(|byte| byte.is_ascii_digit()))
            .and_then(|value| value.parse::<usize>().ok())
            .filter(|hwnd| *hwnd != 0)
            .ok_or_else(|| io::Error::other("BATTLE_PLAN_VIEWER_PARENT must be a nonzero decimal HWND"))?;
        let mut pid = 0;
        // SAFETY: query APIs validate this untrusted handle; no pointer is read.
        unsafe {
            if IsWindow(hwnd as HWND) == 0
                || GetWindowThreadProcessId(hwnd as HWND, &mut pid) == 0
                || pid == std::process::id()
            {
                return Err(io::Error::other(
                    "The desktop parent is not a live external window",
                ));
            }
            let dpi = GetWindowDpiAwarenessContext(hwnd as HWND);
            if AreDpiAwarenessContextsEqual(dpi, DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2) == 0 {
                return Err(io::Error::other(
                    "The desktop host must use per-monitor-v2 DPI awareness",
                ));
            }
            let process = OpenProcess(PROCESS_SYNCHRONIZE, 0, pid);
            if process.is_null() {
                return Err(io::Error::last_os_error());
            }
            let process = OwnedHandle::from_raw_handle(process);
            let previous_dpi = SetThreadDpiAwarenessContext(dpi);
            if previous_dpi.is_null() {
                return Err(io::Error::last_os_error());
            }
            Ok(Some(Self {
                hwnd,
                pid,
                process,
                previous_dpi: previous_dpi as usize,
            }))
        }
    }

    pub fn attach(self, cc: &eframe::CreationContext<'_>) -> io::Result<Controller> {
        let process = self.process.try_clone()?;
        let parent_hwnd = self.hwnd;
        let parent_pid = self.pid;
        let window = NativeWindow::attach(self, cc)?;
        let shared = Arc::new(Shared {
            pending: Mutex::new(Pending::default()),
            outgoing: Mutex::new(Outgoing::default()),
            output_ready: Condvar::new(),
            stopped: AtomicBool::new(false),
            terminal_error: AtomicBool::new(false),
            ctx: cc.egui_ctx.clone(),
        });
        let (output_finished, output_done) = mpsc::sync_channel::<()>(0);
        let controller = Controller {
            window,
            shared: Arc::clone(&shared),
            last_state: None,
            last_report: None,
            closing: false,
            output_done,
        };
        let input = Arc::clone(&shared);
        thread::Builder::new()
            .name("viewer-commands".into())
            .spawn(move || {
                let mut stdin = io::stdin().lock();
                let mut bytes = Vec::new();
                while !input.stopped.load(Ordering::Acquire) {
                    match read_command(&mut stdin, &mut bytes) {
                        Ok(Some(command)) => {
                            let close = matches!(command, NativeCommand::Close);
                            input.input(command);
                            if close {
                                return;
                            }
                        }
                        Ok(None) => break,
                        Err(error) => {
                            input.error(format!("Viewer control pipe: {error}"));
                            break;
                        }
                    }
                }
                input.input(NativeCommand::Close);
            })?;
        let output = Arc::clone(&shared);
        thread::Builder::new()
            .name("viewer-events".into())
            .spawn(move || {
                // Disconnecting this zero-capacity channel signals that all
                // queued output was written, without blocking the writer.
                let _finished = output_finished;
                loop {
                    let mut outgoing = output
                        .outgoing
                        .lock()
                        .unwrap_or_else(|error| error.into_inner());
                    while outgoing.is_empty() && !output.stopped.load(Ordering::Acquire) {
                        outgoing = output
                            .output_ready
                            .wait(outgoing)
                            .unwrap_or_else(|error| error.into_inner());
                    }
                    if outgoing.is_empty() {
                        return;
                    }
                    let batch = std::mem::take(&mut *outgoing);
                    drop(outgoing);
                    let events = [
                        batch.ready.map(|hwnd| NativeEvent::Ready { hwnd }),
                        batch.state.map(ReportedState::into_event),
                        batch.error.map(|message| NativeEvent::Error { message }),
                    ];
                    for event in events.into_iter().flatten() {
                        if write_event(event).is_err() {
                            output.input(NativeCommand::Close);
                            return;
                        }
                    }
                }
            })?;
        let watch = Arc::clone(&shared);
        thread::Builder::new()
            .name("viewer-parent".into())
            .spawn(move || {
                let mut visible = false;
                while !watch.stopped.load(Ordering::Acquire) {
                    // SAFETY: the owned process handle stays open throughout the
                    // wait. HWND ownership is rechecked to catch handle reuse.
                    let alive = unsafe {
                        WaitForSingleObject(process.as_raw_handle(), 250) == WAIT_TIMEOUT
                    } && parent_alive(parent_hwnd, parent_pid);
                    if !alive {
                        watch.input(NativeCommand::Close);
                        break;
                    }
                    let shown = unsafe {
                        IsWindowVisible(parent_hwnd as HWND) != 0
                            && IsIconic(parent_hwnd as HWND) == 0
                    };
                    if shown != visible {
                        visible = shown;
                        watch.ctx.request_repaint();
                    }
                }
            })?;
        shared
            .outgoing
            .lock()
            .unwrap_or_else(|error| error.into_inner())
            .ready = Some(controller.window.hwnd.to_string());
        shared.output_ready.notify_one();
        cc.egui_ctx.request_repaint();
        Ok(controller)
    }
}

impl Drop for Parent {
    fn drop(&mut self) {
        // Parent is created, attached and dropped on eframe's UI thread.
        unsafe {
            SetThreadDpiAwarenessContext(self.previous_dpi as _);
        }
    }
}

pub struct Controller {
    window: NativeWindow,
    shared: Arc<Shared>,
    last_state: Option<ReportedState>,
    last_report: Option<Instant>,
    closing: bool,
    output_done: mpsc::Receiver<()>,
}

impl Controller {
    pub fn poll(&mut self, ctx: &egui::Context) -> Pending {
        let mut pending = std::mem::take(
            &mut *self
                .shared
                .pending
                .lock()
                .unwrap_or_else(|error| error.into_inner()),
        );
        pending.close |= self.closing;
        if !pending.close {
            let applied = (|| {
                if let Some(bounds) = pending.bounds {
                    self.window.bounds(bounds)?;
                }
                if let Some(visible) = pending.visibility {
                    self.window.visibility(visible);
                }
                Ok::<_, io::Error>(())
            })();
            if let Err(error) = applied {
                self.error(format!("Embedding window: {error}"));
                pending.close = true;
            }
        }
        if pending.close {
            self.closing = true;
            self.window.visibility(false);
            ctx.send_viewport_cmd(egui::ViewportCommand::Close);
        }
        pending
    }

    pub fn visible(&self) -> bool {
        self.window.visible()
    }

    fn error(&self, message: impl Into<String>) {
        self.shared.error(message);
    }

    pub fn report(&mut self, state: ViewerState, error: Option<&str>, ctx: &egui::Context) {
        let state = ReportedState {
            viewer: state,
            error: error.map(bounded_message),
        };
        if self.last_state.as_ref() == Some(&state) {
            return;
        }
        if let Some(last) = self.last_report
            && let Some(remaining) = STATE_INTERVAL.checked_sub(last.elapsed())
        {
            ctx.request_repaint_after(remaining);
            return;
        }
        self.last_state = Some(state.clone());
        self.last_report = Some(Instant::now());
        let mut outgoing = self
            .shared
            .outgoing
            .lock()
            .unwrap_or_else(|error| error.into_inner());
        if self.shared.terminal_error.load(Ordering::Acquire) {
            return;
        }
        outgoing.state = Some(state);
        drop(outgoing);
        self.shared.output_ready.notify_one();
    }
}

impl Drop for Controller {
    fn drop(&mut self) {
        {
            // Pair shutdown with the condition variable's mutex so a writer
            // entering its wait cannot miss the final notification.
            let _outgoing = self
                .shared
                .outgoing
                .lock()
                .unwrap_or_else(|error| error.into_inner());
            self.shared.stopped.store(true, Ordering::Release);
        }
        self.shared.output_ready.notify_one();
        // Let a healthy host receive the final error/state before exit, but
        // never hang shutdown on a peer that stopped draining stdout.
        let _ = self.output_done.recv_timeout(Duration::from_millis(100));
        // Close/EOF ends the reader; local window-close may leave it blocked
        // until process exit. Never join a pipe reader or writer on the UI
        // thread. The host owns and reaps this process as a final backstop.
    }
}

fn parent_alive(hwnd: usize, expected_pid: u32) -> bool {
    let mut pid = 0;
    unsafe {
        IsWindow(hwnd as HWND) != 0
            && GetWindowThreadProcessId(hwnd as HWND, &mut pid) != 0
            && pid == expected_pid
    }
}

struct Hook {
    visible: Cell<bool>,
    bounds: Cell<Option<ViewBounds>>,
    dpi: Cell<u32>,
    installed: Cell<bool>,
}

impl Hook {
    fn show(&self) -> bool {
        self.visible.get() && self.bounds.get().is_some()
    }

    fn style(&self, style: u32) -> u32 {
        let style = (style & !(WS_POPUP | WS_OVERLAPPEDWINDOW | WS_VISIBLE))
            | WS_CHILD
            | WS_CLIPCHILDREN
            | WS_CLIPSIBLINGS;
        style | if self.show() { WS_VISIBLE } else { 0 }
    }

    fn ex_style(style: u32) -> u32 {
        style & !(WS_EX_APPWINDOW | WS_EX_WINDOWEDGE | WS_EX_CLIENTEDGE | WS_EX_DLGMODALFRAME)
    }
}

struct NativeWindow {
    parent: Parent,
    hwnd: usize,
    hook: Box<Hook>,
}

impl NativeWindow {
    fn attach(parent: Parent, cc: &eframe::CreationContext<'_>) -> io::Result<Self> {
        let RawWindowHandle::Win32(handle) = cc.window_handle().map_err(io::Error::other)?.as_raw()
        else {
            return Err(io::Error::other(
                "The native viewer did not create a Win32 window",
            ));
        };
        let hwnd = handle.hwnd.get() as HWND;
        let mut pid = 0;
        // SAFETY: this HWND comes directly from the creation context and all
        // following mutations are limited to that process's UI-thread window.
        unsafe {
            if GetWindowThreadProcessId(hwnd, &mut pid) != GetCurrentThreadId()
                || pid != std::process::id()
                || !parent_alive(parent.hwnd, parent.pid)
            {
                return Err(io::Error::other(
                    "Viewer or parent window ownership changed during startup",
                ));
            }
            if AreDpiAwarenessContextsEqual(
                GetWindowDpiAwarenessContext(hwnd),
                GetWindowDpiAwarenessContext(parent.hwnd as HWND),
            ) == 0
            {
                return Err(io::Error::other(
                    "Host and viewer DPI awareness differ; refusing cross-process SetParent",
                ));
            }
            let window = Self {
                parent,
                hwnd: hwnd as usize,
                hook: Box::new(Hook {
                    visible: Cell::new(false),
                    bounds: Cell::new(None),
                    dpi: Cell::new(GetDpiForWindow(hwnd)),
                    installed: Cell::new(false),
                }),
            };
            if SetWindowSubclass(
                hwnd,
                Some(window_proc),
                SUBCLASS_ID,
                (&*window.hook as *const Hook) as usize,
            ) == 0
            {
                return Err(io::Error::other(
                    "Cannot install the viewer's child-window adapter",
                ));
            }
            window.hook.installed.set(true);
            ShowWindow(hwnd, SW_HIDE);
            set_style(
                hwnd,
                GWL_STYLE,
                window.hook.style(GetWindowLongPtrW(hwnd, GWL_STYLE) as u32),
            )?;
            set_style(
                hwnd,
                GWL_EXSTYLE,
                Hook::ex_style(GetWindowLongPtrW(hwnd, GWL_EXSTYLE) as u32),
            )?;
            SetLastError(0);
            let previous = SetParent(hwnd, window.parent.hwnd as HWND);
            if previous.is_null() && GetLastError() != 0 {
                return Err(io::Error::last_os_error());
            }
            if GetParent(hwnd) != window.parent.hwnd as HWND {
                return Err(io::Error::other(
                    "SetParent did not attach the viewer to its host",
                ));
            }
            if SetWindowPos(
                hwnd,
                ptr::null_mut(),
                0,
                0,
                1,
                1,
                SWP_NOACTIVATE | SWP_NOZORDER | SWP_FRAMECHANGED,
            ) == 0
            {
                return Err(io::Error::last_os_error());
            }
            window.sync_dpi();
            Ok(window)
        }
    }

    fn bounds(&self, bounds: ViewBounds) -> io::Result<()> {
        let bounds = bounds.validate().map_err(io::Error::other)?;
        if !self.hook.installed.get() || !parent_alive(self.parent.hwnd, self.parent.pid) {
            return Err(io::Error::other("The embedded viewer's parent closed"));
        }
        self.hook.bounds.set(Some(bounds));
        self.sync_dpi();
        let visibility = if self.hook.show() {
            SWP_SHOWWINDOW
        } else {
            SWP_HIDEWINDOW
        };
        let success = unsafe {
            SetWindowPos(
                self.hwnd as HWND,
                HWND_TOP,
                bounds.x,
                bounds.y,
                bounds.width as i32,
                bounds.height as i32,
                SWP_NOACTIVATE | visibility,
            )
        };
        if success == 0 {
            Err(io::Error::last_os_error())
        } else {
            Ok(())
        }
    }

    fn visibility(&self, visible: bool) {
        self.hook.visible.set(visible);
        if self.hook.installed.get() {
            unsafe {
                ShowWindow(
                    self.hwnd as HWND,
                    if self.hook.show() {
                        SW_SHOWNOACTIVATE
                    } else {
                        SW_HIDE
                    },
                );
                if self.visible()
                    && GetForegroundWindow() == GetAncestor(self.parent.hwnd as HWND, GA_ROOT)
                {
                    SetFocus(self.hwnd as HWND);
                }
            }
        }
    }

    fn visible(&self) -> bool {
        self.hook.show()
            && self.hook.installed.get()
            && unsafe {
                IsWindowVisible(self.hwnd as HWND) != 0 && IsIconic(self.parent.hwnd as HWND) == 0
            }
    }

    fn sync_dpi(&self) {
        unsafe {
            let dpi = GetDpiForWindow(self.hwnd as HWND);
            if dpi != 0 && dpi != self.hook.dpi.get() {
                let mut rect = RECT::default();
                if GetWindowRect(self.hwnd as HWND, &mut rect) != 0 {
                    SendMessageW(
                        self.hwnd as HWND,
                        WM_DPICHANGED,
                        (dpi | (dpi << 16)) as usize,
                        (&rect as *const RECT) as LPARAM,
                    );
                }
            }
        }
    }
}

impl Drop for NativeWindow {
    fn drop(&mut self) {
        self.visibility(false);
        if self.hook.installed.replace(false) {
            // Remove before freeing reference data, even when app creation
            // failed. WM_NCDESTROY removes it itself if eframe closed first.
            unsafe {
                RemoveWindowSubclass(self.hwnd as HWND, Some(window_proc), SUBCLASS_ID);
            }
        }
    }
}

unsafe fn set_style(hwnd: HWND, index: i32, style: u32) -> io::Result<()> {
    unsafe {
        SetLastError(0);
        let previous = SetWindowLongPtrW(hwnd, index, style as isize);
        if previous == 0 && GetLastError() != 0 {
            Err(io::Error::last_os_error())
        } else {
            Ok(())
        }
    }
}

unsafe extern "system" fn window_proc(
    hwnd: HWND,
    message: u32,
    wparam: WPARAM,
    lparam: LPARAM,
    _id: usize,
    data: usize,
) -> LRESULT {
    // SAFETY: SetWindowSubclass supplies this stable Box pointer only on the
    // owning UI thread. Drop removes the subclass before freeing the Box. The
    // OS supplies each message's documented struct pointer synchronously.
    unsafe {
        let hook = &*(data as *const Hook);
        match message {
            WM_LBUTTONDOWN | WM_MBUTTONDOWN | WM_RBUTTONDOWN | WM_XBUTTONDOWN => {
                SetFocus(hwnd);
            }
            WM_SETFOCUS | WM_KILLFOCUS => {
                // winit gates Focused on both keyboard focus and nonclient
                // activation. A child has no independently activated frame;
                // mirror its real focus into that cached activation flag.
                // -1 prevents nonclient repaint during this notification.
                DefSubclassProc(hwnd, WM_NCACTIVATE, usize::from(message == WM_SETFOCUS), -1);
            }
            WM_STYLECHANGING => {
                let styles = &mut *(lparam as *mut STYLESTRUCT);
                if wparam as i32 == GWL_STYLE {
                    styles.styleNew = hook.style(styles.styleNew);
                } else if wparam as i32 == GWL_EXSTYLE {
                    styles.styleNew = Hook::ex_style(styles.styleNew);
                }
            }
            WM_WINDOWPOSCHANGING => {
                let position = &mut *(lparam as *mut WINDOWPOS);
                if !hook.show() {
                    position.flags = (position.flags & !SWP_SHOWWINDOW) | SWP_HIDEWINDOW;
                }
                if let Some(bounds) = hook.bounds.get() {
                    if position.flags & SWP_NOMOVE == 0 {
                        position.x = bounds.x;
                        position.y = bounds.y;
                    }
                    if position.flags & SWP_NOSIZE == 0 {
                        position.cx = bounds.width as i32;
                        position.cy = bounds.height as i32;
                    }
                }
            }
            WM_DPICHANGED => {
                hook.dpi.set((wparam & 0xffff) as u32);
            }
            WM_DPICHANGED_AFTERPARENT => {
                // Per-monitor-v2 child windows receive this notification, but
                // winit 0.30 only updates its cached scale for WM_DPICHANGED.
                // Host geometry remains authoritative through WINDOWPOS above.
                let dpi = GetDpiForWindow(hwnd);
                let mut rect = RECT::default();
                if dpi != 0 && dpi != hook.dpi.get() && GetWindowRect(hwnd, &mut rect) != 0 {
                    hook.dpi.set(dpi);
                    DefSubclassProc(
                        hwnd,
                        WM_DPICHANGED,
                        (dpi | (dpi << 16)) as usize,
                        (&rect as *const RECT) as LPARAM,
                    );
                }
            }
            WM_NCDESTROY => {
                hook.installed.set(false);
                RemoveWindowSubclass(hwnd, Some(window_proc), SUBCLASS_ID);
            }
            _ => {}
        }
        DefSubclassProc(hwnd, message, wparam, lparam)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn coalescing_preserves_open_seek_order_and_terminal_close() {
        let mut pending = Pending::default();
        for hour in 0..10_000 {
            pending
                .push(NativeCommand::Seek {
                    path: "initial.battleplan".into(),
                    revision: "100:10:0".into(),
                    branch: u64::MAX.to_string(),
                    hour,
                })
                .unwrap();
        }
        assert_eq!(
            pending.seek,
            Some(Seek {
                path: PathBuf::from("initial.battleplan"),
                revision: "100:10:0".into(),
                branch: u64::MAX,
                hour: 9999
            })
        );
        pending
            .push(NativeCommand::Open {
                path: "new.battleplan".into(),
                revision: "100:20:0".into(),
            })
            .unwrap();
        assert_eq!(pending.seek, None);
        assert_eq!(
            pending.open.as_ref().unwrap().revision.as_deref(),
            Some("100:20:0")
        );
        pending
            .push(NativeCommand::Seek {
                path: "new.battleplan".into(),
                revision: "100:20:0".into(),
                branch: "2".into(),
                hour: 12,
            })
            .unwrap();
        assert_eq!(
            pending.seek,
            Some(Seek {
                path: PathBuf::from("new.battleplan"),
                revision: "100:20:0".into(),
                branch: 2,
                hour: 12
            })
        );
        pending.push(NativeCommand::Close).unwrap();
        pending
            .push(NativeCommand::Open {
                path: "late.battleplan".into(),
                revision: "100:30:0".into(),
            })
            .unwrap();
        assert!(pending.close && pending.open.is_none() && pending.seek.is_none());
    }

    #[test]
    fn coalesced_seeks_retain_their_own_snapshot_even_with_the_same_position() {
        let mut pending = Pending::default();
        pending
            .push(NativeCommand::Open {
                path: "older.battleplan".into(),
                revision: "100:10:0".into(),
            })
            .unwrap();
        for (path, revision) in [
            ("A.battleplan", "100:20:0"),
            ("B.battleplan", "100:20:0"),
            ("B.battleplan", "120:30:0"),
        ] {
            pending
                .push(NativeCommand::Seek {
                    path: path.into(),
                    revision: revision.into(),
                    branch: u64::MAX.to_string(),
                    hour: 12,
                })
                .unwrap();
            let seek = pending.seek.as_ref().unwrap();
            assert_eq!(seek.path, PathBuf::from(path));
            assert_eq!(seek.revision, revision);
            assert_eq!(seek.branch, u64::MAX);
            assert_eq!(seek.hour, 12);
        }
        // The app can supersede the earlier open using the seek's target;
        // neither the earlier open nor another replay's position identifies it.
        assert_eq!(
            pending.seek.as_ref().unwrap().open_request(),
            OpenRequest {
                path: PathBuf::from("B.battleplan"),
                revision: Some("120:30:0".into()),
            }
        );
        assert_eq!(
            pending.open.as_ref().unwrap().path,
            PathBuf::from("older.battleplan")
        );
    }

    #[test]
    fn malformed_seek_target_cannot_replace_a_valid_pending_seek() {
        let mut pending = Pending::default();
        pending
            .push(NativeCommand::Seek {
                path: "A.battleplan".into(),
                revision: "100:20:0".into(),
                branch: "1".into(),
                hour: 12,
            })
            .unwrap();
        let retained = pending.seek.clone();
        for (path, revision) in [
            (String::new(), "100:20:0".into()),
            ("B\0.battleplan".into(), "100:20:0".into()),
            ("B.battleplan".into(), String::new()),
            ("B.battleplan".into(), "x".repeat(129)),
        ] {
            assert!(
                pending
                    .push(NativeCommand::Seek {
                        path,
                        revision,
                        branch: "1".into(),
                        hour: 12,
                    })
                    .is_err()
            );
            assert_eq!(pending.seek, retained);
        }
        let mut bytes = Vec::new();
        assert!(
            read_command(
                &mut &b"{\"command\":\"seek\",\"branch\":\"1\",\"hour\":12}\n"[..],
                &mut bytes,
            )
            .is_err()
        );
    }

    #[test]
    fn error_only_changes_are_part_of_the_same_revision_state() {
        let mut reported = ReportedState {
            viewer: ViewerState {
                path: Some("session.battleplan".into()),
                branch: Some(u64::MAX.to_string()),
                hour: Some(12),
                revision: Some("100:20:0".into()),
            },
            error: None,
        };
        let previous = reported.clone();
        reported.error = Some("Requested hour is outside the branch".into());
        assert_ne!(previous, reported);
        let NativeEvent::State {
            revision, error, ..
        } = reported.into_event()
        else {
            panic!("wrong event");
        };
        assert_eq!(revision.as_deref(), Some("100:20:0"));
        assert_eq!(
            error.as_deref(),
            Some("Requested hour is outside the branch")
        );
    }

    #[test]
    fn command_reader_bounds_partial_lines_and_accepts_crlf() {
        let mut bytes = Vec::new();
        assert!(matches!(
            read_command(&mut &b"{\"command\":\"close\"}\r\n"[..], &mut bytes).unwrap(),
            Some(NativeCommand::Close)
        ));
        let oversized = vec![b'x'; MAX_MESSAGE_BYTES + 500];
        assert!(read_command(&mut oversized.as_slice(), &mut bytes).is_err());
        assert!(bytes.len() <= MAX_MESSAGE_BYTES + 1);
        assert!(read_command(&mut &b"{\"command\":\"close\"}"[..], &mut bytes).is_err());
        assert!(read_command(&mut &b""[..], &mut bytes).unwrap().is_none());
    }

    #[test]
    fn child_styles_require_bounds_before_visibility() {
        let hook = Hook {
            visible: Cell::new(false),
            bounds: Cell::new(None),
            dpi: Cell::new(96),
            installed: Cell::new(true),
        };
        let style = hook.style(WS_OVERLAPPEDWINDOW | WS_POPUP | WS_VISIBLE);
        assert_eq!(style & (WS_OVERLAPPEDWINDOW | WS_POPUP | WS_VISIBLE), 0);
        assert_ne!(style & WS_CHILD, 0);
        hook.visible.set(true);
        assert_eq!(hook.style(0) & WS_VISIBLE, 0);
        hook.bounds.set(Some(ViewBounds {
            x: 70,
            y: 0,
            width: 800,
            height: 600,
        }));
        assert_ne!(hook.style(0) & WS_VISIBLE, 0);
    }
}
