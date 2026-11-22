use serde::Serialize;
use std::sync::{Condvar, Mutex, MutexGuard, PoisonError};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(tag = "kind", rename_all = "lowercase")]
pub enum Request {
    Close,
    Ask { id: u64 },
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Verdict {
    Saved,
    Unsaved,
    Reloaded,
    Unguarded,
}

impl Verdict {
    pub const UNSAVED: &'static str =
        "BattlePlan has unsaved changes. Save or discard them in BattlePlan, then restart to update.";
    pub const RELOADED: &'static str =
        "BattlePlan reloaded before it said whether its changes are saved. Try again.";

    pub fn refusal(self) -> Option<&'static str> {
        match self {
            Self::Saved | Self::Unguarded => None,
            Self::Unsaved => Some(Self::UNSAVED),
            Self::Reloaded => Some(Self::RELOADED),
        }
    }
}

pub trait Page: Send + 'static {
    fn send(&self, request: Request) -> Result<(), String>;
}

impl Page for tauri::ipc::Channel<Request> {
    fn send(&self, request: Request) -> Result<(), String> {
        tauri::ipc::Channel::send(self, request).map_err(|error| error.to_string())
    }
}

struct State<P> {
    watcher: Option<(u64, P)>,
    next: u64,
    asks: Vec<(u64, Option<Verdict>)>,
}

pub struct Leaving<P> {
    state: Mutex<State<P>>,
    answered: Condvar,
}

impl<P> Default for Leaving<P> {
    fn default() -> Self {
        Self {
            state: Mutex::new(State { watcher: None, next: 0, asks: Vec::new() }),
            answered: Condvar::new(),
        }
    }
}

impl<P: Page> Leaving<P> {
    fn lock(&self) -> MutexGuard<'_, State<P>> {
        self.state.lock().unwrap_or_else(PoisonError::into_inner)
    }

    fn settle(&self, state: &mut State<P>, verdict: Verdict) {
        for (_, waiting) in state.asks.iter_mut().filter(|(_, waiting)| waiting.is_none()) {
            *waiting = Some(verdict);
        }
        self.answered.notify_all();
    }

    fn lost(&self, state: &mut State<P>, error: &str) {
        eprintln!("BattlePlan’s page can’t be reached, so nothing it held can be saved: {error}");
        state.watcher = None;
        self.settle(state, Verdict::Unguarded);
    }

    pub fn watch(&self, seq: u64, page: P) {
        let mut state = self.lock();
        if state.watcher.as_ref().is_some_and(|(current, _)| *current > seq) {
            return;
        }
        let waiting: Vec<u64> = state.asks.iter().filter(|(_, verdict)| verdict.is_none()).map(|(id, _)| *id).collect();
        for id in waiting {
            if let Err(error) = page.send(Request::Ask { id }) {
                return self.lost(&mut state, &error);
            }
        }
        state.watcher = Some((seq, page));
    }

    pub fn close_requested(&self) -> bool {
        let mut state = self.lock();
        let Some((_, page)) = &state.watcher else {
            return false;
        };
        match page.send(Request::Close) {
            Ok(()) => true,
            Err(error) => {
                self.lost(&mut state, &error);
                false
            }
        }
    }

    pub fn ask(&self) -> Verdict {
        let mut state = self.lock();
        let Some((_, page)) = &state.watcher else {
            return Verdict::Unguarded;
        };
        let id = state.next;
        let sent = page.send(Request::Ask { id });
        state.next += 1;
        state.asks.push((id, None));
        if let Err(error) = sent {
            self.lost(&mut state, &error);
        }
        loop {
            if let Some(at) = state.asks.iter().position(|(asked, _)| *asked == id)
                && let Some(verdict) = state.asks[at].1
            {
                state.asks.swap_remove(at);
                return verdict;
            }
            state = self.answered.wait(state).unwrap_or_else(PoisonError::into_inner);
        }
    }

    pub fn answer(&self, id: u64, unsaved: bool) {
        let mut state = self.lock();
        if let Some((_, waiting @ None)) = state.asks.iter_mut().find(|(asked, _)| *asked == id) {
            *waiting = Some(if unsaved { Verdict::Unsaved } else { Verdict::Saved });
            self.answered.notify_all();
        }
    }

    pub fn page_left(&self) {
        let mut state = self.lock();
        state.watcher = None;
        self.settle(&mut state, Verdict::Reloaded);
    }

    pub fn page_died(&self) {
        let mut state = self.lock();
        state.watcher = None;
        self.settle(&mut state, Verdict::Unguarded);
    }

    pub fn install(&self, confirmed: bool, launch: impl FnOnce() -> Result<(), String>) -> Result<(), String> {
        if !confirmed && let Some(refused) = self.ask().refusal() {
            return Err(refused.to_owned());
        }
        launch()
    }

    pub fn close_for_update(&self) -> Result<(), String> {
        self.ask().refusal().map_or(Ok(()), |refused| Err(refused.to_owned()))
    }

    pub fn busy(&self) -> Option<String> {
        self.ask().refusal().map(str::to_owned)
    }
}

#[cfg(test)]
mod tests {
    use super::{Leaving, Page, Request, Verdict};
    use std::sync::{Arc, mpsc};

    #[derive(Clone)]
    struct Fake {
        name: &'static str,
        sent: mpsc::Sender<(&'static str, Request)>,
        reachable: bool,
    }

    impl Page for Fake {
        fn send(&self, request: Request) -> Result<(), String> {
            if !self.reachable {
                return Err("the webview is gone".into());
            }
            self.sent.send((self.name, request)).map_err(|error| error.to_string())
        }
    }

