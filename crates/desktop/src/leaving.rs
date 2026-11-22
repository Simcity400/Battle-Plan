use serde::Serialize;
use std::sync::{Condvar, Mutex, MutexGuard, PoisonError};
use tauri::ipc::Channel;

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

    pub fn refusal(self) -> Option<&'static str> {
        match self {
            Self::Leaving => None,
            Self::Staying => Some(Self::STAYING),
            Self::Reloaded => Some(Self::RELOADED),
            Self::Unresponsive => Some(Self::UNRESPONSIVE),
            Self::Unheard => Some(Self::UNHEARD),
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Unanswered {
    Close,
    Unresponsive,
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
    Watched { seq: u64, requests: Channel<Request> },
}

struct State {
    page: Page,
    next: u64,
    questions: Vec<Question>,
    offering: bool,
}

impl State {
    fn fresh(&mut self) -> u64 {
        let id = self.next;
        self.next += 1;
        id
    }
}

pub struct Leaving {
    state: Mutex<State>,
    answered: Condvar,
}

impl Default for Leaving {
    fn default() -> Self {
        Self {
            state: Mutex::new(State { page: Page::Empty, next: 0, questions: Vec::new(), offering: false }),
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

    fn gone(&self, state: &mut State) {
        state.page = Page::Empty;
        state.questions.retain(|question| question.about != About::Close);
        self.settle(state, Verdict::Leaving);
    }

    fn send(&self, state: &mut State, request: Request) {
        let sent = match &state.page {
            Page::Watched { requests, .. } => requests.send(request),
            Page::Empty | Page::Started => return,
        };
        if let Err(error) = sent {
            eprintln!("BattlePlan’s page can’t be reached, so nothing it held can be saved: {error}");
            self.gone(state);
        }
    }

    fn ask_page(&self, state: &mut State, key: u64) {
        if !matches!(state.page, Page::Watched { .. }) {
            return;
        }
        let id = state.fresh();
        let Some(question) = state.questions.iter_mut().find(|question| question.key == key) else {
            return;
        };
        question.asked = Some(id);
        let about = question.about;
        self.send(state, Request::Ask { id, about });
    }

    pub fn watch(&self, seq: u64, requests: impl FnOnce() -> Channel<Request>) {
        let mut state = self.lock();
        if let Page::Watched { seq: current, .. } = &state.page
            && *current >= seq
        {
            return;
        }
        state.page = Page::Watched { seq, requests: requests() };
        let waiting: Vec<u64> = state.questions.iter().filter(|question| question.verdict.is_none()).map(|question| question.key).collect();
        for key in waiting {
            self.ask_page(&mut state, key);
        }
    }

    pub fn answer(&self, id: u64, stays: bool) -> Answered {
        let mut state = self.lock();
        let Some(at) = state.questions.iter().position(|question| question.asked == Some(id) && question.verdict.is_none()) else {
            return Answered::Unknown;
        };
        if state.questions[at].about == About::Close {
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
        if state.questions.iter().any(|question| question.about == About::Close) {
            if std::mem::replace(&mut state.offering, true) {
                return Closing::Held;
            }
            return Closing::Offer(Unanswered::Close);
        }
        let key = state.fresh();
        state.questions.push(Question { key, about: About::Close, asked: None, verdict: None });
        self.ask_page(&mut state, key);
        if matches!(state.page, Page::Empty) { Closing::Through } else { Closing::Held }
    }

    pub fn chosen(&self) {
        self.lock().offering = false;
    }

    pub fn page_started(&self) {
        let mut state = self.lock();
        state.page = Page::Started;
        for question in &mut state.questions {
            question.asked = None;
        }
        self.settle(&mut state, Verdict::Reloaded);
    }

    pub fn page_gone(&self) {
        let mut state = self.lock();
        self.gone(&mut state);
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
        self.ask_page(&mut state, key);
        Ok(Asked(key))
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

    fn leave_after(&self, asked: Result<Asked, Verdict>, launch: impl FnOnce() -> Result<(), String>) -> Result<(), String> {
        let (verdict, id) = self.wait(asked);
        if let Some(refused) = verdict.refusal() {
            return Err(refused.to_owned());
        }
        let launched = launch();
        if launched.is_err()
            && let Some(id) = id
        {
            let mut state = self.lock();
            self.send(&mut state, Request::Stay { id });
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
    use super::{About, Answered, Closing, Leaving, Request, Unanswered, Verdict};
    use serde_json::{Value, json};
    use std::{
        str::FromStr,
        sync::{Mutex, mpsc},
    };
    use tauri::{
        App, WebviewWindow, WebviewWindowBuilder,
        ipc::{Channel, InvokeResponseBody, JavaScriptChannelId},
        test::{MockRuntime, mock_builder, mock_context, noop_assets},
    };

    struct Page {
        _app: App<MockRuntime>,
        window: WebviewWindow<MockRuntime>,
        heard: mpsc::Receiver<(u32, usize, Value)>,
    }

    impl Page {
        fn new() -> Self {
            let (sent, heard) = mpsc::channel();
            let sent = Mutex::new(sent);
            let app = mock_builder()
                .channel_interceptor(move |_, callback, index, body| {
                    if let InvokeResponseBody::Json(json) = body {
                        let _ = sent.lock().unwrap().send((callback.0, index, serde_json::from_str(json).unwrap()));
                    }
                    true
                })
                .build(mock_context(noop_assets()))
                .unwrap();
            let window = WebviewWindowBuilder::new(&app, "main", Default::default()).build().unwrap();
            Self { _app: app, window, heard }
        }

        fn channel(&self, callback: u32) -> impl FnOnce() -> Channel<Request> + '_ {
            move || JavaScriptChannelId::from_str(&format!("__CHANNEL__:{callback}")).unwrap().channel_on(self.window.as_ref().clone())
        }

        fn next(&self) -> (u32, usize, Value) {
            self.heard.try_recv().expect("the page was sent nothing")
        }

        fn quiet(&self) -> bool {
            self.heard.try_recv().is_err()
        }
    }

    fn id(request: &Value) -> u64 {
        request["id"].as_u64().unwrap()
    }

    #[test]
    fn a_registration_sent_again_after_a_lost_reply_keeps_the_channel_it_made() {
        let page = Page::new();
        let leaving = Leaving::default();
        leaving.page_started();
        leaving.watch(1, page.channel(7));
        assert_eq!(leaving.close_requested(), Closing::Held);
        let (callback, index, close) = page.next();
        assert_eq!((callback, index, &close["about"]), (7, 0, &json!("close")));
        assert_eq!(leaving.answer(id(&close), true), Answered::Heard);

        leaving.watch(1, || unreachable!("a registration sent again builds no second channel for the same callback"));
        leaving.watch(0, || unreachable!("an older registration arriving late builds none either"));
        assert_eq!(leaving.close_requested(), Closing::Held);
        let (callback, index, _) = page.next();
        assert_eq!((callback, index), (7, 1), "the first channel goes on in order, so the page still hears it");
    }

    #[test]
    fn a_superseded_registrations_late_answer_does_not_settle_the_question_it_was_asked() {
        let page = Page::new();
        let leaving = Leaving::default();
        leaving.page_started();
        leaving.watch(1, page.channel(7));
        let asked = leaving.pose(About::Query);
        let (_, _, first) = page.next();
        leaving.watch(2, page.channel(8));
        let (callback, _, second) = page.next();
        assert_eq!(callback, 8, "the question is asked again of the newer registration");
        assert_ne!(id(&first), id(&second), "with a fresh id");
        assert_eq!(leaving.answer(id(&first), false), Answered::Unknown, "the superseded page’s in-flight Saved doesn’t count");
        assert_eq!(leaving.answer(id(&second), true), Answered::Heard);
        assert_eq!(leaving.wait(asked), (Verdict::Staying, Some(id(&second))));
    }

    #[test]
    fn a_close_is_asked_of_the_page_and_only_its_leave_answer_closes() {
        let page = Page::new();
        let leaving = Leaving::default();
        assert_eq!(leaving.close_requested(), Closing::Through, "before any page loads there is nothing to keep");
        leaving.page_started();
        leaving.watch(1, page.channel(7));
        assert_eq!(leaving.close_requested(), Closing::Held);
        let (_, _, unsaved) = page.next();
        assert_eq!(leaving.answer(id(&unsaved), true), Answered::Heard, "unsaved work keeps the window, and the page prompts");
        assert_eq!(leaving.answer(id(&unsaved), false), Answered::Unknown, "an answered close can’t be answered again");
        assert_eq!(leaving.close_requested(), Closing::Held);
        let (_, _, saved) = page.next();
        assert_eq!(leaving.answer(id(&saved), false), Answered::Leave);
    }

    #[test]
    fn a_page_that_started_but_has_not_subscribed_holds_closes_and_refuses_update_questions_by_name() {
        let page = Page::new();
        let leaving = Leaving::default();
        leaving.page_started();
        assert_eq!(leaving.close_for_update(), Err(Verdict::UNHEARD.to_owned()));
        assert_eq!(leaving.busy(), Some(Verdict::UNHEARD.to_owned()));
        assert_eq!(leaving.close_requested(), Closing::Held, "a page that never subscribes can’t lose work to a close");
        assert_eq!(leaving.close_requested(), Closing::Offer(Unanswered::Close), "the user chooses instead");
        assert_eq!(leaving.close_requested(), Closing::Held, "one choice at a time");
        leaving.chosen();
        leaving.watch(1, page.channel(7));
        let (_, _, close) = page.next();
        assert_eq!(close["about"], "close", "the held close is asked once the page subscribes");
        assert_eq!(leaving.answer(id(&close), false), Answered::Leave);
    }

    #[test]
    fn a_close_the_page_leaves_unanswered_offers_the_choice_at_the_next_close() {
        let page = Page::new();
        let leaving = Leaving::default();
        leaving.page_started();
        leaving.watch(1, page.channel(7));
        assert_eq!(leaving.close_requested(), Closing::Held);
        let (_, _, close) = page.next();
        assert_eq!(leaving.close_requested(), Closing::Offer(Unanswered::Close));
        assert!(page.quiet(), "the page isn’t asked twice");
        leaving.chosen();
        assert_eq!(leaving.close_requested(), Closing::Offer(Unanswered::Close), "keeping waiting lets the next close ask again");
        leaving.chosen();
        assert_eq!(leaving.answer(id(&close), true), Answered::Heard);
        assert_eq!(leaving.close_requested(), Closing::Held, "an answered close starts over");
    }

    #[test]
    fn an_unresponsive_page_refuses_the_questions_waiting_on_it_by_name_and_offers_one_choice() {
        let page = Page::new();
        let leaving = Leaving::default();
        leaving.page_started();
        leaving.watch(1, page.channel(7));
        let leave = leaving.pose(About::Leave);
        let query = leaving.pose(About::Query);
        assert_eq!(leaving.close_requested(), Closing::Held);
        let asks: Vec<Value> = (0..3).map(|_| page.next().2).collect();
        assert!(leaving.page_unresponsive(), "the user is offered to close");
        assert_eq!(leaving.leave_after(leave, || unreachable!("nothing launches")), Err(Verdict::UNRESPONSIVE.to_owned()));
        assert_eq!(leaving.wait(query).0.refusal(), Some(Verdict::UNRESPONSIVE));
        assert!(!leaving.page_unresponsive(), "WebView2 reports again while the first choice is open");
        leaving.chosen();
        assert_eq!(leaving.answer(id(&asks[2]), false), Answered::Leave, "the close stays for a page that recovers");
        assert!(leaving.page_unresponsive());
    }

    #[test]
    fn an_install_the_page_agreed_to_that_cannot_start_tells_the_page_to_stay() {
        let page = Page::new();
        let leaving = Leaving::default();
        leaving.page_started();
        leaving.watch(1, page.channel(7));
        let asked = leaving.pose(About::Leave);
        let (_, _, leave) = page.next();
        assert_eq!(leave["about"], "leave");
        assert_eq!(leaving.answer(id(&leave), false), Answered::Heard);
        let failed = "The installer couldn’t be started: Windows refused to open it (code 2).";
        assert_eq!(leaving.leave_after(asked, || Err(failed.into())), Err(failed.into()));
        assert_eq!(page.next().2, json!({ "kind": "stay", "id": id(&leave) }));

        let asked = leaving.pose(About::Leave);
        let (_, _, leave) = page.next();
        leaving.answer(id(&leave), false);
        assert_eq!(leaving.leave_after(asked, || Ok(())), Ok(()));
        assert!(page.quiet(), "a leave that happens says nothing more");

        let asked = leaving.pose(About::Leave);
        let (_, _, leave) = page.next();
        leaving.answer(id(&leave), true);
        assert_eq!(leaving.leave_after(asked, || unreachable!("unsaved work launches nothing")), Err(Verdict::STAYING.to_owned()));
    }

    #[test]
    fn a_reload_refuses_its_questions_by_name_and_a_dead_page_holds_nothing() {
        let page = Page::new();
        let leaving = Leaving::default();
        leaving.page_started();
        leaving.watch(1, page.channel(7));
        let asked = leaving.pose(About::Leave);
        page.next();
        leaving.page_started();
        assert_eq!(leaving.wait(asked).0.refusal(), Some(Verdict::RELOADED));

        leaving.watch(1, page.channel(8));
        let asked = leaving.pose(About::Query);
        assert_eq!(leaving.close_requested(), Closing::Held);
        let (_, _, query) = page.next();
        page.next();
        leaving.page_gone();
        assert_eq!(leaving.wait(asked).0, Verdict::Leaving, "nothing a dead page held can be saved");
        assert_eq!(leaving.answer(id(&query), true), Answered::Unknown);
        assert_eq!(leaving.close_requested(), Closing::Through, "the next close goes through");
        assert_eq!(leaving.close_for_update(), Ok(()));
    }
}
