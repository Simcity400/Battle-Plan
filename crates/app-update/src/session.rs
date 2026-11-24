use std::{
    cell::Cell,
    sync::{
        Arc,
        atomic::{AtomicU64, Ordering},
    },
};
use windows_sys::Win32::{
    Foundation::{HANDLE, HWND, LPARAM, LRESULT, WAIT_OBJECT_0, WPARAM},
    System::{
        LibraryLoader::GetModuleHandleW,
        Shutdown::{ShutdownBlockReasonCreate, ShutdownBlockReasonDestroy},
        Threading::{CreateEventW, INFINITE, SetEvent, WaitForSingleObject},
    },
    UI::{
        Shell::{DefSubclassProc, RemoveWindowSubclass, SetWindowSubclass},
        WindowsAndMessaging::{
            CreateWindowExW, MSG, MWMO_INPUTAVAILABLE, MsgWaitForMultipleObjectsEx, PM_NOREMOVE, PM_QS_SENDMESSAGE, PeekMessageW,
            PostMessageW, QS_SENDMESSAGE, WM_APP, WM_ENDSESSION, WM_NCDESTROY, WM_QUERYENDSESSION, WS_POPUP,
        },
    },
};

const SUBCLASS: usize = 0x4250_5345;
const DRAFTS: u32 = WM_APP + 0x4250;

pub struct Done(HANDLE, std::sync::atomic::AtomicBool);

// SAFETY: an event handle can be waited on and signalled from any thread, and is never closed.
unsafe impl Send for Done {}
// SAFETY: as above.
unsafe impl Sync for Done {}

impl Done {
    pub fn new() -> std::io::Result<Self> {
        // SAFETY: a manual-reset, unsignalled, unnamed event.
        let event = unsafe { CreateEventW(std::ptr::null(), 1, 0, std::ptr::null()) };
        if event.is_null() { Err(std::io::Error::last_os_error()) } else { Ok(Self(event, std::sync::atomic::AtomicBool::new(false))) }
    }

    pub fn set(&self) {
        // SAFETY: the event lives for the process.
        unsafe { SetEvent(self.0) };
    }

    pub fn is_set(&self) -> bool {
        // SAFETY: as above; a zero wait only tests it.
        unsafe { WaitForSingleObject(self.0, 0) == WAIT_OBJECT_0 }
    }

    pub fn wait(&self) {
        // SAFETY: as above.
        unsafe { WaitForSingleObject(self.0, INFINITE) };
    }

    pub fn awaited(&self) -> bool {
        self.1.load(std::sync::atomic::Ordering::Acquire)
    }

    pub fn wait_serving_sent_messages(&self) {
        thread_local! { static WAITING: Cell<bool> = const { Cell::new(false) }; }
        if WAITING.replace(true) {
            return;
        }
        self.1.store(true, std::sync::atomic::Ordering::Release);
        loop {
            // SAFETY: the event lives for the process; the wait returns for it or for a message
            // sent to this thread, which `PeekMessageW` then dispatches without removing anything
            // posted.
            unsafe {
                let woke = MsgWaitForMultipleObjectsEx(1, &raw const self.0, INFINITE, QS_SENDMESSAGE, MWMO_INPUTAVAILABLE);
                if woke == WAIT_OBJECT_0 {
                    break;
                }
                let mut message: MSG = std::mem::zeroed();
                PeekMessageW(&raw mut message, std::ptr::null_mut(), 0, 0, PM_NOREMOVE | PM_QS_SENDMESSAGE);
            }
        }
        WAITING.set(false);
    }
}

pub enum Drafts {
    Clean,
    Unsaved,
    Unknown,
}

pub trait Ending: Send + Sync {
    fn commit(&self);
    fn done(&self) -> &Done;
    fn saving(&self) -> Option<&'static str>;
    fn check_drafts(&self, answer: Box<dyn FnOnce(Drafts) + Send>) {
        answer(Drafts::Clean);
    }
    fn retire_drafts(&self) {}
}

