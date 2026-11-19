//! Dated follow payloads framed for the dashboard data worker.

use std::sync::{Mutex, MutexGuard, PoisonError};
use tauri::{
    UriSchemeResponder,
    http::{HeaderValue, Request, Response, StatusCode, header},
};

/// What a stream holds for the page between its asks.
pub trait Outbox: Default {
    /// Everything waiting, as payloads; none when nothing waits.
    fn take(&mut self) -> Vec<Vec<u8>>;
    /// A new page reads: what is held for the one before is dropped or kept, as the stream rules.
    fn restart(&mut self);
}

/// A stream's asks: one page at a time (its session), its ask held while nothing waits.
pub struct Asks<O, R> {
    session: Option<String>,
    parked: Option<R>,
    pub outbox: O,
}

impl<O: Default, R> Default for Asks<O, R> {
    fn default() -> Self {
        Self { session: None, parked: None, outbox: O::default() }
    }
}

/// An ask answered: who asked, and the answer's body (empty: replaced).
pub type Answer<R> = (R, Vec<u8>);

fn frame(payloads: Vec<Vec<u8>>) -> Vec<u8> {
    let mut body = Vec::with_capacity(payloads.iter().map(|payload| payload.len() + 4).sum());
    for payload in payloads {
        body.extend_from_slice(&(payload.len() as u32).to_le_bytes());
        body.extend_from_slice(&payload);
    }
    body
}

impl<O: Outbox, R> Asks<O, R> {
    /// The outbox changed: the ask held, if one is, is answered with what waits.
    pub fn offered(&mut self) -> Option<Answer<R>> {
        self.parked.as_ref()?;
        let payloads = self.outbox.take();
        if payloads.is_empty() {
            return None;
        }
        Some((self.parked.take().expect("held"), frame(payloads)))
    }

    /// An ask of `session`, the answer before it handled. A new session's first restarts the stream and is
    /// answered empty at once; a later one now with what waits, or held. An ask held before it is answered empty.
    pub fn ask(&mut self, session: String, asker: R) -> Vec<Answer<R>> {
        let mut answers: Vec<Answer<R>> = self.parked.take().map(|replaced| (replaced, Vec::new())).into_iter().collect();
        if self.session.as_deref() != Some(session.as_str()) {
            self.session = Some(session);
            self.outbox.restart();
            answers.push((asker, Vec::new()));
            return answers;
        }
        let payloads = self.outbox.take();
        if payloads.is_empty() {
            self.parked = Some(asker);
        } else {
            answers.push((asker, frame(payloads)));
        }
        answers
    }
}

/// A stream of the `push` protocol: its asks, behind a lock.
pub struct Pull<O> {
    asks: Mutex<Asks<O, UriSchemeResponder>>,
}

impl<O: Default> Default for Pull<O> {
    fn default() -> Self {
        Self { asks: Mutex::new(Asks::default()) }
    }
}

fn respond((responder, body): Answer<UriSchemeResponder>) {
    let mut response = Response::new(body);
    let headers = response.headers_mut();
    headers.insert(header::CONTENT_TYPE, HeaderValue::from_static("application/octet-stream"));
    headers.insert(header::CACHE_CONTROL, HeaderValue::from_static("no-store"));
    // The page is served from another origin than this protocol's.
    headers.insert(header::ACCESS_CONTROL_ALLOW_ORIGIN, HeaderValue::from_static("*"));
    responder.respond(response);
}

/// `responder` refused with `status` and why.
pub fn refuse(responder: UriSchemeResponder, status: StatusCode, why: &str) {
    let mut response = Response::new(why.as_bytes().to_vec());
    *response.status_mut() = status;
    response.headers_mut().insert(header::ACCESS_CONTROL_ALLOW_ORIGIN, HeaderValue::from_static("*"));
    responder.respond(response);
}

/// The value an ask names for `name` (`?name=value`), if it is not empty.
pub fn param<'a>(request: &'a Request<Vec<u8>>, name: &str) -> Option<&'a str> {
    request.uri().query()?.split('&').find_map(|pair| pair.strip_prefix(name)?.strip_prefix('=')).filter(|value| !value.is_empty())
}

