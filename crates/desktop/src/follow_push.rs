pub type FollowPush = crate::pull::Pull<Relay>;

#[derive(Default)]
pub struct Relay {
    held: Vec<Vec<u8>>,
    session: Option<(String, u64)>,
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
    }
}

impl crate::pull::Outbox for Relay {
    fn take(&mut self) -> Vec<Vec<u8>> { std::mem::take(&mut self.held) }
    fn restart(&mut self, session: &str) {
        let epoch = dashboard_follow::registry::registry().restart(session);
        self.begin(session, epoch);
    }
}

#[cfg(test)]
mod tests {
    use super::Relay;
    use crate::pull::Outbox;

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
}
