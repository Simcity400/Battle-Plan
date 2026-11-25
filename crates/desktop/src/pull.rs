
use std::{
    collections::HashSet,
    sync::{Mutex, MutexGuard, PoisonError},
};
use tauri::{
    UriSchemeResponder,
    http::{HeaderValue, Request, Response, StatusCode, header},
};

pub trait Outbox: Default {
    fn take(&mut self) -> Vec<Vec<u8>>;
    fn restart(&mut self, session: &str);
}

pub struct Asks<O, R> {
    session: Option<String>,
    replaced: HashSet<String>,
    parked: Option<R>,
    pub outbox: O,
}

impl<O: Default, R> Default for Asks<O, R> {
    fn default() -> Self {
        Self { session: None, replaced: HashSet::new(), parked: None, outbox: O::default() }
    }
}

pub type Answer<R> = (R, Vec<u8>);

fn frame(payloads: Vec<Vec<u8>>) -> Vec<u8> {
    let mut body = Vec::with_capacity(payloads.iter().map(|payload| payload.len() + 4).sum());
    for payload in payloads {
        body.extend_from_slice(&(payload.len() as u32).to_le_bytes());
        body.extend_from_slice(&payload);
    }
    body
}

fn session_key(session: &str) -> Option<(&str, u64)> {
    let (page, seq) = session.rsplit_once(':')?;
    (!page.is_empty()).then_some((page, seq.parse::<u64>().ok()?))
}

fn page(session: &str) -> &str {
    session_key(session).map_or(session, |(page, _)| page)
}

fn is_older_session(incoming: &str, current: &str) -> bool {
    matches!((session_key(incoming), session_key(current)), (Some((p1, s1)), Some((p2, s2))) if p1 == p2 && s1 < s2)
}

impl<O: Outbox, R> Asks<O, R> {
    pub fn offered(&mut self) -> Option<Answer<R>> {
        self.parked.as_ref()?;
        let payloads = self.outbox.take();
        if payloads.is_empty() {
            return None;
        }
        Some((self.parked.take().expect("held"), frame(payloads)))
    }

    pub fn ask(&mut self, session: String, asker: R) -> Result<Vec<Answer<R>>, R> {
        if self.replaced.contains(page(&session)) || self.session.as_deref().is_some_and(|current| is_older_session(&session, current)) {
            return Err(asker);
        }
        let mut answers: Vec<Answer<R>> = self.parked.take().map(|replaced| (replaced, Vec::new())).into_iter().collect();
        if self.session.as_deref() != Some(session.as_str()) {
            if let Some(previous) = self.session.as_deref().map(page).filter(|&previous| previous != page(&session)) {
                self.replaced.insert(previous.to_owned());
            }
            self.outbox.restart(&session);
            self.session = Some(session);
            answers.push((asker, Vec::new()));
            return Ok(answers);
        }
        let payloads = self.outbox.take();
        if payloads.is_empty() {
            self.parked = Some(asker);
        } else {
            answers.push((asker, frame(payloads)));
        }
        Ok(answers)
    }
}

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
    headers.insert(header::ACCESS_CONTROL_ALLOW_ORIGIN, HeaderValue::from_static("*"));
    responder.respond(response);
}

pub fn refuse(responder: UriSchemeResponder, status: StatusCode, why: &str) {
    let mut response = Response::new(why.as_bytes().to_vec());
    *response.status_mut() = status;
    response.headers_mut().insert(header::ACCESS_CONTROL_ALLOW_ORIGIN, HeaderValue::from_static("*"));
    responder.respond(response);
}

fn hex_nibble(byte: u8) -> Option<u8> {
    match byte {
        b'0'..=b'9' => Some(byte - b'0'),
        b'a'..=b'f' => Some(byte - b'a' + 10),
        b'A'..=b'F' => Some(byte - b'A' + 10),
        _ => None,
    }
}

fn percent_decode(raw: &str) -> Option<String> {
    if !raw.as_bytes().contains(&b'%') {
        return Some(raw.to_owned());
    }
    let bytes = raw.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'%' {
            let hi = hex_nibble(*bytes.get(i + 1)?)?;
            let lo = hex_nibble(*bytes.get(i + 2)?)?;
            out.push((hi << 4) | lo);
            i += 3;
        } else {
            out.push(bytes[i]);
            i += 1;
        }
    }
    String::from_utf8(out).ok()
}

pub fn param(request: &Request<Vec<u8>>, name: &str) -> Option<String> {
    let raw = request.uri().query()?.split('&').find_map(|pair| pair.strip_prefix(name)?.strip_prefix('=')).filter(|value| !value.is_empty())?;
    percent_decode(raw).filter(|value| !value.is_empty())
}

fn session(request: &Request<Vec<u8>>) -> Option<String> {
    param(request, "session")
}

