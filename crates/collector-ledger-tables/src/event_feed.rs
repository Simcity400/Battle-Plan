use engine_native_memory::{Memory, walk::Outcome};
use replay_data::rows::TableId;
use replay_frame::frame::TableView;

pub const N: usize = 1;
pub const TABLES: [TableId; N] = [TableId::EventFeedHistory];
#[derive(Clone, Copy, Default, PartialEq, Eq)]
pub struct Capacities {
    pub events: usize,
}

collector_primitives::capacity_max!(Capacities { events });
const ASSEMBLED: &str = "Event feed assembled by spool";

pub struct Captured {
    snapshot: collector_observers::event_feed_history::Snapshot,
    sampled: bool,
    prepared: Option<(i32, bool)>,
}

impl Captured {
    pub fn vacant() -> Self {
        Self::with_capacities(&Capacities { events: 0 })
    }
    /// # Safety
    /// Caller must own the active session's capture or publication failure path when invalidating
    /// live event-feed observation.
    pub unsafe fn fail(&mut self, hour: i32, reason: &'static str) {
        self.prepared = None;
        // SAFETY: Caller owns the event-feed capture or publication failure path.
        unsafe { collector_observers::event_feed_observer::lost_drain() };
        self.snapshot
            .refuse(crate::world_lifetime::generation(), hour, reason);
        self.sampled = true;
    }
    /// # Safety
    /// Caller must own the active session's capture or publication failure path when invalidating
    /// a prepared event-feed handoff.
    pub unsafe fn fail_prepared(&mut self, reason: &'static str) {
        if let Some((hour, _)) = self.prepared {
            // SAFETY: Caller owns the event-feed capture or publication failure path.
            unsafe { self.fail(hour, reason) };
        }
    }
    pub fn with_capacities(capacity: &Capacities) -> Self {
        Self {
            snapshot: collector_observers::event_feed_history::Snapshot::new(capacity.events),
            sampled: false,
            prepared: None,
        }
    }
    pub fn reset(&mut self) {
        self.sampled = false;
        self.prepared = None;
    }
    pub fn allocated_bytes(&self) -> usize {
        self.snapshot.allocated_bytes()
    }
    pub fn grow(&mut self, capacity: &mut Capacities) {
        crate::table_capture::grow_capacity(&mut capacity.events, self.snapshot.required);
        let _ = self.snapshot.grow(capacity.events);
    }
    /// # Safety
    /// Caller must run on the admitted hourly observer boundary as the sole consumer preparing the
    /// event-feed interval with teardown excluded (or own an isolated test fixture); a stable
    /// absent owner when event-feed hooks are disabled is permitted.
    pub unsafe fn drain(&mut self, hour: i32, partial: bool) {
        self.prepared = None;
        // SAFETY: Caller holds hourly observer-boundary ownership for event-feed preparation.
        match unsafe { collector_observers::event_feed_observer::prepare_growing(hour, partial) } {
            Ok(()) => self.prepared = Some((hour, partial)),
            // SAFETY: Caller holds hourly observer-boundary ownership on drain failure.
            Err(reason) => unsafe { self.fail(hour, reason) },
        }
    }
    /// # Safety
    /// Caller must hold hourly capture ownership as the sole consumer for this frame with teardown
    /// excluded (or own an isolated test fixture); a live event-feed observer owner is required
    /// only when this frame holds a prepared interval (`self.prepared.is_some()`), otherwise
    /// `seal` is a no-op preserving any refused state.
    pub unsafe fn seal(&mut self) {
        if let Some((hour, partial)) = self.prepared {
            // SAFETY: `self.prepared` is `Some`, so the caller guarantees hourly ownership,
            // exclusion of teardown, and a live event-feed observer owner for sealing.
            unsafe {
                collector_observers::event_feed_observer::seal(&mut self.snapshot, hour, partial);
            }
            self.prepared = None;
            self.sampled = true;
        }
    }
    /// # Safety
    /// Caller must be the originating spool or capture recovery path discarding an uncommitted
    /// event-feed publication for this session.
    pub unsafe fn discard(&mut self) {
        if self.sampled {
            // SAFETY: Caller owns discarding this uncommitted event-feed publication receipt.
            unsafe {
                collector_observers::event_feed_observer::discarded(
                    self.snapshot.publication(),
                    self.snapshot.boundary.generation,
                );
            }
            self.sampled = false;
        }
    }
    pub fn prepare_pack(
        &self,
    ) -> std::io::Result<Option<collector_observers::event_feed_history::Prepared>> {
        self.sampled
            .then(|| self.snapshot.prepare_pack())
            .transpose()
    }
    pub fn capture(&mut self, _: &impl Memory, _: usize, _: usize) -> [u64; N] {
        let start = crate::clock::qpc();
        [crate::clock::qpc().saturating_sub(start).max(1) as u64]
    }
    pub fn outcomes(&self) -> [Outcome; N] {
        [Outcome::Unavailable { reason: ASSEMBLED }]
    }
    pub fn tables(&self) -> [TableView; N] {
        [crate::table_conversion::unavailable(ASSEMBLED)]
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use replay_data::event_feed_rows::EventFeedCoverage;

    #[test]
    fn a_refused_preparation_is_sampled_and_no_seal_or_join_failure_replaces_it() {
        let mut captured = Captured::vacant();
        unsafe { captured.drain(7, false) };
        let refused = captured.snapshot.boundary;
        assert!(matches!(refused.coverage, EventFeedCoverage::Unavailable));
        assert_eq!(refused.read_hour, 7);
        let reason = refused.failure.expect("a refused preparation names its reason");
        assert!(captured.prepared.is_none());
        assert!(captured.prepare_pack().unwrap().is_some());

        unsafe { captured.seal() };
        unsafe { captured.fail_prepared("event feed seal job failed") };
        let kept = captured.snapshot.boundary;
        assert!(matches!(kept.coverage, EventFeedCoverage::Unavailable));
        assert_eq!(kept.read_hour, 7);
        assert_eq!(kept.failure, Some(reason));
        assert!(captured.prepare_pack().unwrap().is_some());

        captured.reset();
        assert!(captured.prepare_pack().unwrap().is_none());
        unsafe { captured.seal() };
        assert!(captured.prepare_pack().unwrap().is_none());
    }
}
