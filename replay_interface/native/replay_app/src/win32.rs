
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
    GetClientRect, GetFocus, GetKeyboardState, GetKeyState, LoadCursorW, PeekMessageW, PostQuitMessage, RegisterClassW,
    ReleaseCapture, ScreenToClient, SetCapture, SetWindowTextW, TranslateMessage, CS_HREDRAW,
    CS_VREDRAW, CW_USEDEFAULT, IDC_ARROW, MSG, PM_REMOVE, VK_SHIFT,
    VK_MENU, VK_CONTROL, VK_LMENU, VK_RMENU, VK_LCONTROL, VK_RCONTROL, WM_CAPTURECHANGED, WM_DESTROY,
    WM_SYSKEYDOWN, WM_SYSKEYUP, WM_KILLFOCUS,
    WM_KEYDOWN, WM_KEYUP, WM_LBUTTONDOWN, WM_LBUTTONUP, WM_MBUTTONDOWN, WM_MBUTTONUP, WM_MOUSEMOVE,
    WM_MOUSEWHEEL, WM_QUIT, WM_RBUTTONDOWN, WM_RBUTTONUP, WM_SIZE, WNDCLASSW, WS_OVERLAPPEDWINDOW,
    WS_VISIBLE,
};

use crate::Result;

mod native_ui_desktop;
pub(crate) use native_ui_desktop::native_ui_desktop_mode;

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
    ModifierKeyDown { vk: i32, repeated: bool },
    KeyboardFocusLost,
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
    Wheel {
        delta: i32,
        x: i32,
        y: i32,
    },
    CaptureLost,
}

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
    let x = (lparam & 0xffff) as u16 as i16 as i32;
    let y = ((lparam >> 16) & 0xffff) as u16 as i16 as i32;
    (x, y)
}