    fn page() -> (mpsc::Sender<(&'static str, Request)>, mpsc::Receiver<(&'static str, Request)>) {
        mpsc::channel()
    }

    fn fake(name: &'static str, sent: &mpsc::Sender<(&'static str, Request)>) -> Fake {
        Fake { name, sent: sent.clone(), reachable: true }
    }

    fn asking(leaving: &Arc<Leaving<Fake>>) -> std::thread::JoinHandle<Verdict> {
        let leaving = Arc::clone(leaving);
        std::thread::spawn(move || leaving.ask())
    }

    #[test]
    fn a_close_with_no_page_listening_goes_through_and_one_with_a_page_is_asked() {
        let (sent, heard) = page();
        let leaving = Leaving::default();
        assert!(!leaving.close_requested(), "a page that never listened guards nothing");
        leaving.watch(1, fake("page", &sent));
        assert!(leaving.close_requested(), "the page decides");
        assert_eq!(heard.try_recv(), Ok(("page", Request::Close)));
        leaving.page_left();
        assert!(!leaving.close_requested(), "a reloaded page listens again before it is asked");
    }

    #[test]
    fn a_close_during_an_install_wait_still_asks_the_page() {
        let (sent, heard) = page();
        let leaving = Leaving::default();
        leaving.watch(1, fake("page", &sent));
        let installed = leaving.install(true, || {
            assert!(leaving.close_requested(), "the window stays open for the page to answer");
            assert_eq!(heard.try_recv(), Ok(("page", Request::Close)));
            Err("installer didn’t start".into())
        });
        assert_eq!(installed, Err("installer didn’t start".into()));
        assert!(leaving.close_requested(), "a failed install leaves nothing behind");
        assert_eq!(heard.try_recv(), Ok(("page", Request::Close)));
    }

    #[test]
    fn an_install_the_page_did_not_confirm_asks_the_page_at_its_last_step() {
        let (sent, heard) = page();
        let leaving = Arc::new(Leaving::default());
        leaving.watch(1, fake("page", &sent));
        let installing = {
            let leaving = Arc::clone(&leaving);
            std::thread::spawn(move || leaving.install(false, || Ok(())))
        };
        let Ok(("page", Request::Ask { id })) = heard.recv() else { panic!("the page wasn’t asked") };
        leaving.answer(id, true);
        assert_eq!(installing.join().unwrap(), Err(Verdict::UNSAVED.to_owned()));

        let installing = {
            let leaving = Arc::clone(&leaving);
            std::thread::spawn(move || leaving.install(false, || Ok(())))
        };
        let Ok(("page", Request::Ask { id })) = heard.recv() else { panic!("the page wasn’t asked") };
        leaving.answer(id, false);
        assert_eq!(installing.join().unwrap(), Ok(()));
        assert_eq!(leaving.install(true, || Ok(())), Ok(()), "a confirmed install trusts that confirmation");
        assert!(heard.try_recv().is_err(), "and asks nothing");
    }

    #[test]
    fn a_registration_sent_again_after_its_reply_was_lost_keeps_one_listener() {
        let (sent, heard) = page();
        let leaving = Leaving::default();
        leaving.watch(1, fake("first", &sent));
        leaving.watch(2, fake("second", &sent));
        leaving.watch(2, fake("second", &sent));
        leaving.watch(1, fake("first", &sent));
        assert!(leaving.close_requested());
        assert_eq!(heard.try_recv(), Ok(("second", Request::Close)), "only the newest listener hears, once");
        assert!(heard.try_recv().is_err());
    }

    #[test]
    fn a_question_waiting_on_a_superseded_listener_is_asked_of_its_successor() {
        let (sent, heard) = page();
        let leaving = Arc::new(Leaving::default());
        leaving.watch(1, fake("first", &sent));
        let asked = asking(&leaving);
        let Ok(("first", Request::Ask { id })) = heard.recv() else { panic!("the first listener wasn’t asked") };
        leaving.watch(2, fake("second", &sent));
        assert_eq!(heard.recv(), Ok(("second", Request::Ask { id })));
        leaving.answer(id, false);
        leaving.answer(id, true);
        assert_eq!(asked.join().unwrap(), Verdict::Saved, "the first answer counts and a repeat is ignored");
    }

    #[test]
    fn a_question_the_page_reloads_before_answering_is_refused_by_name() {
        let (sent, heard) = page();
        let leaving = Arc::new(Leaving::default());
        leaving.watch(1, fake("page", &sent));
        let asked = asking(&leaving);
        heard.recv().unwrap();
        leaving.page_left();
        let verdict = asked.join().unwrap();
        assert_eq!(verdict, Verdict::Reloaded);
        assert_eq!(verdict.refusal(), Some(Verdict::RELOADED));
    }

    #[test]
    fn a_dead_page_cannot_hold_the_app_open() {
        let (sent, heard) = page();
        let leaving = Arc::new(Leaving::default());
        leaving.watch(1, fake("page", &sent));
        let asked = asking(&leaving);
        heard.recv().unwrap();
        leaving.page_died();
        assert_eq!(asked.join().unwrap(), Verdict::Unguarded);
        assert!(!leaving.close_requested(), "the next close goes through");
        assert_eq!(leaving.close_for_update(), Ok(()));

        leaving.watch(2, Fake { name: "gone", sent: sent.clone(), reachable: false });
        assert!(!leaving.close_requested(), "a page that can’t be reached can’t be asked");
        assert_eq!(leaving.busy(), None);
    }
}