pub const UNSAVED: &str = "Unsaved layout or theme changes";
pub const UNCHECKED: &str = "BattlePlan couldn’t check for unsaved changes";

trait Blocking {
    fn block(&self, reason: &str);
    fn unblock(&self);
    fn poster(&self) -> Box<dyn Fn(u64, Drafts) + Send>;
}

struct Followed {
    ending: Arc<dyn Ending>,
    query: AtomicU64,
}

fn reason(saving: Option<&str>, drafts: Option<&str>) -> Option<String> {
    match (saving, drafts) {
        (Some(saving), Some(drafts)) => Some(format!("{saving} {drafts}.")),
        (Some(only), None) | (None, Some(only)) => Some(only.to_owned()),
        (None, None) => None,
    }
}

fn on_message(follow: &Followed, window: &dyn Blocking, message: u32, wparam: WPARAM, lparam: LPARAM) {
    let ending = &*follow.ending;
    match message {
        WM_QUERYENDSESSION => {
            let query = follow.query.fetch_add(1, Ordering::AcqRel) + 1;
            if let Some(saving) = ending.saving() {
                window.block(saving);
            }
            let post = window.poster();
            ending.check_drafts(Box::new(move |drafts| post(query, drafts)));
        }
        DRAFTS if wparam as u64 == follow.query.load(Ordering::Acquire) => {
            let drafts = match lparam {
                1 => Some(UNSAVED),
                2 => Some(UNCHECKED),
                _ => None,
            };
            match reason(ending.saving(), drafts) {
                Some(reason) => window.block(&reason),
                None => window.unblock(),
            }
        }
        WM_ENDSESSION => {
            follow.query.fetch_add(1, Ordering::AcqRel);
            ending.retire_drafts();
            if wparam != 0 {
                ending.commit();
                ending.done().wait_serving_sent_messages();
            }
            window.unblock();
        }
        _ => {}
    }
}

struct Window(HWND);

impl Blocking for Window {
    fn block(&self, reason: &str) {
        let text: Vec<u16> = reason.encode_utf16().chain(Some(0)).collect();
        // SAFETY: this runs on the thread that owns the window, inside its window procedure, and
        // the text outlives the call.
        if unsafe { ShutdownBlockReasonCreate(self.0, text.as_ptr()) } == 0 {
            eprintln!("Windows wasn’t told why the session waits: {}", std::io::Error::last_os_error());
        }
    }

    fn unblock(&self) {
        // SAFETY: as above.
        unsafe { ShutdownBlockReasonDestroy(self.0) };
    }

    fn poster(&self) -> Box<dyn Fn(u64, Drafts) + Send> {
        let hwnd = self.0 as usize;
        Box::new(move |query, drafts| {
            let code = match drafts {
                Drafts::Clean => 0,
                Drafts::Unsaved => 1,
                Drafts::Unknown => 2,
            };
            // SAFETY: posting to a window handle is sound even once the window has gone; the
            // window's own thread applies it.
            unsafe { PostMessageW(hwnd as HWND, DRAFTS, query as usize, code) };
        })
    }
}

