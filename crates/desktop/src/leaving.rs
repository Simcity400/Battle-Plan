use serde::Serialize;
use std::sync::{Condvar, Mutex, MutexGuard, PoisonError};

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

pub type Reach = Box<dyn Fn(&Request) -> Result<(), String> + Send + Sync>;

pub struct Asked(u64);

struct Question {
    key: u64,
    about: About,
    asked: Option<u64>,
    verdict: Option<Verdict>,
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
    unsent: Vec<u64>,
    offering: bool,
}

impl State {
    fn fresh(&mut self) -> u64 {
        let id = self.next;
        self.next += 1;
        id
    }

    fn send(&self, request: &Request) -> bool {
        let Page::Watched { reach, .. } = &self.page else {
            return false;
        };
        match reach(request) {
            Ok(()) => true,
            Err(error) => {
                eprintln!("BattlePlan couldn’t pass {request:?} to its page: {error}");
                false
            }
        }
    }

    fn flush(&mut self) {
        let unsent = std::mem::take(&mut self.unsent);
        self.unsent = unsent.into_iter().filter(|&id| !self.send(&Request::Stay { id })).collect();
    }

    fn ask(&mut self, key: u64) -> bool {
        if !matches!(self.page, Page::Watched { .. }) {
            return false;
        }
        self.flush();
        let id = self.fresh();
        let Some(question) = self.questions.iter_mut().find(|question| question.key == key) else {
            return false;
        };
        question.asked = Some(id);
        let about = question.about;
        if self.send(&Request::Ask { id, about }) {
            return true;
        }
        if let Some(question) = self.questions.iter_mut().find(|question| question.key == key) {
            question.asked = None;
        }
        false
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
        Self {
            state: Mutex::new(State {
                page: Page::Empty,
                document: 0,
                next: 0,
                questions: Vec::new(),
                accepted: Vec::new(),
                unsent: Vec::new(),
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

    fn settle(&self, state: &mut State, verdict: Verdict) {
        for question in &mut state.questions {
            if question.about != About::Close && question.verdict.is_none() {
                question.verdict = Some(verdict);
            }
        }
        self.answered.notify_all();
    }

    pub fn document(&self) -> u64 {
        self.lock().document
    }

    pub fn watch(&self, document: u64, seq: u64, reach: impl FnOnce() -> Reach) -> Result<(), &'static str> {
        let mut state = self.lock();
        if document != state.document {
            return Err(REPLACED);
        }
        if let Page::Watched { seq: current, .. } = &state.page
            && *current >= seq
        {
            return Ok(());
        }
        state.page = Page::Watched { seq, reach: reach() };
        state.unsent.clear();
        let waiting: Vec<(u64, About)> = state
            .questions
            .iter()
            .filter(|question| question.verdict.is_none())
            .map(|question| (question.key, question.about))
            .collect();
        for (key, about) in waiting {
            if !state.ask(key)
                && about != About::Close
                && let Some(question) = state.questions.iter_mut().find(|question| question.key == key)
            {
                question.verdict = Some(Verdict::Unreached);
                self.answered.notify_all();
            }
        }
        Ok(())
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
        let mut state = self.lock();
        if matches!(state.page, Page::Empty) {
            return Closing::Through;
        }
        if let Some(close) = state.questions.iter().find(|question| question.about == About::Close) {
            let (key, delivered) = (close.key, close.asked.is_some());
            if delivered || !matches!(state.page, Page::Watched { .. }) {
                return state.offer(Unanswered::Close);
            }
            return if state.ask(key) { Closing::Held } else { state.offer(Unanswered::Unreached) };
        }
        let key = state.fresh();
        state.questions.push(Question { key, about: About::Close, asked: None, verdict: None });
        if matches!(state.page, Page::Watched { .. }) && !state.ask(key) {
            return state.offer(Unanswered::Unreached);
        }
        Closing::Held
    }

    pub fn chosen(&self) {
        self.lock().offering = false;
    }

    pub fn page_started(&self) {
        let mut state = self.lock();
        state.document += 1;
        state.page = Page::Started;
        state.accepted.clear();
        state.unsent.clear();
        for question in &mut state.questions {
            question.asked = None;
        }
        self.settle(&mut state, Verdict::Reloaded);
    }

    pub fn page_gone(&self) {
        let mut state = self.lock();
        state.page = Page::Empty;
        state.accepted.clear();
        state.unsent.clear();
        state.questions.retain(|question| question.about != About::Close);
        self.settle(&mut state, Verdict::Leaving);
    }

    pub fn page_unresponsive(&self) -> bool {
        let mut state = self.lock();
        self.settle(&mut state, Verdict::Unresponsive);
        !std::mem::replace(&mut state.offering, true)
    }

    fn pose(&self, about: About) -> Result<Asked, Verdict> {
        let mut state = self.lock();
        match state.page {
            Page::Empty => return Err(Verdict::Leaving),
            Page::Started => return Err(Verdict::Unheard),
            Page::Watched { .. } => {}
        }
        let key = state.fresh();
        state.questions.push(Question { key, about, asked: None, verdict: None });
        if state.ask(key) {
            return Ok(Asked(key));
        }
        state.questions.retain(|question| question.key != key);
        Err(Verdict::Unreached)
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
        let mut state = self.lock();
        let Some(at) = state.accepted.iter().position(|&accepted| accepted == id) else {
            return;
        };
        state.accepted.remove(at);
        state.unsent.push(id);
        state.flush();
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
    use super::{About, Answered, Closing, Leaving, REPLACED, Reach, Request, Unanswered, Verdict};
    use std::sync::{
        Arc, Mutex,
        atomic::{AtomicBool, Ordering},
    };

    #[derive(Clone, Default)]
    struct Page {
        heard: Arc<Mutex<Vec<(u32, Request)>>>,
        full: Arc<AtomicBool>,
    }

    impl Page {
        fn reach(&self, callback: u32) -> impl FnOnce() -> Reach {
            let page = self.clone();
            move || {
                Box::new(move |request: &Request| {
                    if page.full.load(Ordering::SeqCst) {
                        return Err("the window’s message queue is full".into());
                    }
                    page.heard.lock().unwrap().push((callback, *request));
                    Ok(())
                })
            }
        }

        fn next(&self) -> (u32, Request) {
            let mut heard = self.heard.lock().unwrap();
            assert!(!heard.is_empty(), "the page was sent nothing");
            heard.remove(0)
        }

        fn quiet(&self) -> bool {
            self.heard.lock().unwrap().is_empty()
        }

        fn queue_full(&self, full: bool) {
            self.full.store(full, Ordering::SeqCst);
        }
    }

    fn asked(request: (u32, Request)) -> (u32, u64, About) {
        match request {
            (callback, Request::Ask { id, about }) => (callback, id, about),
            other => panic!("expected a question, got {other:?}"),
        }
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
        assert_eq!(asked(page.next()).0, 7);
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
        assert_eq!(asked(page.next()).0, 8, "its closes are asked of the new page");
    }

    #[test]
    fn a_superseded_registrations_late_answer_does_not_settle_the_question_it_was_asked() {
        let page = Page::default();
        let leaving = Leaving::default();
        watched(&leaving, &page, 7);
        let asking = leaving.pose(About::Query);
        let (_, first, _) = asked(page.next());
        assert_eq!(leaving.watch(leaving.document(), 2, page.reach(8)), Ok(()));
        let (callback, second, _) = asked(page.next());
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
        let (_, id, _) = asked(page.next());
        assert_eq!(leaving.answer(id, false), Answered::Heard);
        let (verdict, answered) = leaving.wait(asking);
        assert_eq!((verdict, answered), (Verdict::Leaving, Some(id)));
        assert_eq!(leaving.answer(id, false), Answered::Heard, "the retry keeps the page locked while the install starts");
        assert_eq!(leaving.answer(id, true), Answered::Unknown, "an agreed leave can’t be taken back");

        leaving.stay(id);
        assert_eq!(page.next().1, Request::Stay { id });
        assert_eq!(leaving.answer(id, false), Answered::Unknown, "after Stay the page goes on");

        assert_eq!(leaving.close_requested(), Closing::Held);
        let (_, close, _) = asked(page.next());
        assert_eq!(leaving.answer(close, false), Answered::Leave);
        assert_eq!(leaving.answer(close, false), Answered::Heard, "a close the host accepted is still accepted, without leaving twice");
    }

    #[test]
    fn a_question_the_page_cant_be_sent_is_refused_by_name_and_a_live_page_is_not_taken_for_gone() {
        let page = Page::default();
        let leaving = Leaving::default();
        watched(&leaving, &page, 7);
        page.queue_full(true);
        assert_eq!(leaving.close_for_update(), Err(Verdict::UNREACHED.to_owned()), "setup is refused, never let through");
        assert_eq!(leaving.busy(), Some(Verdict::UNREACHED.to_owned()));
        assert_eq!(leaving.close_requested(), Closing::Offer(Unanswered::Unreached), "the user chooses instead");
        leaving.chosen();
        page.queue_full(false);
        assert_eq!(leaving.close_requested(), Closing::Held, "the next close asks the page again");
        let (_, close, about) = asked(page.next());
        assert_eq!(about, About::Close);
        assert_eq!(leaving.answer(close, true), Answered::Heard);
        assert!(page.quiet());
    }

    #[test]
    fn a_stay_the_page_cant_be_sent_goes_with_the_next_thing_it_is_sent() {
        let page = Page::default();
        let leaving = Leaving::default();
        watched(&leaving, &page, 7);
        let asking = leaving.pose(About::Leave);
        let (_, id, _) = asked(page.next());
        leaving.answer(id, false);
        page.queue_full(true);
        let failed = "The installer couldn’t be started: Windows refused to open it (code 2).";
        assert_eq!(leaving.leave_after(asking, || Err(failed.into())), Err(failed.into()));
        assert!(page.quiet());
        page.queue_full(false);
        assert_eq!(leaving.close_requested(), Closing::Held);
        assert_eq!(page.next().1, Request::Stay { id }, "the page hears Stay first, so it can judge the close");
        assert_eq!(asked(page.next()).2, About::Close);
    }

    #[test]
    fn a_close_is_asked_of_the_page_and_only_its_leave_answer_closes() {
        let page = Page::default();
        let leaving = Leaving::default();
        assert_eq!(leaving.close_requested(), Closing::Through, "before any page loads there is nothing to keep");
        watched(&leaving, &page, 7);
        assert_eq!(leaving.close_requested(), Closing::Held);
        let (_, unsaved, _) = asked(page.next());
        assert_eq!(leaving.answer(unsaved, true), Answered::Heard, "unsaved work keeps the window, and the page prompts");
        assert_eq!(leaving.answer(unsaved, false), Answered::Unknown, "an answered close can’t be answered again");
        assert_eq!(leaving.close_requested(), Closing::Held);
        let (_, saved, _) = asked(page.next());
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
        let (_, close, about) = asked(page.next());
        assert_eq!(about, About::Close, "the held close is asked once the page subscribes");
        assert_eq!(leaving.answer(close, false), Answered::Leave);
    }

    #[test]
    fn a_close_the_page_leaves_unanswered_offers_the_choice_at_the_next_close() {
        let page = Page::default();
        let leaving = Leaving::default();
        watched(&leaving, &page, 7);
        assert_eq!(leaving.close_requested(), Closing::Held);
        let (_, close, _) = asked(page.next());
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
        let asks: Vec<u64> = (0..3).map(|_| asked(page.next()).1).collect();
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
        let (_, id, about) = asked(page.next());
        assert_eq!(about, About::Leave);
        assert_eq!(leaving.answer(id, false), Answered::Heard);
        let failed = "The installer couldn’t be started: Windows refused to open it (code 2).";
        assert_eq!(leaving.leave_after(asking, || Err(failed.into())), Err(failed.into()));
        assert_eq!(page.next().1, Request::Stay { id });

        let asking = leaving.pose(About::Leave);
        let (_, id, _) = asked(page.next());
        leaving.answer(id, false);
        assert_eq!(leaving.leave_after(asking, || Ok(())), Ok(()));
        assert!(page.quiet(), "a leave that happens says nothing more");

        let asking = leaving.pose(About::Leave);
        let (_, id, _) = asked(page.next());
        leaving.answer(id, true);
        assert_eq!(leaving.leave_after(asking, || unreachable!("unsaved work launches nothing")), Err(Verdict::STAYING.to_owned()));
    }

    #[test]
    fn a_reload_refuses_its_questions_by_name_and_a_dead_page_holds_nothing() {
        let page = Page::default();
        let leaving = Leaving::default();
        watched(&leaving, &page, 7);
        let asking = leaving.pose(About::Leave);
        let (_, id, _) = asked(page.next());
        leaving.page_started();
        assert_eq!(leaving.wait(asking).0.refusal(), Some(Verdict::RELOADED));
        assert_eq!(leaving.answer(id, false), Answered::Unknown, "the replaced page’s answer settles nothing");

        assert_eq!(leaving.watch(leaving.document(), 1, page.reach(8)), Ok(()));
        let asking = leaving.pose(About::Query);
        assert_eq!(leaving.close_requested(), Closing::Held);
        let (_, query, _) = asked(page.next());
        page.next();
        leaving.page_gone();
        assert_eq!(leaving.wait(asking).0, Verdict::Leaving, "nothing a dead page held can be saved");
        assert_eq!(leaving.answer(query, true), Answered::Unknown);
        assert_eq!(leaving.close_requested(), Closing::Through, "the next close goes through");
        assert_eq!(leaving.close_for_update(), Ok(()));
    }
}
