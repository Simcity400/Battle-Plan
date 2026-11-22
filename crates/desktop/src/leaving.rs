use serde::Serialize;
use std::collections::VecDeque;
use std::sync::{Arc, Condvar, Mutex, MutexGuard, PoisonError};

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
    #[serde(skip)]
    pub token: u64,
    pub asks: Vec<Ask>,
    pub waiting: Vec<u64>,
    pub holds: Vec<Ask>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Verdict {
    Leaving,
    Staying,
    Reloaded,
    Unresponsive,
    Unheard,
    Unreached,
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

    pub fn refusal(self) -> Option<&'static str> {
        match self {
            Self::Leaving => None,
            Self::Staying => Some(Self::STAYING),
            Self::Reloaded => Some(Self::RELOADED),
            Self::Unresponsive => Some(Self::UNRESPONSIVE),
            Self::Unheard => Some(Self::UNHEARD),
            Self::Unreached => Some(Self::UNREACHED),
        }
    }
}

pub const REPLACED: &str = "This page was replaced before it could watch for BattlePlan closing.";

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Unanswered {
    Close,
    Unresponsive,
    Unreached,
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

pub struct Posed(u64);

struct Question {
    key: u64,
    about: About,
    asked: Option<u64>,
    verdict: Option<Verdict>,
}

struct Sent {
    token: u64,
    asks: Vec<(u64, u64)>,
}

enum Page {
    Gone,
    Loading,
    Watched { seq: u64, reach: Reach },
}

struct State {
    page: Page,
    document: u64,
    navigations: Vec<u64>,
    next: u64,
    questions: Vec<Question>,
    holds: Vec<Ask>,
    sent: Vec<Sent>,
    outbox: VecDeque<(Reach, Batch)>,
    offering: bool,
}

impl State {
    fn fresh(&mut self) -> u64 {
        let id = self.next;
        self.next += 1;
        id
    }

    fn deliverable(&self) -> bool {
        matches!(self.page, Page::Watched { .. }) && self.navigations.is_empty()
    }

    fn stands(&self, id: u64) -> bool {
        self.holds.iter().any(|hold| hold.id == id)
            || self.questions.iter().any(|question| question.asked == Some(id) && question.verdict.is_none())
    }

    fn exiting(&self) -> bool {
        self.holds.iter().any(|hold| hold.about == About::Close)
    }

    fn send(&mut self, keys: &[u64]) {
        if !self.deliverable() {
            return;
        }
        let Page::Watched { reach, .. } = &self.page else {
            return;
        };
        let reach = Arc::clone(reach);
        let mut asks = Vec::new();
        let mut sent = Vec::new();
        for &key in keys {
            let id = self.fresh();
            if let Some(question) = self.questions.iter_mut().find(|question| question.key == key && question.verdict.is_none()) {
                question.asked = Some(id);
                asks.push(Ask { id, about: question.about });
                sent.push((key, id));
            }
        }
        let waiting = self
            .questions
            .iter()
            .filter(|question| question.verdict.is_none())
            .filter_map(|question| question.asked)
            .collect();
        let token = self.fresh();
        self.sent.push(Sent { token, asks: sent });
        let batch = Batch { token, asks, waiting, holds: self.holds.clone() };
        self.outbox.push_back((reach, batch));
    }

    fn exit(&mut self) {
        if self.exiting() {
            return;
        }
        let id = self.fresh();
        self.holds.push(Ask { id, about: About::Close });
        self.send(&[]);
    }

    fn leave_without_page(&mut self, at: usize) {
        let about = self.questions[at].about;
        if about != About::Query {
            let id = self.fresh();
            self.holds.push(Ask { id, about });
            self.questions[at].asked = Some(id);
        }
        self.questions[at].verdict = Some(Verdict::Leaving);
    }

    fn offer(&mut self, why: Unanswered) -> Closing {
        if std::mem::replace(&mut self.offering, true) { Closing::Held } else { Closing::Offer(why) }
    }

    fn keys(&self, waiting: impl Fn(&Question) -> bool) -> Vec<u64> {
        self.questions
            .iter()
            .filter(|question| question.verdict.is_none() && waiting(question))
            .map(|question| question.key)
            .collect()
    }
}

pub struct Leaving {
    state: Mutex<State>,
    sending: Mutex<()>,
    answered: Condvar,
}

impl Default for Leaving {
    fn default() -> Self {
        Self {
            state: Mutex::new(State {
                page: Page::Loading,
                document: 0,
                navigations: Vec::new(),
                next: 0,
                questions: Vec::new(),
                holds: Vec::new(),
                sent: Vec::new(),
                outbox: VecDeque::new(),
                offering: false,
            }),
            sending: Mutex::new(()),
            answered: Condvar::new(),
        }
    }
}

impl Leaving {
    fn lock(&self) -> MutexGuard<'_, State> {
        self.state.lock().unwrap_or_else(PoisonError::into_inner)
    }

    fn act<T>(&self, change: impl FnOnce(&mut State) -> T) -> T {
        let value = change(&mut self.lock());
        let _sending = self.sending.lock().unwrap_or_else(PoisonError::into_inner);
        while let Some((reach, batch)) = self.lock().outbox.pop_front() {
            reach(batch);
        }
        value
    }

    fn settle(&self, state: &mut State, verdict: Verdict) {
        for question in &mut state.questions {
            if question.about != About::Close && question.verdict.is_none() {
                question.verdict = Some(verdict);
            }
        }
        self.answered.notify_all();
    }

    fn retire(&self, state: &mut State) {
        for question in &mut state.questions {
            question.asked = None;
        }
        self.settle(state, Verdict::Reloaded);
    }

    pub fn document(&self) -> u64 {
        self.lock().document
    }

    pub fn watch(&self, document: u64, seq: u64, reach: impl FnOnce() -> Reach) -> Result<(), &'static str> {
        self.act(|state| {
            if document != state.document || matches!(state.page, Page::Gone) {
                return Err(REPLACED);
            }
            if let Page::Watched { seq: current, .. } = &state.page
                && *current >= seq
            {
                return Ok(());
            }
            state.page = Page::Watched { seq, reach: reach() };
            let waiting = state.keys(|_| true);
            state.send(&waiting);
            Ok(())
        })
    }

    pub fn delivered(&self, token: u64, reached: bool) -> Option<Unanswered> {
        let mut state = self.lock();
        let at = state.sent.iter().position(|sent| sent.token == token)?;
        let sent = state.sent.remove(at);
        if reached {
            return None;
        }
        let mut offer = None;
        for (key, id) in sent.asks {
            let Some(question) = state
                .questions
                .iter_mut()
                .find(|question| question.key == key && question.asked == Some(id) && question.verdict.is_none())
            else {
                continue;
            };
            if question.about == About::Close {
                question.asked = None;
                if let Closing::Offer(why) = state.offer(Unanswered::Unreached) {
                    offer = Some(why);
                }
            } else {
                question.verdict = Some(Verdict::Unreached);
                self.answered.notify_all();
            }
        }
        offer
    }

    pub fn answer(&self, id: u64, stays: bool, basis: &[u64]) -> Answered {
        let answered = self.act(|state| {
            let Some(at) = state.questions.iter().position(|question| question.asked == Some(id) && question.verdict.is_none()) else {
                return if !stays && state.holds.iter().any(|hold| hold.id == id) { Answered::Heard } else { Answered::Unknown };
            };
            let about = state.questions[at].about;
            if !stays && about != About::Query && !basis.iter().all(|&standing| state.stands(standing)) {
                let key = state.questions[at].key;
                state.questions[at].asked = None;
                state.send(&[key]);
                return Answered::Unknown;
            }
            if !stays && about != About::Query {
                state.holds.push(Ask { id, about });
            }
            if about == About::Close {
                state.questions.remove(at);
                return if stays { Answered::Heard } else { Answered::Leave };
            }
            state.questions[at].verdict = Some(if stays { Verdict::Staying } else { Verdict::Leaving });
            Answered::Heard
        });
        self.answered.notify_all();
        answered
    }

    pub fn close_requested(&self) -> Closing {
        self.act(|state| {
            if state.exiting() {
                return Closing::Through;
            }
            if matches!(state.page, Page::Gone) {
                state.exit();
                return Closing::Through;
            }
            let deliverable = state.deliverable();
            if let Some(close) = state.questions.iter().find(|question| question.about == About::Close) {
                let (key, asked) = (close.key, close.asked.is_some());
                if asked || !deliverable {
                    return state.offer(Unanswered::Close);
                }
                state.send(&[key]);
                return Closing::Held;
            }
            let key = state.fresh();
            state.questions.push(Question { key, about: About::Close, asked: None, verdict: None });
            state.send(&[key]);
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
        let mut state = self.lock();
        if !state.holds.is_empty() {
            return false;
        }
        state.navigations.push(navigation);
        self.retire(&mut state);
        true
    }

    pub fn content_loading(&self, navigation: u64) {
        let mut state = self.lock();
        state.navigations.retain(|&pending| pending > navigation);
        state.document += 1;
        state.page = Page::Loading;
        state.sent.clear();
        self.retire(&mut state);
    }

    pub fn navigation_completed(&self, navigation: u64) {
        self.act(|state| {
            state.navigations.retain(|&pending| pending != navigation);
            let unasked = state.keys(|question| question.asked.is_none());
            if !unasked.is_empty() || !state.holds.is_empty() {
                state.send(&unasked);
            }
        });
    }

    pub fn page_gone(&self) -> bool {
        let mut state = self.lock();
        state.page = Page::Gone;
        state.sent.clear();
        let closing = state.questions.iter().any(|question| question.about == About::Close);
        state.questions.retain(|question| question.about != About::Close);
        for at in 0..state.questions.len() {
            if state.questions[at].verdict.is_none() {
                state.leave_without_page(at);
            }
        }
        if closing {
            state.exit();
        }
        self.answered.notify_all();
        closing
    }

    pub fn page_unresponsive(&self) -> bool {
        let mut state = self.lock();
        self.settle(&mut state, Verdict::Unresponsive);
        !std::mem::replace(&mut state.offering, true)
    }

    fn pose(&self, about: About) -> Result<Posed, Verdict> {
        self.act(|state| {
            if state.exiting() {
                return Err(Verdict::Leaving);
            }
            if !state.navigations.is_empty() {
                return Err(Verdict::Reloaded);
            }
            let key = state.fresh();
            match state.page {
                Page::Gone if about == About::Query => return Err(Verdict::Leaving),
                Page::Gone => {
                    state.questions.push(Question { key, about, asked: None, verdict: None });
                    let at = state.questions.len() - 1;
                    state.leave_without_page(at);
                    return Ok(Posed(key));
                }
                Page::Loading => return Err(Verdict::Unheard),
                Page::Watched { .. } => {}
            }
            state.questions.push(Question { key, about, asked: None, verdict: None });
            state.send(&[key]);
            Ok(Posed(key))
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
        let Posed(key) = match posed {
            Ok(posed) => posed,
            Err(verdict) => return (verdict, None),
        };
        let mut state = self.lock();
        loop {
            if let Some(taken) = Self::taken(&mut state, key) {
                return taken;
            }
            state = self.answered.wait(state).unwrap_or_else(PoisonError::into_inner);
        }
    }

    fn stay(&self, id: u64) {
        self.act(|state| {
            let Some(at) = state.holds.iter().position(|hold| hold.id == id && hold.about == About::Install) else {
                return;
            };
            state.holds.remove(at);
            state.send(&[]);
        });
    }

    fn leave_after(&self, posed: Result<Posed, Verdict>, launch: impl FnOnce() -> Result<(), String>) -> Result<(), String> {
        let (verdict, id) = self.wait(posed);
        if let Some(refused) = verdict.refusal() {
            return Err(refused.to_owned());
        }
        let launched = launch();
        match (&launched, id) {
            (Ok(()), _) => self.exit(),
            (Err(_), Some(id)) => self.stay(id),
            (Err(_), None) => {}
        }
        launched
    }

    pub fn install(&self, launch: impl FnOnce() -> Result<(), String>) -> Result<(), String> {
        self.leave_after(self.pose(About::Install), launch)
    }

    pub fn close_for_update(&self) -> Result<(), String> {
        self.wait(self.pose(About::Leave)).0.refusal().map_or(Ok(()), |refused| Err(refused.to_owned()))
    }

    pub fn busy(&self) -> Option<String> {
        self.wait(self.pose(About::Query)).0.refusal().map(str::to_owned)
    }
}

#[cfg(test)]
mod tests {
    use super::{About, Answered, Ask, Batch, Closing, Leaving, REPLACED, Reach, Unanswered, Verdict};
    use std::sync::{
        Arc, Mutex,
        mpsc::{Receiver, Sender, channel},
    };

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
            assert_eq!(leaving.delivered(batch.token, true), None);
            (callback, batch)
        }

        fn misses(&self, leaving: &Leaving) -> (Batch, Option<Unanswered>) {
            let (_, batch) = self.dispatched();
            let offer = leaving.delivered(batch.token, false);
            (batch, offer)
        }
    }

    fn asked(batch: &Batch) -> (u64, About) {
        match batch.asks[..] {
            [Ask { id, about }] => (id, about),
            ref other => panic!("expected one question, got {other:?}"),
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
        assert!(batch.asks.is_empty() && batch.holds.is_empty(), "a new subscription hears what stands: {batch:?}");
    }

    #[test]
    fn a_registration_sent_again_after_a_lost_reply_keeps_the_subscription_it_made() {
        let page = Page::default();
        let leaving = Leaving::default();
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
        let leaving = Leaving::default();
        leaving.content_loading(1);
        let outgoing = leaving.document();
        leaving.content_loading(2);
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
        let leaving = Leaving::default();
        watched(&leaving, &page, 7);
        let asking = leaving.pose(About::Query);
        let (_, first, _) = heard(&leaving, &page);
        assert_eq!(leaving.watch(leaving.document(), 2, page.reach(8)), Ok(()));
        let (callback, batch) = page.hears(&leaving);
        let (second, _) = asked(&batch);
        assert_eq!(callback, 8, "the question is asked again of the newer registration");
        assert_eq!(batch.waiting, [second], "with a fresh id, and the old one no longer stands");
        assert_eq!(leaving.answer(first, false, &[]), Answered::Unknown, "the superseded page’s in-flight Saved doesn’t count");
        assert_eq!(leaving.answer(second, true, &[]), Answered::Heard);
        assert_eq!(leaving.wait(asking), (Verdict::Staying, Some(second)));
    }

    #[test]
    fn a_leave_answer_sent_again_after_a_lost_reply_stands_until_the_host_says_stay() {
        let page = Page::default();
        let leaving = Leaving::default();
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
        let leaving = Leaving::default();
        watched(&leaving, &page, 7);
        std::thread::scope(|scope| {
            let setup = scope.spawn(|| leaving.close_for_update());
            let (_, batch) = page.awaited();
            assert_eq!(leaving.delivered(batch.token, false), None, "WebView2 took the script and didn’t run it");
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
        let leaving = Leaving::default();
        watched(&leaving, &page, 7);
        let asking = leaving.pose(About::Query);
        let (_, stale) = page.dispatched();
        assert_eq!(leaving.watch(leaving.document(), 2, page.reach(8)), Ok(()));
        let (_, id, _) = heard(&leaving, &page);
        assert_eq!(leaving.delivered(stale.token, false), None);
        assert_eq!(leaving.delivered(stale.token, false), None, "a receipt counts once");
        assert_eq!(leaving.answer(id, false, &[]), Answered::Heard, "the question the newer page heard still stands");
        assert_eq!(leaving.wait(asking).0, Verdict::Leaving);
    }

    #[test]
    fn a_close_held_while_the_page_starts_offers_the_choice_when_its_registration_cant_be_reached() {
        let page = Page::default();
        let leaving = Leaving::default();
        assert_eq!(leaving.close_requested(), Closing::Held, "a page that hasn’t subscribed yet may still hold work");
        assert_eq!(leaving.watch(leaving.document(), 1, page.reach(7)), Ok(()));
        let (batch, offer) = page.misses(&leaving);
        assert_eq!(asked(&batch).1, About::Close);
        assert_eq!(offer, Some(Unanswered::Unreached), "the held close isn’t left waiting on a page that never heard it");
    }

    #[test]
    fn every_batch_says_which_leaves_still_stand_so_a_stay_that_never_ran_is_carried_by_the_next() {
        let page = Page::default();
        let leaving = Leaving::default();
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
        assert_eq!((batch.waiting, batch.holds), (vec![close], vec![]), "the close carries the stay that never ran");
    }

    #[test]
    fn a_stay_tells_the_page_and_no_answer_judged_under_the_leave_closes() {
        let page = Page::default();
        let leaving = Leaving::default();
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
        assert!(batch.asks.is_empty() && batch.holds.is_empty(), "the page is told the install no longer stands: {batch:?}");
        assert_eq!(leaving.answer(committed, false, &[install]), Answered::Unknown, "a close judged under the install doesn’t close");
        let (_, batch) = page.hears(&leaving);
        let (close, about) = asked(&batch);
        assert_eq!((about, batch.waiting), (About::Close, vec![close]), "it is asked again, under nothing");
        assert_ne!(close, committed);
        assert_eq!(leaving.answer(close, true, &[]), Answered::Heard, "the page judges it again after the Stay");
    }

    #[test]
    fn a_leave_judged_under_a_commitment_the_host_has_since_refused_is_asked_again() {
        let page = Page::default();
        let leaving = Leaving::default();
        watched(&leaving, &page, 7);
        let asking = leaving.pose(About::Install);
        let (_, install, _) = heard(&leaving, &page);
        assert_eq!(leaving.close_requested(), Closing::Held);
        let (_, batch) = page.dispatched();
        let (close, _) = asked(&batch);
        assert_eq!(batch.waiting, [install, close], "the close was sent while the install still stood");
        assert!(leaving.page_unresponsive());
        assert_eq!(leaving.wait(asking).0, Verdict::Unresponsive);
        assert_eq!(leaving.delivered(batch.token, true), None);
        assert_eq!(leaving.answer(close, false, &[install]), Answered::Unknown, "the page judged the close under the refused install");
        let (_, batch) = page.hears(&leaving);
        let (again, about) = asked(&batch);
        assert_eq!((about, batch.waiting), (About::Close, vec![again]), "it is asked again without the install");
        assert_eq!(leaving.answer(again, true, &[]), Answered::Heard);
    }

    #[test]
    fn a_close_is_asked_of_the_page_and_only_its_leave_answer_closes() {
        let page = Page::default();
        let leaving = Leaving::default();
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
        let leaving = Leaving::default();
        leaving.content_loading(1);
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
        let leaving = Leaving::default();
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
        let leaving = Leaving::default();
        watched(&leaving, &page, 7);
        let leave = leaving.pose(About::Install);
        let query = leaving.pose(About::Query);
        assert_eq!(leaving.close_requested(), Closing::Held);
        let asks: Vec<u64> = (0..3).map(|_| heard(&leaving, &page).1).collect();
        assert!(leaving.page_unresponsive(), "the user is offered to close");
        assert_eq!(leaving.leave_after(leave, || unreachable!("nothing launches")), Err(Verdict::UNRESPONSIVE.to_owned()));
        assert_eq!(leaving.wait(query).0.refusal(), Some(Verdict::UNRESPONSIVE));
        assert!(!leaving.page_unresponsive(), "WebView2 reports again while the first choice is open");
        leaving.chosen();
        assert_eq!(leaving.answer(asks[0], false, &[]), Answered::Unknown, "a refused update can’t be agreed to later");
        assert_eq!(leaving.answer(asks[2], false, &[]), Answered::Leave, "the close stays for a page that recovers");
        assert!(leaving.page_unresponsive());
    }

    #[test]
    fn an_install_the_page_agreed_to_stands_until_it_starts_or_fails() {
        let page = Page::default();
        let leaving = Leaving::default();
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

        let leaving = Leaving::default();
        watched(&leaving, &page, 7);
        let asking = leaving.pose(About::Install);
        let (_, id, _) = heard(&leaving, &page);
        leaving.answer(id, true, &[]);
        assert_eq!(leaving.leave_after(asking, || unreachable!("unsaved work launches nothing")), Err(Verdict::STAYING.to_owned()));
    }

    #[test]
    fn leaving_by_choice_refuses_reloads_while_the_maps_shut_down() {
        let page = Page::default();
        let leaving = Leaving::default();
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
    fn a_reload_refuses_update_questions_by_name_holds_closes_and_an_abandoned_one_keeps_the_page_it_left() {
        let page = Page::default();
        let leaving = Leaving::default();
        watched(&leaving, &page, 7);
        let document = leaving.document();
        let asking = leaving.pose(About::Install);
        let (_, late, _) = heard(&leaving, &page);
        assert_eq!(leaving.close_requested(), Closing::Held);
        let (_, close, _) = heard(&leaving, &page);

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
        assert_eq!(batch.waiting, [again]);
        let asking = leaving.pose(About::Query);
        let (_, query, _) = heard(&leaving, &page);
        assert_eq!(leaving.answer(query, true, &[]), Answered::Heard);
        assert_eq!(leaving.wait(asking).0, Verdict::Staying);

        assert!(leaving.navigation_starting(6));
        leaving.content_loading(6);
        leaving.navigation_completed(6);
        assert_ne!(leaving.document(), document, "a reload that loads replaces the page");
        assert_eq!(leaving.answer(again, false, &[]), Answered::Unknown);
        assert!(page.quiet());
        assert_eq!(leaving.close_requested(), Closing::Offer(Unanswered::Close), "the close waits for the new page");
    }

    #[test]
    fn a_dead_page_holds_nothing_that_can_be_saved() {
        let page = Page::default();
        let leaving = Leaving::default();
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

        let leaving = Leaving::default();
        watched(&leaving, &page, 7);
        assert!(!leaving.page_gone(), "a page that ends with no close asked doesn’t close BattlePlan");
        assert_eq!(leaving.close_requested(), Closing::Through, "the next close goes through");
    }

    mod interleavings {
        use super::super::{About, Answered, Batch, Closing, Leaving, Posed, Reach, Verdict};
        use std::collections::{BTreeMap, BTreeSet, VecDeque};
        use std::sync::{Arc, Mutex};

        struct Rng(u64);

        impl Rng {
            fn below(&mut self, n: usize) -> usize {
                self.0 ^= self.0 << 13;
                self.0 ^= self.0 >> 7;
                self.0 ^= self.0 << 17;
                (self.0 % n as u64) as usize
            }
        }

        #[derive(Default)]
        struct Guard {
            seq: u64,
            ended: bool,
            watching: bool,
            watched: bool,
            watch_failed: bool,
            by_choice: bool,
            committed: BTreeMap<u64, bool>,
            live: BTreeSet<u64>,
            failed: BTreeMap<u64, (About, bool, Vec<u64>)>,
            shown: Option<Option<bool>>,
        }

        #[derive(Default)]
        struct Doc {
            alive: bool,
            guards: Vec<Guard>,
            dirty: bool,
            confirmed: bool,
            discarding: bool,
            locked: bool,
        }

        enum Pending {
            Install(Option<usize>),
            Pipe,
            Query,
        }

        #[derive(Clone, Copy, Debug)]
        enum Step {
            Edit,
            Save,
            Restart,
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
            Remount,
            Register,
            RegistrationLost,
            Reload,
            ReloadLoads,
            ReloadStops,
            PageExits,
            Unresponsive,
            Settle,
            Launch,
            RestartReply,
            Choose,
        }

        const STEPS: [(Step, usize); 26] = [
            (Step::Edit, 18),
            (Step::Save, 6),
            (Step::Restart, 9),
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
            (Step::Remount, 6),
            (Step::Register, 18),
            (Step::RegistrationLost, 3),
            (Step::Reload, 6),
            (Step::ReloadLoads, 6),
            (Step::ReloadStops, 6),
            (Step::PageExits, 1),
            (Step::Unresponsive, 3),
            (Step::Settle, 18),
            (Step::Launch, 3),
            (Step::RestartReply, 12),
            (Step::Choose, 3),
        ];

        const DRAIN: [Step; 10] = [
            Step::Register,
            Step::Retry,
            Step::RunBatch,
            Step::Answer,
            Step::Reply,
            Step::ReloadStops,
            Step::Settle,
            Step::Launch,
            Step::RestartReply,
            Step::Choose,
        ];

        type Batches = Arc<Mutex<VecDeque<(usize, usize, Batch)>>>;

        struct World {
            leaving: Leaving,
            seed: u64,
            rng: Rng,
            log: Vec<String>,
            docs: Vec<Doc>,
            live: usize,
            batches: Batches,
            registrations: VecDeque<(usize, usize, Option<u64>)>,
            answers: VecDeque<(usize, usize, u64, bool, Vec<u64>, About)>,
            replies: VecDeque<(usize, usize, u64, Option<bool>, (About, bool, Vec<u64>))>,
            posed: Vec<(Pending, Posed)>,
            launching: Vec<(Option<usize>, Option<u64>)>,
            restarts: VecDeque<usize>,
            navigations: Vec<u64>,
            next_navigation: u64,
            choosing: bool,
            closing: bool,
            consents: BTreeSet<u64>,
            draining: bool,
            done: bool,
        }

        impl World {
            fn new(seed: u64) -> Self {
                let mut world = Self {
                    leaving: Leaving::default(),
                    seed,
                    rng: Rng(seed.wrapping_mul(0x9E37_79B9_7F4A_7C15) | 1),
                    log: Vec::new(),
                    docs: Vec::new(),
                    live: 0,
                    batches: Arc::default(),
                    registrations: VecDeque::new(),
                    answers: VecDeque::new(),
                    replies: VecDeque::new(),
                    posed: Vec::new(),
                    launching: Vec::new(),
                    restarts: VecDeque::new(),
                    navigations: Vec::new(),
                    next_navigation: 1,
                    choosing: false,
                    closing: false,
                    consents: BTreeSet::new(),
                    draining: false,
                    done: false,
                };
                world.load();
                world
            }

            fn load(&mut self) {
                self.docs.push(Doc { alive: true, ..Doc::default() });
                self.live = self.docs.len() - 1;
                self.mount();
            }

            fn mount(&mut self) {
                let doc = &mut self.docs[self.live];
                if let Some(active) = doc.guards.last_mut() {
                    active.ended = true;
                }
                let seq = doc.guards.len() as u64 + 1;
                doc.guards.push(Guard { seq, ..Guard::default() });
                let guard = doc.guards.len() - 1;
                self.watch(self.live, guard);
            }

            fn watch(&mut self, doc: usize, guard: usize) {
                let state = &mut self.docs[doc].guards[guard];
                if state.ended || state.watching || state.watched {
                    return;
                }
                state.watching = true;
                state.watch_failed = false;
                self.registrations.push_back((doc, guard, None));
            }

            fn reach(&self, doc: usize, guard: usize) -> impl FnOnce() -> Reach {
                let batches = Arc::clone(&self.batches);
                move || Arc::new(move |batch: Batch| batches.lock().unwrap().push_back((doc, guard, batch)))
            }

            fn active(&self) -> Option<usize> {
                let doc = &self.docs[self.live];
                (doc.alive && !doc.guards.is_empty()).then(|| doc.guards.len() - 1)
            }

            fn editable(&self) -> bool {
                let doc = &self.docs[self.live];
                doc.alive && !doc.locked && !doc.confirmed
            }

            fn show(&mut self, doc: usize, guard: usize) {
                let doc = &mut self.docs[doc];
                let state = &mut doc.guards[guard];
                let now = if state.committed.values().any(|&update| update) {
                    Some(true)
                } else if state.by_choice || !state.committed.is_empty() {
                    Some(false)
                } else {
                    None
                };
                if state.ended || state.shown == Some(now) {
                    return;
                }
                state.shown = Some(now);
                doc.locked = now.is_some();
            }

            fn judge(&mut self, doc: usize, guard: usize, id: u64, about: About) {
                let (dirty, confirmed) = (self.docs[doc].dirty, self.docs[doc].confirmed);
                let state = &mut self.docs[doc].guards[guard];
                if state.ended {
                    return;
                }
                state.failed.remove(&id);
                let leaving = state.by_choice || !state.committed.is_empty();
                let mut basis = Vec::new();
                let stays = if about == About::Query {
                    !leaving && dirty
                } else {
                    let stays = !leaving && dirty && !(about == About::Install && confirmed);
                    if !stays && dirty && !state.by_choice {
                        basis = state.committed.keys().copied().collect();
                    }
                    if !stays {
                        state.committed.insert(id, about != About::Close);
                        self.show(doc, guard);
                        if dirty && !leaving {
                            self.consents.insert(id);
                        }
                    }
                    stays
                };
                self.log.push(format!("  doc {doc} guard {guard} judges {about:?} {id}: stays {stays}, basis {basis:?}"));
                self.answers.push_back((doc, guard, id, stays, basis, about));
            }

            fn hear(&mut self, doc: usize, guard: usize, batch: &Batch) -> bool {
                if !self.docs[doc].alive || self.docs[doc].guards[guard].ended {
                    return false;
                }
                self.log.push(format!("  doc {doc} guard {guard} hears {batch:?}"));
                let state = &mut self.docs[doc].guards[guard];
                state.live = batch.waiting.iter().copied().chain(batch.holds.iter().map(|hold| hold.id)).collect();
                let live = state.live.clone();
                state.committed.retain(|id, _| live.contains(id));
                state.failed.retain(|id, _| live.contains(id));
                for hold in &batch.holds {
                    state.committed.entry(hold.id).or_insert(hold.about != About::Close);
                }
                self.show(doc, guard);
                for ask in &batch.asks {
                    self.judge(doc, guard, ask.id, ask.about);
                }
                true
            }

            fn exit(&mut self, consented: bool, how: &str) {
                self.log.push(format!("exit: {how}"));
                let doc = &self.docs[self.live];
                let given_up = {
                    let state = self.leaving.lock();
                    self.consents.iter().any(|&consent| state.stands(consent))
                };
                assert!(
                    !doc.alive || consented || !doc.dirty || given_up || doc.discarding,
                    "seed {}: BattlePlan left ({how}) with edits the user never gave up\n{}",
                    self.seed,
                    self.log.join("\n")
                );
                assert!(!self.leaving.navigation_starting(u64::MAX), "seed {}: a page could reload while BattlePlan exits", self.seed);
                self.done = true;
            }

            fn offered(&mut self, offer: bool) {
                if offer || self.choosing {
                    self.closing = false;
                }
                if offer {
                    assert!(!self.choosing, "seed {}: two native choices at once", self.seed);
                    self.choosing = true;
                }
            }

            fn pose(&mut self, pending: Pending, about: About) {
                match self.leaving.pose(about) {
                    Ok(posed) => self.posed.push((pending, posed)),
                    Err(verdict) => {
                        if let Pending::Install(confirmed) = pending {
                            assert_ne!(verdict, Verdict::Leaving, "seed {}: an install went ahead without its page", self.seed);
                            self.restarts.extend(confirmed);
                        }
                    }
                }
            }

            fn installing(&self) -> bool {
                !self.launching.is_empty() || self.posed.iter().any(|(pending, _)| matches!(pending, Pending::Install(_)))
            }

            fn chance(&mut self, one_in: usize) -> bool {
                !self.draining && self.rng.below(one_in) == 0
            }

            fn run(&mut self, step: Step) -> bool {
                let live = self.live;
                let alive = self.docs[live].alive;
                match step {
                    Step::Edit if self.editable() => self.docs[live].dirty = true,
                    Step::Save if self.editable() => self.docs[live].dirty = false,
                    Step::Restart if alive && !self.docs[live].locked && !self.docs[live].confirmed && !self.installing() => {
                        let confirmed = self.docs[live].dirty.then_some(live);
                        self.docs[live].confirmed = confirmed.is_some();
                        self.pose(Pending::Install(confirmed), About::Install);
                    }
                    Step::CancelRestart if self.docs[live].confirmed && !self.docs[live].locked => self.docs[live].confirmed = false,
                    Step::DiscardAndClose if self.editable() && self.docs[live].dirty => {
                        let guard = self.active().expect("a live page has a guard");
                        self.docs[live].discarding = true;
                        self.docs[live].guards[guard].by_choice = true;
                        self.show(live, guard);
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
                        let Some((doc, guard, batch)) = self.batches.lock().unwrap().pop_front() else { return false };
                        let ran = matches!(step, Step::RunBatch) && self.hear(doc, guard, &batch);
                        let offer = self.leaving.delivered(batch.token, ran).is_some();
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
                        let reply = match answered {
                            Answered::Leave => {
                                self.exit(false, "the page’s answer closed it");
                                return true;
                            }
                            Answered::Heard => true,
                            Answered::Unknown => false,
                        };
                        if reply {
                            self.run(Step::Settle);
                            if self.done {
                                return true;
                            }
                        }
                        let lost = self.chance(4);
                        self.replies.push_back((doc, guard, id, (!lost).then_some(reply), sent));
                    }
                    Step::Reply => {
                        let Some((doc, guard, id, reply, sent)) = self.replies.pop_front() else { return false };
                        if !self.docs[doc].alive || self.docs[doc].guards[guard].ended {
                            return true;
                        }
                        let state = &mut self.docs[doc].guards[guard];
                        match reply {
                            Some(true) => {}
                            Some(false) => {
                                if state.committed.remove(&id).is_some() {
                                    self.show(doc, guard);
                                }
                            }
                            None => {
                                if state.live.contains(&id) {
                                    state.failed.insert(id, sent);
                                }
                            }
                        }
                    }
                    Step::Retry => {
                        let Some(guard) = self.active() else { return false };
                        if self.docs[live].guards[guard].watch_failed {
                            self.watch(live, guard);
                            return true;
                        }
                        let Some(id) = self.docs[live].guards[guard].failed.keys().next().copied() else { return false };
                        let (about, stays, basis) = self.docs[live].guards[guard].failed.remove(&id).expect("the failure is there");
                        self.answers.push_back((live, guard, id, stays, basis, about));
                    }
                    Step::Remount if alive => self.mount(),
                    Step::Register | Step::RegistrationLost => {
                        let Some((doc, guard, document)) = self.registrations.pop_front() else { return false };
                        let lost = matches!(step, Step::RegistrationLost) || (document.is_none() && !self.docs[doc].alive);
                        match document {
                            None if !lost => self.registrations.push_back((doc, guard, Some(self.leaving.document()))),
                            Some(document) if !lost => {
                                let seq = self.docs[doc].guards[guard].seq;
                                let watched = self.leaving.watch(document, seq, self.reach(doc, guard)).is_ok();
                                let state = &mut self.docs[doc].guards[guard];
                                state.watching = false;
                                state.watched = watched;
                                state.watch_failed = !watched;
                            }
                            _ => {
                                let state = &mut self.docs[doc].guards[guard];
                                state.watching = false;
                                state.watch_failed = true;
                            }
                        }
                    }
                    Step::Reload => {
                        let navigation = self.next_navigation;
                        self.next_navigation += 1;
                        if self.leaving.navigation_starting(navigation) {
                            self.navigations.push(navigation);
                        } else {
                            self.leaving.navigation_completed(navigation);
                        }
                    }
                    Step::ReloadLoads if !self.navigations.is_empty() => {
                        let navigation = self.navigations.remove(self.rng.below(self.navigations.len()));
                        assert!(
                            self.leaving.lock().holds.is_empty() && self.launching.is_empty(),
                            "seed {}: a page loaded while BattlePlan held a leave\n{}",
                            self.seed,
                            self.log.join("\n")
                        );
                        self.leaving.content_loading(navigation);
                        self.leaving.navigation_completed(navigation);
                        self.navigations.retain(|&pending| pending > navigation);
                        self.docs[live].alive = false;
                        self.load();
                    }
                    Step::ReloadStops => {
                        let Some(navigation) = self.navigations.pop() else { return false };
                        self.leaving.navigation_completed(navigation);
                    }
                    Step::PageExits if alive => {
                        self.docs[live].alive = false;
                        if self.leaving.page_gone() {
                            self.exit(false, "the page ended while closing");
                        }
                    }
                    Step::Unresponsive => {
                        let offer = self.leaving.page_unresponsive();
                        self.offered(offer);
                    }
                    Step::Settle => {
                        let mut settled = false;
                        for at in (0..self.posed.len()).rev() {
                            let Some((verdict, id)) = Leaving::taken(&mut self.leaving.lock(), self.posed[at].1.0) else { continue };
                            self.log.push(format!("  settled {verdict:?} {id:?}"));
                            settled = true;
                            match self.posed.remove(at).0 {
                                Pending::Install(confirmed) if verdict == Verdict::Leaving => self.launching.push((confirmed, id)),
                                Pending::Install(confirmed) => self.restarts.extend(confirmed),
                                Pending::Pipe if verdict == Verdict::Leaving => {
                                    self.leaving.exit();
                                    self.exit(false, "setup closed it");
                                    return true;
                                }
                                Pending::Pipe | Pending::Query => {}
                            }
                        }
                        return settled;
                    }
                    Step::Launch => {
                        let Some((confirmed, id)) = self.launching.pop() else { return false };
                        if self.chance(3) {
                            self.leaving.exit();
                            self.exit(false, "the installer started");
                        } else {
                            if let Some(id) = id {
                                self.leaving.stay(id);
                            }
                            self.restarts.extend(confirmed);
                        }
                    }
                    Step::RestartReply => {
                        let Some(doc) = self.restarts.pop_front() else { return false };
                        self.docs[doc].confirmed = false;
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

            fn step(&mut self) {
                let mut pick = self.rng.below(STEPS.iter().map(|(_, weight)| weight).sum());
                let step = STEPS
                    .iter()
                    .find_map(|&(step, weight)| if pick < weight { Some(step) } else { pick -= weight; None })
                    .expect("a step is picked");
                if self.run(step) {
                    self.log.push(format!("{step:?}"));
                }
            }

            fn drain(&mut self, retrying: bool) -> bool {
                self.draining = true;
                DRAIN.iter().any(|&step| !self.done && (retrying || !matches!(step, Step::Retry)) && self.run(step))
            }

            fn settled(&mut self, retrying: bool) {
                let mut drained = 0;
                while !self.done && self.drain(retrying) {
                    self.check();
                    drained += 1;
                    assert!(drained < 10_000, "seed {}: the host and page never settle\n{}", self.seed, self.log.join("\n"));
                }
            }

            fn waiting_unseen(&self) -> Vec<u64> {
                let shown: BTreeSet<u64> = match self.active() {
                    Some(guard) => self.docs[self.live].guards[guard].failed.keys().copied().collect(),
                    None => BTreeSet::new(),
                };
                let state = self.leaving.lock();
                self.posed
                    .iter()
                    .filter_map(|(_, posed)| state.questions.iter().find(|question| question.key == posed.0))
                    .filter(|question| question.verdict.is_none() && !question.asked.is_some_and(|id| shown.contains(&id)))
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
                    panic!("seed {}: the live, subscribed page wasn’t asked ({verdict:?})\n{}", self.seed, self.log.join("\n"));
                }
                let asked = self.batches.lock().unwrap().back().map(|&(doc, to, _)| (doc, to));
                assert_eq!(asked, Some((self.live, guard)), "seed {}: the live page isn’t the one asked\n{}", self.seed, self.log.join("\n"));
            }

            fn check(&self) {
                let doc = &self.docs[self.live];
                if !self.done && doc.alive && !doc.locked {
                    assert!(
                        self.leaving.lock().holds.is_empty(),
                        "seed {}: the page can be edited while BattlePlan holds a leave\n{}",
                        self.seed,
                        self.log.join("\n")
                    );
                }
            }
        }

        #[test]
        fn no_interleaving_loses_unsaved_work_or_leaves_a_question_unanswered() {
            for seed in 1..=4000 {
                let mut world = World::new(seed);
                for _ in 0..300 {
                    if world.done {
                        break;
                    }
                    world.step();
                    world.check();
                }
                world.settled(false);
                if world.done {
                    continue;
                }
                let unseen = world.waiting_unseen();
                assert!(unseen.is_empty(), "seed {seed}: setup waits on {unseen:?} with nothing for the user to retry\n{}", world.log.join("\n"));
                world.settled(true);
                if world.done {
                    continue;
                }
                assert!(world.posed.is_empty(), "seed {seed}: a question setup waits on was never settled\n{}", world.log.join("\n"));
                assert!(!world.closing, "seed {seed}: a close went unanswered with no choice offered\n{}", world.log.join("\n"));
                assert!(
                    world.leaving.lock().questions.iter().all(|question| question.verdict.is_some() || question.asked.is_none()),
                    "seed {seed}: the host waits on an answer no page will send\n{}",
                    world.log.join("\n")
                );
                if world.stuck() {
                    world.run(Step::CloseWindow);
                    world.settled(true);
                    if world.done {
                        continue;
                    }
                    assert!(!world.stuck(), "seed {seed}: the page stays locked though nothing stands, even after a close
{}", world.log.join("
"));
                }
                world.probe();
            }
        }
    }
}
