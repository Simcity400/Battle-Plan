//! Raw winapi Win32 window: class registration, message pump, and an
//! event queue drained once per frame. House style: raw unsafe Win32 with
//! explicit error strings (see map_viewer for the D3D11 counterpart).

use std::cell::RefCell;
use std::collections::VecDeque;
use std::mem;
use std::ptr;

use winapi::shared::minwindef::{LPARAM, LRESULT, UINT, WPARAM};
use winapi::shared::windef::{HWND, POINT, RECT};
use winapi::um::errhandlingapi::GetLastError;
use winapi::um::libloaderapi::GetModuleHandleW;
use winapi::um::winuser::{
    AdjustWindowRect, CreateWindowExW, DefWindowProcW, DestroyWindow, DispatchMessageW,
    GetClientRect, GetKeyState, LoadCursorW, PeekMessageW, PostQuitMessage, RegisterClassW,
    ReleaseCapture, ScreenToClient, SetCapture, SetWindowTextW, TranslateMessage, CS_HREDRAW,
    CS_VREDRAW, CW_USEDEFAULT, IDC_ARROW, MSG, PM_REMOVE, VK_SHIFT, WM_CAPTURECHANGED, WM_DESTROY,
    WM_KEYDOWN, WM_KEYUP, WM_LBUTTONDOWN, WM_LBUTTONUP, WM_MBUTTONDOWN, WM_MBUTTONUP, WM_MOUSEMOVE,
    WM_MOUSEWHEEL, WM_QUIT, WM_RBUTTONDOWN, WM_RBUTTONUP, WM_SIZE, WNDCLASSW, WS_OVERLAPPEDWINDOW,
    WS_VISIBLE,
};

use crate::Result;

