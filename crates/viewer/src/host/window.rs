
use eframe::egui;
use std::{
    io, ptr,
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, AtomicU8, Ordering},
        mpsc,
    },
};
use windows_sys::Win32::{
    Foundation::{HWND, LPARAM, LRESULT, POINT, RECT, WPARAM},
    Graphics::Gdi::{ClientToScreen, ValidateRect},
    System::LibraryLoader::GetModuleHandleW,
    UI::{
        HiDpi::GetDpiForWindow,
        Input::KeyboardAndMouse::{
            GetKeyState, ReleaseCapture, SetCapture, SetFocus, TME_LEAVE, TRACKMOUSEEVENT,
            TrackMouseEvent, VK_CONTROL, VK_MENU, VK_SHIFT,
        },
        WindowsAndMessaging::*,
    },
};

const CLASS: &str = "BattlePlanMap";
const WM_MOUSELEAVE: u32 = 0x02a3;
const WM_APPLY_CAPTURE: u32 = WM_APP + 1;
pub const WM_TAKE_FOCUS: u32 = WM_APP + 2;
const WM_RETIRE: u32 = WM_APP + 3;

#[derive(Debug, PartialEq)]
pub(crate) enum Input {
    Event(egui::Event),
    Focus(bool),
    Resized,
    Repaint(std::time::Duration),
    Close,
}

pub(crate) struct Link {
    sender: Mutex<mpsc::Sender<Input>>,
    captured: AtomicBool,
    cursor: AtomicU8,
    tracking: AtomicBool,
    surrogate: Mutex<Option<u16>>,
    window_released: AtomicBool,
    pressed: Mutex<Option<Arc<dyn Fn() + Send + Sync>>>,
}

impl Link {
    pub(crate) fn send(&self, input: Input) {
        let _ = self.sender.lock().unwrap_or_else(|error| error.into_inner()).send(input);
    }
}

#[derive(Clone)]
pub struct Window {
    hwnd: usize,
    link: Arc<Link>,
}

pub struct Inputs {
    pub(crate) received: mpsc::Receiver<Input>,
}

fn wide(text: &str) -> Vec<u16> {
    text.encode_utf16().chain([0]).collect()
}

impl Window {
    pub fn create(parent: Option<usize>, size: [u32; 2], title: &str) -> io::Result<(Self, Inputs)> {
        let (sender, received) = mpsc::channel();
        let link = Arc::new(Link {
            sender: Mutex::new(sender),
            captured: AtomicBool::new(false),
            cursor: AtomicU8::new(0),
            tracking: AtomicBool::new(false),
            surrogate: Mutex::new(None),
            window_released: AtomicBool::new(false),
            pressed: Mutex::new(None),
        });
        let class = wide(CLASS);
        let title = wide(title);
        // SAFETY: a class registered once per process with this module's procedure. The window
        // procedure takes the strong reference passed here at WM_NCCREATE and releases it at
        // WM_NCDESTROY, marking the link; a failed creation releases it only when it was not.
        unsafe {
            let instance = GetModuleHandleW(ptr::null());
            let description = WNDCLASSEXW {
                cbSize: size_of::<WNDCLASSEXW>() as u32,
                style: CS_DBLCLKS,
                lpfnWndProc: Some(window_proc),
                hInstance: instance,
                hCursor: LoadCursorW(ptr::null_mut(), IDC_ARROW),
                lpszClassName: class.as_ptr(),
                ..std::mem::zeroed()
            };
            RegisterClassExW(&description);
            let (style, ex_style, width, height) = match parent {
                Some(_) => (WS_CHILD | WS_CLIPCHILDREN | WS_CLIPSIBLINGS, 0, 1, 1),
                None => (WS_OVERLAPPEDWINDOW | WS_VISIBLE, WS_EX_APPWINDOW, size[0] as i32, size[1] as i32),
            };
            let raw = Arc::into_raw(Arc::clone(&link));
            let hwnd = CreateWindowExW(
                ex_style,
                class.as_ptr(),
                title.as_ptr(),
                style,
                if parent.is_some() { 0 } else { CW_USEDEFAULT },
                if parent.is_some() { 0 } else { CW_USEDEFAULT },
                width,
                height,
                parent.map_or(ptr::null_mut(), |parent| parent as HWND),
                ptr::null_mut(),
                instance,
                raw.cast(),
            );
            if hwnd.is_null() {
                let error = io::Error::last_os_error();
                if !link.window_released.load(Ordering::Acquire) {
                    drop(Arc::from_raw(raw));
                }
                return Err(error);
            }
            if parent.is_none() {
                let mut outer = RECT { left: 0, top: 0, right: size[0] as i32, bottom: size[1] as i32 };
                AdjustWindowRectEx(&mut outer, style, 0, ex_style);
                SetWindowPos(hwnd, ptr::null_mut(), 0, 0, outer.right - outer.left, outer.bottom - outer.top, SWP_NOMOVE | SWP_NOZORDER);
            }
            Ok((Self { hwnd: hwnd as usize, link }, Inputs { received }))
        }
    }

