use crate::pull::{Outbox, frame, param, refuse, respond};
use dashboard_follow::panel::wire::PanelBodies;
use std::collections::VecDeque;
use tauri::{UriSchemeResponder, http::{Request, StatusCode}};

pub type FollowPush = crate::pull::Pull<Relay>;
pub type ExportFollowPush = crate::pull::Pull<ExportRelay>;

#[derive(Default)]
pub struct ExportRelay(Relay);

impl ExportRelay {
    pub fn offer(&mut self, payload: Vec<u8>, epoch: u64) {
        self.0.offer(payload, epoch);
    }

    pub fn epoch_of(&self, session: &str) -> Option<u64> {
        self.0.epoch_of(session)
    }
}

impl crate::pull::Outbox for ExportRelay {
    fn take(&mut self) -> Vec<Vec<u8>> { self.0.take() }
    fn restart(&mut self, session: &str) {
        let epoch = dashboard_follow::registry::export_registry().restart(session);
        self.0.begin(session, epoch);
    }
}

#[derive(Default)]
pub struct Relay {
    held: Vec<Vec<u8>>,
    session: Option<(String, u64)>,
    panel_bodies: PanelBodies,
    batches: VecDeque<(u32, Vec<Vec<u8>>)>,
    serial: u32,
}

impl Relay {
    pub fn offer(&mut self, payload: Vec<u8>, epoch: u64) {
        if self.session.as_ref().is_some_and(|(_, held)| *held == epoch) { self.held.push(payload); }
    }

    pub fn epoch_of(&self, session: &str) -> Option<u64> {
        self.session.as_ref().filter(|(held, _)| held == session).map(|&(_, epoch)| epoch)
    }

    fn begin(&mut self, session: &str, epoch: u64) {
        self.held.clear();
        self.session = Some((session.to_owned(), epoch));
        self.panel_bodies = PanelBodies::default();
        self.batches.clear();
        self.serial = 0;
    }

    fn whole(&self, session: &str, serial: u32) -> Option<Vec<u8>> {
        self.epoch_of(session)?;
        let (_, payloads) = self.batches.iter().find(|(id, _)| *id == serial)?;
        Some(frame(payloads))
    }
}

impl Outbox for Relay {
    fn take(&mut self) -> Vec<Vec<u8>> { std::mem::take(&mut self.held) }
    fn take_body(&mut self) -> Vec<u8> {
        let packets = self.take();
        let patches: Vec<_> = packets.iter().map(|packet| self.panel_bodies.encode(packet)).collect();
        let savings: usize = patches.iter().zip(&packets).map(|(patch, packet)| patch.as_ref().map_or(0, |patch| packet.len() - patch.len())).sum();
        if savings <= 12 { return frame(&packets); }
        self.serial = self.serial.wrapping_add(1);
        let mut marker = *b"PWB1\0\0\0\0";
        marker[4..].copy_from_slice(&self.serial.to_le_bytes());
        let payloads: Vec<&[u8]> = std::iter::once(marker.as_slice())
            .chain(patches.iter().zip(&packets).map(|(patch, packet)| patch.as_deref().unwrap_or(packet))).collect();
        let body = frame(&payloads);
        if self.batches.len() == 2 { self.batches.pop_front(); }
        self.batches.push_back((self.serial, packets));
        body
    }
    fn restart(&mut self, session: &str) {
        let epoch = dashboard_follow::registry::registry().restart(session);
        self.begin(session, epoch);
    }
}

pub fn repair(push: &FollowPush, request: &Request<Vec<u8>>, serial: u32, responder: UriSchemeResponder) {
    let Some(session) = param(request, "session") else {
        return refuse(responder, StatusCode::BAD_REQUEST, "A full panel batch names its session");
    };
    match push.outbox(|relay| relay.whole(&session, serial)) {
        Some(body) => respond((responder, body)),
        None => refuse(responder, StatusCode::GONE, "The full panel batch is no longer outstanding"),
    }
}

#[cfg(test)]
mod tests {
    use super::Relay;
    use crate::pull::Outbox;
    use dashboard_follow::panel::dated;

    #[test]
    fn the_relay_hands_on_only_what_its_session_was_sent_and_names_that_session_for_credit() {
        let mut relay = Relay::default();
        relay.offer(vec![0], 0);
        assert!(relay.take().is_empty(), "nothing is handed on before a page asks");
        relay.begin("a", 1);
        relay.offer(vec![1], 1);
        relay.offer(vec![2, 2], 1);
        assert_eq!(relay.take(), vec![vec![1], vec![2, 2]]);
        relay.offer(vec![3], 1);
        relay.begin("b", 2);
        relay.offer(vec![4], 1);
        assert!(relay.take().is_empty(), "a message taken for the old page never reaches the new one");
        assert_eq!((relay.epoch_of("a"), relay.epoch_of("b")), (None, Some(2)), "an old page's refunds are not the new page's");
        assert_eq!(dashboard_follow::lead::budget().used(), 0, "the one charge is the native queue's, never the relay's");
        relay.restart("c");
        assert!(relay.epoch_of("c").is_some());
    }

    #[test]
    fn a_prefetched_answer_keeps_the_previous_whole_batch_available() {
        let mut relay = Relay::default();
        relay.begin("page:2", 2);
        let packet = |hour, byte| {
            let mut body = vec![b'x'; 256];
            body[100] = byte;
            dated::encoded(0, 5, 3, 101, hour, dated::DATA, &body)
        };
        relay.offer(packet(1, 1), 2);
        assert_eq!(relay.take_body(), super::frame(&[packet(1, 1)]));
        relay.offer(packet(2, 2), 2);
        let first = relay.take_body();
        assert_eq!(&first[4..12], b"PWB1\x01\0\0\0");
        relay.offer(packet(3, 3), 2);
        assert_eq!(&relay.take_body()[4..12], b"PWB1\x02\0\0\0");
        assert_eq!(relay.whole("page:2", 1), Some(super::frame(&[packet(2, 2)])));
        assert!(relay.whole("page:1", 1).is_none());
        relay.offer(packet(4, 4), 2);
        relay.take_body();
        assert!(relay.whole("page:2", 1).is_none());
        relay.begin("page:3", 3);
        assert!(relay.whole("page:3", 2).is_none());
        relay.offer(packet(5, 5), 3);
        assert_eq!(relay.take_body(), super::frame(&[packet(5, 5)]));
    }
}