const WINDOW_CLASS_NAME: &str = "replay_app_window";
const ERROR_CLASS_ALREADY_EXISTS: u32 = 1410;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MouseButton {
    Left,
    Middle,
    Right,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum WindowEvent {
    Resized {
        width: u32,
        height: u32,
    },
    KeyDown {
        vk: i32,
        shift: bool,
    },
    KeyUp {
        vk: i32,
    },
    MouseMove {
        x: i32,
        y: i32,
    },
    MouseDown {
        button: MouseButton,
        x: i32,
        y: i32,
    },
    MouseUp {
        button: MouseButton,
        x: i32,
        y: i32,
    },
    /// Wheel delta in WHEEL_DELTA (120) units; x/y are client coords.
    Wheel {
        delta: i32,
        x: i32,
        y: i32,
    },
    /// Mouse capture was taken away (WM_CAPTURECHANGED): cancel drags.
    CaptureLost,
}

/// Input family implemented by this concrete client event pump.
///
/// This value describes the messages the running standalone viewer actually
/// accepts (WM_KEY* plus WM_MOUSE*). It is deliberately not decoded from a
/// replay carrier. Adding controller or focus-traversal messages requires a
/// new implementation here before XUI25 can admit that family.
pub fn actual_client_input_variant() -> game_ui::variant_admission::InputVariant {
    game_ui::variant_admission::InputVariant::MouseKeyboard
}

thread_local! {
    static EVENT_QUEUE: RefCell<VecDeque<WindowEvent>> = RefCell::new(VecDeque::new());
}

fn push_event(event: WindowEvent) {
    EVENT_QUEUE.with(|queue| queue.borrow_mut().push_back(event));
}

fn lparam_xy(lparam: LPARAM) -> (i32, i32) {
    // Client coords are signed 16-bit packed into lparam.
    let x = (lparam & 0xffff) as u16 as i16 as i32;
    let y = ((lparam >> 16) & 0xffff) as u16 as i16 as i32;
    (x, y)
}

unsafe extern "system" fn wnd_proc(
    hwnd: HWND,
    msg: UINT,
    wparam: WPARAM,
    lparam: LPARAM,
) -> LRESULT {
    match msg {
        WM_DESTROY => {
            PostQuitMessage(0);
            0
        }
        WM_SIZE => {
            let width = (lparam & 0xffff) as u32;
            let height = ((lparam >> 16) & 0xffff) as u32;
            push_event(WindowEvent::Resized { width, height });
            0
        }
        WM_KEYDOWN => {
            push_event(WindowEvent::KeyDown {
                vk: wparam as i32,
                shift: GetKeyState(VK_SHIFT) < 0,
            });
            0
        }
        WM_KEYUP => {
            push_event(WindowEvent::KeyUp { vk: wparam as i32 });
            0
        }
        WM_MOUSEMOVE => {
            let (x, y) = lparam_xy(lparam);
            push_event(WindowEvent::MouseMove { x, y });
            0
        }
        WM_LBUTTONDOWN | WM_MBUTTONDOWN | WM_RBUTTONDOWN => {
            SetCapture(hwnd);
            let (x, y) = lparam_xy(lparam);
            let button = match msg {
                WM_LBUTTONDOWN => MouseButton::Left,
                WM_MBUTTONDOWN => MouseButton::Middle,
                _ => MouseButton::Right,
            };
            push_event(WindowEvent::MouseDown { button, x, y });
            0
        }
        WM_LBUTTONUP | WM_MBUTTONUP | WM_RBUTTONUP => {
            ReleaseCapture();
            let (x, y) = lparam_xy(lparam);
            let button = match msg {
                WM_LBUTTONUP => MouseButton::Left,
                WM_MBUTTONUP => MouseButton::Middle,
                _ => MouseButton::Right,
            };
            push_event(WindowEvent::MouseUp { button, x, y });
            0
        }
        WM_CAPTURECHANGED => {
            // Capture can be stolen (alt-tab, menus) without a button-up;
            // tell the app so a pan drag never sticks.
            push_event(WindowEvent::CaptureLost);
            0
        }
        WM_MOUSEWHEEL => {
            // WM_MOUSEWHEEL reports screen coordinates; convert to client.
            let delta = ((wparam >> 16) & 0xffff) as u16 as i16 as i32;
            let (sx, sy) = lparam_xy(lparam);
            let mut point = POINT { x: sx, y: sy };
            ScreenToClient(hwnd, &mut point);
            push_event(WindowEvent::Wheel {
                delta,
                x: point.x,
                y: point.y,
            });
            0
        }
        _ => DefWindowProcW(hwnd, msg, wparam, lparam),
    }
}

fn wide(text: &str) -> Vec<u16> {
    text.encode_utf16().chain(std::iter::once(0)).collect()
}

pub struct Window {
    hwnd: HWND,
    last_title: String,
}

impl Window {
    /// Create a visible, resizable top-level window with the requested
    /// client area size.
    pub fn create(title: &str, client_width: i32, client_height: i32) -> Result<Window> {
        unsafe {
            let instance = GetModuleHandleW(ptr::null());
            if instance.is_null() {
                return Err("GetModuleHandleW returned null".into());
            }
            let class_name = wide(WINDOW_CLASS_NAME);
            let mut class: WNDCLASSW = mem::zeroed();
            class.style = CS_HREDRAW | CS_VREDRAW;
            class.lpfnWndProc = Some(wnd_proc);
            class.hInstance = instance;
            class.hCursor = LoadCursorW(ptr::null_mut(), IDC_ARROW);
            class.lpszClassName = class_name.as_ptr();
            if RegisterClassW(&class) == 0 {
                let error = GetLastError();
                if error != ERROR_CLASS_ALREADY_EXISTS {
                    return Err(format!("RegisterClassW failed with Win32 error {error}").into());
                }
            }

            let mut rect = RECT {
                left: 0,
                top: 0,
                right: client_width,
                bottom: client_height,
            };
            if AdjustWindowRect(&mut rect, WS_OVERLAPPEDWINDOW, 0) == 0 {
                return Err(format!(
                    "AdjustWindowRect failed with Win32 error {}",
                    GetLastError()
                )
                .into());
            }
            let title_wide = wide(title);
            let hwnd = CreateWindowExW(
                0,
                class_name.as_ptr(),
                title_wide.as_ptr(),
                WS_OVERLAPPEDWINDOW | WS_VISIBLE,
                CW_USEDEFAULT,
                CW_USEDEFAULT,
                rect.right - rect.left,
                rect.bottom - rect.top,
                ptr::null_mut(),
                ptr::null_mut(),
                instance,
                ptr::null_mut(),
            );
            if hwnd.is_null() {
                return Err(
                    format!("CreateWindowExW failed with Win32 error {}", GetLastError()).into(),
                );
            }
            Ok(Window {
                hwnd,
                last_title: title.to_string(),
            })
        }
    }

    pub fn hwnd(&self) -> HWND {
        self.hwnd
    }

    /// Current client area size in pixels.
    pub fn client_size(&self) -> Result<(u32, u32)> {
        unsafe {
            let mut rect: RECT = mem::zeroed();
            if GetClientRect(self.hwnd, &mut rect) == 0 {
                return Err(
                    format!("GetClientRect failed with Win32 error {}", GetLastError()).into(),
                );
            }
            Ok((
                (rect.right - rect.left).max(0) as u32,
                (rect.bottom - rect.top).max(0) as u32,
            ))
        }
    }

    /// Drain pending Win32 messages. Returns queued events and whether
    /// the window is still alive (false once WM_QUIT arrives).
    pub fn pump(&self) -> (Vec<WindowEvent>, bool) {
        let mut alive = true;
        unsafe {
            let mut msg: MSG = mem::zeroed();
            while PeekMessageW(&mut msg, ptr::null_mut(), 0, 0, PM_REMOVE) != 0 {
                if msg.message == WM_QUIT {
                    alive = false;
                    continue;
                }
                TranslateMessage(&msg);
                DispatchMessageW(&msg);
            }
        }
        let events = EVENT_QUEUE.with(|queue| queue.borrow_mut().drain(..).collect());
        (events, alive)
    }

    /// Update the title bar (probe readout lives here in this phase; no
    /// in-window text rendering yet). Skips the call when unchanged.
    pub fn set_title(&mut self, title: &str) {
        if title == self.last_title {
            return;
        }
        let title_wide = wide(title);
        unsafe {
            SetWindowTextW(self.hwnd, title_wide.as_ptr());
        }
        self.last_title = title.to_string();
    }

    /// Ask the window to close (Escape path); WM_DESTROY then ends the pump.
    pub fn request_close(&self) {
        unsafe {
            DestroyWindow(self.hwnd);
        }
    }
}