unsafe extern "system" fn followed(hwnd: HWND, message: u32, wparam: WPARAM, lparam: LPARAM, _: usize, data: usize) -> LRESULT {
    // SAFETY: `data` is the `Arc<Followed>` that `follow` leaked for this window. A clone is taken
    // before handling, so a destroy dispatched while this call waits can't free it under us; the
    // leaked reference is released once, at `WM_NCDESTROY`. Windows calls this on the window's
    // own thread.
    unsafe {
        let held = data as *const Followed;
        Arc::increment_strong_count(held);
        let follow = Arc::from_raw(held);
        on_message(&follow, &Window(hwnd), message, wparam, lparam);
        if message == WM_NCDESTROY {
            RemoveWindowSubclass(hwnd, Some(followed), SUBCLASS);
            drop(Arc::from_raw(held));
        }
        drop(follow);
        DefSubclassProc(hwnd, message, wparam, lparam)
    }
}
pub fn follow(hwnd: isize, ending: Arc<dyn Ending>) -> Result<(), String> {
    let data = Arc::into_raw(Arc::new(Followed { ending, query: AtomicU64::new(0) }));
    // SAFETY: the caller owns the window on this thread; the subclass releases `data` when the
    // window is destroyed, and it is released here if the subclass was never installed.
    if unsafe { SetWindowSubclass(hwnd as HWND, Some(followed), SUBCLASS, data as usize) } == 0 {
        // SAFETY: the subclass wasn't installed, so nothing else holds `data`.
        drop(unsafe { Arc::from_raw(data) });
        return Err(std::io::Error::last_os_error().to_string());
    }
    Ok(())
}

pub fn watch(ending: Arc<dyn Ending>, title: &str) -> Result<isize, String> {
    let (class, title): (Vec<u16>, Vec<u16>) = ("STATIC".encode_utf16().chain(Some(0)).collect(), title.encode_utf16().chain(Some(0)).collect());
    // SAFETY: the window is created on this thread, whose message loop then dispatches its messages,
    // and both strings outlive the call.
    let hwnd = unsafe {
        CreateWindowExW(0, class.as_ptr(), title.as_ptr(), WS_POPUP, 0, 0, 0, 0, std::ptr::null_mut(), std::ptr::null_mut(), GetModuleHandleW(std::ptr::null()), std::ptr::null())
    };
    if hwnd.is_null() {
        return Err(std::io::Error::last_os_error().to_string());
    }
    follow(hwnd as isize, ending)?;
    Ok(hwnd as isize)
}

#[cfg(test)]
mod tests {
    use super::{Blocking, DRAFTS, Done, Drafts, Ending, Followed, UNCHECKED, on_message, watch};
    use std::{
        cell::RefCell,
        sync::{
            Arc, Mutex,
            atomic::{AtomicU64, AtomicUsize, Ordering},
        },
    };
    use windows_sys::Win32::UI::WindowsAndMessaging::{DestroyWindow, ENDSESSION_LOGOFF, SendMessageW, WM_ENDSESSION, WM_NULL, WM_QUERYENDSESSION};

    struct App {
        commits: AtomicUsize,
        retired: AtomicUsize,
        done: Done,
        saving: Mutex<Option<&'static str>>,
        drafts: Mutex<Option<Box<dyn FnOnce(Drafts) + Send>>>,
    }

    impl App {
        fn new(saving: Option<&'static str>) -> Self {
            Self { commits: AtomicUsize::new(0), retired: AtomicUsize::new(0), done: Done::new().unwrap(), saving: Mutex::new(saving), drafts: Mutex::new(None) }
        }
    }

