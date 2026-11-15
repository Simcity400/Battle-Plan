//! Hourly pointer-free Event feed handoff; the spool encodes and acknowledges it.
use engine_native::{Memory, walk::Outcome};
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
    /// The hour and partial flag `drain` prepared, until sealed.
    prepared: Option<(i32, bool)>,
}

impl Captured {
    pub fn vacant() -> Self {
        Self::with_capacities(&Capacities { events: 0 })
    }
    pub fn fail(&mut self, hour: i32, reason: &'static str) {
        self.prepared = None;
        collector_observers::event_feed_observer::lost_drain();
        self.snapshot
            .refuse(crate::world_lifetime::generation(), hour, reason);
        self.sampled = true;
    }
    /// Refuses a prepared hour that will not be sealed; an hour already
    /// refused keeps its own reason.
    pub fn fail_prepared(&mut self, reason: &'static str) {
        if let Some((hour, _)) = self.prepared {
            self.fail(hour, reason);
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
    /// Prepares the hour at the observer boundary; [`Self::seal`]
    /// publishes it.
    pub fn drain(&mut self, hour: i32, partial: bool) {
        self.prepared = None;
        match collector_observers::event_feed_observer::prepare_growing(hour, partial) {
            Ok(()) => self.prepared = Some((hour, partial)),
            Err(reason) => self.fail(hour, reason),
        }
    }
    /// The seal follows the country census, so its markers belong to the
    /// hour they were read. An hour already refused stays refused.
    pub fn seal(&mut self) {
        if let Some((hour, partial)) = self.prepared {
            collector_observers::event_feed_observer::seal(
                &mut self.snapshot,
                hour,
                partial,
            );
            // Kept until sealed: a seal that unwinds still refuses its hour.
            self.prepared = None;
            self.sampled = true;
        }
    }
    pub fn discard(&mut self) {
        if self.sampled {
            collector_observers::event_feed_observer::discarded(self.snapshot.publication());
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

    // This process installs no Event feed owner, so the boundary refuses
    // the hour's preparation.
    #[test]
    fn a_refused_preparation_is_sampled_and_no_seal_or_join_failure_replaces_it() {
        let mut captured = Captured::vacant();
        captured.drain(7, false);
        let refused = captured.snapshot.boundary;
        assert!(matches!(refused.coverage, EventFeedCoverage::Unavailable));
        assert_eq!(refused.read_hour, 7);
        let reason = refused.failure.expect("a refused preparation names its reason");
        assert!(captured.prepared.is_none());
        assert!(captured.prepare_pack().unwrap().is_some());

        // Nothing was prepared: the seal and a failed seal job keep the
        // boundary's own refusal.
        captured.seal();
        captured.fail_prepared("event feed seal job failed");
        let kept = captured.snapshot.boundary;
        assert!(matches!(kept.coverage, EventFeedCoverage::Unavailable));
        assert_eq!(kept.read_hour, 7);
        assert_eq!(kept.failure, Some(reason));
        assert!(captured.prepare_pack().unwrap().is_some());

        captured.reset();
        assert!(captured.prepare_pack().unwrap().is_none());
        captured.seal();
        assert!(captured.prepare_pack().unwrap().is_none());
    }
}
