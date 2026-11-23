use std::{
    sync::{
        Arc,
        atomic::{AtomicBool, AtomicIsize, AtomicUsize, Ordering},
        mpsc,
    },
    thread,
};
use windows_sys::Win32::{
    Foundation::{HWND, LPARAM, LRESULT, RECT, WPARAM},
    Graphics::Gdi::{
        BLACK_BRUSH, EnumDisplayMonitors, GetMonitorInfoW, GetStockObject, HDC, HMONITOR,
        MONITOR_DEFAULTTONEAREST, MONITORINFO, MonitorFromWindow,
    },
    System::LibraryLoader::GetModuleHandleW,
    UI::WindowsAndMessaging::*,
};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Event {
    Resized { width: u32, height: u32 },
    CloseRequested,
    Closed,
}

type Callback = Arc<dyn Fn(Event) + Send + Sync>;

const NEXT_MONITOR: u32 = WM_APP + 1;
const RETIRE: u32 = WM_APP + 2;
const CLASS: &str = "BattlePlanPopOutMap";
const APP_ICON: u16 = 32512;

pub struct Shell {
    hwnd: Arc<AtomicUsize>,
    thread: Option<thread::JoinHandle<()>>,
}

impl Shell {
    pub fn open(beside: usize, callback: Callback) -> Result<Self, String> {
        let hwnd = Arc::new(AtomicUsize::new(0));
        let (ready, started) = mpsc::channel();
        let created = Arc::clone(&hwnd);
        let thread = thread::Builder::new()
            .name("popout-map-window".into())
            .spawn(move || run(beside, callback, created, ready))
            .map_err(|error| error.to_string())?;
        match started.recv() {
            Ok(Ok(())) => Ok(Self {
                hwnd,
                thread: Some(thread),
            }),
            Ok(Err(error)) => {
                let _ = thread.join();
                Err(error)
            }
            Err(_) => {
                let _ = thread.join();
                Err("The pop-out map window didn’t start".into())
            }
        }
    }

    pub fn hwnd(&self) -> usize {
        self.hwnd.load(Ordering::Acquire)
    }

    pub fn client_size(&self) -> Option<(u32, u32)> {
        let hwnd = self.hwnd();
        if hwnd == 0 {
            return None;
        }
        let mut rect = RECT::default();
        (unsafe { GetClientRect(hwnd as HWND, &mut rect) } != 0).then(|| {
            (
                (rect.right - rect.left).max(1) as u32,
                (rect.bottom - rect.top).max(1) as u32,
            )
        })
    }

    pub fn next_monitor(&self) {
        let hwnd = self.hwnd();
        if hwnd != 0 {
            unsafe { PostMessageW(hwnd as HWND, NEXT_MONITOR, 0, 0) };
        }
    }

    pub fn hide(&self) {
        let hwnd = self.hwnd();
        if hwnd != 0 {
            unsafe { ShowWindowAsync(hwnd as HWND, SW_HIDE) };
        }
    }
}

pub fn raise(hwnd: usize) {
    if hwnd != 0 {
        unsafe {
            ShowWindow(hwnd as HWND, SW_SHOW);
            SetForegroundWindow(hwnd as HWND);
        }
    }
}

impl Drop for Shell {
    fn drop(&mut self) {
        let hwnd = self.hwnd();
        if hwnd != 0 {
            unsafe { PostMessageW(hwnd as HWND, RETIRE, 0, 0) };
        }
        if let Some(thread) = self.thread.take() {
            let _ = thread.join();
        }
    }
}

