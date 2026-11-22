pub type FollowPush = crate::pull::Pull<Relay>;

#[derive(Default)]
pub struct Relay {
    held: Vec<Vec<u8>>,
}

impl Relay {
    pub fn offer(&mut self, payload: Vec<u8>) {
        self.held.push(payload);
    }
}

impl crate::pull::Outbox for Relay {
    fn take(&mut self) -> Vec<Vec<u8>> { std::mem::take(&mut self.held) }
    fn restart(&mut self) {
        self.held.clear();
        dashboard_follow::registry::registry().restart();
    }
}

#[cfg(test)]
mod tests {
    use super::Relay;
    use crate::pull::Outbox;

    #[test]
    fn the_relay_hands_payloads_on_in_order_and_charges_nothing_itself() {
        let mut relay = Relay::default();
        relay.offer(vec![1]);
        relay.offer(vec![2, 2]);
        assert_eq!(relay.take(), vec![vec![1], vec![2, 2]]);
        assert_eq!(dashboard_follow::lead::budget().used(), 0, "the one charge is the native queue's, never the relay's");
        relay.offer(vec![3]);
        relay.restart();
        assert!(relay.take().is_empty(), "a restarted page gets nothing held for the old one");
    }
}