/// The session an ask names (`?session=`).
fn session(request: &Request<Vec<u8>>) -> Option<String> {
    param(request, "session").map(str::to_owned)
}

impl<O: Outbox> Pull<O> {
    fn asks(&self) -> MutexGuard<'_, Asks<O, UriSchemeResponder>> {
        self.asks.lock().unwrap_or_else(PoisonError::into_inner)
    }

    /// `put` into the outbox, and the held ask answered with it.
    pub fn offer(&self, put: impl FnOnce(&mut O)) {
        let answer = {
            let mut asks = self.asks();
            put(&mut asks.outbox);
            asks.offered()
        };
        if let Some(answer) = answer {
            respond(answer);
        }
    }

    /// An ask of the page's reader.
    pub fn ask(&self, request: &Request<Vec<u8>>, responder: UriSchemeResponder) {
        let Some(session) = session(request) else {
            return refuse(responder, StatusCode::BAD_REQUEST, "An ask names its session");
        };
        let answers = self.asks().ask(session, responder);
        for answer in answers {
            respond(answer);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{Asks, Outbox};

    /// Payloads in order, each kept as it came; dropped for a new page.
    #[derive(Default)]
    struct Queue {
        held: Vec<Vec<u8>>,
        restarts: usize,
    }

    impl Outbox for Queue {
        fn take(&mut self) -> Vec<Vec<u8>> {
            std::mem::take(&mut self.held)
        }
        fn restart(&mut self) {
            self.held.clear();
            self.restarts += 1;
        }
    }

    fn lengths(body: &[u8]) -> Vec<usize> {
        let mut at = 0;
        let mut lengths = Vec::new();
        while at < body.len() {
            let length = u32::from_le_bytes(body[at..at + 4].try_into().unwrap()) as usize;
            lengths.push(length);
            at += 4 + length;
        }
        lengths
    }

    #[test]
    fn an_ask_is_held_until_something_comes_and_answered_with_everything_waiting_then() {
        let mut asks = Asks::<Queue, u32>::default();
        assert_eq!(asks.ask("a".into(), 0), vec![(0, Vec::new())], "the page's first ask: answered empty at once");
        assert!(asks.ask("a".into(), 1).is_empty(), "nothing waits: held");
        assert!(asks.offered().is_none(), "nothing put: still held");
        asks.outbox.held.push(vec![0; 3]);
        let (asker, body) = asks.offered().expect("the held ask is answered");
        assert_eq!((asker, lengths(&body)), (1, vec![3]));
    }

    #[test]
    fn what_comes_while_the_page_handles_an_answer_waits_for_its_next_ask_in_one_answer() {
        let mut asks = Asks::<Queue, u32>::default();
        asks.ask("a".into(), 0);
        asks.ask("a".into(), 1);
        asks.outbox.held.push(vec![1]);
        asks.offered().expect("answered");
        for length in 2..6 {
            asks.outbox.held.push(vec![0; length]);
            assert!(asks.offered().is_none(), "no ask is held while the page handles the answer");
        }
        let answers = asks.ask("a".into(), 2);
        assert_eq!(answers.len(), 1);
        assert_eq!(lengths(&answers[0].1), [2, 3, 4, 5], "everything waiting, in order");
        assert!(asks.ask("a".into(), 3).is_empty(), "then nothing waits: held");
    }

    #[test]
    fn a_new_page_restarts_the_stream_at_its_first_ask_and_the_ask_held_for_the_old_one_is_answered_empty() {
        let mut asks = Asks::<Queue, u32>::default();
        asks.ask("old".into(), 0);
        assert!(asks.ask("old".into(), 1).is_empty());
        asks.outbox.held.push(vec![7]);
        let answers = asks.ask("new".into(), 2);
        assert_eq!(answers, vec![(1, Vec::new()), (2, Vec::new())], "the old page's ask ends empty; the new page's first is answered empty at once");
        assert_eq!(asks.outbox.restarts, 2, "each page's first ask restarts the stream");
        assert!(asks.ask("new".into(), 3).is_empty(), "what was held for the old page never reaches the new one");
        asks.outbox.held.push(vec![9]);
        assert_eq!(asks.offered().map(|(asker, _)| asker), Some(3), "the new page's ask is held and answered");
    }
}
