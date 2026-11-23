use replay_settings::exit::{Exit, Receipt, What};
use serde::Serialize;
use std::collections::BTreeSet;
use std::sync::{Arc, Condvar, Mutex, MutexGuard, PoisonError};
use std::time::Duration;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum About {
    Close,
    Install,
    Leave,
    Query,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
pub struct Ask {
    pub id: u64,
    pub about: About,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct Batch {
    pub seq: u64,
    pub waiting: Vec<Ask>,
    pub holds: Vec<Ask>,
    pub installing: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Verdict {
    Leaving,
    Staying,
    Reloaded,
    Unresponsive,
    Unheard,
    Unreached,
    Blind,
    Closing,
    Saving,
    Late,
}

impl Verdict {
    pub const STAYING: &'static str =
        "BattlePlan has unsaved changes. Save or discard them in BattlePlan, then restart to update.";
    pub const RELOADED: &'static str =
        "BattlePlan reloaded before it said whether its changes are saved. Try again.";
    pub const UNRESPONSIVE: &'static str =
        "BattlePlan isn’t responding, so it can’t say whether its changes are saved. Try again once it responds.";
    pub const UNHEARD: &'static str =
        "BattlePlan’s page isn’t listening yet, so it can’t say whether its changes are saved. Try again in a moment.";
    pub const UNREACHED: &'static str =
        "BattlePlan couldn’t pass the question to its page, so it can’t say whether its changes are saved. Try again in a moment.";
    pub const BLIND: &'static str =
        "BattlePlan can’t follow its page’s reloads and crashes, so it can’t say whether its changes are saved. Save your work, then close BattlePlan and update.";
    pub const CLOSING: &'static str = "BattlePlan is closing, so it doesn’t restart to update. Open BattlePlan again to update.";
    pub const LATE: &'static str = "BattlePlan didn’t say in time whether its changes are saved. Try again.";

    pub fn refusal(self) -> Option<&'static str> {
        match self {
            Self::Leaving => None,
            Self::Staying => Some(Self::STAYING),
            Self::Reloaded => Some(Self::RELOADED),
            Self::Unresponsive => Some(Self::UNRESPONSIVE),
            Self::Unheard => Some(Self::UNHEARD),
            Self::Unreached => Some(Self::UNREACHED),
            Self::Blind => Some(Self::BLIND),
            Self::Closing => Some(Self::CLOSING),
            Self::Saving => Some(replay_settings::exit::BUSY),
            Self::Late => Some(Self::LATE),
        }
    }
}

pub const REPLACED: &str = "This page was replaced before it could watch for BattlePlan closing.";
pub const UNWATCHED: &str = "This page no longer watches for BattlePlan closing, so it can’t ask again what stands.";
pub const GIVEN_UP: &str = "This restart was given up when its reply was lost. Try again.";
pub const CLOSE_LIMIT: Duration = Duration::from_secs(10);
pub const QUERY_LIMIT: Duration = Duration::from_secs(2);
pub const RELOADING: &str = "This page is reloading, so BattlePlan can’t tell it what stands until the reload loads or stops.";

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Unanswered {
    Close,
    Unresponsive,
    Unreached,
    Blind,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Closing {
    Through,
    Held,
    Offer(Unanswered),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Answered {
    Leave,
    Heard,
    Unknown,
}

pub type Reach = Arc<dyn Fn(Batch) + Send + Sync>;
pub type Stop = Arc<dyn Fn() + Send + Sync>;
pub type Lock = Arc<dyn Fn() + Send + Sync>;

enum Out {
    Batch(Reach, Batch),
    Stop(Stop),
    Lock(Lock),
}

pub struct Posed(u64);

struct Question {
    key: u64,
    about: About,
    asked: Option<u64>,
    verdict: Option<Verdict>,
    carriers: usize,
    reached: bool,
}

enum Page {
    Gone,
    Loading,
    Watched { seq: u64, reach: Reach },
}

#[derive(Default)]
struct Attempts {
    retired: u64,
    open: BTreeSet<(u64, u64)>,
}

struct State {
    page: Page,
    blind: bool,
    document: u64,
    navigations: BTreeSet<u64>,
    next: u64,
    questions: Vec<Question>,
    holds: Vec<Ask>,
    sent: Vec<(u64, Vec<u64>)>,
    outbox: Vec<Out>,
    offering: bool,
    hung: bool,
    launches: Vec<(u64, Receipt<'static>)>,
    attempts: Attempts,
    shutdown_query: Option<u64>,
    exit: &'static Exit,
    stop: Option<Stop>,
    lock: Option<Lock>,
}

impl State {
    fn fresh(&mut self) -> u64 {
        let id = self.next;
        self.next += 1;
        id
    }

    fn held(&self, id: u64) -> bool {
        self.holds.iter().any(|hold| hold.id == id)
    }

    fn exiting(&self) -> bool {
        self.exit.sealed()
    }

    fn hold(&mut self, ask: Ask) -> Result<(), Verdict> {
        match ask.about {
            About::Close => self.exit_as(ask.id),
            About::Install => {
                let receipt = self.exit.admit(What::Install).map_err(|_| if self.exiting() { Verdict::Closing } else { Verdict::Saving })?;
                self.holds.push(ask);
                self.launches.push((ask.id, receipt));
            }
            About::Leave => {
                self.holds.push(ask);
                self.exit();
            }
            About::Query => {}
        }
        Ok(())
    }

    fn askable(&self) -> bool {
        !self.blind && matches!(self.page, Page::Watched { .. }) && self.navigations.is_empty()
    }

    fn send(&mut self) -> Option<u64> {
        let Page::Watched { reach, .. } = &self.page else {
            return None;
        };
        if !self.navigations.is_empty() {
            return None;
        }
        let reach = Arc::clone(reach);
        if !self.askable() && self.holds.is_empty() {
            return None;
        }
        if self.askable() {
            for at in 0..self.questions.len() {
                if self.questions[at].verdict.is_none() && self.questions[at].asked.is_none() {
                    let id = self.fresh();
                    let question = &mut self.questions[at];
                    (question.asked, question.carriers, question.reached) = (Some(id), 0, false);
                }
            }
        }
        let seq = self.fresh();
        let mut waiting = Vec::new();
        for question in self.questions.iter_mut().filter(|question| question.verdict.is_none()) {
            if let Some(id) = question.asked {
                question.carriers += 1;
                waiting.push(Ask { id, about: question.about });
            }
        }
        self.sent.push((seq, waiting.iter().map(|ask| ask.id).collect()));
        self.outbox.push(Out::Batch(reach, Batch { seq, waiting, holds: self.holds.clone(), installing: !self.attempts.open.is_empty() }));
        Some(seq)
    }

    fn exit(&mut self) {
        if !self.exiting() {
            let id = self.fresh();
            self.exit_as(id);
        }
    }

    fn exit_as(&mut self, id: u64) {
        self.holds.push(Ask { id, about: About::Close });
        if !self.exit.seal() {
            return;
        }
        self.questions.retain(|question| question.about != About::Close);
        for question in &mut self.questions {
            if question.verdict.is_none() {
                question.verdict = Some(if question.about == About::Install { Verdict::Closing } else { Verdict::Leaving });
            }
        }
        if let Some(lock) = &self.lock {
            self.outbox.push(Out::Lock(Arc::clone(lock)));
        }
        if !self.navigations.is_empty()
            && let Some(stop) = &self.stop
        {
            self.outbox.push(Out::Stop(Arc::clone(stop)));
        }
        self.send();
    }

    fn leave_without_page(&mut self, at: usize) {
        let about = self.questions[at].about;
        let mut verdict = Verdict::Leaving;
        if about != About::Query {
            let id = self.fresh();
            if let Err(refused) = self.hold(Ask { id, about }) {
                verdict = refused;
            }
            self.questions[at].asked = Some(id);
        }
        self.questions[at].verdict = Some(verdict);
    }

    fn offer(&mut self, why: Unanswered) -> Closing {
        if std::mem::replace(&mut self.offering, true) { Closing::Held } else { Closing::Offer(why) }
    }
}

pub struct Leaving {
    state: Mutex<State>,
    answered: Condvar,
}

impl Default for Leaving {
    fn default() -> Self {
        Self::gated(&replay_settings::exit::EXIT)
    }
}

impl Leaving {
    pub fn gated(exit: &'static Exit) -> Self {
        Self {
            state: Mutex::new(State {
                page: Page::Loading,
                blind: false,
                document: 0,
                navigations: BTreeSet::new(),
                next: 0,
                questions: Vec::new(),
                holds: Vec::new(),
                sent: Vec::new(),
                outbox: Vec::new(),
                offering: false,
                hung: false,
                launches: Vec::new(),
                attempts: Attempts::default(),
                shutdown_query: None,
                exit,
                stop: None,
                lock: None,
            }),
            answered: Condvar::new(),
        }
    }
}

impl Leaving {
    fn lock(&self) -> MutexGuard<'_, State> {
        self.state.lock().unwrap_or_else(PoisonError::into_inner)
    }

    fn act<T>(&self, change: impl FnOnce(&mut State) -> T) -> T {
        let (value, outbox) = {
            let mut state = self.lock();
            let value = change(&mut state);
            (value, std::mem::take(&mut state.outbox))
        };
        self.dispatch(outbox);
        value
    }

    fn dispatch(&self, outbox: Vec<Out>) {
        self.answered.notify_all();
        for out in outbox {
            match out {
                Out::Batch(reach, batch) => reach(batch),
                Out::Stop(stop) => stop(),
                Out::Lock(lock) => lock(),
            }
        }
    }

    fn settle(state: &mut State, verdict: Verdict) {
        for question in &mut state.questions {
            if question.verdict.is_none() {
                question.asked = None;
                if question.about != About::Close {
                    question.verdict = Some(verdict);
                }
            }
        }
    }

    pub fn document(&self) -> u64 {
        self.lock().document
    }

    pub fn blind(&self) {
        self.act(|state| {
            state.blind = true;
            Self::settle(state, Verdict::Blind);
        });
    }

    pub fn follow(&self, stop: Stop, lock: Lock) {
        let mut state = self.lock();
        state.stop = Some(stop);
        state.lock = Some(lock);
    }

    pub fn watch(&self, document: u64, seq: u64, reach: impl FnOnce() -> Reach) -> Result<(), &'static str> {
        self.act(|state| {
            state.hung = false;
            if state.blind {
                return Err(Verdict::BLIND);
            }
            if document != state.document || matches!(state.page, Page::Gone) {
                return Err(REPLACED);
            }
            if let Page::Watched { seq: current, .. } = &state.page
                && *current >= seq
            {
                return Ok(());
            }
            state.page = Page::Watched { seq, reach: reach() };
            for question in &mut state.questions {
                if question.verdict.is_none() {
                    question.asked = None;
                }
            }
            state.sent.clear();
            state.send();
            Ok(())
        })
    }

    pub fn refresh(&self, document: u64, seq: u64, attempt: u64) -> Result<u64, &'static str> {
        self.act(|state| {
            if state.blind {
                return Err(Verdict::BLIND);
            }
            if document == state.document {
                state.attempts.retired = state.attempts.retired.max(attempt);
            }
            match state.page {
                Page::Watched { seq: current, .. } if current == seq && document == state.document => {
                    state.hung = false;
                    state.send().ok_or(RELOADING)
                }
                _ => Err(UNWATCHED),
            }
        })
    }

    pub fn delivered(&self, seq: u64, reached: bool) -> Option<Unanswered> {
        self.act(|state| {
            let at = state.sent.iter().position(|(sent, _)| *sent == seq)?;
            let (_, ids) = state.sent.remove(at);
            state.hung &= !reached;
            let mut offer = None;
            for id in ids {
                let Some(at) = state.questions.iter().position(|question| question.asked == Some(id) && question.verdict.is_none()) else {
                    continue;
                };
                let question = &mut state.questions[at];
                question.carriers = question.carriers.saturating_sub(1);
                question.reached |= reached;
                if question.reached || question.carriers > 0 {
                    continue;
                }
                if question.about == About::Close {
                    question.asked = None;
                    if let Closing::Offer(why) = state.offer(Unanswered::Unreached) {
                        offer = Some(why);
                    }
                } else {
                    question.verdict = Some(Verdict::Unreached);
                }
            }
            offer
        })
    }

    pub fn answer(&self, id: u64, stays: bool, basis: &[u64]) -> Answered {
        self.act(|state| {
            state.hung = false;
            let Some(at) = state.questions.iter().position(|question| question.asked == Some(id) && question.verdict.is_none()) else {
                return if !stays && state.held(id) { Answered::Heard } else { Answered::Unknown };
            };
            let about = state.questions[at].about;
            let leaves = !stays && about != About::Query;
            if leaves && !basis.iter().all(|&standing| state.held(standing)) {
                state.questions[at].asked = None;
                state.send();
                return Answered::Unknown;
            }
            if about == About::Close {
                state.questions.remove(at);
                if leaves {
                    let _ = state.hold(Ask { id, about });
                }
                return if stays { Answered::Heard } else { Answered::Leave };
            }
            state.questions[at].verdict = Some(if stays { Verdict::Staying } else { Verdict::Leaving });
            if leaves && let Err(refused) = state.hold(Ask { id, about }) {
                state.questions[at].verdict = Some(refused);
                state.send();
            }
            Answered::Heard
        })
    }

    pub fn close_requested(&self) -> Closing {
        self.act(|state| {
            state.hung = false;
            if state.exiting() {
                return Closing::Through;
            }
            if state.blind {
                return state.offer(Unanswered::Blind);
            }
            if matches!(state.page, Page::Gone) {
                state.exit();
                return Closing::Through;
            }
            let askable = state.askable();
            if let Some(close) = state.questions.iter().find(|question| question.about == About::Close) {
                if close.asked.is_some() || !askable {
                    return state.offer(Unanswered::Close);
                }
                state.send();
                return Closing::Held;
            }
            let key = state.fresh();
            state.questions.push(Question { key, about: About::Close, asked: None, verdict: None, carriers: 0, reached: false });
            state.send();
            Closing::Held
        })
    }

    pub fn chosen(&self) {
        self.lock().offering = false;
    }

    pub fn exit(&self) {
        self.act(State::exit);
    }

    pub fn navigation_starting(&self, navigation: u64) -> bool {
        self.act(|state| {
            if !state.holds.is_empty() {
                return false;
            }
            state.navigations.insert(navigation);
            Self::settle(state, Verdict::Reloaded);
            true
        })
    }

    pub fn content_loading(&self, navigation: u64) -> bool {
        self.act(|state| {
            state.navigations.remove(&navigation);
            state.document += 1;
            state.attempts.retired = 0;
            state.page = Page::Loading;
            state.hung = false;
            state.sent.clear();
            Self::settle(state, Verdict::Reloaded);
            !state.holds.is_empty()
        })
    }

    pub fn navigation_completed(&self, navigation: u64) {
        self.act(|state| {
            state.navigations.remove(&navigation);
            state.send();
        });
    }

    pub fn page_gone(&self) -> bool {
        self.act(|state| {
            if state.blind {
                return false;
            }
            state.page = Page::Gone;
            state.navigations.clear();
            state.hung = false;
            state.sent.clear();
            let closing = state.questions.iter().any(|question| question.about == About::Close);
            state.questions.retain(|question| question.about != About::Close);
            if closing {
                state.exit();
            }
            for at in 0..state.questions.len() {
                if state.questions[at].verdict.is_none() {
                    state.leave_without_page(at);
                }
            }
            closing
        })
    }

    pub fn page_unresponsive(&self) -> bool {
        self.act(|state| {
            for question in &mut state.questions {
                if question.about != About::Close && question.verdict.is_none() {
                    question.verdict = Some(Verdict::Unresponsive);
                }
            }
            !std::mem::replace(&mut state.hung, true) && !std::mem::replace(&mut state.offering, true)
        })
    }

    fn pose(&self, about: About) -> Result<Posed, Verdict> {
        self.act(|state| {
            if state.exiting() {
                return Err(if about == About::Install { Verdict::Closing } else { Verdict::Leaving });
            }
            if state.blind {
                return Err(Verdict::Blind);
            }
            if !state.navigations.is_empty() {
                return Err(Verdict::Reloaded);
            }
            let key = state.fresh();
            let question = Question { key, about, asked: None, verdict: None, carriers: 0, reached: false };
            match state.page {
                Page::Gone if about == About::Query => Err(Verdict::Leaving),
                Page::Gone => {
                    state.questions.push(question);
                    let at = state.questions.len() - 1;
                    state.leave_without_page(at);
                    Ok(Posed(key))
                }
                Page::Loading => Err(Verdict::Unheard),
                Page::Watched { .. } => {
                    state.questions.push(question);
                    state.send();
                    Ok(Posed(key))
                }
            }
        })
    }

    fn taken(state: &mut State, key: u64) -> Option<(Verdict, Option<u64>)> {
        let Some(at) = state.questions.iter().position(|question| question.key == key) else {
            return Some((Verdict::Unheard, None));
        };
        let verdict = state.questions[at].verdict?;
        let question = state.questions.remove(at);
        Some((verdict, question.asked))
    }

    fn wait(&self, posed: Result<Posed, Verdict>) -> (Verdict, Option<u64>) {
        self.wait_within(posed, None)
    }

    fn wait_within(&self, posed: Result<Posed, Verdict>, limit: Option<Duration>) -> (Verdict, Option<u64>) {
        let Posed(key) = match posed {
            Ok(posed) => posed,
            Err(verdict) => return (verdict, None),
        };
        let deadline = limit.and_then(|limit| std::time::Instant::now().checked_add(limit));
        let mut state = self.lock();
        loop {
            if let Some(taken) = Self::taken(&mut state, key) {
                return taken;
            }
            match deadline.map(|deadline| deadline.saturating_duration_since(std::time::Instant::now())) {
                None => state = self.answered.wait(state).unwrap_or_else(PoisonError::into_inner),
                Some(left) if left.is_zero() => {
                    state.questions.retain(|question| question.key != key);
                    state.send();
                    let outbox = std::mem::take(&mut state.outbox);
                    drop(state);
                    self.dispatch(outbox);
                    return (Verdict::Late, None);
                }
                Some(left) => state = self.answered.wait_timeout(state, left).unwrap_or_else(PoisonError::into_inner).0,
            }
        }
    }

    fn release(state: &mut State, id: u64) {
        state.launches.retain(|(held, _)| *held != id);
        let Some(at) = state.holds.iter().position(|hold| hold.id == id && hold.about == About::Install) else {
            return;
        };
        state.holds.remove(at);
        state.send();
    }

    #[cfg(test)]
    fn stay(&self, id: u64) {
        self.act(|state| Self::release(state, id));
    }

    fn leave_after(&self, posed: Result<Posed, Verdict>, launch: impl FnOnce() -> Result<(), String>) -> Result<(), String> {
        struct Launching<'a> {
            leaving: &'a Leaving,
            id: u64,
            launched: bool,
            _receipt: Receipt<'static>,
        }
        impl Drop for Launching<'_> {
            fn drop(&mut self) {
                let (id, launched) = (self.id, self.launched);
                self.leaving.act(|state| if launched { state.exit() } else { Leaving::release(state, id) });
            }
        }
        let (verdict, id) = self.wait(posed);
        if let Some(refused) = verdict.refusal() {
            return Err(refused.to_owned());
        }
        let taken = id.and_then(|id| {
            let mut state = self.lock();
            let at = state.launches.iter().position(|(held, _)| *held == id)?;
            Some(state.launches.remove(at))
        });
        let Some((id, receipt)) = taken else {
            return Err(Verdict::CLOSING.to_owned());
        };
        let mut launching = Launching { leaving: self, id, launched: false, _receipt: receipt };
        let launched = launch();
        launching.launched = launched.is_ok();
        launched
    }

    pub fn attempt(&self, document: u64, attempt: u64) -> Result<(), &'static str> {
        self.act(|state| {
            if document != state.document {
                return Err(REPLACED);
            }
            if attempt <= state.attempts.retired {
                return Err(GIVEN_UP);
            }
            state.attempts.open.insert((document, attempt));
            Ok(())
        })
    }

    pub fn attempt_ended(&self, document: u64, attempt: u64) {
        self.act(|state| {
            state.attempts.open.remove(&(document, attempt));
            state.send();
        });
    }

    pub fn shutdown_query(&self) -> Option<bool> {
        self.retire_shutdown_query();
        let posed = self.pose(About::Query);
        if let Ok(Posed(key)) = &posed {
            self.lock().shutdown_query = Some(*key);
        }
        match self.wait_within(posed, Some(QUERY_LIMIT)).0 {
            Verdict::Leaving => Some(false),
            Verdict::Staying => Some(true),
            _ => None,
        }
    }

    pub fn retire_shutdown_query(&self) {
        self.act(|state| {
            if let Some(key) = state.shutdown_query.take() {
                state.questions.retain(|question| question.key != key);
            }
        });
    }

    pub fn install(&self, launch: impl FnOnce() -> Result<(), String>) -> Result<(), String> {
        self.leave_after(self.pose(About::Install), launch)
    }

    pub fn close_for_update(&self) -> Result<(), String> {
        self.wait_within(self.pose(About::Leave), Some(CLOSE_LIMIT)).0.refusal().map_or(Ok(()), |refused| Err(refused.to_owned()))
    }

    pub fn busy(&self) -> Option<String> {
        self.wait_within(self.pose(About::Query), Some(QUERY_LIMIT)).0.refusal().map(str::to_owned)
    }
}