    impl Ending for App {
        fn commit(&self) {
            self.commits.fetch_add(1, Ordering::SeqCst);
        }
        fn done(&self) -> &Done {
            &self.done
        }
        fn saving(&self) -> Option<&'static str> {
            *self.saving.lock().unwrap()
        }
        fn check_drafts(&self, answer: Box<dyn FnOnce(Drafts) + Send>) {
            *self.drafts.lock().unwrap() = Some(answer);
        }
        fn retire_drafts(&self) {
            self.retired.fetch_add(1, Ordering::SeqCst);
        }
    }

    #[derive(Default)]
    struct Shown {
        reasons: RefCell<Vec<Option<String>>>,
        posted: Arc<Mutex<Vec<(u64, isize)>>>,
    }

    impl Blocking for Shown {
        fn block(&self, reason: &str) {
            self.reasons.borrow_mut().push(Some(reason.to_owned()));
        }
        fn unblock(&self) {
            self.reasons.borrow_mut().push(None);
        }
        fn poster(&self) -> Box<dyn Fn(u64, Drafts) + Send> {
            let posted = Arc::clone(&self.posted);
            Box::new(move |query, drafts| posted.lock().unwrap().push((query, match drafts { Drafts::Clean => 0, Drafts::Unsaved => 1, Drafts::Unknown => 2 })))
        }
    }

    const SAVING: &str = "Saving the recording…";

    #[test]
    fn asking_to_end_the_session_commits_nothing_and_only_a_current_draft_answer_names_a_reason() {
        let app = Arc::new(App::new(Some(SAVING)));
        let follow = Followed { ending: Arc::clone(&app) as Arc<dyn Ending>, query: AtomicU64::new(0) };
        let shown = Shown::default();
        on_message(&follow, &shown, WM_QUERYENDSESSION, 0, 0);
        assert_eq!(app.commits.load(Ordering::SeqCst), 0, "the query is answered at once and commits nothing");
        assert_eq!(*shown.reasons.borrow(), [Some(SAVING.to_owned())], "a save in flight is named at once");
        app.drafts.lock().unwrap().take().unwrap()(Drafts::Unknown);
        let (query, code) = shown.posted.lock().unwrap()[0];
        on_message(&follow, &shown, DRAFTS, query as usize, code);
        assert_eq!(shown.reasons.borrow().last().cloned().flatten(), Some(format!("{SAVING} {UNCHECKED}.")), "an unknown draft check keeps the known save reason");
        on_message(&follow, &shown, WM_ENDSESSION, 0, 0);
        assert_eq!(app.retired.load(Ordering::SeqCst), 1, "a cancelled end retires the page question");
        assert_eq!(shown.reasons.borrow().last(), Some(&None));
        let before = shown.reasons.borrow().len();
        on_message(&follow, &shown, DRAFTS, query as usize, 1);
        assert_eq!(shown.reasons.borrow().len(), before, "a late answer to a retired query changes nothing");
        assert_eq!(app.commits.load(Ordering::SeqCst), 0, "recording and editing carry on");
        on_message(&follow, &shown, WM_QUERYENDSESSION, 0, 0);
        assert_eq!(shown.reasons.borrow().last().cloned().flatten(), Some(SAVING.to_owned()));
        *app.saving.lock().unwrap() = None;
        app.drafts.lock().unwrap().take().unwrap()(Drafts::Clean);
        let (query, code) = *shown.posted.lock().unwrap().last().unwrap();
        on_message(&follow, &shown, DRAFTS, query as usize, code);
        assert_eq!(shown.reasons.borrow().last(), Some(&None), "a save that ended before a clean draft answer leaves no reason registered");
    }

    #[test]
    fn the_session_ending_commits_and_returns_only_at_done_while_still_answering_messages_sent_to_its_window() {
        let app = Arc::new(App::new(None));
        let hwnd = watch(Arc::clone(&app) as Arc<dyn Ending>, "BattlePlan (test)").unwrap();
        std::thread::scope(|scope| {
            let sender = {
                let app = Arc::clone(&app);
                scope.spawn(move || {
                    while app.commits.load(Ordering::SeqCst) == 0 {
                        std::thread::yield_now();
                    }
                    // SAFETY: a synchronous message to a live window owned by the waiting thread.
                    unsafe { SendMessageW(hwnd as _, WM_NULL, 0, 0) };
                    app.done.set();
                })
            };
            // SAFETY: the window was created on this thread.
            unsafe { SendMessageW(hwnd as _, WM_ENDSESSION, 1, ENDSESSION_LOGOFF as isize) };
            sender.join().unwrap();
        });
        assert_eq!(app.commits.load(Ordering::SeqCst), 1);
        // SAFETY: as above; a second delivery commits again and returns at once, DONE being set.
        unsafe {
            SendMessageW(hwnd as _, WM_ENDSESSION, 1, ENDSESSION_LOGOFF as isize);
            DestroyWindow(hwnd as _);
        }
        assert_eq!(app.commits.load(Ordering::SeqCst), 2);
    }
}