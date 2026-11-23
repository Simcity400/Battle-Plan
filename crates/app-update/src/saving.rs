use std::{
    sync::{Arc, Mutex, PoisonError},
    thread,
};
use windows_sys::Win32::{
    Foundation::{HWND, LPARAM, LRESULT, RECT, WPARAM},
    Graphics::Gdi::{COLOR_WINDOW, DEFAULT_GUI_FONT, GetStockObject, HBRUSH},
    System::LibraryLoader::GetModuleHandleW,
    UI::WindowsAndMessaging::*,
};

const SAVED: u32 = WM_APP + 1;
const SAY: u32 = WM_APP + 2;
const SS_CENTER: u32 = 0x1;
const SS_CENTERIMAGE: u32 = 0x200;
const CLASS: &str = "BattlePlanSaving";
const WIDTH: i32 = 420;
const HEIGHT: i32 = 130;
const UNSEEN: std::time::Duration = std::time::Duration::from_millis(300);
const NOTICED: std::time::Duration = std::time::Duration::from_secs(4);

pub fn wait_until_empty(
    title: &str,
    notices: &[String],
    empty_within: &dyn Fn(std::time::Duration) -> bool,
    held: &dyn Fn() -> Option<&'static str>,
    hurry: &dyn Fn() -> bool,
) {
    if notices.is_empty() && empty_within(UNSEEN) {
        return;
    }
    let text = || held().into_iter().map(str::to_owned).chain(notices.iter().cloned()).collect::<Vec<_>>().join("\n");
    let shown = std::time::Instant::now();
    let saving = Saving::show(title, &text());
    while !empty_within(std::time::Duration::from_secs(1)) {
        saving.say(&text());
    }
    if notices.is_empty() {
        return;
    }
    saving.say(&text());
    while shown.elapsed() < NOTICED && !hurry() {
        thread::sleep(std::time::Duration::from_millis(100));
    }
}

#[derive(Default)]
struct Shown {
    hwnd: usize,
    text: Vec<u16>,
    ended: bool,
}

pub struct Saving(Arc<Mutex<Shown>>);

impl Saving {
    pub fn show(title: &str, text: &str) -> Self {
        let shown = Arc::new(Mutex::new(Shown { text: wide(text), ..Shown::default() }));
        let window = Arc::clone(&shown);
        let title = title.to_owned();
        if let Err(error) = thread::Builder::new().name("saving".into()).spawn(move || run(&window, &title)) {
            eprintln!("Couldn’t show what is still being finished before closing: {error}");
        }
        Self(shown)
    }

    pub fn say(&self, text: &str) {
        let mut shown = self.0.lock().unwrap_or_else(PoisonError::into_inner);
        shown.text = wide(text);
        if shown.hwnd != 0 {
            // SAFETY: posting to a window handle is sound even once the window has gone.
            unsafe { PostMessageW(shown.hwnd as HWND, SAY, 0, 0) };
        }
    }
}

impl Drop for Saving {
    fn drop(&mut self) {
        let mut shown = self.0.lock().unwrap_or_else(PoisonError::into_inner);
        shown.ended = true;
        if shown.hwnd != 0 {
            // SAFETY: posting to a window handle is sound even once the window has gone.
            unsafe { PostMessageW(shown.hwnd as HWND, SAVED, 0, 0) };
        }
    }
}

fn wide(text: &str) -> Vec<u16> {
    text.encode_utf16().chain(Some(0)).collect()
}

