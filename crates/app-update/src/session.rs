use std::{sync::Arc, time::Duration};
use windows_sys::Win32::{
    Foundation::{HWND, LPARAM, LRESULT, WPARAM},
    System::{
        LibraryLoader::GetModuleHandleW,
        Shutdown::{ShutdownBlockReasonCreate, ShutdownBlockReasonDestroy},
    },
    UI::{
        Shell::{DefSubclassProc, RemoveWindowSubclass, SetWindowSubclass},
        WindowsAndMessaging::{CreateWindowExW, WM_ENDSESSION, WM_NCDESTROY, WM_QUERYENDSESSION, WS_POPUP},
    },
};

const SUBCLASS: usize = 0x4250_5345;

pub trait Ending: Send + Sync {
    fn commit(&self);
    fn drained_within(&self, limit: Duration) -> bool;
}

trait Blocking {
    fn block(&self, reason: &str);
    fn unblock(&self);
}

pub fn end(ending: &dyn Ending) {
    ending.commit();
    while !ending.drained_within(Duration::from_secs(60)) {}
}

fn on_message(ending: &dyn Ending, blocking: &dyn Blocking, message: u32, wparam: WPARAM, reason: &str) {
    match message {
        WM_QUERYENDSESSION => {
            ending.commit();
            if !ending.drained_within(Duration::ZERO) {
                blocking.block(reason);
            }
        }
        WM_ENDSESSION if wparam != 0 => {
            end(ending);
            blocking.unblock();
        }
        WM_ENDSESSION => blocking.unblock(),
        _ => {}
    }
}

struct Followed {
    ending: Arc<dyn Ending>,
    reason: &'static str,
}

struct Reason(HWND);

impl Blocking for Reason {
    fn block(&self, reason: &str) {
        let text: Vec<u16> = reason.encode_utf16().chain(Some(0)).collect();
        // SAFETY: this runs on the thread that owns the window, inside its window procedure, and
        // the text outlives the call.
        if unsafe { ShutdownBlockReasonCreate(self.0, text.as_ptr()) } == 0 {
            eprintln!("Windows wasn’t told why the session waits: {}", std::io::Error::last_os_error());
        }
    }

    fn unblock(&self) {
        // SAFETY: this runs on the thread that owns the window, inside its window procedure.
        unsafe { ShutdownBlockReasonDestroy(self.0) };
    }
}

unsafe extern "system" fn followed(hwnd: HWND, message: u32, wparam: WPARAM, lparam: LPARAM, _: usize, data: usize) -> LRESULT {
    // SAFETY: `data` is the `Followed` that `follow` leaked for this window; it is freed only here,
    // once the window is destroyed and the subclass removed, and Windows calls this on the window's
    // own thread.
    unsafe {
        let follow = &*(data as *const Followed);
        on_message(&*follow.ending, &Reason(hwnd), message, wparam, follow.reason);
        if message == WM_NCDESTROY {
            RemoveWindowSubclass(hwnd, Some(followed), SUBCLASS);
            drop(Box::from_raw(data as *mut Followed));
        }
        DefSubclassProc(hwnd, message, wparam, lparam)
    }
}

pub fn follow(hwnd: isize, ending: Arc<dyn Ending>, reason: &'static str) -> Result<(), String> {
    let data = Box::into_raw(Box::new(Followed { ending, reason }));
    // SAFETY: the caller owns the window on this thread; the subclass frees `data` when the window
    // is destroyed, and it is freed here if the subclass was never installed.
    if unsafe { SetWindowSubclass(hwnd as HWND, Some(followed), SUBCLASS, data as usize) } == 0 {
        // SAFETY: the subclass wasn't installed, so nothing else holds `data`.
        drop(unsafe { Box::from_raw(data) });
        return Err(std::io::Error::last_os_error().to_string());
    }
    Ok(())
}

pub fn watch(ending: Arc<dyn Ending>, reason: &'static str) -> Result<isize, String> {
    let (class, title): (Vec<u16>, Vec<u16>) = ("STATIC".encode_utf16().chain(Some(0)).collect(), reason.encode_utf16().chain(Some(0)).collect());
    // SAFETY: the window is created on this thread, whose message loop then dispatches its messages,
    // and both strings outlive the call.
    let hwnd = unsafe {
        CreateWindowExW(0, class.as_ptr(), title.as_ptr(), WS_POPUP, 0, 0, 0, 0, std::ptr::null_mut(), std::ptr::null_mut(), GetModuleHandleW(std::ptr::null()), std::ptr::null())
    };
    if hwnd.is_null() {
        return Err(std::io::Error::last_os_error().to_string());
    }
    follow(hwnd as isize, ending, reason)?;
    Ok(hwnd as isize)
}

