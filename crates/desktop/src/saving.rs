use std::{sync::mpsc, thread};
use windows_sys::Win32::{
    Foundation::{HWND, LPARAM, LRESULT, RECT, WPARAM},
    Graphics::Gdi::{COLOR_WINDOW, DEFAULT_GUI_FONT, GetStockObject, HBRUSH},
    System::LibraryLoader::GetModuleHandleW,
    UI::WindowsAndMessaging::*,
};

const SAVED: u32 = WM_APP + 1;
const SS_CENTER: u32 = 0x1;
const SS_CENTERIMAGE: u32 = 0x200;
const CLASS: &str = "BattlePlanSaving";
const TEXT: &str = "Saving your changes before BattlePlan closes…";
const WIDTH: i32 = 420;
const HEIGHT: i32 = 110;

pub struct Saving {
    hwnd: usize,
    thread: Option<thread::JoinHandle<()>>,
}

impl Saving {
    pub fn show() -> Result<Self, String> {
        let (ready, shown) = mpsc::channel();
        let thread = thread::Builder::new()
            .name("saving".into())
            .spawn(move || run(&ready))
            .map_err(|error| error.to_string())?;
        match shown.recv() {
            Ok(Ok(hwnd)) => Ok(Self { hwnd, thread: Some(thread) }),
            Ok(Err(error)) => {
                let _ = thread.join();
                Err(error)
            }
            Err(_) => {
                let _ = thread.join();
                Err("its window didn’t start".into())
            }
        }
    }
}

impl Drop for Saving {
    fn drop(&mut self) {
        // SAFETY: posting to a window handle is sound even once the window has gone.
        unsafe { PostMessageW(self.hwnd as HWND, SAVED, 0, 0) };
        if let Some(thread) = self.thread.take() {
            let _ = thread.join();
        }
    }
}

fn wide(text: &str) -> Vec<u16> {
    text.encode_utf16().chain(Some(0)).collect()
}

fn run(ready: &mpsc::Sender<Result<usize, String>>) {
    let (class, title, text, label) = (wide(CLASS), wide("BattlePlan"), wide(TEXT), wide("STATIC"));
    // SAFETY: every call below runs on this thread, which owns both windows and their message
    // loop, and every string outlives the calls that read it.
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
            let _ = ready.send(Err(std::io::Error::last_os_error().to_string()));
            return;
        }
        let mut client = RECT::default();
        GetClientRect(hwnd, &mut client);
        let shown = CreateWindowExW(
            0,
            label.as_ptr(),
            text.as_ptr(),
            WS_CHILD | WS_VISIBLE | SS_CENTER | SS_CENTERIMAGE,
            0,
            0,
            client.right,
            client.bottom,
            hwnd,
            std::ptr::null_mut(),
            instance,
            std::ptr::null(),
        );
        if !shown.is_null() {
            SendMessageW(shown, WM_SETFONT, GetStockObject(DEFAULT_GUI_FONT) as usize, 0);
        }
        ShowWindow(hwnd, SW_SHOWNORMAL);
        let _ = ready.send(Ok(hwnd as usize));
        let mut message: MSG = std::mem::zeroed();
        while GetMessageW(&mut message, std::ptr::null_mut(), 0, 0) > 0 {
            TranslateMessage(&message);
            DispatchMessageW(&message);
        }
    }
}

unsafe extern "system" fn window_proc(hwnd: HWND, message: u32, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
    // SAFETY: Windows calls this on the window's own thread with its own handle.
    unsafe {
        match message {
            SAVED => {
                DestroyWindow(hwnd);
                0
            }
            WM_CLOSE => 0,
            WM_DESTROY => {
                PostQuitMessage(0);
                0
            }
            _ => DefWindowProcW(hwnd, message, wparam, lparam),
        }
    }
}