fn run(shown: &Arc<Mutex<Shown>>, title: &str) {
    let (class, title, label) = (wide(CLASS), wide(title), wide("STATIC"));
    // SAFETY: every call below runs on this thread, which owns both windows and their message
    // loop, and every string outlives the calls that read it. The label's handle is kept in the
    // window's user data, read only by this thread's window procedure.
    unsafe {
        let instance = GetModuleHandleW(std::ptr::null());
        let mut description: WNDCLASSEXW = std::mem::zeroed();
        description.cbSize = size_of::<WNDCLASSEXW>() as u32;
        description.lpfnWndProc = Some(window_proc);
        description.hInstance = instance;
        description.hCursor = LoadCursorW(std::ptr::null_mut(), IDC_WAIT);
        description.hbrBackground = (COLOR_WINDOW + 1) as usize as HBRUSH;
        description.lpszClassName = class.as_ptr();
        RegisterClassExW(&description);
        let x = (GetSystemMetrics(SM_CXSCREEN) - WIDTH) / 2;
        let y = (GetSystemMetrics(SM_CYSCREEN) - HEIGHT) / 2;
        let hwnd = CreateWindowExW(
            WS_EX_TOPMOST | WS_EX_APPWINDOW | WS_EX_DLGMODALFRAME,
            class.as_ptr(),
            title.as_ptr(),
            WS_POPUP | WS_CAPTION,
            x,
            y,
            WIDTH,
            HEIGHT,
            std::ptr::null_mut(),
            std::ptr::null_mut(),
            instance,
            std::ptr::null(),
        );
        if hwnd.is_null() {
            eprintln!("Couldn’t show what is still being finished before closing: {}", std::io::Error::last_os_error());
            return;
        }
        let mut client = RECT::default();
        GetClientRect(hwnd, &mut client);
        let text = shown.lock().unwrap_or_else(PoisonError::into_inner).text.clone();
        let line = CreateWindowExW(
            0,
            label.as_ptr(),
            text.as_ptr(),
            WS_CHILD | WS_VISIBLE | SS_CENTER | SS_CENTERIMAGE,
            8,
            0,
            client.right - 16,
            client.bottom,
            hwnd,
            std::ptr::null_mut(),
            instance,
            std::ptr::null(),
        );
        if !line.is_null() {
            SendMessageW(line, WM_SETFONT, GetStockObject(DEFAULT_GUI_FONT) as usize, 0);
        }
        SetWindowLongPtrW(hwnd, GWLP_USERDATA, Arc::into_raw(Arc::clone(shown)) as isize);
        SetPropW(hwnd, wide("line").as_ptr(), line.cast());
        ShowWindow(hwnd, SW_SHOWNORMAL);
        let ended = {
            let mut state = shown.lock().unwrap_or_else(PoisonError::into_inner);
            state.hwnd = hwnd as usize;
            state.ended
        };
        if ended {
            DestroyWindow(hwnd);
        }
        let mut message: MSG = std::mem::zeroed();
        while GetMessageW(&mut message, std::ptr::null_mut(), 0, 0) > 0 {
            TranslateMessage(&message);
            DispatchMessageW(&message);
        }
    }
}

unsafe extern "system" fn window_proc(hwnd: HWND, message: u32, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
    // SAFETY: Windows calls this on the window's own thread with its own handle; the user data
    // is the `Arc` that `run` stored, released once at `WM_NCDESTROY`.
    unsafe {
        match message {
            SAY => {
                let shown = GetWindowLongPtrW(hwnd, GWLP_USERDATA) as *const Mutex<Shown>;
                if !shown.is_null() {
                    let text = (*shown).lock().unwrap_or_else(PoisonError::into_inner).text.clone();
                    SetWindowTextW(GetPropW(hwnd, wide("line").as_ptr()) as HWND, text.as_ptr());
                }
                0
            }
            SAVED => {
                DestroyWindow(hwnd);
                0
            }
            WM_CLOSE => 0,
            WM_DESTROY => {
                PostQuitMessage(0);
                0
            }
            WM_NCDESTROY => {
                let shown = SetWindowLongPtrW(hwnd, GWLP_USERDATA, 0) as *const Mutex<Shown>;
                if !shown.is_null() {
                    drop(Arc::from_raw(shown));
                }
                RemovePropW(hwnd, wide("line").as_ptr());
                DefWindowProcW(hwnd, message, wparam, lparam)
            }
            _ => DefWindowProcW(hwnd, message, wparam, lparam),
        }
    }
}