impl<O: Outbox> Pull<O> {
    fn asks(&self) -> MutexGuard<'_, Asks<O, UriSchemeResponder>> {
        self.asks.lock().unwrap_or_else(PoisonError::into_inner)
    }

    pub fn outbox<T>(&self, read: impl FnOnce(&O) -> T) -> T {
        read(&self.asks().outbox)
    }

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

    pub fn ask(&self, request: &Request<Vec<u8>>, responder: UriSchemeResponder) {
        let Some(session) = session(request) else {
            return refuse(responder, StatusCode::BAD_REQUEST, "An ask names its session");
        };
        match self.asks().ask(session, responder) {
            Ok(answers) => {
                for answer in answers {
                    respond(answer);
                }
            }
            Err(stale) => refuse(stale, StatusCode::CONFLICT, "Superseded follow session"),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{Asks, Outbox};

    #[derive(Default)]
    struct Queue {
        held: Vec<Vec<u8>>,
        restarts: usize,
    }

    impl Outbox for Queue {
        fn take(&mut self) -> Vec<Vec<u8>> {
            std::mem::take(&mut self.held)
        }
        fn restart(&mut self, _session: &str) {
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
        assert_eq!(asks.ask("a".into(), 0), Ok(vec![(0, Vec::new())]), "the page's first ask: answered empty at once");
        assert_eq!(asks.ask("a".into(), 1), Ok(Vec::new()), "nothing waits: held");
        assert!(asks.offered().is_none(), "nothing put: still held");
        asks.outbox.held.push(vec![0; 3]);
        let (asker, body) = asks.offered().expect("the held ask is answered");
        assert_eq!((asker, lengths(&body)), (1, vec![3]));
    }

    #[test]
    fn what_comes_while_the_page_handles_an_answer_waits_for_its_next_ask_in_one_answer() {
        let mut asks = Asks::<Queue, u32>::default();
        assert!(asks.ask("a".into(), 0).is_ok());
        assert!(asks.ask("a".into(), 1).is_ok());
        asks.outbox.held.push(vec![1]);
        asks.offered().expect("answered");
        for length in 2..6 {
            asks.outbox.held.push(vec![0; length]);
            assert!(asks.offered().is_none(), "no ask is held while the page handles the answer");
        }
        let answers = asks.ask("a".into(), 2).expect("same session accepted");
        assert_eq!(answers.len(), 1);
        assert_eq!(lengths(&answers[0].1), [2, 3, 4, 5], "everything waiting, in order");
        assert_eq!(asks.ask("a".into(), 3), Ok(Vec::new()), "then nothing waits: held");
    }

    #[test]
    fn a_new_page_restarts_the_stream_at_its_first_ask_and_the_ask_held_for_the_old_one_is_answered_empty() {
        let mut asks = Asks::<Queue, u32>::default();
        assert!(asks.ask("old".into(), 0).is_ok());
        assert_eq!(asks.ask("old".into(), 1), Ok(Vec::new()));
        asks.outbox.held.push(vec![7]);
        let answers = asks.ask("new".into(), 2).expect("new session accepted");
        assert_eq!(answers, vec![(1, Vec::new()), (2, Vec::new())], "the old page's ask ends empty; the new page's first is answered empty at once");
        assert_eq!(asks.outbox.restarts, 2, "each page's first ask restarts the stream");
        assert_eq!(asks.ask("new".into(), 3), Ok(Vec::new()), "what was held for the old page never reaches the new one");
        asks.outbox.held.push(vec![9]);
        assert_eq!(asks.offered().map(|(asker, _)| asker), Some(3), "the new page's ask is held and answered");
    }

    #[test]
    fn an_older_session_arriving_after_a_newer_one_changes_nothing() {
        let mut asks = Asks::<Queue, u32>::default();
        assert_eq!(asks.ask("page1:2".into(), 0), Ok(vec![(0, Vec::new())]));
        assert_eq!(asks.ask("page1:2".into(), 1), Ok(Vec::new()));
        assert_eq!(asks.ask("page1:1".into(), 99), Err(99), "retired session is ignored");
        assert_eq!(asks.outbox.restarts, 1, "retired session does not restart the host outbox");
        asks.outbox.held.push(vec![0; 4]);
        let (asker, body) = asks.offered().expect("the newer session's parked ask stays held and receives payloads");
        assert_eq!((asker, lengths(&body)), (1, vec![4]));
        assert_eq!(asks.ask("page2:1".into(), 2), Ok(vec![(2, Vec::new())]), "a reloaded page can still take over");
        assert_eq!(asks.outbox.restarts, 2);
        assert_eq!(asks.ask("page2:1".into(), 3), Ok(Vec::new()));
        assert_eq!(asks.ask("page1:3".into(), 4), Err(4), "a late ask from the replaced page is refused, whatever its sequence");
        assert_eq!(asks.outbox.restarts, 2, "and never restarts the stream");
        asks.outbox.held.push(vec![0; 5]);
        assert_eq!(asks.offered().map(|(asker, body)| (asker, lengths(&body))), Some((3, vec![5])), "the live page keeps its held ask and its stream");
    }

    #[test]
    fn percent_encoded_session_param_decodes_colon_for_registry_and_ordering() {
        let req = tauri::http::Request::builder()
            .uri("http://push.localhost/follow?session=page-id%3A2&released=64")
            .body(Vec::new())
            .unwrap();
        let decoded = super::param(&req, "session").expect("decoded session");
        assert_eq!(decoded, "page-id:2");
        assert_eq!(super::param(&req, "released").as_deref(), Some("64"));
        let old = tauri::http::Request::builder()
            .uri("http://push.localhost/follow?session=page-id%3A1")
            .body(Vec::new())
            .unwrap();
        let mut asks = Asks::<Queue, u32>::default();
        assert_eq!(asks.ask(decoded, 0), Ok(vec![(0, Vec::new())]));
        assert_eq!(asks.ask(super::param(&old, "session").unwrap(), 1), Err(1));
    }
}
