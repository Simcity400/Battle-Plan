use serde::Serialize;
use std::sync::{Arc, Condvar, Mutex, MutexGuard, PoisonError};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum About {
    Close,
    Leave,
    Query,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(tag = "kind", rename_all = "lowercase")]
pub enum Request {
    Ask { id: u64, about: About },
    Stay { id: u64 },
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Batch {
    pub token: u64,
    pub requests: Vec<Request>,
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

pub struct Asked(u64);

struct Question {
    key: u64,
    about: About,
    asked: Option<u64>,
    verdict: Option<Verdict>,
}

struct Sent {
    token: u64,
    stays: Vec<u64>,
    asks: Vec<(u64, u64)>,
}

enum Page {
    Empty,
    Started,
    Watched { seq: u64, reach: Reach },
}

struct State {
    page: Page,
    document: u64,
    next: u64,
    questions: Vec<Question>,
    accepted: Vec<u64>,
    stays: Vec<u64>,
    sent: Vec<Sent>,
    outbox: Vec<(Reach, Batch)>,
    offering: bool,
}

impl State {
    fn fresh(&mut self) -> u64 {
        let id = self.next;
        self.next += 1;
        id
    }

    fn send(&mut self, keys: &[u64]) {
        let Page::Watched { reach, .. } = &self.page else {
            return;
        };
        let reach = Arc::clone(reach);
        let mut requests: Vec<Request> = self.stays.iter().map(|&id| Request::Stay { id }).collect();
        let mut asks = Vec::new();
        for &key in keys {
            let id = self.fresh();
            if let Some(question) = self.questions.iter_mut().find(|question| question.key == key) {
                question.asked = Some(id);
                requests.push(Request::Ask { id, about: question.about });
                asks.push((key, id));
            }
        }
        if requests.is_empty() {
            return;
        }
        let token = self.fresh();
        self.sent.push(Sent { token, stays: self.stays.clone(), asks });
        self.outbox.push((reach, Batch { token, requests }));
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
    answered: Condvar,
}

impl Default for Leaving {
    fn default() -> Self {
        Self {
            state: Mutex::new(State {
                page: Page::Empty,
                document: 0,
                next: 0,
                questions: Vec::new(),
                accepted: Vec::new(),
                stays: Vec::new(),
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
        for (reach, batch) in outbox {
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

    fn start(&self, state: &mut State) {
        state.document += 1;
        state.page = Page::Started;
        state.stays.clear();
        state.sent.clear();
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
            if document != state.document {
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
            state.stays.retain(|id| !sent.stays.contains(id));
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

    pub fn answer(&self, id: u64, stays: bool) -> Answered {
        let mut state = self.lock();
        let Some(at) = state.questions.iter().position(|question| question.asked == Some(id) && question.verdict.is_none()) else {
            return if !stays && state.accepted.contains(&id) { Answered::Heard } else { Answered::Unknown };
        };
        let about = state.questions[at].about;
        if !stays && about != About::Query {
            state.accepted.push(id);
        }
        if about == About::Close {
            state.questions.remove(at);
            return if stays { Answered::Heard } else { Answered::Leave };
        }
        state.questions[at].verdict = Some(if stays { Verdict::Staying } else { Verdict::Leaving });
        self.answered.notify_all();
        Answered::Heard
    }

    pub fn close_requested(&self) -> Closing {
        self.act(|state| {
            if matches!(state.page, Page::Empty) {
                return Closing::Through;
            }
            let watched = matches!(state.page, Page::Watched { .. });
            if let Some(close) = state.questions.iter().find(|question| question.about == About::Close) {
                let (key, asked) = (close.key, close.asked.is_some());
                if asked || !watched {
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

    pub fn navigating(&self) -> bool {
        let mut state = self.lock();
        if !state.accepted.is_empty() {
            return false;
        }
        if !matches!(state.page, Page::Empty) {
            self.start(&mut state);
        }
        true
    }

    pub fn page_started(&self) {
        let mut state = self.lock();
        self.start(&mut state);
    }

    pub fn page_gone(&self) {
        let mut state = self.lock();
        state.page = Page::Empty;
        state.accepted.clear();
        state.stays.clear();
        state.sent.clear();
        state.questions.retain(|question| question.about != About::Close);
        self.settle(&mut state, Verdict::Leaving);
    }

    pub fn page_unresponsive(&self) -> bool {
        let mut state = self.lock();
        self.settle(&mut state, Verdict::Unresponsive);
        !std::mem::replace(&mut state.offering, true)
    }

    fn pose(&self, about: About) -> Result<Asked, Verdict> {
        self.act(|state| {
            match state.page {
                Page::Empty => return Err(Verdict::Leaving),
                Page::Started => return Err(Verdict::Unheard),
                Page::Watched { .. } => {}
            }
            let key = state.fresh();
            state.questions.push(Question { key, about, asked: None, verdict: None });
            state.send(&[key]);
            Ok(Asked(key))
        })
    }

    fn wait(&self, asked: Result<Asked, Verdict>) -> (Verdict, Option<u64>) {
        let Asked(key) = match asked {
            Ok(asked) => asked,
            Err(verdict) => return (verdict, None),
        };
        let mut state = self.lock();
        loop {
            let Some(at) = state.questions.iter().position(|question| question.key == key) else {
                return (Verdict::Unheard, None);
            };
            if let Some(verdict) = state.questions[at].verdict {
                let question = state.questions.remove(at);
                return (verdict, question.asked);
            }
            state = self.answered.wait(state).unwrap_or_else(PoisonError::into_inner);
        }
    }

    fn stay(&self, id: u64) {
        self.act(|state| {
            let Some(at) = state.accepted.iter().position(|&accepted| accepted == id) else {
                return;
            };
            state.accepted.remove(at);
            state.stays.push(id);
            let asked = state.keys(|question| question.asked.is_some());
            state.send(&asked);
        });
    }

    fn leave_after(&self, asked: Result<Asked, Verdict>, launch: impl FnOnce() -> Result<(), String>) -> Result<(), String> {
        let (verdict, id) = self.wait(asked);
        if let Some(refused) = verdict.refusal() {
            return Err(refused.to_owned());
        }
        let launched = launch();
        if launched.is_err()
            && let Some(id) = id
        {
            self.stay(id);
        }
        launched
    }

    pub fn install(&self, launch: impl FnOnce() -> Result<(), String>) -> Result<(), String> {
        self.leave_after(self.pose(About::Leave), launch)
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
    use super::{About, Answered, Batch, Closing, Leaving, REPLACED, Reach, Request, Unanswered, Verdict};
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

        fn hears(&self, leaving: &Leaving) -> (u32, Vec<Request>) {
            let (callback, batch) = self.dispatched();
            assert_eq!(leaving.delivered(batch.token, true), None);
            (callback, batch.requests)
        }

        fn misses(&self, leaving: &Leaving) -> (Vec<Request>, Option<Unanswered>) {
            let (_, batch) = self.dispatched();
            let offer = leaving.delivered(batch.token, false);
            (batch.requests, offer)
        }
    }

    fn asked(requests: &[Request]) -> (u64, About) {
        match requests {
            [Request::Ask { id, about }] => (*id, *about),
            other => panic!("expected one question, got {other:?}"),
        }
    }

    fn heard(leaving: &Leaving, page: &Page) -> (u32, u64, About) {
        let (callback, requests) = page.hears(leaving);
        let (id, about) = asked(&requests);
        (callback, id, about)
    }

    fn watched(leaving: &Leaving, page: &Page, callback: u32) {
        leaving.page_started();
        assert_eq!(leaving.watch(leaving.document(), 1, page.reach(callback)), Ok(()));
    }

    #[test]
    fn a_registration_sent_again_after_a_lost_reply_keeps_the_subscription_it_made() {
        let page = Page::default();
        let leaving = Leaving::default();
        watched(&leaving, &page, 7);
        let document = leaving.document();
        assert_eq!(leaving.watch(document, 1, || unreachable!("a registration sent again replaces nothing")), Ok(()));
        assert_eq!(leaving.watch(document, 0, || unreachable!("an older registration arriving late replaces nothing")), Ok(()));
        assert_eq!(leaving.close_requested(), Closing::Held);
        assert_eq!(heard(&leaving, &page).0, 7);
    }

    #[test]
    fn a_registration_from_the_page_a_reload_replaced_is_refused_so_the_new_page_subscribes() {
        let page = Page::default();
        let leaving = Leaving::default();
        leaving.page_started();
        let outgoing = leaving.document();
        leaving.page_started();
        let current = leaving.document();
        assert_ne!(outgoing, current);
        assert_eq!(
            leaving.watch(outgoing, 3, || unreachable!("the outgoing page’s late registration subscribes nothing")),
            Err(REPLACED)
        );
        assert_eq!(leaving.watch(current, 1, page.reach(8)), Ok(()), "the new page’s first registration wins");
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
        let (callback, second, _) = heard(&leaving, &page);
        assert_eq!(callback, 8, "the question is asked again of the newer registration");
        assert_ne!(first, second, "with a fresh id");
        assert_eq!(leaving.answer(first, false), Answered::Unknown, "the superseded page’s in-flight Saved doesn’t count");
        assert_eq!(leaving.answer(second, true), Answered::Heard);
        assert_eq!(leaving.wait(asking), (Verdict::Staying, Some(second)));
    }

    #[test]
    fn a_leave_answer_sent_again_after_a_lost_reply_stands_until_the_host_says_stay() {
        let page = Page::default();
        let leaving = Leaving::default();
        watched(&leaving, &page, 7);
        let asking = leaving.pose(About::Leave);
        let (_, id, _) = heard(&leaving, &page);
        assert_eq!(leaving.answer(id, false), Answered::Heard);
        let (verdict, answered) = leaving.wait(asking);
        assert_eq!((verdict, answered), (Verdict::Leaving, Some(id)));
        assert_eq!(leaving.answer(id, false), Answered::Heard, "the retry keeps the page locked while the install starts");
        assert_eq!(leaving.answer(id, true), Answered::Unknown, "an agreed leave can’t be taken back");

        leaving.stay(id);
        assert_eq!(page.hears(&leaving).1, [Request::Stay { id }]);
        assert_eq!(leaving.answer(id, false), Answered::Unknown, "after Stay the page goes on");

        assert_eq!(leaving.close_requested(), Closing::Held);
        let (_, close, _) = heard(&leaving, &page);
        assert_eq!(leaving.answer(close, false), Answered::Leave);
        assert_eq!(leaving.answer(close, false), Answered::Heard, "a close the host accepted is still accepted, without leaving twice");
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
        assert_eq!(leaving.answer(close, true), Answered::Heard);
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
        assert_eq!(leaving.answer(id, false), Answered::Heard, "the question the newer page heard still stands");
        assert_eq!(leaving.wait(asking).0, Verdict::Leaving);
    }

    #[test]
    fn a_close_held_while_the_page_starts_offers_the_choice_when_its_registration_cant_be_reached() {
        let page = Page::default();
        let leaving = Leaving::default();
        leaving.page_started();
        assert_eq!(leaving.close_requested(), Closing::Held);
        assert_eq!(leaving.watch(leaving.document(), 1, page.reach(7)), Ok(()));
        let (requests, offer) = page.misses(&leaving);
        assert_eq!(asked(&requests).1, About::Close);
        assert_eq!(offer, Some(Unanswered::Unreached), "the held close isn’t left waiting on a page that never heard it");
    }

    #[test]
    fn every_question_carries_the_stays_the_page_has_not_confirmed_before_it() {
        let page = Page::default();
        let leaving = Leaving::default();
        watched(&leaving, &page, 7);
        let asking = leaving.pose(About::Leave);
        let (_, id, _) = heard(&leaving, &page);
        leaving.answer(id, false);
        let failed = "The installer couldn’t be started: Windows refused to open it (code 2).";
        assert_eq!(leaving.leave_after(asking, || Err(failed.into())), Err(failed.into()));
        let (_, in_flight) = page.dispatched();
        assert_eq!(in_flight.requests, [Request::Stay { id }]);

        assert_eq!(leaving.close_requested(), Closing::Held);
        let (requests, _) = page.misses(&leaving);
        assert!(matches!(requests[..], [Request::Stay { id: stay }, Request::Ask { about: About::Close, .. }] if stay == id),
            "a Stay not yet confirmed goes before the close, in one operation: {requests:?}");
        leaving.chosen();
        assert_eq!(leaving.delivered(in_flight.token, false), None);
        assert_eq!(leaving.close_requested(), Closing::Held);
        let (_, requests) = page.hears(&leaving);
        assert!(matches!(requests[..], [Request::Stay { id: stay }, Request::Ask { about: About::Close, .. }] if stay == id),
            "a Stay that never ran goes with the next thing the page is sent: {requests:?}");

        assert_eq!(leaving.close_requested(), Closing::Offer(Unanswered::Close));
        leaving.chosen();
        let asking = leaving.pose(About::Query);
        let (_, query, _) = heard(&leaving, &page);
        assert_eq!(leaving.answer(query, true), Answered::Heard, "once confirmed, the Stay goes no further");
        assert_eq!(leaving.wait(asking).0, Verdict::Staying);
    }

    #[test]
    fn a_stay_asks_again_what_the_page_may_have_judged_while_still_committed() {
        let page = Page::default();
        let leaving = Leaving::default();
        watched(&leaving, &page, 7);
        let asking = leaving.pose(About::Leave);
        let (_, id, _) = heard(&leaving, &page);
        leaving.answer(id, false);
        let (verdict, _) = leaving.wait(asking);
        assert_eq!(verdict, Verdict::Leaving);
        assert_eq!(leaving.close_requested(), Closing::Held);
        let (_, committed, _) = heard(&leaving, &page);

        leaving.stay(id);
        let (_, requests) = page.hears(&leaving);
        let [Request::Stay { id: stay }, Request::Ask { id: close, about: About::Close }] = requests[..] else {
            panic!("expected the Stay and then the close asked again, got {requests:?}");
        };
        assert_eq!(stay, id);
        assert_ne!(close, committed);
        assert_eq!(leaving.answer(committed, false), Answered::Unknown, "a close judged before the Stay doesn’t close");
        assert_eq!(leaving.answer(close, true), Answered::Heard, "the page judges it again after the Stay");
    }

    #[test]
    fn a_close_is_asked_of_the_page_and_only_its_leave_answer_closes() {
        let page = Page::default();
        let leaving = Leaving::default();
        assert_eq!(leaving.close_requested(), Closing::Through, "before any page loads there is nothing to keep");
        watched(&leaving, &page, 7);
        assert_eq!(leaving.close_requested(), Closing::Held);
        let (_, unsaved, _) = heard(&leaving, &page);
        assert_eq!(leaving.answer(unsaved, true), Answered::Heard, "unsaved work keeps the window, and the page prompts");
        assert_eq!(leaving.answer(unsaved, false), Answered::Unknown, "an answered close can’t be answered again");
        assert_eq!(leaving.close_requested(), Closing::Held);
        let (_, saved, _) = heard(&leaving, &page);
        assert_eq!(leaving.answer(saved, false), Answered::Leave);
    }

    #[test]
    fn a_page_that_started_but_has_not_subscribed_holds_closes_and_refuses_update_questions_by_name() {
        let page = Page::default();
        let leaving = Leaving::default();
        leaving.page_started();
        assert_eq!(leaving.close_for_update(), Err(Verdict::UNHEARD.to_owned()));
        assert_eq!(leaving.busy(), Some(Verdict::UNHEARD.to_owned()));
        assert_eq!(leaving.close_requested(), Closing::Held, "a page that never subscribes can’t lose work to a close");
        assert_eq!(leaving.close_requested(), Closing::Offer(Unanswered::Close), "the user chooses instead");
        assert_eq!(leaving.close_requested(), Closing::Held, "one choice at a time");
        leaving.chosen();
        assert_eq!(leaving.watch(leaving.document(), 1, page.reach(7)), Ok(()));
        let (_, close, about) = heard(&leaving, &page);
        assert_eq!(about, About::Close, "the held close is asked once the page subscribes");
        assert_eq!(leaving.answer(close, false), Answered::Leave);
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
        assert_eq!(leaving.answer(close, true), Answered::Heard);
        assert_eq!(leaving.close_requested(), Closing::Held, "an answered close starts over");
    }

    #[test]
    fn an_unresponsive_page_refuses_the_questions_waiting_on_it_by_name_and_offers_one_choice() {
        let page = Page::default();
        let leaving = Leaving::default();
        watched(&leaving, &page, 7);
        let leave = leaving.pose(About::Leave);
        let query = leaving.pose(About::Query);
        assert_eq!(leaving.close_requested(), Closing::Held);
        let asks: Vec<u64> = (0..3).map(|_| heard(&leaving, &page).1).collect();
        assert!(leaving.page_unresponsive(), "the user is offered to close");
        assert_eq!(leaving.leave_after(leave, || unreachable!("nothing launches")), Err(Verdict::UNRESPONSIVE.to_owned()));
        assert_eq!(leaving.wait(query).0.refusal(), Some(Verdict::UNRESPONSIVE));
        assert!(!leaving.page_unresponsive(), "WebView2 reports again while the first choice is open");
        leaving.chosen();
        assert_eq!(leaving.answer(asks[2], false), Answered::Leave, "the close stays for a page that recovers");
        assert!(leaving.page_unresponsive());
    }

    #[test]
    fn an_install_the_page_agreed_to_that_cannot_start_tells_the_page_to_stay() {
        let page = Page::default();
        let leaving = Leaving::default();
        watched(&leaving, &page, 7);
        let asking = leaving.pose(About::Leave);
        let (_, id, about) = heard(&leaving, &page);
        assert_eq!(about, About::Leave);
        assert_eq!(leaving.answer(id, false), Answered::Heard);
        let failed = "The installer couldn’t be started: Windows refused to open it (code 2).";
        assert_eq!(leaving.leave_after(asking, || Err(failed.into())), Err(failed.into()));
        assert_eq!(page.hears(&leaving).1, [Request::Stay { id }]);

        let asking = leaving.pose(About::Leave);
        let (_, id, _) = heard(&leaving, &page);
        leaving.answer(id, false);
        assert_eq!(leaving.leave_after(asking, || Ok(())), Ok(()));
        assert!(page.quiet(), "a leave that happens says nothing more");

        let asking = leaving.pose(About::Leave);
        let (_, id, _) = heard(&leaving, &page);
        leaving.answer(id, true);
        assert_eq!(leaving.leave_after(asking, || unreachable!("unsaved work launches nothing")), Err(Verdict::STAYING.to_owned()));
    }

    #[test]
    fn a_reload_is_refused_while_a_leave_stands_and_otherwise_replaces_the_page_at_once() {
        let page = Page::default();
        let leaving = Leaving::default();
        assert!(leaving.navigating(), "the first page loads");
        watched(&leaving, &page, 7);
        let asking = leaving.pose(About::Leave);
        let (_, id, _) = heard(&leaving, &page);
        leaving.answer(id, false);
        assert_eq!(leaving.wait(asking).0, Verdict::Leaving);
        let document = leaving.document();
        assert!(!leaving.navigating(), "F5 can’t unlock a page while its installer starts");
        assert_eq!(leaving.document(), document);
        assert_eq!(leaving.answer(id, false), Answered::Heard, "the leave still stands");

        leaving.stay(id);
        page.hears(&leaving);
        let asking = leaving.pose(About::Leave);
        let (_, late, _) = heard(&leaving, &page);
        assert!(leaving.navigating());
        assert_ne!(leaving.document(), document, "the outgoing page can no longer subscribe");
        assert_eq!(leaving.wait(asking).0.refusal(), Some(Verdict::RELOADED));
        assert_eq!(leaving.answer(late, false), Answered::Unknown, "nor commit the new page to leaving");
        assert!(leaving.navigating());
    }

    #[test]
    fn a_reload_refuses_its_questions_by_name_and_a_dead_page_holds_nothing() {
        let page = Page::default();
        let leaving = Leaving::default();
        watched(&leaving, &page, 7);
        let asking = leaving.pose(About::Leave);
        let (_, id, _) = heard(&leaving, &page);
        leaving.page_started();
        assert_eq!(leaving.wait(asking).0.refusal(), Some(Verdict::RELOADED));
        assert_eq!(leaving.answer(id, false), Answered::Unknown, "the replaced page’s answer settles nothing");

        assert_eq!(leaving.watch(leaving.document(), 1, page.reach(8)), Ok(()));
        let asking = leaving.pose(About::Query);
        assert_eq!(leaving.close_requested(), Closing::Held);
        let (_, query, _) = heard(&leaving, &page);
        page.hears(&leaving);
        leaving.page_gone();
        assert_eq!(leaving.wait(asking).0, Verdict::Leaving, "nothing a dead page held can be saved");
        assert_eq!(leaving.answer(query, true), Answered::Unknown);
        assert_eq!(leaving.close_requested(), Closing::Through, "the next close goes through");
        assert_eq!(leaving.close_for_update(), Ok(()));
    }
}