    pub fn hwnd(&self) -> usize {
        self.hwnd
    }

    pub(crate) fn link(&self) -> &Arc<Link> {
        &self.link
    }

    pub fn destroy(&self) {
        unsafe { PostMessageW(self.hwnd as HWND, WM_RETIRE, 0, 0) };
    }

    pub(crate) fn metrics(&self) -> ([u32; 2], f32) {
        let mut client = RECT::default();
        let dpi = unsafe {
            GetClientRect(self.hwnd as HWND, &mut client);
            GetDpiForWindow(self.hwnd as HWND)
        };
        let size = [(client.right - client.left).max(0) as u32, (client.bottom - client.top).max(0) as u32];
        (size, if dpi == 0 { 1.0 } else { dpi as f32 / 96.0 })
    }

    pub fn on_press(&self, pressed: impl Fn() + Send + Sync + 'static) {
        *self.link.pressed.lock().unwrap_or_else(|error| error.into_inner()) = Some(Arc::new(pressed));
    }

    pub fn focus(&self) {
        unsafe { PostMessageW(self.hwnd as HWND, WM_TAKE_FOCUS, 0, 0) };
    }

    pub(crate) fn send_to_back(&self) {
        unsafe { SetWindowPos(self.hwnd as HWND, HWND_BOTTOM, 0, 0, 0, 0, SWP_NOMOVE | SWP_NOSIZE | SWP_NOACTIVATE) };
    }

    pub(crate) fn shown(&self) -> bool {
        unsafe {
            IsWindowVisible(self.hwnd as HWND) != 0 && IsIconic(GetAncestor(self.hwnd as HWND, GA_ROOT)) == 0
        }
    }

    pub(crate) fn capture(&self, captured: bool) {
        if self.link.captured.swap(captured, Ordering::AcqRel) != captured {
            unsafe { PostMessageW(self.hwnd as HWND, WM_APPLY_CAPTURE, 0, 0) };
        }
    }

    pub(crate) fn cursor(&self, icon: egui::CursorIcon) {
        self.link.cursor.store(cursor_index(icon), Ordering::Release);
    }
}

fn cursor_index(icon: egui::CursorIcon) -> u8 {
    use egui::CursorIcon::*;
    match icon {
        None => 1,
        PointingHand | Grab | Grabbing => 2,
        Text | VerticalText => 3,
        Crosshair | Cell => 4,
        Move | AllScroll => 5,
        Wait => 6,
        Progress => 7,
        NotAllowed | NoDrop => 8,
        ResizeHorizontal | ResizeEast | ResizeWest | ResizeColumn => 9,
        ResizeVertical | ResizeNorth | ResizeSouth | ResizeRow => 10,
        ResizeNeSw | ResizeNorthEast | ResizeSouthWest => 11,
        ResizeNwSe | ResizeNorthWest | ResizeSouthEast => 12,
        Help => 13,
        _ => 0,
    }
}