fn keyboard_event(msg: UINT, vk: i32, lparam: LPARAM, shift: bool) -> Option<WindowEvent> {
    let right = lparam & (1 << 24) != 0;
    let modifier = match vk {
        VK_MENU => Some(if right { VK_RMENU } else { VK_LMENU }),
        VK_CONTROL => Some(if right { VK_RCONTROL } else { VK_LCONTROL }),
        VK_LMENU | VK_RMENU | VK_LCONTROL | VK_RCONTROL => Some(vk),
        _ => None,
    };
    match msg {
        WM_KEYDOWN | WM_SYSKEYDOWN if modifier.is_some() => Some(WindowEvent::ModifierKeyDown {
            vk: modifier.unwrap(), repeated: lparam & (1 << 30) != 0,
        }),
        WM_KEYUP | WM_SYSKEYUP if modifier.is_some() => Some(WindowEvent::KeyUp { vk: modifier.unwrap() }),
        WM_KEYDOWN => Some(WindowEvent::KeyDown { vk, shift }),
        WM_KEYUP => Some(WindowEvent::KeyUp { vk }),
        _ => None,
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ViewerKeyRoute { MapOwned, UiConsumed }

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct ViewerKeyRouting {
    held: [Option<ViewerKeyRoute>; 4],
}

impl ViewerKeyRouting {
    fn modifier_index(vk: i32) -> Option<usize> {
        [VK_LMENU, VK_RMENU, VK_LCONTROL, VK_RCONTROL].iter().position(|key| *key == vk)
    }

    pub fn key_down(&mut self, vk: i32, route: ViewerKeyRoute, repeated: bool) {
        if let Some(index) = Self::modifier_index(vk) {
            // Autorepeat neither transfers the original press nor proves one lost with focus.
            if !repeated { self.held[index] = Some(route); }
        }
    }

    pub fn key_up(&mut self, vk: i32) {
        if let Some(index) = Self::modifier_index(vk) { self.held[index] = None; }
    }

    pub fn focus_lost(&mut self) { self.held = [None; 4]; }

    pub fn rollback(&mut self, checkpoint: Self) {
        *self = checkpoint;
        // OS key and focus transitions cannot be rolled back with an undisplayed frame.
        self.focus_lost();
    }

    fn project(&self, mut keyboard: map_viewer::Xmap15ArrowKeyboard) -> map_viewer::Xmap15ArrowKeyboard {
        let owned = self.held.map(|route| route.map(|route| route == ViewerKeyRoute::MapOwned));
        keyboard.ui = map_viewer::Xmap15ArrowUiCapture::ViewerEventRouting {
            alt_map_owned: [owned[0], owned[1]], control_map_owned: [owned[2], owned[3]],
        };
        keyboard
    }
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
        WM_KEYDOWN | WM_KEYUP | WM_SYSKEYDOWN | WM_SYSKEYUP => {
            if let Some(event) = keyboard_event(msg, wparam as i32, lparam,
                GetKeyState(VK_SHIFT) < 0) {
                push_event(event);
            }
            if msg == WM_SYSKEYDOWN || msg == WM_SYSKEYUP {
                DefWindowProcW(hwnd, msg, wparam, lparam)
            } else { 0 }
        }
        WM_KILLFOCUS => {
            push_event(WindowEvent::KeyboardFocusLost);
            DefWindowProcW(hwnd, msg, wparam, lparam)
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
            push_event(WindowEvent::CaptureLost);
            0
        }
        WM_MOUSEWHEEL => {
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

    pub fn arrow_keyboard(&self, routing: &ViewerKeyRouting) -> map_viewer::Xmap15ArrowKeyboard {
        let mut keys = [0u8; 256];
        let observed = unsafe { GetFocus() == self.hwnd && GetKeyboardState(keys.as_mut_ptr()) != 0 };
        routing.project(arrow_keyboard_snapshot(observed.then_some(&keys)))
    }

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

    pub fn request_close(&self) {
        unsafe {
            DestroyWindow(self.hwnd);
        }
    }
}

fn arrow_keyboard_snapshot(keys: Option<&[u8; 256]>) -> map_viewer::Xmap15ArrowKeyboard {
    let pressed = |key: i32| keys.map(|keys| keys[key as usize] & 0x80 != 0);
    map_viewer::Xmap15ArrowKeyboard {
        alt: pressed(VK_LMENU).zip(pressed(VK_RMENU)).map(|(left, right)| [left, right]),
        control: pressed(VK_LCONTROL).zip(pressed(VK_RCONTROL)).map(|(left, right)| [left, right]),
        ui: map_viewer::Xmap15ArrowUiCapture::Unavailable,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn arrow_keyboard_snapshot_preserves_failure_and_both_sides_without_inventing_ui_capture() {
        assert_eq!(arrow_keyboard_snapshot(None), map_viewer::Xmap15ArrowKeyboard::default());
        for mask in 0u8..16 {
            let mut bytes = [1u8; 256];
            for (i, vk) in [VK_LMENU, VK_RMENU, VK_LCONTROL, VK_RCONTROL].into_iter().enumerate() {
                bytes[vk as usize] |= if mask & (1 << i) != 0 { 0x80 } else { 0 };
            }
            let snapshot = arrow_keyboard_snapshot(Some(&bytes));
            assert_eq!(snapshot.alt, Some([mask & 1 != 0, mask & 2 != 0]));
            assert_eq!(snapshot.control, Some([mask & 4 != 0, mask & 8 != 0]));
            assert_eq!(snapshot.ui, map_viewer::Xmap15ArrowUiCapture::Unavailable);
        }
    }
    #[test]
    fn native_keyboard_messages_keep_modifier_sides_repeat_and_system_shortcuts_distinct() {
        for (generic, left, right) in [(VK_MENU, VK_LMENU, VK_RMENU),
            (VK_CONTROL, VK_LCONTROL, VK_RCONTROL)] {
            for (extended, physical) in [(0, left), (1 << 24, right)] {
                for (down, up) in [(WM_KEYDOWN, WM_KEYUP), (WM_SYSKEYDOWN, WM_SYSKEYUP)] {
                    assert_eq!(keyboard_event(down, generic, extended, false),
                        Some(WindowEvent::ModifierKeyDown { vk: physical, repeated: false }));
                    assert_eq!(keyboard_event(down, generic, extended | (1 << 30), false),
                        Some(WindowEvent::ModifierKeyDown { vk: physical, repeated: true }));
                    assert_eq!(keyboard_event(up, generic, extended, false),
                        Some(WindowEvent::KeyUp { vk: physical }));
                }
            }
        }
        assert_eq!(keyboard_event(WM_SYSKEYDOWN, winapi::um::winuser::VK_F4, 0, false), None);
        assert_eq!(keyboard_event(WM_SYSKEYUP, winapi::um::winuser::VK_F4, 0, false), None);
        assert_eq!(keyboard_event(WM_KEYDOWN, winapi::um::winuser::VK_TAB, 0, true),
            Some(WindowEvent::KeyDown { vk: winapi::um::winuser::VK_TAB, shift: true }));
    }

    fn routed_key(routing: &mut ViewerKeyRouting, msg: UINT, vk: i32, flags: LPARAM,
        route: ViewerKeyRoute) {
        match keyboard_event(msg, vk, flags, false).unwrap() {
            WindowEvent::ModifierKeyDown { vk, repeated } => routing.key_down(vk, route, repeated),
            WindowEvent::KeyUp { vk } => routing.key_up(vk),
            event => panic!("unexpected event: {event:?}"),
        }
    }

    fn routed_snapshot(routing: &ViewerKeyRouting, down: &[i32]) -> map_viewer::Xmap15ArrowKeyboard {
        let mut keys = [0; 256];
        for vk in down { keys[*vk as usize] = 0x80; }
        routing.project(arrow_keyboard_snapshot(Some(&keys)))
    }

    #[test]
    fn consumed_press_release_and_independent_sides_preserve_actual_routes() {
        use map_viewer::Xmap15ArrowUiCapture::ViewerEventRouting;
        let mut routing = ViewerKeyRouting::default();
        routed_key(&mut routing, WM_KEYDOWN, VK_CONTROL, 0, ViewerKeyRoute::UiConsumed);
        let consumed_press = routed_snapshot(&routing, &[VK_LCONTROL]);
        routed_key(&mut routing, WM_KEYDOWN, VK_CONTROL, 1 << 30, ViewerKeyRoute::MapOwned);
        assert_eq!(routed_snapshot(&routing, &[VK_LCONTROL]), consumed_press);
        assert_eq!(consumed_press.ui, ViewerEventRouting {
            alt_map_owned: [None; 2], control_map_owned: [Some(false), None] });
        routed_key(&mut routing, WM_KEYDOWN, VK_CONTROL, 1 << 24, ViewerKeyRoute::MapOwned);
        let mixed_press = routed_snapshot(&routing, &[VK_LCONTROL, VK_RCONTROL]);
        routed_key(&mut routing, WM_KEYDOWN, VK_CONTROL, (1 << 24) | (1 << 30), ViewerKeyRoute::UiConsumed);
        assert_eq!(routed_snapshot(&routing, &[VK_LCONTROL, VK_RCONTROL]), mixed_press);
        assert_eq!(routed_snapshot(&routing, &[VK_LCONTROL, VK_RCONTROL]).ui,
            ViewerEventRouting { alt_map_owned: [None; 2], control_map_owned: [Some(false), Some(true)] });
        routed_key(&mut routing, WM_KEYUP, VK_CONTROL, 0, ViewerKeyRoute::UiConsumed);
        let snapshot = routed_snapshot(&routing, &[VK_RCONTROL]);
        assert_eq!(snapshot.control, Some([false, true]));
        assert_eq!(snapshot.ui, ViewerEventRouting {
            alt_map_owned: [None; 2], control_map_owned: [None, Some(true)] });
        routed_key(&mut routing, WM_KEYUP, VK_CONTROL, 1 << 24, ViewerKeyRoute::MapOwned);
        assert_eq!(routed_snapshot(&routing, &[]).control, Some([false; 2]));
        assert_eq!(routing.held, [None; 4]);
        routed_key(&mut routing, WM_SYSKEYDOWN, VK_MENU, 0, ViewerKeyRoute::MapOwned);
        routed_key(&mut routing, WM_SYSKEYDOWN, VK_MENU, 1 << 24, ViewerKeyRoute::UiConsumed);
        assert_eq!(routed_snapshot(&routing, &[VK_LMENU, VK_RMENU]).ui,
            ViewerEventRouting { alt_map_owned: [Some(true), Some(false)], control_map_owned: [None; 2] });
    }

    #[test]
    fn focus_reacquisition_and_frame_rollback_cannot_reuse_a_held_press_witness() {
        let mut routing = ViewerKeyRouting::default();
        routed_key(&mut routing, WM_SYSKEYDOWN, VK_MENU, 0, ViewerKeyRoute::MapOwned);
        let checkpoint = routing.clone();
        routing.focus_lost();
        assert_eq!(routing.project(arrow_keyboard_snapshot(None)).alt, None);
        routed_key(&mut routing, WM_SYSKEYDOWN, VK_MENU, 1 << 30, ViewerKeyRoute::MapOwned);
        assert_eq!(routing.held, [None; 4]);
        assert_eq!(routed_snapshot(&routing, &[VK_LMENU]).alt, Some([true, false]));
        routing.rollback(checkpoint);
        assert_eq!(routing.held, [None; 4]);
        routed_key(&mut routing, WM_SYSKEYDOWN, VK_MENU, 1 << 30, ViewerKeyRoute::MapOwned);
        assert_eq!(routing.held, [None; 4]);
        routed_key(&mut routing, WM_SYSKEYUP, VK_MENU, 0, ViewerKeyRoute::MapOwned);
        routed_key(&mut routing, WM_SYSKEYDOWN, VK_MENU, 0, ViewerKeyRoute::MapOwned);
        assert_eq!(routing.held[0], Some(ViewerKeyRoute::MapOwned));
        let checkpoint = routing.clone();
        routed_key(&mut routing, WM_SYSKEYUP, VK_MENU, 0, ViewerKeyRoute::MapOwned);
        routing.rollback(checkpoint);
        assert_eq!(routing.held, [None; 4]);
        assert_eq!(routed_snapshot(&routing, &[]).alt, Some([false; 2]));
    }

}