struct State {
    callback: Callback,
    hwnd: Arc<AtomicUsize>,
    adopted: Arc<AtomicBool>,
    fitted: AtomicIsize,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Area {
    pub left: i32,
    pub top: i32,
    pub right: i32,
    pub bottom: i32,
}

impl From<RECT> for Area {
    fn from(rect: RECT) -> Self {
        Self {
            left: rect.left,
            top: rect.top,
            right: rect.right,
            bottom: rect.bottom,
        }
    }
}

pub fn monitor_order(areas: &[Area]) -> Vec<usize> {
    let mut order: Vec<usize> = (0..areas.len()).collect();
    order.sort_by_key(|&index| (areas[index].left, areas[index].top));
    order
}

pub fn initial_choice(areas: &[Area], main: usize) -> usize {
    monitor_order(areas)
        .into_iter()
        .find(|&index| index != main)
        .unwrap_or(main)
}

pub fn next_choice(areas: &[Area], current: Option<usize>) -> Option<usize> {
    let order = monitor_order(areas);
    if order.is_empty() {
        return None;
    }
    let position = current
        .and_then(|current| order.iter().position(|&index| index == current))
        .map_or(0, |position| position + 1);
    Some(order[position % order.len()])
}

pub fn fitted(area: Area) -> Option<(i32, i32, i32, i32)> {
    let width = area.right.checked_sub(area.left)?;
    let height = area.bottom.checked_sub(area.top)?;
    (width > 0 && height > 0).then_some((area.left, area.top, width, height))
}

fn wide(text: &str) -> Vec<u16> {
    text.encode_utf16().chain(Some(0)).collect()
}

fn run(
    beside: usize,
    callback: Callback,
    hwnd_out: Arc<AtomicUsize>,
    ready: mpsc::Sender<Result<(), String>>,
) {
    let class = wide(CLASS);
    let title = wide("BattlePlan map");
    // SAFETY: every Win32 call below runs on this thread, which owns the window and its
    // message loop. WM_NCCREATE adopts the state box and sets `adopted`, and WM_NCDESTROY then
    // frees it; when creation fails before adoption, this function frees it instead, so the box
    // is released exactly once.
    unsafe {
        let instance = GetModuleHandleW(std::ptr::null());
        let mut description: WNDCLASSEXW = std::mem::zeroed();
        description.cbSize = size_of::<WNDCLASSEXW>() as u32;
        description.lpfnWndProc = Some(window_proc);
        description.hInstance = instance;
        description.hCursor = LoadCursorW(std::ptr::null_mut(), IDC_ARROW);
        description.hbrBackground = GetStockObject(BLACK_BRUSH);
        description.lpszClassName = class.as_ptr();
        let icon = LoadImageW(
            instance,
            APP_ICON as usize as *const u16,
            IMAGE_ICON,
            0,
            0,
            LR_DEFAULTSIZE | LR_SHARED,
        );
        description.hIcon = icon;
        description.hIconSm = icon;
        RegisterClassExW(&description);
        let adopted = Arc::new(AtomicBool::new(false));
        let state = Box::into_raw(Box::new(State {
            callback,
            hwnd: Arc::clone(&hwnd_out),
            adopted: Arc::clone(&adopted),
            fitted: AtomicIsize::new(0),
        }));
        let hwnd = CreateWindowExW(
            WS_EX_APPWINDOW,
            class.as_ptr(),
            title.as_ptr(),
            WS_POPUP | WS_SYSMENU | WS_MINIMIZEBOX | WS_CLIPCHILDREN,
            0,
            0,
            1,
            1,
            std::ptr::null_mut(),
            std::ptr::null_mut(),
            instance,
            state.cast(),
        );
        if hwnd.is_null() {
            let error = std::io::Error::last_os_error();
            if !adopted.load(Ordering::Acquire) {
                drop(Box::from_raw(state));
            }
            let _ = ready.send(Err(format!(
                "Couldn’t create the pop-out map window: {error}"
            )));
            return;
        }
        hwnd_out.store(hwnd as usize, Ordering::Release);
        let monitor = initial_monitor(beside as HWND);
        fill(hwnd, monitor);
        ShowWindow(hwnd, SW_SHOW);
        let _ = ready.send(Ok(()));
        let mut message: MSG = std::mem::zeroed();
        while GetMessageW(&mut message, std::ptr::null_mut(), 0, 0) > 0 {
            TranslateMessage(&message);
            DispatchMessageW(&message);
        }
    }
}

fn monitors() -> Vec<HMONITOR> {
    unsafe extern "system" fn each(monitor: HMONITOR, _: HDC, _: *mut RECT, data: LPARAM) -> i32 {
        // SAFETY: `data` is the vector passed below, alive for the enumeration.
        unsafe { (*(data as *mut Vec<HMONITOR>)).push(monitor) };
        1
    }
    let mut found: Vec<HMONITOR> = Vec::new();
    unsafe {
        EnumDisplayMonitors(
            std::ptr::null_mut(),
            std::ptr::null(),
            Some(each),
            &mut found as *mut _ as LPARAM,
        )
    };
    found
}

fn work_area(monitor: HMONITOR) -> Area {
    let mut info: MONITORINFO = unsafe { std::mem::zeroed() };
    info.cbSize = size_of::<MONITORINFO>() as u32;
    unsafe { GetMonitorInfoW(monitor, &mut info) };
    info.rcWork.into()
}

fn initial_monitor(beside: HWND) -> HMONITOR {
    let main = unsafe { MonitorFromWindow(beside, MONITOR_DEFAULTTONEAREST) };
    let all = monitors();
    let areas: Vec<Area> = all.iter().map(|&monitor| work_area(monitor)).collect();
    match all.iter().position(|&monitor| monitor == main) {
        Some(index) => all[initial_choice(&areas, index)],
        None => main,
    }
}

fn fill(hwnd: HWND, monitor: HMONITOR) {
    let Some((x, y, width, height)) = fitted(work_area(monitor)) else {
        return;
    };
    // SAFETY: this thread owns `hwnd`; the state pointer is the window's own box.
    unsafe {
        let state = GetWindowLongPtrW(hwnd, GWLP_USERDATA) as *const State;
        if !state.is_null() {
            (*state).fitted.store(monitor as isize, Ordering::Release);
        }
        SetWindowPos(
            hwnd,
            std::ptr::null_mut(),
            x,
            y,
            width,
            height,
            SWP_NOZORDER | SWP_NOACTIVATE,
        );
    }
}

fn current_monitor(hwnd: HWND) -> HMONITOR {
    unsafe { MonitorFromWindow(hwnd, MONITOR_DEFAULTTONEAREST) }
}

unsafe extern "system" fn window_proc(
    hwnd: HWND,
    message: u32,
    wparam: WPARAM,
    lparam: LPARAM,
) -> LRESULT {
    // SAFETY: WM_NCCREATE stores the state box CreateWindowExW passed; WM_NCDESTROY frees it.
    unsafe {
        if message == WM_NCCREATE {
            let create = &*(lparam as *const CREATESTRUCTW);
            SetWindowLongPtrW(hwnd, GWLP_USERDATA, create.lpCreateParams as isize);
            let state = create.lpCreateParams as *const State;
            if !state.is_null() {
                (*state).adopted.store(true, Ordering::Release);
            }
            return DefWindowProcW(hwnd, message, wparam, lparam);
        }
        let state = GetWindowLongPtrW(hwnd, GWLP_USERDATA) as *const State;
        match message {
            WM_SIZE if !state.is_null() && wparam as u32 != SIZE_MINIMIZED => {
                let width = (lparam & 0xffff) as u32;
                let height = ((lparam >> 16) & 0xffff) as u32;
                if width > 0 && height > 0 {
                    ((*state).callback)(Event::Resized { width, height });
                }
                0
            }
            WM_DPICHANGED | WM_DISPLAYCHANGE | WM_SETTINGCHANGE => {
                fill(hwnd, current_monitor(hwnd));
                DefWindowProcW(hwnd, message, wparam, lparam)
            }
            NEXT_MONITOR => {
                let all = monitors();
                let areas: Vec<Area> = all.iter().map(|&monitor| work_area(monitor)).collect();
                let here = current_monitor(hwnd);
                let current = all.iter().position(|&monitor| monitor == here);
                if let Some(next) = next_choice(&areas, current) {
                    fill(hwnd, all[next]);
                }
                0
            }
            WM_WINDOWPOSCHANGED if !state.is_null() => {
                let here = current_monitor(hwnd);
                if (*state).fitted.load(Ordering::Acquire) != here as isize {
                    fill(hwnd, here);
                }
                DefWindowProcW(hwnd, message, wparam, lparam)
            }
            WM_SETFOCUS => {
                let child = GetWindow(hwnd, GW_CHILD);
                if !child.is_null() {
                    PostMessageW(child, viewer::host::WM_TAKE_FOCUS, 0, 0);
                }
                0
            }
            WM_CLOSE if !state.is_null() => {
                ((*state).callback)(Event::CloseRequested);
                0
            }
            WM_CLOSE | RETIRE => {
                DestroyWindow(hwnd);
                0
            }
            WM_DESTROY => {
                if !state.is_null() {
                    (*state).hwnd.store(0, Ordering::Release);
                    ((*state).callback)(Event::Closed);
                }
                PostQuitMessage(0);
                0
            }
            WM_NCDESTROY => {
                if !state.is_null() {
                    SetWindowLongPtrW(hwnd, GWLP_USERDATA, 0);
                    drop(Box::from_raw(state as *mut State));
                }
                DefWindowProcW(hwnd, message, wparam, lparam)
            }
            _ => DefWindowProcW(hwnd, message, wparam, lparam),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn area(left: i32, top: i32, width: i32, height: i32) -> Area {
        Area {
            left,
            top,
            right: left + width,
            bottom: top + height,
        }
    }

    fn three() -> Vec<Area> {
        vec![
            area(0, 0, 2560, 1400),
            area(-1920, 120, 1920, 1040),
            area(0, -1080, 1920, 1040),
        ]
    }

    #[test]
    fn monitors_step_left_to_right_then_top_to_bottom_including_negative_origins() {
        assert_eq!(monitor_order(&three()), [1, 2, 0]);
    }

    #[test]
    fn the_pop_out_opens_beside_the_main_window() {
        assert_eq!(initial_choice(&three(), 0), 1);
        assert_eq!(initial_choice(&three(), 1), 2);
        assert_eq!(initial_choice(&[area(0, 0, 100, 100)], 0), 0);
    }

    #[test]
    fn next_monitor_wraps_around_and_starts_from_the_first() {
        let areas = three();
        assert_eq!(next_choice(&areas, Some(1)), Some(2));
        assert_eq!(next_choice(&areas, Some(2)), Some(0));
        assert_eq!(next_choice(&areas, Some(0)), Some(1));
        assert_eq!(next_choice(&areas, None), Some(1));
        assert_eq!(next_choice(&areas[..1], Some(0)), Some(0));
        assert_eq!(next_choice(&[], None), None);
    }

    #[test]
    fn the_window_fills_the_work_area_and_refuses_an_empty_one() {
        assert_eq!(
            fitted(area(-1920, 120, 1920, 1040)),
            Some((-1920, 120, 1920, 1040))
        );
        assert_eq!(fitted(area(10, 10, 0, 50)), None);
        assert_eq!(
            fitted(Area {
                left: 5,
                top: 5,
                right: 1,
                bottom: 9
            }),
            None
        );
    }
}
