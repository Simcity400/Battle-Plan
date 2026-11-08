//! One writer of a prepared store across processes; the headless preparer yields to the app.

use std::{
    collections::BTreeMap,
    ffi::OsStr,
    fs::{File, OpenOptions, TryLockError},
    io,
    os::windows::ffi::OsStrExt,
    path::{Path, PathBuf},
    sync::{
        Mutex,
        atomic::{AtomicBool, Ordering},
    },
    time::Duration,
};
use windows_sys::Win32::{
    Foundation::{CloseHandle, HANDLE, WAIT_OBJECT_0},
    System::Threading::{
        CreateEventW, EVENT_MODIFY_STATE, INFINITE, OpenEventW, ResetEvent, SetEvent,
        WaitForMultipleObjects,
    },
};

static YIELDS: AtomicBool = AtomicBool::new(false);

static YIELDED: Mutex<BTreeMap<PathBuf, u64>> = Mutex::new(BTreeMap::new());

/// Times this process has given the store at `folder` up to another process.
pub fn yielded(folder: &Path) -> u64 {
    YIELDED
        .lock()
        .unwrap_or_else(|error| error.into_inner())
        .get(folder)
        .copied()
        .unwrap_or(0)
}

/// What a job of a yielding process fails with when another process holds the store.
pub const ELSEWHERE: &str = "Another process is preparing this recording";

/// What it fails with when, holding the store, it finds the recording no longer asked for.
pub const WITHDRAWN: &str = "The recording is no longer asked for";

/// This process gives its stores up when asked, and leaves one another process holds.
pub fn yield_to_others() {
    YIELDS.store(true, Ordering::Release);
}

/// Whether `error` says the store is the app's: it holds it, or the user cleared it there.
pub fn held_elsewhere(error: &str) -> bool {
    error == ELSEWHERE || error == WITHDRAWN
}

/// Runs `work` as the store's only writer; a yielding process is stopped through `cancelled`.
pub fn hold<T>(
    folder: &Path,
    recording: &Path,
    cancelled: &AtomicBool,
    work: impl FnOnce() -> io::Result<T>,
) -> io::Result<T> {
    let lock = lock_file(folder)?;
    let name = event_name(folder);
    let yields = YIELDS.load(Ordering::Acquire);
    loop {
        match lock.try_lock() {
            Ok(()) => break,
            Err(TryLockError::WouldBlock) if yields => {
                return Err(io::Error::new(io::ErrorKind::WouldBlock, ELSEWHERE));
            }
            Err(TryLockError::WouldBlock) => {
                // Asked again each turn: a holder that took the store just now resets its event.
                ask_to_yield(&name);
                crate::metadata::check_cancel(cancelled)?;
                std::thread::sleep(Duration::from_millis(100));
            }
            Err(TryLockError::Error(error)) => return Err(error),
        }
    }
    if !yields {
        return work();
    }
    // Under the lock: the app's Clear takes back the asks before it takes the same lock.
    if !crate::background::pending(recording)? {
        return Err(io::Error::new(io::ErrorKind::WouldBlock, WITHDRAWN));
    }
    let asked = Event::create(Some(&name))?;
    // A request made before this process held the store was for its earlier holder.
    unsafe { ResetEvent(asked.0) };
    let done = Event::create(None)?;
    // Addresses, so the waiting thread owns them; both events outlive the scope.
    let handles = [asked.0 as usize, done.0 as usize];
    let store = folder.to_owned();
    std::thread::scope(|scope| {
        scope.spawn(move || {
            let handles = handles.map(|handle| handle as HANDLE);
            let woke = unsafe { WaitForMultipleObjects(2, handles.as_ptr(), 0, INFINITE) };
            if woke == WAIT_OBJECT_0 {
                *YIELDED
                    .lock()
                    .unwrap_or_else(|error| error.into_inner())
                    .entry(store)
                    .or_default() += 1;
                cancelled.store(true, Ordering::Release);
            }
        });
        // Signalled on unwind too, or the scope would wait for the waiter forever.
        let _done = Signal(&done);
        work()
    })
}

struct Signal<'a>(&'a Event);

impl Drop for Signal<'_> {
    fn drop(&mut self) {
        unsafe { SetEvent(self.0.0) };
    }
}

/// Beside the store, which its own cleanup never lists.
fn lock_file(folder: &Path) -> io::Result<File> {
    if let Some(root) = folder.parent() {
        std::fs::create_dir_all(root)?;
    }
    OpenOptions::new()
        .read(true)
        .write(true)
        .create(true)
        .truncate(false)
        .open(folder.with_extension("preparing"))
}

fn event_name(folder: &Path) -> Vec<u16> {
    let store = folder.file_name().unwrap_or(OsStr::new("store"));
    OsStr::new("Local\\HOI4Replay.prepare.")
        .encode_wide()
        .chain(store.encode_wide())
        .chain([0])
        .collect()
}

/// A holder that does not yield has no event, and is waited for.
fn ask_to_yield(name: &[u16]) {
    let event = unsafe { OpenEventW(EVENT_MODIFY_STATE, 0, name.as_ptr()) };
    if !event.is_null() {
        unsafe {
            SetEvent(event);
            CloseHandle(event);
        }
    }
}

struct Event(HANDLE);

impl Event {
    fn create(name: Option<&[u16]>) -> io::Result<Self> {
        let name = name.map_or(std::ptr::null(), <[u16]>::as_ptr);
        let event = unsafe { CreateEventW(std::ptr::null(), 0, 0, name) };
        if event.is_null() {
            return Err(io::Error::last_os_error());
        }
        Ok(Self(event))
    }
}

impl Drop for Event {
    fn drop(&mut self) {
        unsafe { CloseHandle(self.0) };
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_waiting_writer_takes_the_store_once_its_holder_lets_it_go() {
        let root = std::env::temp_dir().join(format!("prepared-owner-{}", std::process::id()));
        let folder = root.join("store");
        let held = lock_file(&folder).unwrap();
        held.lock().unwrap();
        let cancelled = AtomicBool::new(true);
        let waited = hold(&folder, &folder, &cancelled, || Ok(()));
        assert_eq!(waited.unwrap_err().kind(), io::ErrorKind::Interrupted);
        drop(held);
        assert_eq!(
            hold(&folder, &folder, &AtomicBool::new(false), || Ok(7)).unwrap(),
            7
        );
        std::fs::remove_dir_all(root).unwrap();
    }
}