fn system_cursor(index: u8) -> Option<windows_sys::core::PCWSTR> {
    Some(match index {
        1 => return None,
        2 => IDC_HAND,
        3 => IDC_IBEAM,
        4 => IDC_CROSS,
        5 => IDC_SIZEALL,
        6 => IDC_WAIT,
        7 => IDC_APPSTARTING,
        8 => IDC_NO,
        9 => IDC_SIZEWE,
        10 => IDC_SIZENS,
        11 => IDC_SIZENESW,
        12 => IDC_SIZENWSE,
        13 => IDC_HELP,
        _ => IDC_ARROW,
    })
}

fn modifiers() -> egui::Modifiers {
    let down = |key: u16| unsafe { GetKeyState(i32::from(key)) } < 0;
    let ctrl = down(VK_CONTROL);
    egui::Modifiers { alt: down(VK_MENU), ctrl, shift: down(VK_SHIFT), mac_cmd: false, command: ctrl }
}

fn points(hwnd: HWND, lparam: LPARAM) -> egui::Pos2 {
    let x = (lparam & 0xffff) as i16 as f32;
    let y = ((lparam >> 16) & 0xffff) as i16 as f32;
    let dpi = unsafe { GetDpiForWindow(hwnd) };
    let scale = if dpi == 0 { 1.0 } else { dpi as f32 / 96.0 };
    egui::pos2(x / scale, y / scale)
}

pub(crate) fn key(vk: u16, shifted: bool) -> Option<egui::Key> {
    use egui::Key;
    use windows_sys::Win32::UI::Input::KeyboardAndMouse::*;
    const LETTERS: [Key; 26] = [
        Key::A, Key::B, Key::C, Key::D, Key::E, Key::F, Key::G, Key::H, Key::I, Key::J, Key::K, Key::L, Key::M,
        Key::N, Key::O, Key::P, Key::Q, Key::R, Key::S, Key::T, Key::U, Key::V, Key::W, Key::X, Key::Y, Key::Z,
    ];
    const DIGITS: [Key; 10] = [
        Key::Num0, Key::Num1, Key::Num2, Key::Num3, Key::Num4, Key::Num5, Key::Num6, Key::Num7, Key::Num8, Key::Num9,
    ];
    const FUNCTIONS: [Key; 24] = [
        Key::F1, Key::F2, Key::F3, Key::F4, Key::F5, Key::F6, Key::F7, Key::F8, Key::F9, Key::F10, Key::F11, Key::F12,
        Key::F13, Key::F14, Key::F15, Key::F16, Key::F17, Key::F18, Key::F19, Key::F20, Key::F21, Key::F22, Key::F23,
        Key::F24,
    ];
    Some(match vk {
        0x41..=0x5a => LETTERS[usize::from(vk - 0x41)],
        0x30..=0x39 => DIGITS[usize::from(vk - 0x30)],
        _ if (VK_NUMPAD0..=VK_NUMPAD9).contains(&vk) => DIGITS[usize::from(vk - VK_NUMPAD0)],
        _ if (VK_F1..=VK_F24).contains(&vk) => FUNCTIONS[usize::from(vk - VK_F1)],
        VK_LEFT => Key::ArrowLeft,
        VK_RIGHT => Key::ArrowRight,
        VK_UP => Key::ArrowUp,
        VK_DOWN => Key::ArrowDown,
        VK_ESCAPE => Key::Escape,
        VK_TAB => Key::Tab,
        VK_BACK => Key::Backspace,
        VK_RETURN => Key::Enter,
        VK_SPACE => Key::Space,
        VK_INSERT => Key::Insert,
        VK_DELETE => Key::Delete,
        VK_HOME => Key::Home,
        VK_END => Key::End,
        VK_PRIOR => Key::PageUp,
        VK_NEXT => Key::PageDown,
        VK_OEM_PLUS if shifted => Key::Plus,
        VK_OEM_PLUS => Key::Equals,
        VK_ADD => Key::Plus,
        VK_OEM_MINUS | VK_SUBTRACT => Key::Minus,
        VK_OEM_COMMA => Key::Comma,
        VK_OEM_PERIOD | VK_DECIMAL => Key::Period,
        VK_OEM_1 => Key::Semicolon,
        VK_OEM_2 | VK_DIVIDE => Key::Slash,
        VK_OEM_3 => Key::Backtick,
        VK_OEM_4 => Key::OpenBracket,
        VK_OEM_5 => Key::Backslash,
        VK_OEM_6 => Key::CloseBracket,
        VK_OEM_7 => Key::Quote,
        _ => return None,
    })
}

