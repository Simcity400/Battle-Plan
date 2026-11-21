#![cfg(windows)]

use std::{
    ffi::c_void,
    ptr,
    sync::atomic::{AtomicI32, AtomicPtr, AtomicU32, AtomicU64, Ordering},
};
use windows_sys::Win32::{
    Foundation::*,
    System::{LibraryLoader::*, Memory::*, Threading::GetCurrentProcessId},
    UI::{Input::KeyboardAndMouse::*, WindowsAndMessaging::*},
};

#[repr(C)]
struct Control {
    magic: AtomicU32,
    version: AtomicU32,
    mask: AtomicU32,
    flags: AtomicU32,
    x: AtomicI32,
    y: AtomicI32,
    window: AtomicU64,
    calls: [AtomicU32; 7],
    reserved: AtomicU32,
    last_async: AtomicU32,
    last_state: AtomicU32,
    error: AtomicU32,
    installed: AtomicU32,
    process: AtomicU32,
}
const _: () = assert!(
    std::mem::offset_of!(Control, flags) == 12
        && std::mem::offset_of!(Control, calls) == 32
        && std::mem::offset_of!(Control, last_async) == 64
);
static CONTROL: AtomicPtr<Control> = AtomicPtr::new(ptr::null_mut());
const MAGIC: u32 = 0x4d485348;
const MAPPING: &[u16] = &[
    76, 111, 99, 97, 108, 92, 104, 111, 105, 52, 95, 105, 110, 112, 117, 116, 115, 104, 105, 109,
    95, 118, 49, 0,
];

fn active(hook: usize, flag: u32) -> Option<&'static Control> {
    let control = unsafe { CONTROL.load(Ordering::Acquire).as_ref() }?;
    control.calls[hook].fetch_add(1, Ordering::Relaxed);
    (control.magic.load(Ordering::Acquire) == MAGIC
        && control.mask.load(Ordering::Relaxed) & (1 << hook) != 0
        && control.flags.load(Ordering::Acquire) & flag != 0)
        .then_some(control)
}
unsafe extern "system" fn cursor(point: *mut POINT) -> i32 {
    if let Some(control) = active(0, 1) {
        if point.is_null() {
            return 0;
        }
        unsafe {
            point.write(POINT {
                x: control.x.load(Ordering::Relaxed),
                y: control.y.load(Ordering::Relaxed),
            });
        }
        1
    } else {
        unsafe { GetCursorPos(point) }
    }
}
unsafe extern "system" fn asynchronous(key: i32) -> i16 {
    if let Some(control) = active(1, 2) {
        control.last_async.store(key as u32, Ordering::Relaxed);
        if key == 1 && control.flags.load(Ordering::Acquire) & 4 != 0 {
            i16::MIN
        } else {
            0
        }
    } else {
        unsafe { GetAsyncKeyState(key) }
    }
}
unsafe extern "system" fn state(key: i32) -> i16 {
    if let Some(control) = active(2, 2) {
        control.last_state.store(key as u32, Ordering::Relaxed);
        if key == 1 && control.flags.load(Ordering::Acquire) & 4 != 0 {
            i16::MIN
        } else {
            0
        }
    } else {
        unsafe { GetKeyState(key) }
    }
}
unsafe extern "system" fn keyboard(keys: *mut u8) -> i32 {
    if let Some(control) = active(3, 2) {
        if keys.is_null() {
            return 0;
        }
        unsafe {
            ptr::write_bytes(keys, 0, 256);
            if control.flags.load(Ordering::Acquire) & 4 != 0 {
                keys.add(1).write(0x80);
            }
        }
        1
    } else {
        unsafe { GetKeyboardState(keys) }
    }
}
unsafe extern "system" fn foreground() -> HWND {
    active(4, 2).map_or_else(
        || unsafe { GetForegroundWindow() },
        |control| control.window.load(Ordering::Relaxed) as HWND,
    )
}
unsafe extern "system" fn window_active() -> HWND {
    active(5, 2).map_or_else(
        || unsafe { GetActiveWindow() },
        |control| control.window.load(Ordering::Relaxed) as HWND,
    )
}
unsafe extern "system" fn focus() -> HWND {
    active(6, 2).map_or_else(
        || unsafe { GetFocus() },
        |control| control.window.load(Ordering::Relaxed) as HWND,
    )
}
struct Slot {
    address: *mut usize,
    replacement: usize,
    hook: usize,
}