#[cfg(test)]
mod tests {
    use super::{About, Answered, Ask, Batch, Closing, Leaving, RELOADING, REPLACED, Reach, UNWATCHED, Unanswered, Verdict};
    use replay_settings::exit::{Exit, What};
    use std::sync::{
        Arc, Mutex,
        mpsc::{Receiver, Sender, channel},
    };
    use std::time::Duration;

    fn gate() -> &'static Exit {
        Box::leak(Box::new(Exit::new()))
    }

    impl Leaving {
        fn launching(&self) -> usize {
            self.lock().exit.count(What::Install)
        }
    }

    struct Page {
        sent: Sender<(u32, Batch)>,
        heard: Mutex<Receiver<(u32, Batch)>>,
    }

    impl Default for Page {
        fn default() -> Self {
            let (sent, heard) = channel();
            Self { sent, heard: Mutex::new(heard) }
        }
    }

    impl Page {
        fn reach(&self, callback: u32) -> impl FnOnce() -> Reach {
            let sent = self.sent.clone();
            move || {
                Arc::new(move |batch: Batch| {
                    sent.send((callback, batch)).unwrap();
                })
            }
        }

        fn dispatched(&self) -> (u32, Batch) {
            self.heard.lock().unwrap().try_recv().expect("the page was sent nothing")
        }

        fn awaited(&self) -> (u32, Batch) {
            self.heard.lock().unwrap().recv().unwrap()
        }

        fn quiet(&self) -> bool {
            self.heard.lock().unwrap().try_recv().is_err()
        }

        fn hears(&self, leaving: &Leaving) -> (u32, Batch) {
            let (callback, batch) = self.dispatched();
            assert_eq!(leaving.delivered(batch.seq, true), None);
            (callback, batch)
        }

        fn misses(&self, leaving: &Leaving) -> (Batch, Option<Unanswered>) {
            let (_, batch) = self.dispatched();
            let offer = leaving.delivered(batch.seq, false);
            (batch, offer)
        }
    }

    fn asked(batch: &Batch) -> (u64, About) {
        match batch.waiting[..] {
            [Ask { id, about }] => (id, about),
            ref other => panic!("expected one question waiting, got {other:?}"),
        }
    }

    fn waiting(batch: &Batch, about: About) -> u64 {
        match batch.waiting.iter().filter(|ask| ask.about == about).collect::<Vec<_>>()[..] {
            [ask] => ask.id,
            ref other => panic!("expected one {about:?} waiting, got {other:?}"),
        }
    }

    fn heard(leaving: &Leaving, page: &Page) -> (u32, u64, About) {
        let (callback, batch) = page.hears(leaving);
        let (id, about) = asked(&batch);
        (callback, id, about)
    }

    fn watched(leaving: &Leaving, page: &Page, callback: u32) {
        assert_eq!(leaving.watch(leaving.document(), 1, page.reach(callback)), Ok(()));
        let (_, batch) = page.hears(leaving);
        assert!(batch.waiting.is_empty() && batch.holds.is_empty(), "a new subscription hears what stands: {batch:?}");
    }

    #[test]
    fn a_registration_sent_again_after_a_lost_reply_keeps_the_subscription_it_made() {
        let page = Page::default();
        let leaving = Leaving::gated(gate());
        watched(&leaving, &page, 7);
        let document = leaving.document();
        assert_eq!(leaving.watch(document, 1, || unreachable!("a registration sent again replaces nothing")), Ok(()));
        assert_eq!(leaving.watch(document, 0, || unreachable!("an older registration arriving late replaces nothing")), Ok(()));
        assert!(page.quiet());
        assert_eq!(leaving.close_requested(), Closing::Held);
        assert_eq!(heard(&leaving, &page).0, 7);
    }

    #[test]
    fn a_registration_from_the_page_a_reload_replaced_is_refused_so_the_new_page_subscribes() {
        let page = Page::default();
        let leaving = Leaving::gated(gate());
        assert!(!leaving.content_loading(1));
        let outgoing = leaving.document();
        assert!(!leaving.content_loading(2));
        let current = leaving.document();
        assert_ne!(outgoing, current);
        assert_eq!(
            leaving.watch(outgoing, 3, || unreachable!("the outgoing page’s late registration subscribes nothing")),
            Err(REPLACED)
        );
        watched(&leaving, &page, 8);
        assert_eq!(leaving.close_requested(), Closing::Held);
        assert_eq!(heard(&leaving, &page).0, 8, "its closes are asked of the new page");
    }

    #[test]
    fn a_superseded_registrations_late_answer_does_not_settle_the_question_it_was_asked() {
        let page = Page::default();
        let leaving = Leaving::gated(gate());
        watched(&leaving, &page, 7);
        let asking = leaving.pose(About::Query);
        let (_, first, _) = heard(&leaving, &page);
        assert_eq!(leaving.watch(leaving.document(), 2, page.reach(8)), Ok(()));
        let (callback, batch) = page.hears(&leaving);
        let (second, _) = asked(&batch);
        assert_eq!(callback, 8, "the question is asked again of the newer registration");
        assert_ne!(second, first, "with a fresh id, and the old one no longer stands");
        assert_eq!(leaving.answer(first, false, &[]), Answered::Unknown, "the superseded page’s in-flight Saved doesn’t count");
        assert_eq!(leaving.answer(second, true, &[]), Answered::Heard);
        assert_eq!(leaving.wait(asking), (Verdict::Staying, Some(second)));
    }

    #[test]
    fn a_leave_answer_sent_again_after_a_lost_reply_stands_until_the_host_says_stay() {
        let page = Page::default();
        let leaving = Leaving::gated(gate());
        watched(&leaving, &page, 7);
        let asking = leaving.pose(About::Install);
        let (_, id, _) = heard(&leaving, &page);
        assert_eq!(leaving.answer(id, false, &[]), Answered::Heard);
        let (verdict, answered) = leaving.wait(asking);
        assert_eq!((verdict, answered), (Verdict::Leaving, Some(id)));
        assert_eq!(leaving.answer(id, false, &[]), Answered::Heard, "the retry keeps the page locked while the install starts");
        assert_eq!(leaving.answer(id, true, &[]), Answered::Unknown, "an agreed leave can’t be taken back");

        leaving.stay(id);
        let (_, stayed) = page.hears(&leaving);
        assert!(stayed.holds.is_empty() && stayed.waiting.is_empty(), "the page is told the leave no longer stands: {stayed:?}");
        assert_eq!(leaving.answer(id, false, &[]), Answered::Unknown, "after Stay the page goes on");

        assert_eq!(leaving.close_requested(), Closing::Held);
        let (_, close, _) = heard(&leaving, &page);
        assert_eq!(leaving.answer(close, false, &[]), Answered::Leave);
        assert_eq!(leaving.answer(close, false, &[]), Answered::Heard, "a close the host accepted is still accepted, without leaving twice");
    }

    #[test]
    fn a_question_the_page_never_runs_is_refused_by_name_and_wakes_setup() {
        let page = Page::default();
        let leaving = Leaving::gated(gate());
        watched(&leaving, &page, 7);
        std::thread::scope(|scope| {
            let setup = scope.spawn(|| leaving.close_for_update());
            let (_, batch) = page.awaited();
            assert_eq!(leaving.delivered(batch.seq, false), None, "WebView2 took the script and didn’t run it");
            assert_eq!(setup.join().unwrap(), Err(Verdict::UNREACHED.to_owned()), "setup is refused, never left waiting");
        });
        let asking = leaving.pose(About::Query);
        page.misses(&leaving);
        assert_eq!(leaving.wait(asking).0.refusal(), Some(Verdict::UNREACHED));

        assert_eq!(leaving.close_requested(), Closing::Held);
        assert_eq!(page.misses(&leaving).1, Some(Unanswered::Unreached), "the user chooses instead");
        leaving.chosen();
        assert_eq!(leaving.close_requested(), Closing::Held, "the next close asks the page again");
        let (_, close, about) = heard(&leaving, &page);
        assert_eq!(about, About::Close);
        assert_eq!(leaving.answer(close, true, &[]), Answered::Heard);
        assert!(page.quiet());
    }

    #[test]
    fn a_receipt_for_a_question_asked_again_settles_nothing() {
        let page = Page::default();
        let leaving = Leaving::gated(gate());
        watched(&leaving, &page, 7);
        let asking = leaving.pose(About::Query);
        let (_, stale) = page.dispatched();
        assert_eq!(leaving.watch(leaving.document(), 2, page.reach(8)), Ok(()));
        let (_, id, _) = heard(&leaving, &page);
        assert_eq!(leaving.delivered(stale.seq, false), None);
        assert_eq!(leaving.delivered(stale.seq, false), None, "a receipt counts once");
        assert_eq!(leaving.answer(id, false, &[]), Answered::Heard, "the question the newer page heard still stands");
        assert_eq!(leaving.wait(asking).0, Verdict::Leaving);
    }

    #[test]
    fn a_later_batch_carries_every_question_an_earlier_one_asked_so_the_order_they_run_in_loses_nothing() {
        let page = Page::default();
        let leaving = Leaving::gated(gate());
        watched(&leaving, &page, 7);
        let asking = leaving.pose(About::Query);
        let (_, earlier) = page.dispatched();
        assert_eq!(leaving.close_requested(), Closing::Held);
        let (_, later) = page.dispatched();
        assert!(later.seq > earlier.seq, "the page applies only a batch newer than the last it applied");
        let query = waiting(&later, About::Query);
        assert_eq!(asked(&earlier), (query, About::Query), "the later batch carries the question the earlier one asked, by the same id");
        assert_eq!(leaving.delivered(later.seq, true), None);
        assert_eq!(leaving.delivered(earlier.seq, false), None, "the earlier batch is superseded, not lost");
        assert_eq!(leaving.answer(query, false, &[]), Answered::Heard);
        assert_eq!(leaving.wait(asking).0, Verdict::Leaving);
    }

    #[test]
    fn a_dispatch_refused_at_once_drops_its_receipt_without_the_broker_lock() {
        let leaving = Arc::new(Leaving::gated(gate()));
        let reports = Arc::new(Mutex::new(Vec::new()));
        let weak = Arc::downgrade(&leaving);
        let reported = Arc::clone(&reports);
        let refuse = move || -> Reach {
            Arc::new(move |batch: Batch| {
                let leaving = weak.upgrade().expect("the broker outlives its dispatches");
                assert!(leaving.state.try_lock().is_ok(), "a dispatch ran under the broker lock, so its dropped receipt would deadlock");
                reported.lock().unwrap().push(leaving.delivered(batch.seq, false));
            })
        };
        assert_eq!(leaving.watch(leaving.document(), 1, refuse), Ok(()));
        assert_eq!(leaving.close_requested(), Closing::Held);
        assert_eq!(*reports.lock().unwrap(), [None, Some(Unanswered::Unreached)], "the close the page never heard offers the choice");
        assert_eq!(leaving.busy(), Some(Verdict::UNREACHED.to_owned()), "setup is refused by name, never left waiting");
    }

    #[test]
    fn a_close_held_while_the_page_starts_offers_the_choice_when_its_registration_cant_be_reached() {
        let page = Page::default();
        let leaving = Leaving::gated(gate());
        assert_eq!(leaving.close_requested(), Closing::Held, "a page that hasn’t subscribed yet may still hold work");
        assert_eq!(leaving.watch(leaving.document(), 1, page.reach(7)), Ok(()));
        let (batch, offer) = page.misses(&leaving);
        assert_eq!(asked(&batch).1, About::Close);
        assert_eq!(offer, Some(Unanswered::Unreached), "the held close isn’t left waiting on a page that never heard it");
    }

    #[test]
    fn every_batch_says_which_leaves_still_stand_so_a_stay_that_never_ran_is_carried_by_the_next() {
        let page = Page::default();
        let leaving = Leaving::gated(gate());
        watched(&leaving, &page, 7);
        let asking = leaving.pose(About::Install);
        let (_, id, _) = heard(&leaving, &page);
        leaving.answer(id, false, &[]);
        let failed = "The installer couldn’t be started: Windows refused to open it (code 2).";
        assert_eq!(leaving.leave_after(asking, || Err(failed.into())), Err(failed.into()));
        let (stayed, _) = page.misses(&leaving);
        assert!(stayed.holds.is_empty());

        assert_eq!(leaving.close_requested(), Closing::Held);
        let (_, batch) = page.hears(&leaving);
        let (close, _) = asked(&batch);
        assert_eq!((batch.waiting, batch.holds), (vec![Ask { id: close, about: About::Close }], vec![]), "the close carries the stay that never ran");
    }

    #[test]
    fn a_stay_tells_the_page_and_no_answer_judged_under_the_leave_closes() {
        let page = Page::default();
        let leaving = Leaving::gated(gate());
        watched(&leaving, &page, 7);
        let asking = leaving.pose(About::Install);
        let (_, install, _) = heard(&leaving, &page);
        leaving.answer(install, false, &[]);
        assert_eq!(leaving.wait(asking).0, Verdict::Leaving);
        assert_eq!(leaving.close_requested(), Closing::Held);
        let (_, batch) = page.hears(&leaving);
        let (committed, _) = asked(&batch);
        assert_eq!(batch.holds, [Ask { id: install, about: About::Install }]);

        leaving.stay(install);
        let (_, batch) = page.hears(&leaving);
        assert_eq!((batch.waiting, batch.holds), (vec![Ask { id: committed, about: About::Close }], vec![]), "the page is told the install no longer stands");
        assert_eq!(leaving.answer(committed, false, &[install]), Answered::Unknown, "a close judged under the install doesn’t close");
        let (_, batch) = page.hears(&leaving);
        let (close, about) = asked(&batch);
        assert_eq!(about, About::Close, "it is asked again, under nothing");
        assert_ne!(close, committed);
        assert_eq!(leaving.answer(close, true, &[]), Answered::Heard, "the page judges it again after the Stay");
    }

    #[test]
    fn a_leave_judged_under_a_commitment_the_host_has_since_refused_is_asked_again() {
        let page = Page::default();
        let leaving = Leaving::gated(gate());
        watched(&leaving, &page, 7);
        let asking = leaving.pose(About::Install);
        let (_, install, _) = heard(&leaving, &page);
        assert_eq!(leaving.close_requested(), Closing::Held);
        let (_, batch) = page.dispatched();
        let close = waiting(&batch, About::Close);
        assert_eq!(waiting(&batch, About::Install), install, "the close was sent while the install still stood");
        assert!(leaving.page_unresponsive());
        assert_eq!(leaving.wait(asking).0, Verdict::Unresponsive);
        assert_eq!(leaving.delivered(batch.seq, true), None);
        assert_eq!(leaving.answer(close, false, &[install]), Answered::Unknown, "the page judged the close under the refused install");
        let (_, batch) = page.hears(&leaving);
        let (again, about) = asked(&batch);
        assert_eq!(about, About::Close, "it is asked again without the install");
        assert_ne!(again, close);
        assert_eq!(leaving.answer(again, true, &[]), Answered::Heard);
    }

    #[test]
    fn a_leave_resting_on_a_question_the_host_only_waits_on_is_asked_again() {
        let page = Page::default();
        let leaving = Leaving::gated(gate());
        watched(&leaving, &page, 7);
        let restart = leaving.pose(About::Install);
        let (_, install, _) = heard(&leaving, &page);
        assert_eq!(leaving.answer(install, false, &[]), Answered::Heard, "Discard and restart");
        assert_eq!(leaving.close_requested(), Closing::Held);
        let (_, batch) = page.hears(&leaving);
        let close = waiting(&batch, About::Close);
        let failed = "The installer couldn’t be started: Windows refused to open it (code 2).";
        assert_eq!(leaving.leave_after(restart, || Err(failed.into())), Err(failed.into()));
        let (_, batch) = page.hears(&leaving);
        assert_eq!((batch.waiting, batch.holds), (vec![Ask { id: close, about: About::Close }], vec![]), "the close the page agreed to under the restart never arrived");

        let setup = leaving.pose(About::Leave);
        let (_, batch) = page.hears(&leaving);
        let leave = waiting(&batch, About::Leave);
        assert_eq!(leaving.answer(leave, false, &[close]), Answered::Unknown, "a close the host only waits on is no consent to rest on");
        let (_, batch) = page.hears(&leaving);
        let again = waiting(&batch, About::Leave);
        assert_ne!(again, leave, "setup’s close is asked again");
        assert_eq!(leaving.answer(close, false, &[install]), Answered::Unknown, "the lost close, sent again, rests on the failed restart");
        let (_, batch) = page.hears(&leaving);
        let close = waiting(&batch, About::Close);
        assert_eq!(leaving.answer(again, true, &[]), Answered::Heard);
        assert_eq!(leaving.wait(setup).0.refusal(), Some(Verdict::STAYING), "setup hears of the unsaved work");
        assert_eq!(leaving.answer(close, true, &[]), Answered::Heard);
    }

    #[test]
    fn a_close_is_asked_of_the_page_and_only_its_leave_answer_closes() {
        let page = Page::default();
        let leaving = Leaving::gated(gate());
        watched(&leaving, &page, 7);
        assert_eq!(leaving.close_requested(), Closing::Held);
        let (_, unsaved, _) = heard(&leaving, &page);
        assert_eq!(leaving.answer(unsaved, true, &[]), Answered::Heard, "unsaved work keeps the window, and the page prompts");
        assert_eq!(leaving.answer(unsaved, false, &[]), Answered::Unknown, "an answered close can’t be answered again");
        assert_eq!(leaving.close_requested(), Closing::Held);
        let (_, saved, _) = heard(&leaving, &page);
        assert_eq!(leaving.answer(saved, false, &[]), Answered::Leave);
        assert_eq!(leaving.close_requested(), Closing::Through, "once leaving, closes go through");
    }

    #[test]
    fn a_page_that_started_but_has_not_subscribed_holds_closes_and_refuses_update_questions_by_name() {
        let page = Page::default();
        let leaving = Leaving::gated(gate());
        assert!(!leaving.content_loading(1));
        assert_eq!(leaving.close_for_update(), Err(Verdict::UNHEARD.to_owned()));
        assert_eq!(leaving.busy(), Some(Verdict::UNHEARD.to_owned()));
        assert_eq!(leaving.close_requested(), Closing::Held, "a page that never subscribes can’t lose work to a close");
        assert_eq!(leaving.close_requested(), Closing::Offer(Unanswered::Close), "the user chooses instead");
        assert_eq!(leaving.close_requested(), Closing::Held, "one choice at a time");
        leaving.chosen();
        assert_eq!(leaving.watch(leaving.document(), 1, page.reach(7)), Ok(()));
        let (_, close, about) = heard(&leaving, &page);
        assert_eq!(about, About::Close, "the held close is asked once the page subscribes");
        assert_eq!(leaving.answer(close, false, &[]), Answered::Leave);
    }

    #[test]
    fn a_close_the_page_leaves_unanswered_offers_the_choice_at_the_next_close() {
        let page = Page::default();
        let leaving = Leaving::gated(gate());
        watched(&leaving, &page, 7);
        assert_eq!(leaving.close_requested(), Closing::Held);
        let (_, close, _) = heard(&leaving, &page);
        assert_eq!(leaving.close_requested(), Closing::Offer(Unanswered::Close));
        assert!(page.quiet(), "the page isn’t asked twice");
        leaving.chosen();
        assert_eq!(leaving.close_requested(), Closing::Offer(Unanswered::Close), "keeping waiting lets the next close ask again");
        leaving.chosen();
        assert_eq!(leaving.answer(close, true, &[]), Answered::Heard);
        assert_eq!(leaving.close_requested(), Closing::Held, "an answered close starts over");
    }

    #[test]
    fn an_unresponsive_page_refuses_the_questions_waiting_on_it_by_name_and_offers_one_choice() {
        let page = Page::default();
        let leaving = Leaving::gated(gate());
        watched(&leaving, &page, 7);
        let leave = leaving.pose(About::Install);
        let query = leaving.pose(About::Query);
        assert_eq!(leaving.close_requested(), Closing::Held);
        page.hears(&leaving);
        page.hears(&leaving);
        let (_, batch) = page.hears(&leaving);
        let (install, close) = (waiting(&batch, About::Install), waiting(&batch, About::Close));
        assert!(leaving.page_unresponsive(), "the user is offered to close");
        assert_eq!(leaving.leave_after(leave, || unreachable!("nothing launches")), Err(Verdict::UNRESPONSIVE.to_owned()));
        assert_eq!(leaving.wait(query).0.refusal(), Some(Verdict::UNRESPONSIVE));
        assert!(!leaving.page_unresponsive(), "WebView2 reports again while the first choice is open");
        leaving.chosen();
        assert!(!leaving.page_unresponsive(), "No keeps waiting through this hang, however often WebView2 reports it");
        assert_eq!(leaving.answer(install, false, &[]), Answered::Unknown, "a refused update can’t be agreed to later");
        assert_eq!(leaving.answer(close, false, &[]), Answered::Leave, "the close stays for a page that recovers");
        assert!(matches!(page.hears(&leaving).1.holds[..], [Ask { about: About::Close, .. }]), "the page hears that BattlePlan leaves");
        assert!(leaving.page_unresponsive(), "a page that answered has responded, so a new hang offers the choice again");

        let leaving = Leaving::gated(gate());
        watched(&leaving, &page, 7);
        assert!(leaving.page_unresponsive());
        leaving.chosen();
        assert_eq!(leaving.close_requested(), Closing::Held, "closing again asks the page");
        assert!(leaving.page_unresponsive(), "and the hang it meets offers the choice again");
    }

    #[test]
    fn an_install_the_page_agreed_to_stands_until_it_starts_or_fails() {
        let page = Page::default();
        let leaving = Leaving::gated(gate());
        watched(&leaving, &page, 7);
        let asking = leaving.pose(About::Install);
        let (_, id, about) = heard(&leaving, &page);
        assert_eq!(about, About::Install);
        assert_eq!(leaving.answer(id, false, &[]), Answered::Heard);
        let failed = "The installer couldn’t be started: Windows refused to open it (code 2).";
        assert_eq!(leaving.leave_after(asking, || Err(failed.into())), Err(failed.into()));
        assert!(page.hears(&leaving).1.holds.is_empty());
        assert!(leaving.navigation_starting(1), "a page that stays may reload");
        leaving.navigation_completed(1);
        page.hears(&leaving);

        let asking = leaving.pose(About::Install);
        let (_, id, _) = heard(&leaving, &page);
        leaving.answer(id, false, &[]);
        std::thread::scope(|scope| {
            let (launching, launch) = channel::<Result<(), String>>();
            let install = scope.spawn(|| leaving.leave_after(asking, move || launch.recv().unwrap()));
            assert!(!leaving.navigation_starting(2), "F5 can’t unlock the page while its installer starts");
            leaving.page_gone();
            assert!(!leaving.navigation_starting(3), "nor once the page’s process has ended");
            launching.send(Ok(())).unwrap();
            assert_eq!(install.join().unwrap(), Ok(()));
        });
        assert!(!leaving.navigation_starting(4), "nor while BattlePlan exits");
        assert_eq!(leaving.close_requested(), Closing::Through);

        let leaving = Leaving::gated(gate());
        watched(&leaving, &page, 7);
        let asking = leaving.pose(About::Install);
        let (_, id, _) = heard(&leaving, &page);
        leaving.answer(id, true, &[]);
        assert_eq!(leaving.leave_after(asking, || unreachable!("unsaved work launches nothing")), Err(Verdict::STAYING.to_owned()));
    }

    #[test]
    fn a_close_the_page_agrees_to_while_its_installer_starts_leaves_only_once_the_launch_ends() {
        let page = Page::default();
        let leaving = Leaving::gated(gate());
        watched(&leaving, &page, 7);
        let asking = leaving.pose(About::Install);
        let (_, id, _) = heard(&leaving, &page);
        leaving.answer(id, false, &[]);
        std::thread::scope(|scope| {
            let (started, begun) = channel();
            let (launching, launch) = channel::<Result<(), String>>();
            let install = scope.spawn(|| {
                leaving.leave_after(asking, move || {
                    started.send(()).unwrap();
                    launch.recv().unwrap()
                })
            });
            begun.recv().unwrap();
            assert_eq!(leaving.close_requested(), Closing::Held);
            let (_, close, _) = heard(&leaving, &page);
            assert_eq!(leaving.answer(close, false, &[]), Answered::Leave, "the locked page agrees to Alt+F4");
            assert_eq!(leaving.launching(), 1, "so the exit waits for the installer still being written and opened");
            launching.send(Err("The installer couldn’t be started.".into())).unwrap();
            assert!(install.join().unwrap().is_err());
        });
        assert_eq!(leaving.launching(), 0, "a launch that fails ends the wait too");
    }

    #[test]
    fn an_install_asked_for_once_battleplan_closes_never_launches() {
        let page = Page::default();
        let leaving = Leaving::gated(gate());
        watched(&leaving, &page, 7);
        leaving.exit();
        page.hears(&leaving);
        assert_eq!(leaving.install(|| unreachable!("no installer starts once BattlePlan closes")), Err(Verdict::CLOSING.to_owned()));
        assert_eq!(leaving.launching(), 0, "and nothing is reserved, so the exit waits for nothing");
    }

    #[test]
    fn closing_while_the_page_judges_an_install_refuses_it_by_name_and_launches_nothing() {
        let page = Page::default();
        let leaving = Leaving::gated(gate());
        watched(&leaving, &page, 7);
        let asking = leaving.pose(About::Install);
        let (_, install, _) = heard(&leaving, &page);
        leaving.exit();
        let (_, batch) = page.hears(&leaving);
        assert!(batch.waiting.is_empty(), "the install is no longer asked: {batch:?}");
        assert_eq!(leaving.answer(install, false, &[]), Answered::Unknown, "the page’s late agreement takes nothing the exit refused");
        assert_eq!(leaving.leave_after(asking, || unreachable!("an install agreed to after the exit launches nothing")), Err(Verdict::CLOSING.to_owned()));
        assert_eq!(leaving.launching(), 0);

        let leaving = Leaving::gated(gate());
        watched(&leaving, &page, 7);
        let asking = leaving.pose(About::Install);
        let (_, install, _) = heard(&leaving, &page);
        leaving.exit();
        page.hears(&leaving);
        assert!(!leaving.page_gone(), "a page that ends after the exit agrees to nothing more");
        assert_eq!(leaving.leave_after(asking, || unreachable!("nothing launches")), Err(Verdict::CLOSING.to_owned()));
        assert_eq!(leaving.answer(install, false, &[]), Answered::Unknown);
        assert_eq!(leaving.launching(), 0);
    }

    #[test]
    fn an_install_the_host_accepts_is_reserved_at_once_so_an_exit_before_its_launch_waits_for_it() {
        let page = Page::default();
        let leaving = Leaving::gated(gate());
        watched(&leaving, &page, 7);
        let asking = leaving.pose(About::Install);
        let (_, install, _) = heard(&leaving, &page);
        assert_eq!(leaving.answer(install, false, &[]), Answered::Heard);
        assert_eq!(leaving.launching(), 1, "the launch is reserved under the lock that accepted it, before its waiter wakes");
        leaving.exit();
        assert_eq!(leaving.launching(), 1, "an exit committed before the launch began still waits for it");
        std::thread::scope(|scope| {
            let coordinator = leaving.lock().exit;
            let exit = scope.spawn(move || coordinator.empty_within(Duration::MAX));
            assert_eq!(leaving.leave_after(asking, || Ok(())), Ok(()));
            assert!(exit.join().unwrap());
        });
        assert_eq!(leaving.launching(), 0);
    }

    fn committed(leaving: &Leaving, writes: &Exit, how: &str) {
        assert!(leaving.lock().exiting(), "{how}: BattlePlan committed to exit");
        assert_eq!(writes.admit(What::Change).err().map(|error| error.to_string()), Some(replay_settings::exit::CLOSED.to_owned()), "{how}: no write is admitted once it has");
        assert_eq!(leaving.install(|| unreachable!("{how}: no installer starts once BattlePlan exits")), Err(Verdict::CLOSING.to_owned()), "{how}");
        assert_eq!(leaving.close_requested(), Closing::Through, "{how}");
    }

    #[test]
    fn every_way_battleplan_leaves_commits_once_closing_writes_and_refusing_the_install_it_was_judging() {
        type Path = fn(&Leaving, &Page) -> bool;
        let paths: [(&str, Path); 5] = [
            ("the page agrees to close", |leaving, page| {
                assert_eq!(leaving.close_requested(), Closing::Held);
                let (_, batch) = page.hears(leaving);
                leaving.answer(waiting(&batch, About::Close), false, &[]) == Answered::Leave
            }),
            ("Discard and close, the native choice or setup’s close", |leaving, _| {
                leaving.exit();
                true
            }),
            ("Alt+F4 once the page has gone", |leaving, _| {
                assert!(!leaving.page_gone());
                leaving.close_requested() == Closing::Through
            }),
            ("the page ends while a close waits", |leaving, page| {
                assert_eq!(leaving.close_requested(), Closing::Held);
                page.hears(leaving);
                leaving.page_gone()
            }),
            ("an installer starts", |leaving, _| {
                leaving.act(super::State::exit);
                true
            }),
        ];
        for (how, leave) in paths {
            let page = Page::default();
            let writes = gate();
            let leaving = Leaving::gated(writes);
            watched(&leaving, &page, 7);
            let asking = leaving.pose(About::Install);
            let (_, install, _) = heard(&leaving, &page);
            assert!(leave(&leaving, &page), "{how}");
            committed(&leaving, writes, how);
            if how.starts_with("Alt+F4") {
                assert_eq!(leaving.wait(asking).0, Verdict::Leaving, "{how}: the install a dead page held was agreed to before the exit, so the exit waits for it");
                continue;
            }
            assert_eq!(leaving.answer(install, false, &[]), Answered::Unknown, "{how}: the page’s late agreement takes nothing");
            assert_eq!(leaving.leave_after(asking, || unreachable!("{how}: the install being judged never launches")), Err(Verdict::CLOSING.to_owned()), "{how}");
            assert_eq!(leaving.launching(), 0, "{how}: and the exit waits for no launch");
        }
    }

    #[test]
    fn leaving_by_choice_refuses_reloads_while_the_maps_shut_down() {
        let page = Page::default();
        let leaving = Leaving::gated(gate());
        watched(&leaving, &page, 7);
        leaving.exit();
        let (_, batch) = page.hears(&leaving);
        assert!(matches!(batch.holds[..], [Ask { about: About::Close, .. }]), "the page is told it is leaving: {batch:?}");
        assert!(!leaving.navigation_starting(1));
        assert_eq!(leaving.close_requested(), Closing::Through, "the ordered exit holds the window");
        assert_eq!(leaving.busy(), None);
        assert_eq!(leaving.close_for_update(), Ok(()));
    }

    #[test]
    fn a_reload_already_past_its_veto_when_battleplan_leaves_is_stopped_so_the_page_it_leaves_hears_the_hold() {
        let page = Page::default();
        let leaving = Leaving::gated(gate());
        let stops = Arc::new(Mutex::new(0));
        let stopped = Arc::clone(&stops);
        leaving.follow(Arc::new(move || *stopped.lock().unwrap() += 1), Arc::new(|| {}));
        watched(&leaving, &page, 7);
        assert!(leaving.navigation_starting(1), "a slow reload starts while nothing stands");
        assert_eq!(leaving.close_requested(), Closing::Held);
        assert_eq!(leaving.close_requested(), Closing::Offer(Unanswered::Close));
        leaving.chosen();
        leaving.exit();
        assert_eq!(*stops.lock().unwrap(), 1, "the reload is stopped, so the page it would replace stays");
        assert!(page.quiet(), "WebView2 runs a script sent after NavigationStarting in the incoming document, so nothing is sent while a navigation is pending");
        leaving.navigation_completed(1);
        let (_, batch) = page.hears(&leaving);
        assert!(matches!(batch.holds[..], [Ask { about: About::Close, .. }]), "the page the reload would have replaced is locked: {batch:?}");
        assert_eq!(leaving.close_requested(), Closing::Through);

        let leaving = Leaving::gated(gate());
        leaving.follow(Arc::new(|| {}), Arc::new(|| {}));
        watched(&leaving, &page, 7);
        assert!(leaving.navigation_starting(2));
        leaving.exit();
        assert!(page.quiet());
        assert!(leaving.content_loading(2), "a document that began loading before the stop took hold is stopped too");
        leaving.navigation_completed(2);
        assert!(page.quiet(), "the stopped document never subscribed, so it is told nothing");
        assert_eq!(leaving.close_requested(), Closing::Through);
    }

    #[test]
    fn committing_to_exit_takes_the_window_out_of_input_at_once_however_the_page_hears_of_it() {
        let page = Page::default();
        let leaving = Leaving::gated(gate());
        let locks = Arc::new(Mutex::new(0));
        let locked = Arc::clone(&locks);
        leaving.follow(Arc::new(|| {}), Arc::new(move || *locked.lock().unwrap() += 1));
        watched(&leaving, &page, 7);
        assert!(leaving.navigation_starting(1), "a slow reload starts while nothing stands");
        assert_eq!(leaving.close_requested(), Closing::Held);
        assert_eq!(leaving.close_requested(), Closing::Offer(Unanswered::Close));
        leaving.chosen();
        leaving.exit();
        assert_eq!(*locks.lock().unwrap(), 1, "the user’s Yes stops input before the page could hear anything");
        assert!(page.quiet(), "nothing is sent while the reload is pending");
        leaving.exit();
        assert_eq!(leaving.close_requested(), Closing::Through);
        assert_eq!(*locks.lock().unwrap(), 1, "once");

        let leaving = Leaving::gated(gate());
        let locks = Arc::new(Mutex::new(0));
        let locked = Arc::clone(&locks);
        leaving.follow(Arc::new(|| {}), Arc::new(move || *locked.lock().unwrap() += 1));
        watched(&leaving, &page, 7);
        let asking = leaving.pose(About::Install);
        let (_, install, _) = heard(&leaving, &page);
        assert_eq!(leaving.answer(install, false, &[]), Answered::Heard);
        assert_eq!(leaving.wait(asking).0, Verdict::Leaving);
        assert_eq!(*locks.lock().unwrap(), 0, "a restart the page agreed to locks the page itself, and BattlePlan isn’t leaving until its installer starts");
        assert_eq!(leaving.close_requested(), Closing::Held);
        let (_, batch) = page.hears(&leaving);
        assert_eq!(leaving.answer(waiting(&batch, About::Close), false, &[install]), Answered::Leave);
        assert_eq!(*locks.lock().unwrap(), 1, "the page’s own leave answer commits BattlePlan to exit, so input stops then too");
    }

    #[test]
    fn a_page_asking_again_hears_what_stands_now_so_a_close_its_receipt_retired_is_asked_with_a_fresh_id() {
        let page = Page::default();
        let leaving = Leaving::gated(gate());
        watched(&leaving, &page, 7);
        let document = leaving.document();
        assert_eq!(leaving.close_requested(), Closing::Held);
        let (batch, offer) = page.misses(&leaving);
        assert_eq!(offer, Some(Unanswered::Unreached));
        let (retired, _) = asked(&batch);
        leaving.chosen();
        assert_eq!(leaving.refresh(document, 0, 0), Err(UNWATCHED), "a subscription that was replaced can’t ask");
        assert_eq!(leaving.refresh(document + 1, 1, 0), Err(UNWATCHED));
        assert!(page.quiet());
        let refreshed = leaving.refresh(document, 1, 0);
        let (callback, batch) = page.hears(&leaving);
        assert_eq!(refreshed, Ok(batch.seq), "the page learns which batch answers it");
        let (again, about) = asked(&batch);
        assert_eq!((callback, about), (7, About::Close));
        assert_ne!(again, retired, "the close the native choice kept waiting is asked again, with a fresh id");
        assert_eq!(leaving.answer(retired, false, &[]), Answered::Unknown);
        assert_eq!(leaving.answer(again, false, &[]), Answered::Leave);
    }

    #[test]
    fn a_reload_refuses_update_questions_by_name_holds_closes_and_an_abandoned_one_keeps_the_page_it_left() {
        let page = Page::default();
        let leaving = Leaving::gated(gate());
        watched(&leaving, &page, 7);
        let document = leaving.document();
        let asking = leaving.pose(About::Install);
        let (_, late, _) = heard(&leaving, &page);
        assert_eq!(leaving.close_requested(), Closing::Held);
        let (_, batch) = page.hears(&leaving);
        let close = waiting(&batch, About::Close);

        assert!(leaving.navigation_starting(5));
        assert_eq!(leaving.wait(asking).0.refusal(), Some(Verdict::RELOADED));
        assert_eq!(leaving.answer(late, false, &[]), Answered::Unknown, "the outgoing page can’t commit to leaving");
        assert_eq!(leaving.answer(close, false, &[]), Answered::Unknown);
        assert_eq!(leaving.busy(), Some(Verdict::RELOADED.to_owned()));
        assert!(page.quiet(), "nothing is asked while the page may be replaced");

        leaving.navigation_completed(5);
        assert_eq!(leaving.document(), document, "a stopped reload leaves the page in place");
        let (callback, batch) = page.hears(&leaving);
        let (again, about) = asked(&batch);
        assert_eq!((callback, about), (7, About::Close), "its subscription still stands, and the close is asked again");
        let asking = leaving.pose(About::Query);
        let (_, batch) = page.hears(&leaving);
        let query = waiting(&batch, About::Query);
        assert_eq!(leaving.answer(query, true, &[]), Answered::Heard);
        assert_eq!(leaving.wait(asking).0, Verdict::Staying);

        assert!(leaving.navigation_starting(6));
        assert!(!leaving.content_loading(6));
        leaving.navigation_completed(6);
        assert_ne!(leaving.document(), document, "a reload that loads replaces the page");
        assert_eq!(leaving.answer(again, false, &[]), Answered::Unknown);
        assert!(page.quiet());
        assert_eq!(leaving.close_requested(), Closing::Offer(Unanswered::Close), "the close waits for the new page");
    }

    #[test]
    fn a_refresh_during_a_reload_says_so_and_the_page_an_abandoned_reload_leaves_hears_what_stands_even_when_nothing_does() {
        let page = Page::default();
        let leaving = Leaving::gated(gate());
        watched(&leaving, &page, 7);
        let document = leaving.document();
        let asking = leaving.pose(About::Install);
        let (_, install, _) = heard(&leaving, &page);
        assert!(leaving.navigation_starting(5));
        assert_eq!(leaving.wait(asking).0, Verdict::Reloaded, "the reload retires the install the page was locked for");
        assert_eq!(leaving.refresh(document, 1, 0), Err(RELOADING), "nothing reaches the page while it may be replaced, so Try again isn’t told it did");
        assert!(page.quiet());
        leaving.navigation_completed(5);
        let (_, batch) = page.hears(&leaving);
        assert!(batch.waiting.is_empty() && batch.holds.is_empty(), "the page that stays learns its install is gone: {batch:?}");
        assert_eq!(leaving.answer(install, false, &[]), Answered::Unknown);
        let refreshed = leaving.refresh(document, 1, 0);
        assert_eq!(refreshed, Ok(page.hears(&leaving).1.seq));
    }

    #[test]
    fn navigations_are_followed_by_identity_so_one_loading_never_settles_another() {
        let page = Page::default();
        let leaving = Leaving::gated(gate());
        watched(&leaving, &page, 7);
        assert!(leaving.navigation_starting(9));
        assert!(leaving.navigation_starting(3), "WebView2 gives navigation ids no order");
        assert!(!leaving.content_loading(9));
        leaving.navigation_completed(9);
        assert_eq!(leaving.busy(), Some(Verdict::RELOADED.to_owned()), "the other navigation is still under way");
        leaving.navigation_completed(3);
        assert_eq!(leaving.watch(leaving.document(), 2, page.reach(8)), Ok(()));
        page.hears(&leaving);
        let asking = leaving.pose(About::Query);
        let (_, query, _) = heard(&leaving, &page);
        assert_eq!(leaving.answer(query, true, &[]), Answered::Heard);
        assert_eq!(leaving.wait(asking).0, Verdict::Staying);
    }

    #[test]
    fn a_page_the_host_cant_follow_refuses_updates_by_name_and_offers_the_choice_at_each_close() {
        let page = Page::default();
        let leaving = Leaving::gated(gate());
        watched(&leaving, &page, 7);
        let asking = leaving.pose(About::Query);
        page.hears(&leaving);
        leaving.blind();
        assert_eq!(leaving.wait(asking).0.refusal(), Some(Verdict::BLIND), "a question waiting when the host goes blind is refused by name");
        assert_eq!(leaving.watch(leaving.document(), 2, || unreachable!("a page the host can’t follow isn’t asked")), Err(Verdict::BLIND));
        assert_eq!(leaving.busy(), Some(Verdict::BLIND.to_owned()));
        assert_eq!(leaving.close_for_update(), Err(Verdict::BLIND.to_owned()));
        assert_eq!(leaving.close_requested(), Closing::Offer(Unanswered::Blind), "the first close offers the choice, naming why");
        leaving.chosen();
        assert_eq!(leaving.close_requested(), Closing::Offer(Unanswered::Blind));
        assert!(page.quiet());

        let leaving = Leaving::gated(gate());
        watched(&leaving, &page, 7);
        leaving.blind();
        assert!(!leaving.page_gone(), "a crash the host sees while blind to reloads closes nothing");
        assert_eq!(
            leaving.close_requested(),
            Closing::Offer(Unanswered::Blind),
            "the page may have been reloaded and edited since, unseen, so the user chooses"
        );
        assert_eq!(leaving.close_for_update(), Err(Verdict::BLIND.to_owned()));
    }

    #[test]
    fn a_dead_page_holds_nothing_that_can_be_saved() {
        let page = Page::default();
        let leaving = Leaving::gated(gate());
        watched(&leaving, &page, 7);
        let asking = leaving.pose(About::Query);
        assert_eq!(leaving.close_requested(), Closing::Held);
        let (_, query, _) = heard(&leaving, &page);
        page.hears(&leaving);
        assert!(leaving.page_gone(), "the close the user asked for goes ahead");
        assert_eq!(leaving.wait(asking).0, Verdict::Leaving, "nothing a dead page held can be saved");
        assert_eq!(leaving.answer(query, true, &[]), Answered::Unknown);
        assert_eq!(leaving.watch(leaving.document(), 2, || unreachable!("a dead page subscribes nothing")), Err(REPLACED));
        assert_eq!(leaving.close_for_update(), Ok(()));
        assert_eq!(leaving.close_requested(), Closing::Through);
        assert!(!leaving.navigation_starting(1), "nothing reloads while BattlePlan closes");

        let leaving = Leaving::gated(gate());
        watched(&leaving, &page, 7);
        assert!(!leaving.page_gone(), "a page that ends with no close asked doesn’t close BattlePlan");
        assert_eq!(leaving.close_requested(), Closing::Through, "the next close goes through");
    }

    #[test]
    fn every_batch_says_whether_an_install_the_page_asked_for_is_still_on_its_way_and_a_refresh_gives_up_those_not_yet_seen() {
        let page = Page::default();
        let leaving = Leaving::gated(gate());
        watched(&leaving, &page, 7);
        let document = leaving.document();
        assert_eq!(leaving.attempt(document, 1), Ok(()));
        let seq = leaving.refresh(document, 1, 0).unwrap();
        let (_, batch) = page.hears(&leaving);
        assert_eq!(batch.seq, seq);
        assert!(batch.installing && batch.waiting.is_empty() && batch.holds.is_empty(), "nothing stands yet, but the install the page asked for may still be posed: {batch:?}");
        leaving.attempt_ended(document, 1);
        let (_, batch) = page.hears(&leaving);
        assert!(!batch.installing, "its end is sent to the page, so a page that lost the reply hears it: {batch:?}");

        leaving.refresh(document, 1, 3).unwrap();
        page.hears(&leaving);
        assert_eq!(leaving.attempt(document, 3), Err(super::GIVEN_UP), "a restart the page gave up before the host saw it never starts");
        assert_eq!(leaving.attempt(document, 2), Err(super::GIVEN_UP), "nor any earlier one");
        assert_eq!(leaving.attempt(document + 1, 4), Err(REPLACED), "a restart from a page since replaced is refused");
        assert_eq!(leaving.attempt(document, 4), Ok(()));
    }

    #[test]
    fn a_question_setup_asks_is_given_up_by_name_when_the_page_doesnt_answer_in_time() {
        let page = Page::default();
        let leaving = Leaving::gated(gate());
        watched(&leaving, &page, 7);
        let posed = leaving.pose(About::Leave);
        let (_, id, _) = heard(&leaving, &page);
        assert_eq!(leaving.wait_within(posed, Some(Duration::ZERO)).0, Verdict::Late);
        let (_, given_up) = page.hears(&leaving);
        assert!(given_up.waiting.is_empty() && given_up.holds.is_empty(), "the page hears at once that setup’s question is gone, so a lock it took for it ends: {given_up:?}");
        assert_eq!(leaving.answer(id, false, &[]), Answered::Unknown, "the page’s late answer closes nothing after setup was told no");
        assert!(!leaving.lock().exit.sealed());
    }

    #[test]
    fn the_page_agreeing_to_setups_close_commits_at_once_and_the_exit_waits_to_answer_setup() {
        let page = Page::default();
        let writes = gate();
        let leaving = Leaving::gated(writes);
        watched(&leaving, &page, 7);
        let closing = leaving.pose(About::Leave);
        let asking = leaving.pose(About::Install);
        let (_, batch) = page.hears(&leaving);
        assert_eq!(leaving.answer(waiting(&batch, About::Leave), false, &[]), Answered::Heard);
        committed(&leaving, writes, "the page agreed to setup’s close");
        assert_eq!(leaving.leave_after(asking, || unreachable!("an install still asked when setup’s close was agreed to never launches")), Err(Verdict::CLOSING.to_owned()));
        assert_eq!(leaving.wait(closing).0, Verdict::Leaving);

        let page = Page::default();
        let leaving = Leaving::gated(gate());
        watched(&leaving, &page, 7);
        leaving.exit();
        page.hears(&leaving);
        assert_eq!(leaving.close_for_update(), Ok(()), "setup’s close after the exit is agreed to at once");
        assert_eq!(leaving.busy(), None, "and the other app’s query is answered free");

        let page = Page::default();
        let leaving = Leaving::gated(gate());
        watched(&leaving, &page, 7);
        let closing = leaving.pose(About::Leave);
        let querying = leaving.pose(About::Query);
        assert_eq!(leaving.close_requested(), Closing::Held);
        leaving.exit();
        assert_eq!(leaving.wait(closing).0, Verdict::Leaving, "the seal settles setup’s close waiting on the page, so it is owed a Y");
        assert_eq!(leaving.wait(querying).0, Verdict::Leaving, "and a query waiting on the page");
        assert!(leaving.lock().questions.is_empty(), "a window close still waiting is retired, so no failed batch can offer the choice again");
    }

    #[test]
    fn a_page_process_that_ends_during_a_reload_takes_its_reload_with_it() {
        let page = Page::default();
        let leaving = Leaving::gated(gate());
        watched(&leaving, &page, 7);
        assert!(leaving.navigation_starting(3));
        assert!(!leaving.page_gone());
        assert_eq!(leaving.close_for_update(), Ok(()), "setup hears that nothing a dead page held can be saved, not that it reloaded");
    }

    mod interleavings {
        use super::super::{About, Answered, Ask, Batch, Closing, Leaving, Page as Shown, Posed, Reach, Verdict};
        use serde_json::{Value, json};
        use std::collections::{BTreeMap, BTreeSet, VecDeque};
        use std::sync::{Arc, Mutex, Weak};

        const SEEDS: u64 = 4000;
        const REGRESSIONS: [u64; 1] = [10103];
        const TRACES: &str = "../../ui/src/shell/closeGuard.traces.json";

        struct Rng(u64);

        impl Rng {
            fn below(&mut self, n: usize) -> usize {
                self.0 ^= self.0 << 13;
                self.0 ^= self.0 >> 7;
                self.0 ^= self.0 << 17;
                (self.0 % n as u64) as usize
            }
        }

        #[derive(Clone)]
        enum Failure {
            Watch,
            Judge,
            Recheck,
            Answer(About, bool, Vec<u64>),
        }

        #[derive(Default)]
        struct Guard {
            trace: usize,
            seq: u64,
            ended: bool,
            watching: bool,
            watched: bool,
            by_choice: bool,
            applied: Option<u64>,
            document: Option<u64>,
            resend: Vec<(u64, u64, u64)>,
            committed: BTreeMap<u64, (bool, bool)>,
            judged: BTreeSet<u64>,
            waiting: Vec<Ask>,
            failures: BTreeMap<String, (u64, Failure)>,
            failed: u64,
            refreshing: BTreeSet<u64>,
            refreshes: usize,
            recheck_after: Option<u64>,
            unresolved: bool,
            installing: bool,
            shown: Option<Option<bool>>,
        }

        impl Guard {
            fn fail(&mut self, key: String, failure: Failure) {
                self.failed += 1;
                self.failures.insert(key, (self.failed, failure));
            }

            fn bound(&self, key: &str) -> Option<u64> {
                self.failures.get(key).map(|&(failure, _)| failure)
            }
        }

        struct Asking {
            doc: usize,
            guard: usize,
            key: String,
            bound: Option<u64>,
            ordinal: usize,
        }

        #[derive(Default)]
        struct Doc {
            alive: bool,
            guards: Vec<Guard>,
            dirty: bool,
            unreadable: bool,
            confirmed: bool,
            discarding: bool,
            locked: bool,
        }

        #[derive(Clone, Copy)]
        struct Restart {
            doc: usize,
            confirmed: bool,
            attempt: Option<(u64, u64)>,
        }

        enum Pending {
            Install(Restart, bool),
            Pipe,
            Query,
        }

        #[derive(Clone, Copy, Debug, PartialEq, Eq)]
        enum Step {
            Edit,
            Save,
            Restart,
            Pose,
            CancelRestart,
            DiscardAndClose,
            CloseWindow,
            SetupCloses,
            SetupAsks,
            RunBatch,
            DropBatch,
            Answer,
            AnswerLost,
            Reply,
            Retry,
            Refresh,
            RefreshLost,
            RefreshReply,
            Register,
            RegistrationLost,
            Reload,
            ReloadLoads,
            ReloadStops,
            PageExits,
            Unresponsive,
            Unreadable,
            Settle,
            Launch,
            RestartReply,
            Choose,
        }

        const STEPS: [(Step, usize); 30] = [
            (Step::Edit, 18),
            (Step::Save, 6),
            (Step::Restart, 9),
            (Step::Pose, 9),
            (Step::CancelRestart, 6),
            (Step::DiscardAndClose, 1),
            (Step::CloseWindow, 2),
            (Step::SetupCloses, 3),
            (Step::SetupAsks, 9),
            (Step::RunBatch, 30),
            (Step::DropBatch, 6),
            (Step::Answer, 24),
            (Step::AnswerLost, 3),
            (Step::Reply, 24),
            (Step::Retry, 9),
            (Step::Refresh, 9),
            (Step::RefreshLost, 2),
            (Step::RefreshReply, 9),
            (Step::Register, 18),
            (Step::RegistrationLost, 3),
            (Step::Reload, 6),
            (Step::ReloadLoads, 6),
            (Step::ReloadStops, 6),
            (Step::PageExits, 1),
            (Step::Unresponsive, 3),
            (Step::Unreadable, 3),
            (Step::Settle, 18),
            (Step::Launch, 3),
            (Step::RestartReply, 12),
            (Step::Choose, 3),
        ];

        const AFTER_EXIT: [Step; 17] = [
            Step::Edit,
            Step::Pose,
            Step::CloseWindow,
            Step::RunBatch,
            Step::DropBatch,
            Step::Answer,
            Step::Reply,
            Step::Retry,
            Step::Refresh,
            Step::RefreshReply,
            Step::Register,
            Step::Reload,
            Step::ReloadLoads,
            Step::ReloadStops,
            Step::PageExits,
            Step::Settle,
            Step::Launch,
        ];

        const DRAIN: [Step; 13] = [
            Step::Register,
            Step::Pose,
            Step::Refresh,
            Step::RefreshReply,
            Step::RunBatch,
            Step::Answer,
            Step::Reply,
            Step::ReloadStops,
            Step::Settle,
            Step::Launch,
            Step::RestartReply,
            Step::Choose,
            Step::Retry,
        ];

        #[derive(Default)]
        struct Wire {
            batches: Vec<(usize, usize, Batch)>,
            refused: Vec<bool>,
            dispatched: Vec<u64>,
            last: Option<(usize, usize)>,
            locks: usize,
        }

        struct World {
            leaving: Arc<Leaving>,
            seed: u64,
            rng: Rng,
            log: Vec<String>,
            docs: Vec<Doc>,
            live: usize,
            wire: Arc<Mutex<Wire>>,
            registrations: VecDeque<(usize, usize, Option<u64>)>,
            refreshes: Vec<Asking>,
            refreshed_with: Vec<(Asking, Option<u64>)>,
            answers: VecDeque<(usize, usize, u64, bool, Vec<u64>, About)>,
            replies: VecDeque<(usize, usize, u64, Option<bool>, (About, bool, Vec<u64>))>,
            posed: Vec<(Pending, Posed)>,
            starting: Vec<(Restart, bool)>,
            attempts: u64,
            launching: Vec<(Restart, Option<u64>, bool)>,
            restarts: VecDeque<Restart>,
            navigations: Vec<u64>,
            gone: bool,
            next_navigation: u64,
            disabled: bool,
            choosing: bool,
            closing: bool,
            consents: BTreeSet<u64>,
            draining: bool,
            exited: bool,
            reserved: Option<BTreeSet<u64>>,
            trace: Vec<Value>,
            shapes: BTreeSet<String>,
            guards: usize,
            emitted: Vec<Value>,
            prompted: usize,
            refreshed: usize,
        }

        fn about(about: About) -> &'static str {
            match about {
                About::Close => "close",
                About::Install => "install",
                About::Leave => "leave",
                About::Query => "query",
            }
        }

        fn shape(event: &Value) -> String {
            let kind = event[0].as_str().expect("an event names its kind");
            let (facts, out) = match kind {
                "b" => (format!("{} {} {} {} {}", event[2], event[7], event[5].as_array().is_some_and(|asks| !asks.is_empty()), event[6].as_array().is_some_and(|asks| !asks.is_empty()), event[9]), &event[8]),
                "r" => (format!("{} {}", event[2], event[5]), &event[6]),
                "t" => (format!("{} {}", event[2], event[4].as_str().is_some_and(|key| key.starts_with("answer:"))), &event[5]),
                "w" => (event[2].to_string(), &event[3]),
                "f" => (event[3].is_null().to_string(), &event[4]),
                "l" | "k" => (String::new(), &event[2]),
                _ => return kind.to_owned(),
            };
            let answers = out[0].as_array().map_or(&[][..], Vec::as_slice);
            let leaves = answers.iter().any(|answer| answer[1] == false);
            let based = answers.iter().any(|answer| answer[2].as_array().is_some_and(|basis| !basis.is_empty()));
            let failing = out[3].as_array().map(|failures| !failures.is_empty());
            format!(
                "{kind} {facts} | {} {leaves} {based} {} {} {failing:?} {}",
                answers.len().min(3),
                out[1],
                out[2].as_u64().is_some_and(|prompts| prompts > 0),
                out[4].as_u64().is_some_and(|refreshes| refreshes > 0)
            )
        }

        fn asks(asks: &[Ask]) -> Value {
            Value::Array(asks.iter().map(|ask| json!([ask.id, about(ask.about)])).collect())
        }

        impl World {
            fn new(seed: u64) -> Self {
                let mut world = Self {
                    leaving: Arc::new(Leaving::gated(super::gate())),
                    seed,
                    rng: Rng(seed.wrapping_mul(0x9E37_79B9_7F4A_7C15) | 1),
                    log: Vec::new(),
                    docs: Vec::new(),
                    live: 0,
                    wire: Arc::default(),
                    registrations: VecDeque::new(),
                    refreshes: Vec::new(),
                    refreshed_with: Vec::new(),
                    answers: VecDeque::new(),
                    replies: VecDeque::new(),
                    posed: Vec::new(),
                    starting: Vec::new(),
                    attempts: 0,
                    launching: Vec::new(),
                    restarts: VecDeque::new(),
                    navigations: Vec::new(),
                    gone: false,
                    next_navigation: 1,
                    disabled: false,
                    choosing: false,
                    closing: false,
                    consents: BTreeSet::new(),
                    draining: false,
                    exited: false,
                    reserved: None,
                    trace: Vec::new(),
                    shapes: BTreeSet::new(),
                    guards: 0,
                    emitted: Vec::new(),
                    prompted: 0,
                    refreshed: 0,
                };
                let wire = Arc::clone(&world.wire);
                world.leaving.follow(Arc::new(|| {}), Arc::new(move || wire.lock().unwrap().locks += 1));
                world.load();
                world
            }

            fn fail(&self, what: &str) -> String {
                format!("seed {}: {what}\n{}", self.seed, self.log.join("\n"))
            }

            fn record(&mut self, event: Value) {
                self.shapes.insert(shape(&event));
                self.trace.push(event);
            }

            fn begin(&mut self) {
                self.emitted.clear();
                self.prompted = 0;
                self.refreshed = 0;
            }

            fn out(&mut self, doc: usize, guard: usize) -> Value {
                let state = &self.docs[doc].guards[guard];
                let locked = match state.shown.flatten() {
                    None => 0,
                    Some(false) => 1,
                    Some(true) => 2,
                };
                let failures = if state.ended { Value::Null } else { json!(state.failures.keys().collect::<Vec<_>>()) };
                json!([std::mem::take(&mut self.emitted), locked, self.prompted, failures, self.refreshed])
            }

            fn facts(&self, doc: usize) -> (Value, bool) {
                let doc = &self.docs[doc];
                (if doc.unreadable { Value::Null } else { json!(doc.dirty) }, doc.confirmed)
            }

            fn load(&mut self) {
                self.docs.push(Doc { alive: true, ..Doc::default() });
                self.live = self.docs.len() - 1;
                self.mount();
                if self.rng.below(2) == 0 {
                    self.mount();
                }
            }

            fn mount(&mut self) {
                let live = self.live;
                if let Some(active) = self.docs[live].guards.len().checked_sub(1) {
                    self.docs[live].guards[active].ended = true;
                    let trace = self.docs[live].guards[active].trace;
                    self.record(json!(["e", trace]));
                }
                let trace = self.guards;
                self.guards += 1;
                let seq = self.docs[live].guards.len() as u64 + 1;
                self.docs[live].guards.push(Guard { trace, seq, ..Guard::default() });
                self.record(json!(["m", trace]));
                let guard = self.docs[live].guards.len() - 1;
                self.watch(live, guard);
            }

            fn watch(&mut self, doc: usize, guard: usize) {
                let state = &mut self.docs[doc].guards[guard];
                if state.ended || state.watching || state.watched {
                    return;
                }
                state.watching = true;
                self.registrations.push_back((doc, guard, None));
            }

            fn reach(&self, doc: usize, guard: usize) -> impl FnOnce() -> Reach {
                let wire = Arc::clone(&self.wire);
                let leaving: Weak<Leaving> = Arc::downgrade(&self.leaving);
                let seed = self.seed;
                move || {
                    Arc::new(move |batch: Batch| {
                        let refused = (batch.seq ^ seed).wrapping_mul(0x2545_F491_4F6C_DD1D) >> 61 == 0;
                        wire.lock().unwrap().last = Some((doc, guard));
                        wire.lock().unwrap().dispatched.push(batch.seq);
                        if !refused {
                            wire.lock().unwrap().batches.push((doc, guard, batch));
                            return;
                        }
                        let leaving = leaving.upgrade().expect("the broker outlives its dispatches");
                        assert!(
                            leaving.state.try_lock().is_ok(),
                            "seed {seed}: a dispatch ran under the broker lock, so a dropped receipt would deadlock"
                        );
                        let offer = leaving.delivered(batch.seq, false).is_some();
                        wire.lock().unwrap().refused.push(offer);
                    })
                }
            }

            fn active(&self) -> Option<usize> {
                let doc = &self.docs[self.live];
                (doc.alive && !doc.guards.is_empty()).then(|| doc.guards.len() - 1)
            }

            fn editable(&self) -> bool {
                let doc = &self.docs[self.live];
                doc.alive && !doc.locked && !doc.confirmed && !self.disabled
            }

            fn show(&mut self, at: usize, guard: usize) {
                let doc = &mut self.docs[at];
                let state = &mut doc.guards[guard];
                let now = if state.committed.values().any(|&(update, _)| update) {
                    Some(true)
                } else if state.by_choice || !state.committed.is_empty() {
                    Some(false)
                } else {
                    None
                };
                if !state.ended && state.shown == Some(Some(true)) && now != Some(true) {
                    state.unresolved = false;
                }
                if state.committed.is_empty() {
                    if !state.unresolved {
                        state.recheck_after = None;
                    }
                    state.failures.remove("recheck");
                }
                if state.ended || state.shown == Some(now) {
                    return;
                }
                state.shown = Some(now);
                doc.locked = now.is_some();
                if now == Some(true) && state.unresolved && !state.failures.contains_key("recheck") {
                    self.recheck_now(at, guard);
                }
            }

            fn send(&mut self, doc: usize, guard: usize, id: u64, about: About, stays: bool, basis: Vec<u64>) {
                let state = &mut self.docs[doc].guards[guard];
                if state.ended {
                    return;
                }
                state.failures.remove(&format!("answer:{id}"));
                self.log.push(format!("  doc {doc} guard {guard} answers {about:?} {id}: stays {stays}, basis {basis:?}"));
                self.emitted.push(json!([id, stays, basis]));
                self.answers.push_back((doc, guard, id, stays, basis, about));
            }

            fn judge(&mut self, doc: usize, guard: usize, asks: Vec<Ask>, whole: bool) -> bool {
                let (dirty, confirmed) = (self.docs[doc].dirty, self.docs[doc].confirmed);
                for ask in asks {
                    let state = &mut self.docs[doc].guards[guard];
                    if state.ended {
                        return false;
                    }
                    if state.judged.contains(&ask.id) {
                        continue;
                    }
                    if std::mem::take(&mut self.docs[doc].unreadable) {
                        self.log.push(format!("  doc {doc} guard {guard} can’t read its drafts while judging {}", ask.id));
                        self.docs[doc].guards[guard].fail("judge".into(), Failure::Judge);
                        return false;
                    }
                    let state = &mut self.docs[doc].guards[guard];
                    let leaving = state.by_choice || !state.committed.is_empty();
                    if ask.about == About::Query {
                        state.judged.insert(ask.id);
                        self.send(doc, guard, ask.id, ask.about, !leaving && dirty, Vec::new());
                        continue;
                    }
                    let confirmed = ask.about == About::Install && confirmed;
                    if !leaving && dirty && !confirmed {
                        state.judged.insert(ask.id);
                        if ask.about == About::Close {
                            self.prompted += 1;
                        }
                        self.send(doc, guard, ask.id, ask.about, true, Vec::new());
                        continue;
                    }
                    let leaning = dirty && !state.by_choice && !confirmed;
                    if leaning && state.committed.values().any(|&(_, accepted)| !accepted) {
                        continue;
                    }
                    let basis = if leaning { state.committed.keys().copied().collect() } else { Vec::new() };
                    state.judged.insert(ask.id);
                    state.committed.insert(ask.id, (ask.about != About::Close, false));
                    if dirty && !leaving {
                        self.consents.insert(ask.id);
                    }
                    self.show(doc, guard);
                    self.send(doc, guard, ask.id, ask.about, false, basis);
                }
                let state = &mut self.docs[doc].guards[guard];
                if whole || state.waiting.iter().all(|ask| state.judged.contains(&ask.id)) {
                    state.failures.remove("judge");
                }
                true
            }

            fn hear(&mut self, doc: usize, guard: usize, batch: &Batch) -> bool {
                if !self.docs[doc].alive {
                    return false;
                }
                self.begin();
                let (dirty, confirmed) = self.facts(doc);
                let returned = self.apply(doc, guard, batch);
                let trace = self.docs[doc].guards[guard].trace;
                let out = self.out(doc, guard);
                self.record(json!(["b", trace, dirty, confirmed, batch.seq, asks(&batch.waiting), asks(&batch.holds), returned, out, batch.installing]));
                returned
            }

            fn apply(&mut self, doc: usize, guard: usize, batch: &Batch) -> bool {
                let state = &mut self.docs[doc].guards[guard];
                if state.ended {
                    return false;
                }
                if state.applied.is_some_and(|applied| applied >= batch.seq) {
                    self.log.push(format!("  doc {doc} guard {guard} judges what older {} carries", batch.seq));
                    let carried = batch.waiting.iter().filter(|ask| state.waiting.iter().any(|standing| standing.id == ask.id)).copied().collect();
                    return self.judge(doc, guard, carried, false);
                }
                self.log.push(format!("  doc {doc} guard {guard} hears {batch:?}"));
                state.applied = Some(batch.seq);
                state.waiting.clone_from(&batch.waiting);
                state.installing = batch.installing;
                let live: BTreeSet<u64> = batch.waiting.iter().chain(&batch.holds).map(|ask| ask.id).collect();
                state.committed.retain(|id, _| live.contains(id));
                state.judged.retain(|id| live.contains(id));
                let asked: BTreeSet<u64> = batch.waiting.iter().map(|ask| ask.id).collect();
                let rechecked = state.recheck_after.is_some_and(|after| batch.seq >= after);
                state.failures.retain(|key, _| key.strip_prefix("answer:").is_none_or(|id| asked.contains(&id.parse().unwrap())));
                for hold in &batch.holds {
                    state.committed.entry(hold.id).and_modify(|(_, accepted)| *accepted = true).or_insert((hold.about != About::Close, true));
                }
                self.show(doc, guard);
                self.resend_due(doc, guard);
                let waiting = self.docs[doc].guards[guard].waiting.clone();
                let handled = self.judge(doc, guard, waiting, true);
                let state = &mut self.docs[doc].guards[guard];
                if !state.ended && rechecked && !state.committed.values().any(|&(update, _)| update) && !state.installing {
                    state.recheck_after = None;
                    state.unresolved = false;
                    state.failures.remove("recheck");
                }
                handled
            }

            fn resend_due(&mut self, doc: usize, guard: usize) {
                let state = &mut self.docs[doc].guards[guard];
                let applied = state.applied;
                let (due, later): (Vec<(u64, u64, u64)>, Vec<(u64, u64, u64)>) =
                    std::mem::take(&mut state.resend).into_iter().partition(|&(_, after, _)| applied.is_some_and(|applied| applied >= after));
                state.resend = later;
                for (id, _, bound) in due {
                    let state = &self.docs[doc].guards[guard];
                    let Some((failure, Failure::Answer(about, stays, basis))) = state.failures.get(&format!("answer:{id}")).cloned() else { continue };
                    if failure != bound || !state.waiting.iter().any(|ask| ask.id == id) {
                        continue;
                    }
                    assert!(!self.leaving.lock().held(id), "{}", self.fail(&format!("Try again resent {id}, which the host already took")));
                    self.send(doc, guard, id, about, stays, basis);
                }
            }

            fn ask_again(&mut self, doc: usize, guard: usize, key: String) {
                let state = &mut self.docs[doc].guards[guard];
                let bound = state.bound(&key);
                if bound.is_some_and(|bound| state.refreshing.contains(&bound)) {
                    return;
                }
                state.refreshing.extend(bound);
                let ordinal = state.refreshes;
                state.refreshes += 1;
                self.refreshed += 1;
                self.refreshes.push(Asking { doc, guard, key, bound, ordinal });
            }

            fn recheck_now(&mut self, doc: usize, guard: usize) {
                let state = &mut self.docs[doc].guards[guard];
                if !state.failures.contains_key("recheck") && !state.committed.is_empty() {
                    state.fail("recheck".into(), Failure::Recheck);
                }
                self.ask_again(doc, guard, "recheck".into());
            }

            fn recheck(&mut self, doc: usize, guard: usize) {
                self.begin();
                if !self.docs[doc].guards[guard].ended {
                    self.docs[doc].guards[guard].unresolved = true;
                    self.recheck_now(doc, guard);
                }
                let trace = self.docs[doc].guards[guard].trace;
                let out = self.out(doc, guard);
                self.record(json!(["k", trace, out]));
            }

            fn refreshed(&mut self, asking: &Asking, seq: Option<u64>) {
                let state = &mut self.docs[asking.doc].guards[asking.guard];
                if state.ended {
                    return;
                }
                if let Some(bound) = asking.bound {
                    state.refreshing.remove(&bound);
                }
                let current = state.failures.get(&asking.key).cloned();
                if current.as_ref().map(|&(failure, _)| failure) != asking.bound {
                    return;
                }
                let Some(after) = seq else {
                    if asking.key != "recheck" || !state.committed.is_empty() {
                        state.fail(asking.key.clone(), current.map_or(Failure::Recheck, |(_, failure)| failure));
                    }
                    return;
                };
                if asking.key == "recheck" {
                    if state.applied.is_some_and(|applied| applied >= after) && !state.committed.values().any(|&(update, _)| update) && !state.installing {
                        state.recheck_after = None;
                        state.unresolved = false;
                        state.failures.remove("recheck");
                    } else {
                        state.recheck_after = Some(after);
                        if !state.committed.is_empty() {
                            state.fail("recheck".into(), Failure::Recheck);
                        }
                    }
                }
                if let (Some(id), Some(bound)) = (asking.key.strip_prefix("answer:").and_then(|id| id.parse().ok()), asking.bound) {
                    match state.resend.iter_mut().find(|(resent, _, _)| *resent == id) {
                        Some(entry) => (entry.1, entry.2) = (after, bound),
                        None => state.resend.push((id, after, bound)),
                    }
                    self.resend_due(asking.doc, asking.guard);
                }
            }

            fn reply(&mut self, doc: usize, guard: usize, id: u64, reply: Option<bool>, sent: (About, bool, Vec<u64>)) {
                let state = &mut self.docs[doc].guards[guard];
                if state.ended {
                    return;
                }
                let Some(heard) = reply else {
                    if state.waiting.iter().any(|ask| ask.id == id) {
                        state.fail(format!("answer:{id}"), Failure::Answer(sent.0, sent.1, sent.2));
                    }
                    return;
                };
                state.failures.remove(&format!("answer:{id}"));
                let Some(commitment) = state.committed.get_mut(&id) else { return };
                if heard {
                    commitment.1 = true;
                } else {
                    state.committed.remove(&id);
                }
                self.show(doc, guard);
                let state = &self.docs[doc].guards[guard];
                if !state.failures.contains_key("judge") {
                    let waiting = state.waiting.clone();
                    self.judge(doc, guard, waiting, true);
                }
            }

            fn exit(&mut self, consented: bool, how: &str) {
                self.log.push(format!("exit: {how}"));
                if self.exited {
                    return;
                }
                let doc = &self.docs[self.live];
                let given_up = {
                    let state = self.leaving.lock();
                    self.consents.iter().any(|&consent| state.held(consent))
                };
                assert!(
                    !doc.alive || consented || !doc.dirty || given_up || doc.discarding,
                    "{}",
                    self.fail(&format!("BattlePlan left ({how}) with edits the user never gave up"))
                );
                assert!(!self.leaving.navigation_starting(u64::MAX), "{}", self.fail("a page could reload while BattlePlan exits"));
                self.exited = true;
                self.reserved = Some(self.installs_held());
            }

            fn installs_held(&self) -> BTreeSet<u64> {
                self.leaving.lock().holds.iter().filter(|hold| hold.about == About::Install).map(|hold| hold.id).collect()
            }

            fn offered(&mut self, offer: bool) {
                if offer || self.choosing {
                    self.closing = false;
                }
                if offer {
                    assert!(!self.choosing, "{}", self.fail("two native choices at once"));
                    self.choosing = true;
                }
            }

            fn pose(&mut self, pending: Pending, about: About) {
                match self.leaving.pose(about) {
                    Ok(posed) => self.posed.push((pending, posed)),
                    Err(verdict) => {
                        assert!(
                            !(self.gone && verdict == Verdict::Reloaded),
                            "{}",
                            self.fail("setup heard that a page whose process ended had reloaded")
                        );
                        if let Pending::Install(restart, early) = pending {
                            assert_ne!(verdict, Verdict::Leaving, "{}", self.fail("an install went ahead without its page"));
                            if !early {
                                self.restarts.push_back(restart);
                            }
                            self.end_attempt(restart);
                        }
                    }
                }
            }

            fn end_attempt(&self, restart: Restart) {
                if let Some((document, attempt)) = restart.attempt {
                    self.leaving.attempt_ended(document, attempt);
                }
            }

            fn installing(&self) -> bool {
                !self.starting.is_empty() || !self.launching.is_empty() || self.posed.iter().any(|(pending, _)| matches!(pending, Pending::Install(..)))
            }

            fn chance(&mut self, one_in: usize) -> bool {
                !self.draining && self.rng.below(one_in) == 0
            }

            fn run(&mut self, step: Step) -> bool {
                let ran = self.act(step);
                let (refused, locks) = {
                    let mut wire = self.wire.lock().unwrap();
                    (std::mem::take(&mut wire.refused), std::mem::take(&mut wire.locks))
                };
                for offer in refused {
                    self.offered(offer);
                }
                self.disabled |= locks > 0;
                ran
            }

            fn act(&mut self, step: Step) -> bool {
                let live = self.live;
                let alive = self.docs[live].alive;
                match step {
                    Step::Edit if self.editable() => self.docs[live].dirty = true,
                    Step::Save if self.editable() => self.docs[live].dirty = false,
                    Step::Restart if alive && !self.docs[live].locked && !self.docs[live].confirmed && !self.installing() => {
                        let confirmed = self.docs[live].dirty;
                        self.docs[live].confirmed = confirmed;
                        let document = self.leaving.document();
                        self.attempts += 1;
                        let attempt = self.leaving.attempt(document, self.attempts).ok().map(|()| (document, self.attempts));
                        let restart = Restart { doc: live, confirmed, attempt };
                        let early = self.chance(4);
                        self.starting.push((restart, early));
                        if early {
                            self.restarts.push_back(restart);
                        }
                    }
                    Step::Pose => {
                        let Some((restart, early)) = self.starting.pop() else { return false };
                        if self.chance(5) {
                            self.log.push("  the other app is busy, so the install is refused before it is posed".into());
                            if !early {
                                self.restarts.push_back(restart);
                            }
                            self.end_attempt(restart);
                        } else {
                            self.pose(Pending::Install(restart, early), About::Install);
                        }
                    }
                    Step::CancelRestart if self.docs[live].confirmed && !self.docs[live].locked => self.docs[live].confirmed = false,
                    Step::DiscardAndClose if self.editable() && self.docs[live].dirty => {
                        let guard = self.active().expect("a live page has a guard");
                        self.docs[live].discarding = true;
                        self.begin();
                        self.docs[live].guards[guard].by_choice = true;
                        self.show(live, guard);
                        let trace = self.docs[live].guards[guard].trace;
                        let out = self.out(live, guard);
                        self.record(json!(["l", trace, out]));
                        self.leaving.exit();
                        self.exit(true, "discard and close");
                    }
                    Step::CloseWindow => match self.leaving.close_requested() {
                        Closing::Through => self.exit(false, "the window closed through"),
                        Closing::Held => self.closing = true,
                        Closing::Offer(_) => self.offered(true),
                    },
                    Step::SetupCloses => self.pose(Pending::Pipe, About::Leave),
                    Step::SetupAsks => self.pose(Pending::Query, About::Query),
                    Step::RunBatch | Step::DropBatch => {
                        let picked = {
                            let wire = self.wire.lock().unwrap();
                            (!wire.batches.is_empty()).then_some(wire.batches.len())
                        };
                        let Some(count) = picked else { return false };
                        let (doc, guard, batch) = self.wire.lock().unwrap().batches.remove(self.rng.below(count));
                        let reaches = self.navigations.is_empty();
                        let ran = matches!(step, Step::RunBatch) && reaches && self.hear(doc, guard, &batch);
                        let offer = self.leaving.delivered(batch.seq, ran).is_some();
                        self.offered(offer);
                    }
                    Step::Answer | Step::AnswerLost => {
                        let Some((doc, guard, id, stays, basis, about)) = self.answers.pop_front() else { return false };
                        if matches!(step, Step::AnswerLost) {
                            self.replies.push_back((doc, guard, id, None, (about, stays, basis)));
                            return true;
                        }
                        let answered = self.leaving.answer(id, stays, &basis);
                        let sent = (about, stays, basis);
                        self.log.push(format!("  host answers {id}: {answered:?}"));
                        if about == About::Close && answered != Answered::Unknown {
                            self.closing = false;
                        }
                        let reply = answered != Answered::Unknown;
                        if answered == Answered::Leave {
                            self.exit(false, "the page’s answer closed it");
                        } else if reply {
                            self.act(Step::Settle);
                        }
                        let lost = self.chance(4);
                        self.replies.push_back((doc, guard, id, (!lost).then_some(reply), sent));
                    }
                    Step::Reply => {
                        let Some((doc, guard, id, reply, sent)) = self.replies.pop_front() else { return false };
                        if !self.docs[doc].alive {
                            return true;
                        }
                        self.begin();
                        let (dirty, confirmed) = self.facts(doc);
                        self.reply(doc, guard, id, reply, sent);
                        let trace = self.docs[doc].guards[guard].trace;
                        let out = self.out(doc, guard);
                        self.record(json!(["r", trace, dirty, confirmed, id, reply, out]));
                    }
                    Step::Retry => {
                        let Some(guard) = self.active() else { return false };
                        let state = &self.docs[live].guards[guard];
                        let open: Vec<(String, Failure)> = state
                            .failures
                            .iter()
                            .filter(|(_, (failure, _))| !self.draining || !state.refreshing.contains(failure))
                            .map(|(key, (_, failure))| (key.clone(), failure.clone()))
                            .collect();
                        if open.is_empty() {
                            return false;
                        }
                        let (key, failure) = open[self.rng.below(open.len())].clone();
                        self.begin();
                        let (dirty, confirmed) = self.facts(live);
                        match failure {
                            Failure::Watch => self.watch(live, guard),
                            Failure::Judge | Failure::Recheck | Failure::Answer(..) => self.ask_again(live, guard, key.clone()),
                        }
                        let trace = self.docs[live].guards[guard].trace;
                        let out = self.out(live, guard);
                        self.record(json!(["t", trace, dirty, confirmed, key, out]));
                    }
                    Step::Refresh | Step::RefreshLost => {
                        if self.refreshes.is_empty() {
                            return false;
                        }
                        let asking = self.refreshes.remove(self.rng.below(self.refreshes.len()));
                        let (doc, guard) = (asking.doc, asking.guard);
                        if !self.docs[doc].alive {
                            return true;
                        }
                        let (document, seq) = (self.docs[doc].guards[guard].document, self.docs[doc].guards[guard].seq);
                        let answered = match document {
                            Some(document) if matches!(step, Step::Refresh) => {
                                let refreshed = self.leaving.refresh(document, seq, 0);
                                self.log.push(format!("  doc {doc} guard {guard} asks again what stands: {refreshed:?}"));
                                if let Ok(sent) = refreshed {
                                    assert!(self.wire.lock().unwrap().dispatched.contains(&sent), "{}", self.fail("a refresh said it sent a batch it never dispatched"));
                                }
                                let lost = self.chance(4);
                                refreshed.ok().filter(|_| !lost)
                            }
                            _ => None,
                        };
                        self.refreshed_with.push((asking, answered));
                    }
                    Step::RefreshReply => {
                        if self.refreshed_with.is_empty() {
                            return false;
                        }
                        let (asking, answered) = self.refreshed_with.remove(self.rng.below(self.refreshed_with.len()));
                        if !self.docs[asking.doc].alive {
                            return true;
                        }
                        self.begin();
                        self.refreshed(&asking, answered);
                        let trace = self.docs[asking.doc].guards[asking.guard].trace;
                        let out = self.out(asking.doc, asking.guard);
                        self.record(json!(["f", trace, asking.ordinal, answered, out]));
                    }
                    Step::Register | Step::RegistrationLost => {
                        let Some((doc, guard, document)) = self.registrations.pop_front() else { return false };
                        if !self.docs[doc].alive {
                            return true;
                        }
                        let lost = matches!(step, Step::RegistrationLost);
                        let watched = match document {
                            None if !lost => {
                                let document = self.leaving.document();
                                self.docs[doc].guards[guard].document = Some(document);
                                self.registrations.push_back((doc, guard, Some(document)));
                                return true;
                            }
                            Some(document) if !lost => {
                                let seq = self.docs[doc].guards[guard].seq;
                                self.leaving.watch(document, seq, self.reach(doc, guard)).is_ok()
                            }
                            _ => false,
                        };
                        self.begin();
                        let state = &mut self.docs[doc].guards[guard];
                        state.watching = false;
                        if watched {
                            state.watched = true;
                            state.failures.remove("watch");
                        } else if !state.ended {
                            state.fail("watch".into(), Failure::Watch);
                        }
                        let trace = state.trace;
                        let out = self.out(doc, guard);
                        self.record(json!(["w", trace, watched, out]));
                    }
                    Step::Reload => {
                        let navigation = self.next_navigation;
                        self.next_navigation += 1;
                        if self.leaving.navigation_starting(navigation) {
                            self.navigations.push(navigation);
                            self.gone = false;
                        } else {
                            self.leaving.navigation_completed(navigation);
                        }
                    }
                    Step::ReloadLoads if !self.navigations.is_empty() => {
                        let navigation = self.navigations.remove(self.rng.below(self.navigations.len()));
                        let held = !self.leaving.lock().holds.is_empty();
                        let stopped = self.leaving.content_loading(navigation);
                        assert_eq!(stopped, held, "{}", self.fail("a document began loading while BattlePlan held a leave, and wasn’t stopped"));
                        assert!(stopped || self.launching.is_empty(), "{}", self.fail("a page loaded while an installer starts"));
                        self.leaving.navigation_completed(navigation);
                        self.docs[live].alive = false;
                        if stopped {
                            self.docs.push(Doc::default());
                            self.live = self.docs.len() - 1;
                        } else {
                            self.load();
                        }
                    }
                    Step::ReloadStops => {
                        if self.navigations.is_empty() {
                            return false;
                        }
                        let navigation = self.navigations.remove(self.rng.below(self.navigations.len()));
                        let before = self.wire.lock().unwrap().dispatched.len();
                        self.leaving.navigation_completed(navigation);
                        let owed = {
                            let state = self.leaving.lock();
                            matches!(state.page, Shown::Watched { .. }) && !state.blind && state.navigations.is_empty()
                        };
                        let sent = self.wire.lock().unwrap().dispatched.len() > before;
                        assert!(sent || !owed, "{}", self.fail("an abandoned reload left its page without what stands"));
                    }

                    Step::PageExits if alive => {
                        self.docs[live].alive = false;
                        self.navigations.clear();
                        self.gone = true;
                        if self.leaving.page_gone() {
                            self.exit(false, "the page ended while closing");
                        }
                    }
                    Step::Unresponsive => {
                        let offer = self.leaving.page_unresponsive();
                        self.offered(offer);
                    }
                    Step::Unreadable if alive => self.docs[live].unreadable = true,
                    Step::Settle => {
                        let mut settled = false;
                        for at in (0..self.posed.len()).rev() {
                            let Some((verdict, id)) = Leaving::taken(&mut self.leaving.lock(), self.posed[at].1.0) else { continue };
                            self.log.push(format!("  settled {verdict:?} {id:?}"));
                            settled = true;
                            match self.posed.remove(at).0 {
                                Pending::Install(restart, early) if verdict == Verdict::Leaving => {
                                    let lost = early || self.chance(3);
                                    if lost && !early {
                                        self.restarts.push_back(restart);
                                    }
                                    self.launching.push((restart, id, lost));
                                }
                                Pending::Install(restart, early) => {
                                    if !early {
                                        self.restarts.push_back(restart);
                                    }
                                    self.end_attempt(restart);
                                }
                                Pending::Pipe if verdict == Verdict::Leaving => {
                                    self.leaving.exit();
                                    self.exit(false, "setup closed it");
                                }
                                Pending::Pipe | Pending::Query => {}
                            }
                        }
                        return settled;
                    }
                    Step::Launch => {
                        let Some((restart, id, replied)) = self.launching.pop() else { return false };
                        let started = self.chance(3);
                        self.leaving.act(|state| {
                            if started {
                                state.exit();
                                state.launches.retain(|(held, _)| Some(*held) != id);
                            } else if let Some(id) = id {
                                Leaving::release(state, id);
                            }
                        });
                        self.end_attempt(restart);
                        if started {
                            self.exit(false, "the installer started");
                        } else if !replied {
                            self.restarts.push_back(restart);
                        }
                    }
                    Step::RestartReply => {
                        let Some(Restart { doc, confirmed, .. }) = self.restarts.pop_front() else { return false };
                        if confirmed {
                            self.docs[doc].confirmed = false;
                        }
                        if self.docs[doc].alive
                            && let Some(guard) = self.docs[doc].guards.len().checked_sub(1)
                        {
                            self.recheck(doc, guard);
                        }
                    }
                    Step::Choose if self.choosing => {
                        self.choosing = false;
                        self.leaving.chosen();
                        if self.chance(4) {
                            self.leaving.exit();
                            self.exit(true, "the user chose to close");
                        }
                    }
                    _ => return false,
                }
                true
            }

            fn step(&mut self, steps: &[(Step, usize)]) {
                let mut pick = self.rng.below(steps.iter().map(|(_, weight)| weight).sum());
                let step = steps
                    .iter()
                    .find_map(|&(step, weight)| if pick < weight { Some(step) } else { pick -= weight; None })
                    .expect("a step is picked");
                if self.run(step) {
                    self.log.push(format!("{step:?}"));
                }
            }

            fn drain(&mut self, retrying: bool) -> bool {
                self.draining = true;
                DRAIN.iter().any(|&step| !self.exited && (retrying || !matches!(step, Step::Retry)) && self.run(step))
            }

            fn settled(&mut self, retrying: bool) {
                let mut drained = 0;
                while !self.exited && self.drain(retrying) {
                    self.check();
                    drained += 1;
                    assert!(drained < 10_000, "{}", self.fail("the host and page never settle"));
                }
            }

            fn waiting_unseen(&self) -> Vec<u64> {
                let (shown, deferred): (BTreeSet<u64>, BTreeSet<u64>) = match self.active() {
                    Some(guard) => {
                        let state = &self.docs[self.live].guards[guard];
                        let shown: BTreeSet<u64> = state.failures.keys().filter_map(|key| key.strip_prefix("answer:")?.parse().ok()).collect();
                        let blocked = state.failures.contains_key("judge") || state.committed.iter().any(|(id, &(_, accepted))| !accepted && shown.contains(id));
                        let deferred = if blocked { state.waiting.iter().map(|ask| ask.id).filter(|id| !state.judged.contains(id)).collect() } else { BTreeSet::new() };
                        (shown, deferred)
                    }
                    None => (BTreeSet::new(), BTreeSet::new()),
                };
                let state = self.leaving.lock();
                self.posed
                    .iter()
                    .filter_map(|(_, posed)| state.questions.iter().find(|question| question.key == posed.0))
                    .filter(|question| question.verdict.is_none() && !question.asked.is_some_and(|id| shown.contains(&id) || deferred.contains(&id)))
                    .map(|question| question.key)
                    .collect()
            }

            fn stuck(&self) -> bool {
                let doc = &self.docs[self.live];
                doc.alive
                    && doc.locked
                    && self.active().is_some_and(|guard| !doc.guards[guard].by_choice)
                    && self.leaving.lock().holds.is_empty()
            }

            fn probe(&mut self) {
                let Some(guard) = self.active() else { return };
                if !self.docs[self.live].guards[guard].watched {
                    return;
                }
                if let Err(verdict) = self.leaving.pose(About::Query) {
                    panic!("{}", self.fail(&format!("the live, subscribed page wasn’t asked ({verdict:?})")));
                }
                let asked = self.wire.lock().unwrap().last;
                assert_eq!(asked, Some((self.live, guard)), "{}", self.fail("the live page isn’t the one asked"));
            }

            fn check(&self) {
                if let Some(reserved) = &self.reserved {
                    let late: Vec<u64> = self.installs_held().difference(reserved).copied().collect();
                    assert!(late.is_empty(), "{}", self.fail(&format!("BattlePlan accepted installs {late:?} after it committed to exit")));
                }
                let doc = &self.docs[self.live];
                assert!(
                    !doc.alive || doc.locked || self.disabled || self.leaving.lock().holds.is_empty(),
                    "{}",
                    self.fail("the page can be edited while BattlePlan holds a leave")
                );
            }

            fn after_exit(&mut self) {
                let steps: Vec<(Step, usize)> = AFTER_EXIT.iter().map(|&step| (step, 1)).collect();
                self.draining = false;
                for _ in 0..40 {
                    self.step(&steps);
                    self.check();
                }
            }

            fn explore(&mut self) {
                for _ in 0..300 {
                    if self.exited {
                        break;
                    }
                    self.step(&STEPS);
                    self.check();
                }
                self.settled(false);
                if self.exited {
                    return self.after_exit();
                }
                let unseen = self.waiting_unseen();
                assert!(unseen.is_empty(), "{}", self.fail(&format!("setup waits on {unseen:?} with nothing for the user to retry")));
                self.settled(true);
                if self.exited {
                    return self.after_exit();
                }
                assert!(self.posed.is_empty() && self.starting.is_empty(), "{}", self.fail("a question setup waits on was never settled"));
                assert!(!self.closing, "{}", self.fail("a close went unanswered with no choice offered"));
                assert!(
                    self.leaving.lock().questions.iter().all(|question| question.verdict.is_some() || question.asked.is_none()),
                    "{}",
                    self.fail("the host waits on an answer no page will send")
                );
                if self.stuck() {
                    let guards: Vec<String> = self.docs[self.live]
                        .guards
                        .iter()
                        .map(|guard| {
                            format!(
                                "ended {} watching {} watched {} applied {:?} committed {:?} failures {:?} refreshing {:?} shown {:?}",
                                guard.ended,
                                guard.watching,
                                guard.watched,
                                guard.applied,
                                guard.committed,
                                guard.failures.keys().collect::<Vec<_>>(),
                                guard.refreshing,
                                guard.shown
                            )
                        })
                        .collect();
                    let host = {
                        let state = self.leaving.lock();
                        let page = match state.page {
                            Shown::Gone => "gone".to_owned(),
                            Shown::Loading => "loading".to_owned(),
                            Shown::Watched { seq, .. } => format!("watched {seq}"),
                        };
                        format!("{page}, navigations {:?}", state.navigations)
                    };
                    panic!(
                        "{}",
                        self.fail(&format!(
                            "the page stays locked though nothing stands, with nothing for the user to retry\nguards:\n{}\nhost: {host}; batches {}, refreshes {}, replies {}",
                            guards.join("\n"),
                            self.wire.lock().unwrap().batches.len(),
                            self.refreshes.len(),
                            self.refreshed_with.len()
                        ))
                    );
                }
                self.probe();
            }
        }

        #[test]
        fn no_interleaving_loses_unsaved_work_or_leaves_a_question_unanswered() {
            let mut traces = Vec::new();
            let mut covered = BTreeSet::new();
            for seed in (1..=SEEDS).chain(REGRESSIONS) {
                let mut world = World::new(seed);
                world.explore();
                if !world.shapes.is_subset(&covered) {
                    covered.append(&mut world.shapes);
                    traces.push(serde_json::to_string(&json!([seed, world.trace])).expect("a trace is JSON"));
                }
            }
            let written = format!("[\n{}\n]\n", traces.join(",\n"));
            let folder = std::env::var_os("CARGO_MANIFEST_DIR").expect("cargo names the crate’s folder");
            let path = std::path::Path::new(&folder).join(TRACES);
            if std::env::var_os("BATTLEPLAN_WRITE_TRACES").is_some() {
                std::fs::write(&path, written).expect("the traces are written");
                return;
            }
            let committed = std::fs::read_to_string(&path).unwrap_or_default().replace("\r\n", "\n");
            assert!(
                committed == written,
                "{} no longer holds what this model of the page guard does; closeGuard.test.ts replays it through the real guard. Run this test with BATTLEPLAN_WRITE_TRACES=1 and commit the file",
                path.display()
            );
        }
    }
}