pub(crate) fn physical_key(scan: u32, extended: bool) -> Option<egui::Key> {
    use egui::Key;
    if extended {
        return None;
    }
    const ROWS: [(u32, &[Key]); 4] = [
        (0x02, &[Key::Num1, Key::Num2, Key::Num3, Key::Num4, Key::Num5, Key::Num6, Key::Num7, Key::Num8, Key::Num9, Key::Num0, Key::Minus, Key::Equals]),
        (0x10, &[Key::Q, Key::W, Key::E, Key::R, Key::T, Key::Y, Key::U, Key::I, Key::O, Key::P, Key::OpenBracket, Key::CloseBracket]),
        (0x1e, &[Key::A, Key::S, Key::D, Key::F, Key::G, Key::H, Key::J, Key::K, Key::L, Key::Semicolon, Key::Quote, Key::Backtick]),
        (0x2c, &[Key::Z, Key::X, Key::C, Key::V, Key::B, Key::N, Key::M, Key::Comma, Key::Period, Key::Slash]),
    ];
    ROWS.iter().find_map(|(first, keys)| scan.checked_sub(*first).and_then(|at| keys.get(at as usize).copied()))
}

fn button(message: u32, wparam: WPARAM) -> Option<(egui::PointerButton, bool)> {
    use egui::PointerButton::*;
    Some(match message {
        WM_LBUTTONDOWN | WM_LBUTTONDBLCLK => (Primary, true),
        WM_LBUTTONUP => (Primary, false),
        WM_RBUTTONDOWN | WM_RBUTTONDBLCLK => (Secondary, true),
        WM_RBUTTONUP => (Secondary, false),
        WM_MBUTTONDOWN | WM_MBUTTONDBLCLK => (Middle, true),
        WM_MBUTTONUP => (Middle, false),
        WM_XBUTTONDOWN | WM_XBUTTONDBLCLK | WM_XBUTTONUP => (
            if (wparam >> 16) & 0xffff == 1 { Extra1 } else { Extra2 },
            message != WM_XBUTTONUP,
        ),
        _ => return None,
    })
}

/// # Safety
/// `hwnd` is the live window whose procedure is running on this thread.
unsafe fn center(hwnd: HWND) -> POINT {
    let mut client = RECT::default();
    unsafe { GetClientRect(hwnd, &mut client) };
    let mut point = POINT { x: client.right / 2, y: client.bottom / 2 };
    unsafe { ClientToScreen(hwnd, &mut point) };
    point
}

