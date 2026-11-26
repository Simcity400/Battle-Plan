use std::{cell::Cell, rc::Rc};

use webview2_com::Microsoft::Web::WebView2::Win32::{
    COREWEBVIEW2_MEMORY_USAGE_TARGET_LEVEL_LOW, COREWEBVIEW2_MEMORY_USAGE_TARGET_LEVEL_NORMAL,
    ICoreWebView2, ICoreWebView2_19,
};
use windows_core::Interface;
use windows_sys::Win32::{
    Foundation::{HWND, LPARAM, LRESULT, WPARAM},
    UI::{
        Shell::{DefSubclassProc, RemoveWindowSubclass, SetWindowSubclass},
        WindowsAndMessaging::{IsIconic, IsWindowVisible, WM_NCDESTROY, WM_WINDOWPOSCHANGED},
    },
};

struct PageMemory {
    page: ICoreWebView2_19,
    low: Cell<bool>,
}

impl PageMemory {
    fn follow_window(&self, hwnd: HWND) {
        let low = unsafe { IsIconic(hwnd) != 0 || IsWindowVisible(hwnd) == 0 };
        if self.low.replace(low) != low {
            let level = if low {
                COREWEBVIEW2_MEMORY_USAGE_TARGET_LEVEL_LOW
            } else {
                COREWEBVIEW2_MEMORY_USAGE_TARGET_LEVEL_NORMAL
            };
            if let Err(error) = unsafe { self.page.SetMemoryUsageTargetLevel(level) } {
                self.low.set(!low);
                eprintln!("BattlePlan could not change its web view memory target: {error}");
            }
        }
    }
}

unsafe extern "system" fn window_changed(
    hwnd: HWND,
    message: u32,
    wparam: WPARAM,
    lparam: LPARAM,
    id: usize,
    data: usize,
) -> LRESULT {
    let memory = data as *const PageMemory;
    if message == WM_WINDOWPOSCHANGED {
        unsafe { Rc::increment_strong_count(memory) };
        unsafe { Rc::from_raw(memory) }.follow_window(hwnd);
    } else if message == WM_NCDESTROY {
        unsafe { RemoveWindowSubclass(hwnd, Some(window_changed), id) };
        drop(unsafe { Rc::from_raw(memory) });
    }
    unsafe { DefSubclassProc(hwnd, message, wparam, lparam) }
}

pub(super) fn watch(hwnd: isize, page: &ICoreWebView2) {
    let page = match page.cast::<ICoreWebView2_19>() {
        Ok(page) => page,
        Err(error) => {
            eprintln!("BattlePlan cannot set a memory target on this WebView2 runtime: {error}");
            return;
        }
    };
    let hwnd = hwnd as HWND;
    let memory = Rc::new(PageMemory {
        page,
        low: Cell::new(false),
    });
    let held = Rc::into_raw(Rc::clone(&memory));
    if unsafe { SetWindowSubclass(hwnd, Some(window_changed), 0, held as usize) } == 0 {
        drop(unsafe { Rc::from_raw(held) });
        eprintln!("BattlePlan could not follow its window's visibility for web view memory");
        return;
    }
    memory.follow_window(hwnd);
}