/// # Safety
/// The process's main module stays loaded, every page of the `SizeOfImage` span its headers
/// declare is readable, and nothing writes that span while this reads it.
unsafe fn imports() -> Option<Vec<Slot>> {
    let base = unsafe { GetModuleHandleW(ptr::null()) } as *mut u8;
    if base.is_null() || unsafe { base.cast::<u16>().read_unaligned() } != 0x5a4d {
        return None;
    }
    let nt = unsafe { base.add(0x3c).cast::<u32>().read_unaligned() } as usize;
    if nt > 0x1000
        || unsafe { base.add(nt).cast::<u32>().read_unaligned() } != 0x4550
        || unsafe { base.add(nt + 24).cast::<u16>().read_unaligned() } != 0x20b
    {
        return None;
    }
    let size = unsafe { base.add(nt + 80).cast::<u32>().read_unaligned() } as usize;
    let bytes = unsafe { std::slice::from_raw_parts(base, size) };
    let u32_at = |at: usize| {
        Some(u32::from_le_bytes(
            bytes.get(at..at.checked_add(4)?)?.try_into().ok()?,
        ))
    };
    let u64_at = |at: usize| {
        Some(u64::from_le_bytes(
            bytes.get(at..at.checked_add(8)?)?.try_into().ok()?,
        ))
    };
    let names: [&[u8]; 7] = [
        b"GetCursorPos",
        b"GetAsyncKeyState",
        b"GetKeyState",
        b"GetKeyboardState",
        b"GetForegroundWindow",
        b"GetActiveWindow",
        b"GetFocus",
    ];
    let replacements = [
        cursor as *const () as usize,
        asynchronous as *const () as usize,
        state as *const () as usize,
        keyboard as *const () as usize,
        foreground as *const () as usize,
        window_active as *const () as usize,
        focus as *const () as usize,
    ];
    let directory = u32_at(nt + 144)? as usize;
    let end = directory.checked_add(u32_at(nt + 148)? as usize)?;
    if directory == 0 || end > size {
        return None;
    }
    let mut slots = Vec::new();
    for descriptor in (directory..end).step_by(20) {
        let lookup = u32_at(descriptor)? as usize;
        let table = u32_at(descriptor + 16)? as usize;
        if table == 0 {
            break;
        }
        if lookup == 0 {
            continue;
        }
        let mut index = 0;
        loop {
            let imported = u64_at(lookup.checked_add(index)?)?;
            if imported == 0 {
                break;
            }
            if imported >> 63 == 0 {
                let start = (imported as usize).checked_add(2)?;
                let tail = bytes.get(start..)?;
                let length = tail.iter().position(|&byte| byte == 0)?;
                if let Some(hook) = names.iter().position(|&name| name == &tail[..length]) {
                    let address = table.checked_add(index)?;
                    u64_at(address)?;
                    if address % 8 != 0 {
                        return None;
                    }
                    slots.push(Slot {
                        address: unsafe { base.add(address).cast() },
                        replacement: replacements[hook],
                        hook,
                    });
                }
            }
            index = index.checked_add(8)?;
        }
    }
    (!slots.is_empty()).then_some(slots)
}

/// # Safety
/// Only `DllMain` calls this, once, at process attach. The requirements of [`imports`] apply.
unsafe fn install() -> bool {
    let Some(slots) = (unsafe { imports() }) else {
        return false;
    };
    let mapping = unsafe {
        CreateFileMappingW(
            INVALID_HANDLE_VALUE,
            ptr::null(),
            PAGE_READWRITE,
            0,
            4096,
            MAPPING.as_ptr(),
        )
    };
    if mapping.is_null() {
        return false;
    }
    if unsafe { GetLastError() } == ERROR_ALREADY_EXISTS {
        unsafe {
            CloseHandle(mapping);
        }
        return false;
    }
    let view = unsafe { MapViewOfFile(mapping, FILE_MAP_ALL_ACCESS, 0, 0, 4096) };
    if view.Value.is_null() {
        unsafe {
            CloseHandle(mapping);
        }
        return false;
    }
    let control = unsafe { &*view.Value.cast::<Control>() };
    control.version.store(1, Ordering::Relaxed);
    control.mask.store(127, Ordering::Relaxed);
    control
        .process
        .store(unsafe { GetCurrentProcessId() }, Ordering::Relaxed);
    CONTROL.store(view.Value.cast(), Ordering::Release);
    let mut installed = 0;
    for slot in slots {
        let mut previous = 0;
        if unsafe {
            VirtualProtect(
                slot.address.cast(),
                size_of::<usize>(),
                PAGE_READWRITE,
                &mut previous,
            )
        } == 0
        {
            control
                .error
                .store(unsafe { GetLastError() }, Ordering::Release);
            return true;
        }
        unsafe {
            (&*slot.address.cast::<std::sync::atomic::AtomicUsize>())
                .store(slot.replacement, Ordering::Release);
        }
        let mut unused = 0;
        if unsafe {
            VirtualProtect(
                slot.address.cast(),
                size_of::<usize>(),
                previous,
                &mut unused,
            )
        } == 0
        {
            control
                .error
                .store(unsafe { GetLastError() }, Ordering::Release);
            return true;
        }
        installed |= 1 << slot.hook;
    }
    control.installed.store(installed, Ordering::Relaxed);
    control.magic.store(MAGIC, Ordering::Release);
    true
}

/// # Safety
/// Only the Windows loader calls this, with its own arguments.
#[unsafe(no_mangle)]
pub unsafe extern "system" fn DllMain(
    _module: HINSTANCE,
    reason: u32,
    _reserved: *mut c_void,
) -> i32 {
    if reason == 1 {
        i32::from(unsafe { install() })
    } else {
        1
    }
}