unsafe extern "system" fn window_proc(hwnd: HWND, message: u32, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
    // SAFETY: the link pointer is the window's own strong reference from WM_NCCREATE until
    // WM_NCDESTROY, read only on this, the window's, thread; the OS supplies each message's
    // documented struct pointer for the call.
    unsafe {
        if message == WM_NCCREATE {
            let created = &*(lparam as *const CREATESTRUCTW);
            SetWindowLongPtrW(hwnd, GWLP_USERDATA, created.lpCreateParams as isize);
            return DefWindowProcW(hwnd, message, wparam, lparam);
        }
        let raw = GetWindowLongPtrW(hwnd, GWLP_USERDATA) as *const Link;
        if raw.is_null() {
            return DefWindowProcW(hwnd, message, wparam, lparam);
        }
        if message == WM_NCDESTROY {
            SetWindowLongPtrW(hwnd, GWLP_USERDATA, 0);
            (*raw).window_released.store(true, Ordering::Release);
            drop(Arc::from_raw(raw));
            return DefWindowProcW(hwnd, message, wparam, lparam);
        }
        let link = &*raw;
        match message {
            WM_MOUSEMOVE => {
                if link.captured.load(Ordering::Acquire) {
                    let middle = center(hwnd);
                    let mut at = POINT { x: (lparam & 0xffff) as i16 as i32, y: ((lparam >> 16) & 0xffff) as i16 as i32 };
                    ClientToScreen(hwnd, &mut at);
                    let delta = egui::vec2((at.x - middle.x) as f32, (at.y - middle.y) as f32);
                    if delta != egui::Vec2::ZERO {
                        link.send(Input::Event(egui::Event::MouseMoved(delta)));
                        SetCursorPos(middle.x, middle.y);
                    }
                    return 0;
                }
                if !link.tracking.swap(true, Ordering::AcqRel) {
                    let mut track = TRACKMOUSEEVENT { cbSize: size_of::<TRACKMOUSEEVENT>() as u32, dwFlags: TME_LEAVE, hwndTrack: hwnd, dwHoverTime: 0 };
                    TrackMouseEvent(&mut track);
                }
                link.send(Input::Event(egui::Event::PointerMoved(points(hwnd, lparam))));
                0
            }
            WM_MOUSELEAVE => {
                link.tracking.store(false, Ordering::Release);
                link.send(Input::Event(egui::Event::PointerGone));
                0
            }
            WM_LBUTTONDOWN | WM_LBUTTONDBLCLK | WM_LBUTTONUP | WM_RBUTTONDOWN | WM_RBUTTONDBLCLK | WM_RBUTTONUP
            | WM_MBUTTONDOWN | WM_MBUTTONDBLCLK | WM_MBUTTONUP | WM_XBUTTONDOWN | WM_XBUTTONDBLCLK | WM_XBUTTONUP => {
                let (button, pressed) = button(message, wparam).expect("a button message");
                if pressed {
                    SetFocus(hwnd);
                    SetCapture(hwnd);
                    let notify = link.pressed.lock().unwrap_or_else(|error| error.into_inner()).clone();
                    if let Some(notify) = notify {
                        notify();
                    }
                } else {
                    ReleaseCapture();
                }
                link.send(Input::Event(egui::Event::PointerButton { pos: points(hwnd, lparam), button, pressed, modifiers: modifiers() }));
                if matches!(message, WM_XBUTTONDOWN | WM_XBUTTONDBLCLK | WM_XBUTTONUP) { 1 } else { 0 }
            }
            WM_MOUSEWHEEL | WM_MOUSEHWHEEL => {
                let notches = ((wparam >> 16) & 0xffff) as i16 as f32 / WHEEL_DELTA as f32;
                let delta = if message == WM_MOUSEWHEEL { egui::vec2(0.0, notches) } else { egui::vec2(notches, 0.0) };
                link.send(Input::Event(egui::Event::MouseWheel {
                    unit: egui::MouseWheelUnit::Line,
                    delta,
                    phase: egui::TouchPhase::Move,
                    modifiers: modifiers(),
                }));
                0
            }
            WM_KEYDOWN | WM_SYSKEYDOWN | WM_KEYUP | WM_SYSKEYUP => {
                let pressed = matches!(message, WM_KEYDOWN | WM_SYSKEYDOWN);
                let state = modifiers();
                if matches!(wparam as u16, VK_SHIFT | VK_CONTROL | VK_MENU) {
                    link.send(Input::Event(egui::Event::ModifiersChanged(state)));
                } else if let Some(logical) = key(wparam as u16, state.shift) {
                    let scan = ((lparam >> 16) & 0xff) as u32;
                    let extended = (lparam >> 24) & 1 == 1;
                    link.send(Input::Event(egui::Event::Key {
                        key: logical,
                        physical_key: physical_key(scan, extended).or(Some(logical)),
                        pressed,
                        repeat: pressed && (lparam >> 30) & 1 == 1,
                        modifiers: state,
                    }));
                }
                if matches!(message, WM_SYSKEYDOWN | WM_SYSKEYUP) {
                    DefWindowProcW(hwnd, message, wparam, lparam)
                } else {
                    0
                }
            }
            WM_CHAR => {
                let unit = wparam as u16;
                let mut held = link.surrogate.lock().unwrap_or_else(|error| error.into_inner());
                let text = if (0xd800..0xdc00).contains(&unit) {
                    *held = Some(unit);
                    None
                } else if (0xdc00..0xe000).contains(&unit) {
                    held.take().and_then(|high| String::from_utf16(&[high, unit]).ok())
                } else {
                    char::from_u32(u32::from(unit)).map(String::from)
                };
                if let Some(text) = text.filter(|text| text.chars().all(|c| !c.is_control())) {
                    link.send(Input::Event(egui::Event::Text(text)));
                }
                0
            }
            WM_SETFOCUS | WM_KILLFOCUS => {
                link.send(Input::Focus(message == WM_SETFOCUS));
                if message == WM_SETFOCUS {
                    link.send(Input::Event(egui::Event::ModifiersChanged(modifiers())));
                }
                0
            }
            WM_SIZE | WM_DPICHANGED | WM_DPICHANGED_AFTERPARENT | WM_SHOWWINDOW => {
                link.send(Input::Resized);
                DefWindowProcW(hwnd, message, wparam, lparam)
            }
            WM_PAINT => {
                ValidateRect(hwnd, ptr::null());
                link.send(Input::Repaint(std::time::Duration::ZERO));
                0
            }
            WM_ERASEBKGND => 1,
            WM_SETCURSOR if (lparam & 0xffff) as u32 == HTCLIENT => {
                let icon = if link.captured.load(Ordering::Acquire) { 1 } else { link.cursor.load(Ordering::Acquire) };
                SetCursor(system_cursor(icon).map_or(ptr::null_mut(), |name| LoadCursorW(ptr::null_mut(), name)));
                1
            }
            WM_APPLY_CAPTURE => {
                if link.captured.load(Ordering::Acquire) {
                    let mut client = RECT::default();
                    GetClientRect(hwnd, &mut client);
                    let mut corner = POINT { x: 0, y: 0 };
                    ClientToScreen(hwnd, &mut corner);
                    let area = RECT { left: corner.x, top: corner.y, right: corner.x + client.right, bottom: corner.y + client.bottom };
                    ClipCursor(&area);
                    let middle = center(hwnd);
                    SetCursorPos(middle.x, middle.y);
                    SetCursor(ptr::null_mut());
                } else {
                    ClipCursor(ptr::null());
                }
                0
            }
            WM_TAKE_FOCUS => {
                SetFocus(hwnd);
                0
            }
            WM_CLOSE => {
                link.send(Input::Close);
                0
            }
            WM_RETIRE => {
                DestroyWindow(hwnd);
                0
            }
            WM_DESTROY => {
                if link.captured.load(Ordering::Acquire) {
                    ClipCursor(ptr::null());
                }
                link.send(Input::Close);
                if GetWindowLongPtrW(hwnd, GWL_STYLE) as u32 & WS_CHILD == 0 {
                    PostQuitMessage(0);
                }
                0
            }
            _ => DefWindowProcW(hwnd, message, wparam, lparam),
        }
    }
}

