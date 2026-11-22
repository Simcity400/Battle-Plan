use std::{
    sync::mpsc,
    thread,
    time::{Duration, Instant},
};
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
const WIDTH: i32 = 420;
const HEIGHT: i32 = 110;
const UNSEEN: Duration = Duration::from_millis(300);

pub struct Wait<'a> {
    pub what: &'static str,
    pub done_within: &'a dyn Fn(Duration) -> bool,
}

pub fn drained(waits: &[Wait<'_>], show: impl FnOnce(&'static str) -> Result<Saving, String>) {
    let unseen = Instant::now() + UNSEEN;
    let Some(first) = waits.iter().position(|wait| !(wait.done_within)(unseen.saturating_duration_since(Instant::now()))) else {
        return;
    };
    let shown = show(waits[first].what);
    if let Err(error) = &shown {
        eprintln!("Couldn’t show what is still being finished before closing: {error}");
    }
    for wait in &waits[first..] {
        if let Ok(saving) = &shown {
            saving.say(wait.what);
        }
        while !(wait.done_within)(Duration::from_secs(60)) {}
    }
}

pub struct Saving {
    hwnd: usize,
    label: usize,
    thread: Option<thread::JoinHandle<()>>,
}

impl Saving {
    pub fn show(title: &str, text: &str) -> Result<Self, String> {
        let (ready, shown) = mpsc::channel();
        let (title, text) = (title.to_owned(), text.to_owned());
        let thread = thread::Builder::new()
            .name("saving".into())
            .spawn(move || run(&ready, &title, &text))
            .map_err(|error| error.to_string())?;
        match shown.recv() {
            Ok(Ok((hwnd, label))) => Ok(Self { hwnd, label, thread: Some(thread) }),
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

impl Saving {
    pub fn say(&self, text: &str) {
        let text = wide(text);
        // SAFETY: the label belongs to the window thread, which keeps pumping messages until this
        // value is dropped, and the text outlives the call.
        unsafe { SetWindowTextW(self.label as HWND, text.as_ptr()) };
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

fn run(ready: &mpsc::Sender<Result<(usize, usize), String>>, title: &str, text: &str) {
    let (class, title, text, label) = (wide(CLASS), wide(title), wide(text), wide("STATIC"));
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
        let _ = ready.send(Ok((hwnd as usize, shown as usize)));
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

#[cfg(test)]
mod tests {
    use super::{Wait, drained};
    use std::{sync::Mutex, sync::mpsc::channel, time::Duration};

    #[test]
    fn an_exit_drains_what_it_admitted_behind_one_window_that_names_each_wait_installer_handoff_included() {
        let done = |_: Duration| true;
        drained(&[Wait { what: "launch", done_within: &done }, Wait { what: "writes", done_within: &done }], |_| unreachable!("nothing is under way, so nothing is shown"));

        let (launching, launch) = channel::<()>();
        let (writing, write) = channel::<()>();
        let (launch, write) = (Mutex::new(launch), Mutex::new(write));
        let launched = |limit: Duration| launch.lock().unwrap().recv_timeout(limit).is_ok();
        let written = |limit: Duration| write.lock().unwrap().recv_timeout(limit).is_ok();
        std::thread::scope(|scope| {
            let (showing, shown) = channel();
            let exit = scope.spawn(|| {
                drained(&[Wait { what: "launch", done_within: &launched }, Wait { what: "writes", done_within: &written }], move |what| {
                    showing.send(what).unwrap();
                    Err("no window in a test".into())
                })
            });
            assert_eq!(shown.recv(), Ok("launch"), "a slow installer handoff shows the window too, saying so");
            launching.send(()).unwrap();
            assert!(!exit.is_finished(), "and then it waits for the writes");
            writing.send(()).unwrap();
            exit.join().unwrap();
        });
    }
}