#[cfg(test)]
mod tests {
    use super::{Blocking, Ending, on_message, watch};
    use std::{
        cell::RefCell,
        sync::{
            Arc, Mutex,
            atomic::{AtomicBool, AtomicUsize, Ordering},
        },
        time::Duration,
    };
    use windows_sys::Win32::UI::WindowsAndMessaging::{DestroyWindow, ENDSESSION_LOGOFF, SendMessageW, WM_ENDSESSION, WM_QUERYENDSESSION};

    #[derive(Default)]
    struct Work {
        commits: AtomicUsize,
        drained: AtomicBool,
        waits: Mutex<Vec<Duration>>,
    }

    impl Ending for Work {
        fn commit(&self) {
            self.commits.fetch_add(1, Ordering::SeqCst);
        }

        fn drained_within(&self, limit: Duration) -> bool {
            self.waits.lock().unwrap().push(limit);
            if !limit.is_zero() && !self.drained.load(Ordering::SeqCst) {
                std::thread::yield_now();
            }
            self.drained.load(Ordering::SeqCst)
        }
    }

    #[derive(Default)]
    struct Shown(RefCell<Vec<Option<String>>>);

    impl Blocking for Shown {
        fn block(&self, reason: &str) {
            self.0.borrow_mut().push(Some(reason.to_owned()));
        }

        fn unblock(&self) {
            self.0.borrow_mut().push(None);
        }
    }

    const SAVING: &str = "Saving your changes before BattlePlan closes…";

    #[test]
    fn a_session_ending_commits_names_its_wait_and_returns_from_its_end_only_once_admitted_work_has_drained() {
        let work = Work::default();
        let shown = Shown::default();
        on_message(&work, &shown, WM_QUERYENDSESSION, 0, SAVING);
        assert_eq!(work.commits.load(Ordering::SeqCst), 1, "the query runs the one exit transition at once");
        assert_eq!(*shown.0.borrow(), [Some(SAVING.to_owned())], "and tells Windows what the session waits for");

        let ending = Arc::new(Work::default());
        std::thread::scope(|scope| {
            let finishing = Arc::clone(&ending);
            let ended = scope.spawn(move || {
                let shown = Shown::default();
                on_message(&*finishing, &shown, WM_ENDSESSION, 1, SAVING);
                shown.0.into_inner()
            });
            while ending.waits.lock().unwrap().len() < 3 {
                std::thread::yield_now();
            }
            assert!(!ended.is_finished(), "the end of the session waits for the work it admitted");
            ending.drained.store(true, Ordering::SeqCst);
            assert_eq!(ended.join().unwrap(), [None], "and releases its reason once it has drained");
        });
        assert_eq!(ending.commits.load(Ordering::SeqCst), 1, "an end with no query before it still commits");

        let work = Work { drained: AtomicBool::new(true), ..Work::default() };
        let shown = Shown::default();
        on_message(&work, &shown, WM_QUERYENDSESSION, 0, SAVING);
        on_message(&work, &shown, WM_ENDSESSION, 0, SAVING);
        assert_eq!(*shown.0.borrow(), [None], "nothing to wait for blocks nothing, and a cancelled end releases the reason");
    }

    #[test]
    fn the_window_that_follows_the_session_hears_its_end_through_the_subclass() {
        let work = Arc::new(Work { drained: AtomicBool::new(true), ..Work::default() });
        let hwnd = watch(Arc::clone(&work) as Arc<dyn Ending>, SAVING).unwrap();
        // SAFETY: the window was created on this thread, so these calls run its procedure here.
        unsafe {
            assert_eq!(SendMessageW(hwnd as _, WM_QUERYENDSESSION, 0, ENDSESSION_LOGOFF as isize), 1, "the session may end");
            SendMessageW(hwnd as _, WM_ENDSESSION, 1, ENDSESSION_LOGOFF as isize);
            DestroyWindow(hwnd as _);
        }
        assert_eq!(work.commits.load(Ordering::SeqCst), 2);
        assert!(work.waits.lock().unwrap().iter().any(|limit| !limit.is_zero()), "the end waited on the drain itself");
    }
}