pub fn pump() {
    let mut message = MSG::default();
    unsafe {
        while GetMessageW(&mut message, ptr::null_mut(), 0, 0) > 0 {
            TranslateMessage(&message);
            DispatchMessageW(&message);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use egui::Key;

    #[test]
    fn keys_take_egui_names_and_controls_their_place_on_a_us_keyboard() {
        assert_eq!(key(0x57, false), Some(Key::W));
        assert_eq!(key(0x33, false), Some(Key::Num3));
        assert_eq!(key(0x72, false), Some(Key::F3));
        assert_eq!(key(0x7b, false), Some(Key::F12));
        assert_eq!(key(0xbb, false), Some(Key::Equals));
        assert_eq!(key(0xbb, true), Some(Key::Plus));
        assert_eq!(key(0x6b, false), Some(Key::Plus));
        assert_eq!(key(0xbd, false), Some(Key::Minus));
        assert_eq!(key(0x10, false), None, "Shift is a modifier, not a key");
        assert_eq!(physical_key(0x11, false), Some(Key::W));
        assert_eq!(physical_key(0x1e, false), Some(Key::A));
        assert_eq!(physical_key(0x0b, false), Some(Key::Num0));
        assert_eq!(physical_key(0x48, true), None);
    }

    fn dispatch_posted() {
        let mut message = MSG::default();
        unsafe {
            while PeekMessageW(&mut message, ptr::null_mut(), 0, 0, PM_REMOVE) != 0 {
                DispatchMessageW(&message);
            }
        }
    }

    #[test]
    fn a_close_asks_the_map_to_stop_and_only_the_map_destroys_the_window() {
        let (window, inputs) = Window::create(Some(HWND_MESSAGE as usize), [1, 1], "close test").unwrap();
        let hwnd = window.hwnd() as HWND;
        dispatch_posted();
        let _ = inputs.received.try_iter().count();
        unsafe {
            SendMessageW(hwnd, WM_CLOSE, 0, 0);
            assert_ne!(IsWindow(hwnd), 0, "the window outlives a close");
        }
        assert_eq!(inputs.received.try_iter().collect::<Vec<_>>(), [Input::Close]);
        window.destroy();
        dispatch_posted();
        assert_eq!(unsafe { IsWindow(hwnd) }, 0, "the map destroys the window once it lets it go");
    }

    #[test]
    fn gaining_the_focus_tells_the_map_which_modifiers_are_down() {
        use windows_sys::Win32::UI::Input::KeyboardAndMouse::{GetKeyboardState, SetKeyboardState};
        let (window, inputs) = Window::create(Some(HWND_MESSAGE as usize), [1, 1], "focus test").unwrap();
        let hwnd = window.hwnd() as HWND;
        dispatch_posted();
        let _ = inputs.received.try_iter().count();
        let mut keys = [0_u8; 256];
        unsafe {
            GetKeyboardState(keys.as_mut_ptr());
            let held = keys;
            keys[usize::from(VK_SHIFT)] = 0x80;
            SetKeyboardState(keys.as_ptr());
            SendMessageW(hwnd, WM_SETFOCUS, 0, 0);
            SetKeyboardState(held.as_ptr());
        }
        assert_eq!(
            inputs.received.try_iter().collect::<Vec<_>>(),
            [Input::Focus(true), Input::Event(egui::Event::ModifiersChanged(egui::Modifiers::SHIFT))]
        );
        window.destroy();
        dispatch_posted();
    }

    #[test]
    fn every_pointer_shape_the_map_asks_for_has_a_system_cursor_or_none() {
        assert_eq!(system_cursor(cursor_index(egui::CursorIcon::None)), None);
        assert_eq!(system_cursor(cursor_index(egui::CursorIcon::PointingHand)), Some(IDC_HAND));
        assert_eq!(system_cursor(cursor_index(egui::CursorIcon::Default)), Some(IDC_ARROW));
        assert_eq!(system_cursor(cursor_index(egui::CursorIcon::ResizeHorizontal)), Some(IDC_SIZEWE));
    }
}
