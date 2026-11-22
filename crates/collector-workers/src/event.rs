use std::{io, ptr};
use windows_sys::Win32::{
    Foundation::{CloseHandle, HANDLE, WAIT_OBJECT_0},
    System::Threading::{CreateEventW, ResetEvent, SetEvent, WaitForSingleObject},
};

pub struct Event(HANDLE);
// SAFETY: an event handle may be signalled, reset and waited on from any thread.
unsafe impl Send for Event {}
unsafe impl Sync for Event {}
impl Event {
    pub fn create(name: &str) -> Result<Self, &'static str> {
        if name.is_empty() || name.contains('\0') {
            return Err("event name is empty or contains NUL");
        }
        let wide: Vec<u16> = name.encode_utf16().chain(Some(0)).collect();
        let handle = unsafe { CreateEventW(ptr::null(), 1, 0, wide.as_ptr()) };
        if handle.is_null() {
            collector_memory::terminal::native_error(&io::Error::last_os_error());
            Err("CreateEventW failed")
        } else {
            Ok(Self(handle))
        }
    }
    /// # Safety
    /// Mutate a live event only as its owning collector role, at the matching publication or reset
    /// point in the session.
    pub unsafe fn set(&self) {
        unsafe {
            SetEvent(self.0);
        }
    }
    /// # Safety
    /// Mutate a live event only as its owning collector role, at the matching publication or reset
    /// point in the session.
    pub unsafe fn reset(&self) {
        unsafe {
            ResetEvent(self.0);
        }
    }
    pub fn signaled(&self) -> bool {
        unsafe { WaitForSingleObject(self.0, 0) == WAIT_OBJECT_0 }
    }
}
impl Drop for Event {
    fn drop(&mut self) {
        unsafe {
            CloseHandle(self.0);
        }
    }
}
