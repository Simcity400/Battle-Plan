pub type FollowPush = crate::pull::Pull<dashboard_follow::push::Push>;

impl crate::pull::Outbox for dashboard_follow::push::Push {
    fn take(&mut self) -> Vec<Vec<u8>> { self.take_waiting() }
    fn restart(&mut self) { *self = Self::default(); }
}
