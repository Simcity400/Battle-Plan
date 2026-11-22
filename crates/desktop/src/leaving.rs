use serde::Serialize;
use std::collections::BTreeSet;
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
    pub seq: u64,
    pub waiting: Vec<Ask>,
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
    Blind,
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

    pub fn refusal(self) -> Option<&'static str> {
        match self {
            Self::Leaving => None,
            Self::Staying => Some(Self::STAYING),
            Self::Reloaded => Some(Self::RELOADED),
            Self::Unresponsive => Some(Self::UNRESPONSIVE),
            Self::Unheard => Some(Self::UNHEARD),
            Self::Unreached => Some(Self::UNREACHED),
            Self::Blind => Some(Self::BLIND),
        }
    }
}

pub const REPLACED: &str = "This page was replaced before it could watch for BattlePlan closing.";

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

struct State {
    page: Page,
    blind: bool,
    document: u64,
    navigations: BTreeSet<u64>,
    next: u64,
    questions: Vec<Question>,
    holds: Vec<Ask>,
    sent: Vec<(u64, Vec<u64>)>,
    outbox: Vec<(Reach, Batch)>,
    offering: bool,
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
        self.holds.iter().any(|hold| hold.about == About::Close)
    }

    fn askable(&self) -> bool {
        !self.blind && matches!(self.page, Page::Watched { .. }) && self.navigations.is_empty()
    }

    fn send(&mut self) {
        let Page::Watched { reach, .. } = &self.page else {
            return;
        };
        let reach = Arc::clone(reach);
        if !self.askable() && self.holds.is_empty() {
            return;
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
        self.outbox.push((reach, Batch { seq, waiting, holds: self.holds.clone() }));
    }

    fn exit(&mut self) {
        if self.exiting() {
            return;
        }
        let id = self.fresh();
        self.holds.push(Ask { id, about: About::Close });
        self.send();
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

    fn unasked(&self) -> bool {
        self.questions.iter().any(|question| question.verdict.is_none() && question.asked.is_none())
    }
}

pub struct Leaving {
    state: Mutex<State>,
    answered: Condvar,
}

impl Default for Leaving {
    fn default() -> Self {
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
        self.answered.notify_all();
        for (reach, batch) in outbox {
            reach(batch);
        }
        value
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

    pub fn watch(&self, document: u64, seq: u64, reach: impl FnOnce() -> Reach) -> Result<(), &'static str> {
        self.act(|state| {
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

    pub fn delivered(&self, seq: u64, reached: bool) -> Option<Unanswered> {
        self.act(|state| {
            let at = state.sent.iter().position(|(sent, _)| *sent == seq)?;
            let (_, ids) = state.sent.remove(at);
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
            if leaves {
                state.holds.push(Ask { id, about });
            }
            if about == About::Close {
                state.questions.remove(at);
                return if stays { Answered::Heard } else { Answered::Leave };
            }
            state.questions[at].verdict = Some(if stays { Verdict::Staying } else { Verdict::Leaving });
            Answered::Heard
        })
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
            if state.blind {
                return state.offer(Unanswered::Blind);
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
            state.page = Page::Loading;
            state.sent.clear();
            Self::settle(state, Verdict::Reloaded);
            !state.holds.is_empty()
        })
    }

    pub fn navigation_completed(&self, navigation: u64) {
        self.act(|state| {
            state.navigations.remove(&navigation);
            if state.unasked() || !state.holds.is_empty() {
                state.send();
            }
        });
    }

    pub fn page_gone(&self) -> bool {
        self.act(|state| {
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
            !std::mem::replace(&mut state.offering, true)
        })
    }

    fn pose(&self, about: About) -> Result<Posed, Verdict> {
        self.act(|state| {
            if state.exiting() {
                return Err(Verdict::Leaving);
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
            state.send();
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
        let leaving = Leaving::default();
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
        let leaving = Leaving::default();
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
        let leaving = Leaving::default();
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
        let leaving = Arc::new(Leaving::default());
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
        assert_eq!((batch.waiting, batch.holds), (vec![Ask { id: close, about: About::Close }], vec![]), "the close carries the stay that never ran");
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
        let leaving = Leaving::default();
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
        let leaving = Leaving::default();
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
        page.hears(&leaving);
        page.hears(&leaving);
        let (_, batch) = page.hears(&leaving);
        let (install, close) = (waiting(&batch, About::Install), waiting(&batch, About::Close));
        assert!(leaving.page_unresponsive(), "the user is offered to close");
        assert_eq!(leaving.leave_after(leave, || unreachable!("nothing launches")), Err(Verdict::UNRESPONSIVE.to_owned()));
        assert_eq!(leaving.wait(query).0.refusal(), Some(Verdict::UNRESPONSIVE));
        assert!(!leaving.page_unresponsive(), "WebView2 reports again while the first choice is open");
        leaving.chosen();
        assert_eq!(leaving.answer(install, false, &[]), Answered::Unknown, "a refused update can’t be agreed to later");
        assert_eq!(leaving.answer(close, false, &[]), Answered::Leave, "the close stays for a page that recovers");
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
    fn a_reload_already_past_its_veto_when_battleplan_leaves_is_stopped_and_the_page_it_leaves_is_locked() {
        let page = Page::default();
        let leaving = Leaving::default();
        watched(&leaving, &page, 7);
        assert!(leaving.navigation_starting(1), "a slow reload starts while nothing stands");
        assert_eq!(leaving.close_requested(), Closing::Held);
        assert_eq!(leaving.close_requested(), Closing::Offer(Unanswered::Close));
        leaving.chosen();
        leaving.exit();
        let (_, batch) = page.hears(&leaving);
        assert!(matches!(batch.holds[..], [Ask { about: About::Close, .. }]), "the page the reload is leaving is locked: {batch:?}");
        assert!(leaving.content_loading(1), "the document the reload began loading is stopped");
        leaving.navigation_completed(1);
        assert_eq!(leaving.close_requested(), Closing::Through);
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
    fn navigations_are_followed_by_identity_so_one_loading_never_settles_another() {
        let page = Page::default();
        let leaving = Leaving::default();
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
        let leaving = Leaving::default();
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
        use super::super::{About, Answered, Ask, Batch, Closing, Leaving, Posed, Reach, Verdict};
        use serde_json::{Value, json};
        use std::collections::{BTreeMap, BTreeSet, VecDeque};
        use std::sync::{Arc, Mutex, Weak};

        const SEEDS: u64 = 4000;
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
            committed: BTreeMap<u64, (bool, bool)>,
            judged: BTreeSet<u64>,
            waiting: Vec<Ask>,
            live: BTreeSet<u64>,
            failures: BTreeMap<String, Failure>,
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

        #[derive(Clone, Copy, Debug, PartialEq, Eq)]
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

        const AFTER_EXIT: [Step; 13] = [
            Step::Edit,
            Step::CloseWindow,
            Step::RunBatch,
            Step::DropBatch,
            Step::Answer,
            Step::Reply,
            Step::Retry,
            Step::Remount,
            Step::Register,
            Step::Reload,
            Step::ReloadLoads,
            Step::ReloadStops,
            Step::PageExits,
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

        #[derive(Default)]
        struct Wire {
            batches: Vec<(usize, usize, Batch)>,
            refused: Vec<bool>,
            last: Option<(usize, usize)>,
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
            exited: bool,
            trace: Vec<Value>,
            shapes: BTreeSet<String>,
            guards: usize,
            emitted: Vec<Value>,
            prompted: usize,
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
                "b" => (format!("{} {} {} {}", event[2], event[7], event[5].as_array().is_some_and(|asks| !asks.is_empty()), event[6].as_array().is_some_and(|asks| !asks.is_empty())), &event[8]),
                "r" => (format!("{} {}", event[2], event[5]), &event[6]),
                "t" => (format!("{} {}", event[2], event[4].as_str().is_some_and(|key| key.starts_with("answer:"))), &event[5]),
                "w" => (event[2].to_string(), &event[3]),
                "l" => (String::new(), &event[2]),
                _ => return kind.to_owned(),
            };
            let answers = out[0].as_array().map_or(&[][..], Vec::as_slice);
            let leaves = answers.iter().any(|answer| answer[1] == false);
            let based = answers.iter().any(|answer| answer[2].as_array().is_some_and(|basis| !basis.is_empty()));
            let failing = out[3].as_array().map(|failures| !failures.is_empty());
            format!("{kind} {facts} | {} {leaves} {based} {} {} {failing:?}", answers.len().min(3), out[1], out[2].as_u64().is_some_and(|prompts| prompts > 0))
        }

        fn asks(asks: &[Ask]) -> Value {
            Value::Array(asks.iter().map(|ask| json!([ask.id, about(ask.about)])).collect())
        }

        impl World {
            fn new(seed: u64) -> Self {
                let mut world = Self {
                    leaving: Arc::default(),
                    seed,
                    rng: Rng(seed.wrapping_mul(0x9E37_79B9_7F4A_7C15) | 1),
                    log: Vec::new(),
                    docs: Vec::new(),
                    live: 0,
                    wire: Arc::default(),
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
                    exited: false,
                    trace: Vec::new(),
                    shapes: BTreeSet::new(),
                    guards: 0,
                    emitted: Vec::new(),
                    prompted: 0,
                };
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
            }

            fn out(&mut self, doc: usize, guard: usize) -> Value {
                let state = &self.docs[doc].guards[guard];
                let locked = match state.shown.flatten() {
                    None => 0,
                    Some(false) => 1,
                    Some(true) => 2,
                };
                let failures = if state.ended { Value::Null } else { json!(state.failures.keys().collect::<Vec<_>>()) };
                json!([std::mem::take(&mut self.emitted), locked, self.prompted, failures])
            }

            fn facts(&self, doc: usize) -> (bool, bool) {
                (self.docs[doc].dirty, self.docs[doc].confirmed)
            }

            fn load(&mut self) {
                self.docs.push(Doc { alive: true, ..Doc::default() });
                self.live = self.docs.len() - 1;
                self.mount();
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
                doc.alive && !doc.locked && !doc.confirmed
            }

            fn show(&mut self, doc: usize, guard: usize) {
                let doc = &mut self.docs[doc];
                let state = &mut doc.guards[guard];
                let now = if state.committed.values().any(|&(update, _)| update) {
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

            fn judge(&mut self, doc: usize, guard: usize) {
                let (dirty, confirmed) = self.facts(doc);
                for ask in self.docs[doc].guards[guard].waiting.clone() {
                    let state = &mut self.docs[doc].guards[guard];
                    if state.ended {
                        return;
                    }
                    if state.judged.contains(&ask.id) {
                        continue;
                    }
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
            }

            fn hear(&mut self, doc: usize, guard: usize, batch: &Batch) -> bool {
                if !self.docs[doc].alive {
                    return false;
                }
                self.begin();
                let returned = self.apply(doc, guard, batch);
                let (dirty, confirmed) = self.facts(doc);
                let trace = self.docs[doc].guards[guard].trace;
                let out = self.out(doc, guard);
                self.record(json!(["b", trace, dirty, confirmed, batch.seq, asks(&batch.waiting), asks(&batch.holds), returned, out]));
                returned
            }

            fn apply(&mut self, doc: usize, guard: usize, batch: &Batch) -> bool {
                let state = &mut self.docs[doc].guards[guard];
                if state.ended {
                    return false;
                }
                if state.applied.is_some_and(|applied| applied >= batch.seq) {
                    self.log.push(format!("  doc {doc} guard {guard} passes over {}", batch.seq));
                    return true;
                }
                self.log.push(format!("  doc {doc} guard {guard} hears {batch:?}"));
                state.applied = Some(batch.seq);
                state.live = batch.waiting.iter().chain(&batch.holds).map(|ask| ask.id).collect();
                let live = state.live.clone();
                state.committed.retain(|id, _| live.contains(id));
                state.judged.retain(|id| live.contains(id));
                state.failures.retain(|key, _| key.strip_prefix("answer:").is_none_or(|id| live.contains(&id.parse().unwrap())));
                for hold in &batch.holds {
                    state.committed.entry(hold.id).and_modify(|(_, accepted)| *accepted = true).or_insert((hold.about != About::Close, true));
                }
                state.waiting.clone_from(&batch.waiting);
                self.show(doc, guard);
                self.judge(doc, guard);
                true
            }

            fn reply(&mut self, doc: usize, guard: usize, id: u64, reply: Option<bool>, sent: (About, bool, Vec<u64>)) {
                let state = &mut self.docs[doc].guards[guard];
                if state.ended {
                    return;
                }
                let Some(heard) = reply else {
                    if state.live.contains(&id) {
                        state.failures.insert(format!("answer:{id}"), Failure::Answer(sent.0, sent.1, sent.2));
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
                self.judge(doc, guard);
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
                        if let Pending::Install(confirmed) = pending {
                            assert_ne!(verdict, Verdict::Leaving, "{}", self.fail("an install went ahead without its page"));
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
                let ran = self.act(step);
                let refused = std::mem::take(&mut self.wire.lock().unwrap().refused);
                for offer in refused {
                    self.offered(offer);
                }
                ran
            }

            fn act(&mut self, step: Step) -> bool {
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
                        let ran = matches!(step, Step::RunBatch) && self.hear(doc, guard, &batch);
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
                        let Some((key, failure)) = self.docs[live].guards[guard].failures.iter().next().map(|(key, failure)| (key.clone(), failure.clone())) else {
                            return false;
                        };
                        self.begin();
                        let (dirty, confirmed) = self.facts(live);
                        match failure {
                            Failure::Watch => self.watch(live, guard),
                            Failure::Answer(about, stays, basis) => {
                                let id = key.strip_prefix("answer:").and_then(|id| id.parse().ok()).expect("an answer failure names its id");
                                self.send(live, guard, id, about, stays, basis);
                            }
                        }
                        let trace = self.docs[live].guards[guard].trace;
                        let out = self.out(live, guard);
                        self.record(json!(["t", trace, dirty, confirmed, key, out]));
                    }
                    Step::Remount if alive => self.mount(),
                    Step::Register | Step::RegistrationLost => {
                        let Some((doc, guard, document)) = self.registrations.pop_front() else { return false };
                        if !self.docs[doc].alive {
                            return true;
                        }
                        let lost = matches!(step, Step::RegistrationLost);
                        let watched = match document {
                            None if !lost => {
                                self.registrations.push_back((doc, guard, Some(self.leaving.document())));
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
                            state.failures.insert("watch".into(), Failure::Watch);
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
                        let blocked = state.committed.iter().any(|(id, &(_, accepted))| !accepted && shown.contains(id));
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
                let doc = &self.docs[self.live];
                if !self.exited && doc.alive && !doc.locked {
                    assert!(self.leaving.lock().holds.is_empty(), "{}", self.fail("the page can be edited while BattlePlan holds a leave"));
                }
            }

            fn after_exit(&mut self) {
                let steps: Vec<(Step, usize)> = AFTER_EXIT.iter().map(|&step| (step, 1)).collect();
                self.draining = false;
                for _ in 0..40 {
                    self.step(&steps);
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
                assert!(self.posed.is_empty(), "{}", self.fail("a question setup waits on was never settled"));
                assert!(!self.closing, "{}", self.fail("a close went unanswered with no choice offered"));
                assert!(
                    self.leaving.lock().questions.iter().all(|question| question.verdict.is_some() || question.asked.is_none()),
                    "{}",
                    self.fail("the host waits on an answer no page will send")
                );
                if self.stuck() {
                    self.run(Step::CloseWindow);
                    self.settled(true);
                    if self.exited {
                        return self.after_exit();
                    }
                    assert!(!self.stuck(), "{}", self.fail("the page stays locked though nothing stands, even after a close"));
                }
                self.probe();
            }
        }

        #[test]
        fn no_interleaving_loses_unsaved_work_or_leaves_a_question_unanswered() {
            let mut traces = Vec::new();
            let mut covered = BTreeSet::new();
            for seed in 1..=SEEDS {
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
