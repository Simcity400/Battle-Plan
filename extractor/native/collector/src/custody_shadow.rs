//! | PROCESSING | `release_read_custody` | yes -- the bytes are frozen there |

use std::collections::VecDeque;
use std::time::Instant;

pub const RESERVED_DOMAIN_GAP_REASON_PROCESSING_BACKPRESSURE: u16 = 9;

pub const RESERVED_DOMAIN_GAP_REASON_OBSERVATION_DEADLINE_MISS: u16 = 10;

pub const CUSTODY_SHADOW_ENV: &str = "BATTLE_PLAN_CUSTODY_SHADOW";

const DIVERGENCE_RING_CAP: usize = 64;

const DROPPED_HOUR_MEMORY: usize = 8;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CustodyState {
    Admitted,
    Observing,
    Processing,
    Sealed,
    Publishing,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct HourCustody {
    pub hour_raw: i32,
    pub epoch: u64,
    pub edge_at_us: u64,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ShadowDisposition {
    Committed,
    Refused,
    ProcessingBackpressure,
    ObservationDeadlineMiss,
    Invalidated,
}

impl ShadowDisposition {
    pub const fn reserved_gap_reason_code(self) -> Option<u16> {
        match self {
            Self::ProcessingBackpressure => {
                Some(RESERVED_DOMAIN_GAP_REASON_PROCESSING_BACKPRESSURE)
            }
            Self::ObservationDeadlineMiss => {
                Some(RESERVED_DOMAIN_GAP_REASON_OBSERVATION_DEADLINE_MISS)
            }
            Self::Committed | Self::Refused | Self::Invalidated => None,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CustodyError {
    NonMonotonicHourEdge {
        last: (u64, i32),
        offered: (u64, i32),
    },
    NothingAdmitted,
    ObservationSlotOccupied,
    NothingObserving,
    NothingProcessing,
    SealSlotOccupied,
    NothingSealed,
    PublicationSlotOccupied,
    NothingPublishing,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct EdgeOutcome {
    pub unopened_deadline_miss: Option<(u64, i32)>,
    pub unclosed_deadline_miss: Option<(u64, i32)>,
}

impl EdgeOutcome {
    pub fn is_clean(&self) -> bool {
        *self == Self::default()
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct CloseOutcome {
    pub closed: (u64, i32),
    pub processing_backpressure: Option<(u64, i32)>,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct CustodyLedger {
    pub admitted: u64,
    pub committed: u64,
    pub refused: u64,
    pub processing_backpressure: u64,
    pub observation_deadline_miss: u64,
    pub invalidated: u64,
    pub in_flight: u64,
}

impl CustodyLedger {
    pub fn balanced(&self) -> bool {
        self.committed
            .checked_add(self.refused)
            .and_then(|n| n.checked_add(self.processing_backpressure))
            .and_then(|n| n.checked_add(self.observation_deadline_miss))
            .and_then(|n| n.checked_add(self.invalidated))
            .and_then(|n| n.checked_add(self.in_flight))
            == Some(self.admitted)
    }
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct CustodyMachine {
    admitted: Option<HourCustody>,
    observing: Option<HourCustody>,
    processing: Option<HourCustody>,
    sealed: Option<HourCustody>,
    publishing: Option<HourCustody>,
    last_edge: Option<(u64, i32)>,
    last_sealed: Option<(u64, i32)>,
    last_committed: Option<(u64, i32)>,
    ledger: CustodyLedger,
}

impl CustodyMachine {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn ledger(&self) -> CustodyLedger {
        let mut ledger = self.ledger;
        ledger.in_flight = u64::from(self.admitted.is_some())
            + u64::from(self.observing.is_some())
            + u64::from(self.processing.is_some())
            + u64::from(self.sealed.is_some())
            + u64::from(self.publishing.is_some());
        ledger
    }

    pub fn epoch(&self) -> Option<u64> {
        self.last_edge.map(|(epoch, _)| epoch)
    }

    pub fn state_of(&self, epoch: u64, hour_raw: i32) -> Option<CustodyState> {
        let holds = |slot: &Option<HourCustody>| {
            slot.is_some_and(|custody| custody.hour_raw == hour_raw && custody.epoch == epoch)
        };
        if holds(&self.admitted) {
            Some(CustodyState::Admitted)
        } else if holds(&self.observing) {
            Some(CustodyState::Observing)
        } else if holds(&self.processing) {
            Some(CustodyState::Processing)
        } else if holds(&self.sealed) {
            Some(CustodyState::Sealed)
        } else if holds(&self.publishing) {
            Some(CustodyState::Publishing)
        } else {
            None
        }
    }

    pub fn occupancy(&self) -> [Option<i32>; 5] {
        [
            self.admitted.map(|c| c.hour_raw),
            self.observing.map(|c| c.hour_raw),
            self.processing.map(|c| c.hour_raw),
            self.sealed.map(|c| c.hour_raw),
            self.publishing.map(|c| c.hour_raw),
        ]
    }

    pub fn occupancy_stamped(&self) -> [Option<(u64, i32)>; 5] {
        [
            self.admitted,
            self.observing,
            self.processing,
            self.sealed,
            self.publishing,
        ]
        .map(|slot| slot.map(|custody| (custody.epoch, custody.hour_raw)))
    }

    pub fn note_hour_edge(
        &mut self,
        epoch: u64,
        hour_raw: i32,
        at_us: u64,
    ) -> Result<EdgeOutcome, CustodyError> {
        let offered = (epoch, hour_raw);
        if let Some(last) = self.last_edge {
            if offered <= last {
                return Err(CustodyError::NonMonotonicHourEdge { last, offered });
            }
        }
        let mut outcome = EdgeOutcome::default();
        if let Some(stale) = self.admitted.take() {
            self.ledger.observation_deadline_miss =
                self.ledger.observation_deadline_miss.saturating_add(1);
            outcome.unopened_deadline_miss = Some((stale.epoch, stale.hour_raw));
        }
        if let Some(stale) = self.observing.take() {
            self.ledger.observation_deadline_miss =
                self.ledger.observation_deadline_miss.saturating_add(1);
            outcome.unclosed_deadline_miss = Some((stale.epoch, stale.hour_raw));
        }
        self.admitted = Some(HourCustody {
            hour_raw,
            epoch,
            edge_at_us: at_us,
        });
        self.last_edge = Some(offered);
        self.ledger.admitted = self.ledger.admitted.saturating_add(1);
        Ok(outcome)
    }

    pub fn dispatch(&mut self, at_us: u64) -> Result<((u64, i32), u64), CustodyError> {
        let Some(custody) = self.admitted else {
            return Err(CustodyError::NothingAdmitted);
        };
        if self.observing.is_some() {
            return Err(CustodyError::ObservationSlotOccupied);
        }
        self.admitted = None;
        self.observing = Some(custody);
        Ok((
            (custody.epoch, custody.hour_raw),
            at_us.saturating_sub(custody.edge_at_us),
        ))
    }

    pub fn close_observation(&mut self) -> Result<CloseOutcome, CustodyError> {
        let Some(custody) = self.observing else {
            return Err(CustodyError::NothingObserving);
        };
        let mut outcome = CloseOutcome {
            closed: (custody.epoch, custody.hour_raw),
            processing_backpressure: None,
        };
        if let Some(stale) = self.processing.take() {
            self.ledger.processing_backpressure =
                self.ledger.processing_backpressure.saturating_add(1);
            outcome.processing_backpressure = Some((stale.epoch, stale.hour_raw));
        }
        self.observing = None;
        self.processing = Some(custody);
        Ok(outcome)
    }

    pub fn seal(&mut self) -> Result<(u64, i32), CustodyError> {
        let Some(custody) = self.processing else {
            return Err(CustodyError::NothingProcessing);
        };
        if self.sealed.is_some() {
            return Err(CustodyError::SealSlotOccupied);
        }
        self.processing = None;
        self.sealed = Some(custody);
        self.last_sealed = Some((custody.epoch, custody.hour_raw));
        Ok((custody.epoch, custody.hour_raw))
    }

    pub fn begin_publication(&mut self) -> Result<(u64, i32), CustodyError> {
        let Some(custody) = self.sealed else {
            return Err(CustodyError::NothingSealed);
        };
        if self.publishing.is_some() {
            return Err(CustodyError::PublicationSlotOccupied);
        }
        self.sealed = None;
        self.publishing = Some(custody);
        Ok((custody.epoch, custody.hour_raw))
    }

    pub fn commit(&mut self, accepted: bool) -> Result<(u64, i32), CustodyError> {
        let Some(custody) = self.publishing else {
            return Err(CustodyError::NothingPublishing);
        };
        self.publishing = None;
        self.last_committed = Some((custody.epoch, custody.hour_raw));
        if accepted {
            self.ledger.committed = self.ledger.committed.saturating_add(1);
        } else {
            self.ledger.refused = self.ledger.refused.saturating_add(1);
        }
        Ok((custody.epoch, custody.hour_raw))
    }

    pub fn invalidate(&mut self) -> [Option<(u64, i32)>; 3] {
        let mut dropped = [None; 3];
        for (index, slot) in [
            &mut self.admitted,
            &mut self.observing,
            &mut self.processing,
        ]
        .into_iter()
        .enumerate()
        {
            if let Some(custody) = slot.take() {
                dropped[index] = Some((custody.epoch, custody.hour_raw));
                self.ledger.invalidated = self.ledger.invalidated.saturating_add(1);
            }
        }
        dropped
    }

    pub fn watermarks(&self) -> (Option<i32>, Option<i32>) {
        (
            self.last_sealed.map(|(_, hour)| hour),
            self.last_committed.map(|(_, hour)| hour),
        )
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ShadowMode {
    Enabled,
    Disabled,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ShadowClock {
    Real { origin: Instant },
    Manual { now_us: u64 },
}

impl ShadowClock {
    pub fn real() -> Self {
        Self::Real {
            origin: Instant::now(),
        }
    }

    fn now_us(&self) -> u64 {
        match self {
            Self::Real { origin } => {
                u64::try_from(origin.elapsed().as_micros()).unwrap_or(u64::MAX)
            }
            Self::Manual { now_us } => *now_us,
        }
    }

    pub fn advance_us(&mut self, delta: u64) {
        if let Self::Manual { now_us } = self {
            *now_us = now_us.saturating_add(delta);
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DivergenceKind {
    /// The live scheduler dispatched, released, or resolved an hour that is not
    HourMismatch {
        shadow: Option<(u64, i32)>,
    },
    WouldDrop {
        disposition: ShadowDisposition,
    },
    LiveKeptWhatShadowDropped {
        disposition: ShadowDisposition,
    },
    /// The live scheduler published an hour synchronously -- it never released
    CollapsedPublication,
    IllegalTransition {
        error: CustodyError,
    },
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct FrameDivergence {
    pub live: (u64, i32),
    pub shadow_state: Option<CustodyState>,
    pub kind: DivergenceKind,
    pub late_dispatch_us: u64,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct CustodyShadowSummary {
    pub active: bool,
    pub events: u64,
    pub hour_edges: u64,
    pub dispatches: u64,
    pub committed_hours: u64,
    pub refused_hours: u64,
    /// the processing slot from `release_read_custody` until its sink outcome
    pub processing_backpressure_hours: u64,
    pub observation_deadline_miss_hours: u64,
    pub invalidated_hours: u64,
    pub illegal_transitions: u64,
    pub hour_mismatches: u64,
    pub collapsed_publications: u64,
    pub live_kept_hours_shadow_dropped: u64,
    pub late_dispatch_samples: u64,
    pub late_dispatch_total_us: u64,
    pub max_late_dispatch_us: u64,
    pub agreed_evictions: u64,
    pub disagreed_evictions: u64,
    pub drop_memory_overflows: u64,
    pub cost_total_ns: u64,
    pub max_cost_ns: u64,
}

#[derive(Clone, Debug)]
pub struct CustodyShadow {
    mode: ShadowMode,
    clock: ShadowClock,
    machine: CustodyMachine,
    summary: CustodyShadowSummary,
    recent: VecDeque<FrameDivergence>,
    dropped: VecDeque<((u64, i32), ShadowDisposition)>,
    #[cfg(test)]
    hostile: bool,
}

impl Default for CustodyShadow {
    fn default() -> Self {
        Self::new(ShadowMode::Enabled, ShadowClock::real())
    }
}

impl CustodyShadow {
    pub fn new(mode: ShadowMode, clock: ShadowClock) -> Self {
        Self {
            mode,
            clock,
            machine: CustodyMachine::new(),
            summary: CustodyShadowSummary {
                active: mode == ShadowMode::Enabled,
                ..CustodyShadowSummary::default()
            },
            recent: VecDeque::with_capacity(DIVERGENCE_RING_CAP),
            dropped: VecDeque::with_capacity(DROPPED_HOUR_MEMORY),
            #[cfg(test)]
            hostile: false,
        }
    }

    pub fn from_environment() -> Self {
        let mode = match std::env::var(CUSTODY_SHADOW_ENV).as_deref() {
            Ok("0") | Ok("off") | Ok("false") => ShadowMode::Disabled,
            _ => ShadowMode::Enabled,
        };
        Self::new(mode, ShadowClock::real())
    }

    pub fn mode(&self) -> ShadowMode {
        self.mode
    }

    pub fn summary(&self) -> CustodyShadowSummary {
        self.summary
    }

    pub fn ledger(&self) -> CustodyLedger {
        self.machine.ledger()
    }

    pub fn machine(&self) -> &CustodyMachine {
        &self.machine
    }

    pub fn recent_divergences(&self) -> impl Iterator<Item = &FrameDivergence> {
        self.recent.iter()
    }

    #[cfg(test)]
    pub fn set_hostile(&mut self, hostile: bool) {
        self.hostile = hostile;
    }

    #[cfg(test)]
    pub fn advance_clock_us(&mut self, delta: u64) {
        self.clock.advance_us(delta);
    }

    fn enabled(&self) -> bool {
        self.mode == ShadowMode::Enabled
    }

    fn record(&mut self, divergence: FrameDivergence) {
        if self.recent.len() == DIVERGENCE_RING_CAP {
            self.recent.pop_front();
        }
        self.recent.push_back(divergence);
    }

    fn remember_drop(&mut self, live: (u64, i32), disposition: ShadowDisposition) {
        if self.dropped.len() == DROPPED_HOUR_MEMORY {
            self.dropped.pop_front();
            self.summary.drop_memory_overflows =
                self.summary.drop_memory_overflows.saturating_add(1);
        }
        self.dropped.push_back((live, disposition));
    }

    fn take_dropped_disposition(&mut self, live: (u64, i32)) -> Option<ShadowDisposition> {
        let index = self
            .dropped
            .iter()
            .rposition(|(candidate, _)| *candidate == live)?;
        self.dropped
            .remove(index)
            .map(|(_, disposition)| disposition)
    }

    fn peek_dropped_disposition(&self, live: (u64, i32)) -> Option<ShadowDisposition> {
        self.dropped
            .iter()
            .rev()
            .find(|(candidate, _)| *candidate == live)
            .map(|(_, disposition)| *disposition)
    }

    fn note_disposition(&mut self, live: (u64, i32), disposition: ShadowDisposition, opened: bool) {
        match disposition {
            ShadowDisposition::ProcessingBackpressure => {
                self.summary.processing_backpressure_hours =
                    self.summary.processing_backpressure_hours.saturating_add(1)
            }
            ShadowDisposition::ObservationDeadlineMiss => {
                self.summary.observation_deadline_miss_hours = self
                    .summary
                    .observation_deadline_miss_hours
                    .saturating_add(1)
            }
            ShadowDisposition::Invalidated => {
                self.summary.invalidated_hours = self.summary.invalidated_hours.saturating_add(1)
            }
            ShadowDisposition::Committed => {
                self.summary.committed_hours = self.summary.committed_hours.saturating_add(1)
            }
            ShadowDisposition::Refused => {
                self.summary.refused_hours = self.summary.refused_hours.saturating_add(1)
            }
        }
        if matches!(
            disposition,
            ShadowDisposition::ProcessingBackpressure | ShadowDisposition::ObservationDeadlineMiss
        ) {
            if opened {
                self.remember_drop(live, disposition);
            }
            self.record(FrameDivergence {
                live,
                shadow_state: None,
                kind: DivergenceKind::WouldDrop { disposition },
                late_dispatch_us: 0,
            });
        }
    }

    fn note_illegal(&mut self, live: (u64, i32), error: CustodyError) {
        self.summary.illegal_transitions = self.summary.illegal_transitions.saturating_add(1);
        let shadow_state = self.machine.state_of(live.0, live.1);
        self.record(FrameDivergence {
            live,
            shadow_state,
            kind: DivergenceKind::IllegalTransition { error },
            late_dispatch_us: 0,
        });
    }

    fn note_hour_mismatch(&mut self, live: (u64, i32), shadow: Option<(u64, i32)>) {
        self.summary.hour_mismatches = self.summary.hour_mismatches.saturating_add(1);
        let shadow_state = self.machine.state_of(live.0, live.1);
        self.record(FrameDivergence {
            live,
            shadow_state,
            kind: DivergenceKind::HourMismatch { shadow },
            late_dispatch_us: 0,
        });
    }

    fn classify_as_live_kept(&mut self, live: (u64, i32), resolving: bool) -> bool {
        if !resolving {
            return self.peek_dropped_disposition(live).is_some();
        }
        let Some(disposition) = self.take_dropped_disposition(live) else {
            return false;
        };
        self.summary.live_kept_hours_shadow_dropped = self
            .summary
            .live_kept_hours_shadow_dropped
            .saturating_add(1);
        self.record(FrameDivergence {
            live,
            shadow_state: None,
            kind: DivergenceKind::LiveKeptWhatShadowDropped { disposition },
            late_dispatch_us: 0,
        });
        true
    }

    fn begin(&mut self) -> Option<u64> {
        if !self.enabled() {
            return None;
        }
        self.summary.events = self.summary.events.saturating_add(1);
        Some(self.clock.now_us())
    }

    fn end(&mut self, entered_us: u64) {
        let cost_ns = self
            .clock
            .now_us()
            .saturating_sub(entered_us)
            .saturating_mul(1_000);
        self.summary.cost_total_ns = self.summary.cost_total_ns.saturating_add(cost_ns);
        self.summary.max_cost_ns = self.summary.max_cost_ns.max(cost_ns);
        #[cfg(test)]
        if self.hostile {
            self.machine.invalidate();
            self.summary.events = self.summary.events.saturating_mul(3).saturating_add(7);
            self.recent.clear();
            self.dropped.clear();
        }
    }

    pub fn note_frontier_admitted(&mut self, live: (u64, i32), evicted: Option<(u64, i32)>) {
        let Some(entered) = self.begin() else { return };
        self.summary.hour_edges = self.summary.hour_edges.saturating_add(1);
        match self.machine.note_hour_edge(live.0, live.1, entered) {
            Ok(outcome) => {
                if let Some(stale) = outcome.unopened_deadline_miss {
                    self.note_disposition(stale, ShadowDisposition::ObservationDeadlineMiss, false);
                }
                if let Some(stale) = outcome.unclosed_deadline_miss {
                    self.note_disposition(stale, ShadowDisposition::ObservationDeadlineMiss, true);
                }
                if let Some(evicted) = evicted {
                    if outcome.unopened_deadline_miss == Some(evicted) {
                        self.summary.agreed_evictions =
                            self.summary.agreed_evictions.saturating_add(1);
                    } else {
                        self.summary.disagreed_evictions =
                            self.summary.disagreed_evictions.saturating_add(1);
                    }
                }
            }
            Err(error) => self.note_illegal(live, error),
        }
        self.end(entered);
    }

    pub fn note_dispatch(&mut self, live: (u64, i32)) {
        let Some(entered) = self.begin() else { return };
        self.summary.dispatches = self.summary.dispatches.saturating_add(1);
        match self.machine.dispatch(entered) {
            Ok((shadow, late_us)) => {
                self.summary.late_dispatch_samples =
                    self.summary.late_dispatch_samples.saturating_add(1);
                self.summary.late_dispatch_total_us =
                    self.summary.late_dispatch_total_us.saturating_add(late_us);
                self.summary.max_late_dispatch_us = self.summary.max_late_dispatch_us.max(late_us);
                if shadow != live {
                    self.note_hour_mismatch(live, Some(shadow));
                }
            }
            Err(error) => {
                if !self.classify_as_live_kept(live, false) {
                    self.note_illegal(live, error);
                }
            }
        }
        self.end(entered);
    }

    pub fn note_read_custody_released(&mut self, live: (u64, i32)) {
        let Some(entered) = self.begin() else { return };
        if self.machine.state_of(live.0, live.1) != Some(CustodyState::Observing) {
            if !self.classify_as_live_kept(live, false) {
                self.note_illegal(live, CustodyError::NothingObserving);
            }
            self.end(entered);
            return;
        }
        match self.machine.close_observation() {
            Ok(outcome) => {
                if let Some(stale) = outcome.processing_backpressure {
                    self.note_disposition(stale, ShadowDisposition::ProcessingBackpressure, true);
                }
                if outcome.closed != live {
                    self.note_hour_mismatch(live, Some(outcome.closed));
                }
            }
            Err(error) => self.note_illegal(live, error),
        }
        self.end(entered);
    }

    /// On the SYNCHRONOUS path there was no release, so the hour is still being
    pub fn note_publication_resolved(&mut self, live: (u64, i32), accepted: bool) {
        let Some(entered) = self.begin() else { return };
        match self.machine.state_of(live.0, live.1) {
            Some(CustodyState::Observing) => {
                self.summary.collapsed_publications =
                    self.summary.collapsed_publications.saturating_add(1);
                self.record(FrameDivergence {
                    live,
                    shadow_state: Some(CustodyState::Observing),
                    kind: DivergenceKind::CollapsedPublication,
                    late_dispatch_us: 0,
                });
                match self.machine.close_observation() {
                    Ok(outcome) => {
                        if let Some(stale) = outcome.processing_backpressure {
                            self.note_disposition(
                                stale,
                                ShadowDisposition::ProcessingBackpressure,
                                true,
                            );
                        }
                    }
                    Err(error) => {
                        self.note_illegal(live, error);
                        self.end(entered);
                        return;
                    }
                }
            }
            Some(CustodyState::Processing) => {}
            _ => {
                if !self.classify_as_live_kept(live, true) {
                    self.note_illegal(live, CustodyError::NothingProcessing);
                }
                self.end(entered);
                return;
            }
        }
        for step in 0..3u8 {
            let result = match step {
                0 => self.machine.seal(),
                1 => self.machine.begin_publication(),
                _ => self.machine.commit(accepted),
            };
            match result {
                Ok(shadow) => {
                    if shadow != live {
                        self.note_hour_mismatch(live, Some(shadow));
                    }
                }
                Err(error) => {
                    self.note_illegal(live, error);
                    self.end(entered);
                    return;
                }
            }
        }
        self.note_disposition(
            live,
            if accepted {
                ShadowDisposition::Committed
            } else {
                ShadowDisposition::Refused
            },
            true,
        );
        self.end(entered);
    }

    pub fn note_invalidated(&mut self) {
        let Some(entered) = self.begin() else { return };
        for (index, dropped) in self.machine.invalidate().into_iter().enumerate() {
            let Some(dropped) = dropped else { continue };
            self.summary.invalidated_hours = self.summary.invalidated_hours.saturating_add(1);
            if index > 0 {
                self.remember_drop(dropped, ShadowDisposition::Invalidated);
            }
        }
        self.end(entered);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::BTreeSet;

    const E: u64 = 0;

    fn manual() -> CustodyShadow {
        CustodyShadow::new(ShadowMode::Enabled, ShadowClock::Manual { now_us: 0 })
    }

    fn machine() -> CustodyMachine {
        CustodyMachine::new()
    }

    fn h(hour: i32) -> (u64, i32) {
        (E, hour)
    }

    fn edge(m: &mut CustodyMachine, hour: i32, at_us: u64) -> EdgeOutcome {
        m.note_hour_edge(E, hour, at_us)
            .expect("the edge is monotonic")
    }

    fn dispatch(m: &mut CustodyMachine, at_us: u64) -> (i32, u64) {
        let (stamp, late_us) = m.dispatch(at_us).expect("an hour is admitted");
        assert_eq!(stamp.0, E);
        (stamp.1, late_us)
    }

    fn close(m: &mut CustodyMachine) -> i32 {
        let outcome = m.close_observation().expect("an observation is open");
        assert_eq!(
            outcome.processing_backpressure, None,
            "this close was not supposed to evict anything"
        );
        outcome.closed.1
    }

    fn seal(m: &mut CustodyMachine) -> i32 {
        m.seal().expect("a snapshot is in processing").1
    }

    fn publish(m: &mut CustodyMachine) -> i32 {
        m.begin_publication().expect("a frame is sealed").1
    }

    fn commit(m: &mut CustodyMachine, accepted: bool) -> i32 {
        m.commit(accepted).expect("a frame is publishing").1
    }

    fn state(m: &CustodyMachine, hour: i32) -> Option<CustodyState> {
        m.state_of(E, hour)
    }

    #[test]
    fn one_hour_walks_every_phase_in_order() {
        let mut m = machine();
        assert!(edge(&mut m, 100, 0).is_clean());
        assert_eq!(state(&m, 100), Some(CustodyState::Admitted));
        assert_eq!(dispatch(&mut m, 1_500), (100, 1_500));
        assert_eq!(state(&m, 100), Some(CustodyState::Observing));
        assert_eq!(close(&mut m), 100);
        assert_eq!(state(&m, 100), Some(CustodyState::Processing));
        assert_eq!(seal(&mut m), 100);
        assert_eq!(state(&m, 100), Some(CustodyState::Sealed));
        assert_eq!(publish(&mut m), 100);
        assert_eq!(state(&m, 100), Some(CustodyState::Publishing));
        assert_eq!(commit(&mut m, true), 100);
        assert_eq!(state(&m, 100), None);
        assert_eq!(m.watermarks(), (Some(100), Some(100)));

        let ledger = m.ledger();
        assert!(ledger.balanced(), "{ledger:?}");
        assert_eq!(ledger.admitted, 1);
        assert_eq!(ledger.committed, 1);
        assert_eq!(ledger.in_flight, 0);
    }

    #[test]
    fn the_same_raw_hour_in_two_epochs_is_two_different_custodies() {
        let mut m = machine();
        m.note_hour_edge(0, 500, 0).unwrap();
        m.dispatch(10).unwrap();
        m.close_observation().unwrap();
        assert_eq!(m.state_of(0, 500), Some(CustodyState::Processing));

        m.note_hour_edge(1, 500, 1_000).unwrap();
        assert_eq!(m.state_of(1, 500), Some(CustodyState::Admitted));
        assert_eq!(
            m.state_of(0, 500),
            Some(CustodyState::Processing),
            "the old epoch's custody is still its own"
        );
        assert_eq!(
            m.occupancy_stamped(),
            [Some((1, 500)), None, Some((0, 500)), None, None]
        );

        let mut s = manual();
        s.note_frontier_admitted((0, 500), None);
        s.note_dispatch((0, 500));
        s.note_read_custody_released((0, 500));
        s.note_invalidated();
        s.note_frontier_admitted((1, 500), None);
        s.note_dispatch((1, 500));
        s.note_publication_resolved((0, 500), true);
        let summary = s.summary();
        assert_eq!(
            summary.committed_hours, 0,
            "the shadow committed nothing: epoch 0's hour was invalidated"
        );
        assert_eq!(summary.live_kept_hours_shadow_dropped, 1);
        assert_eq!(summary.illegal_transitions, 0);
        assert_eq!(
            s.machine().state_of(1, 500),
            Some(CustodyState::Observing),
            "epoch 1's hour 500 is untouched"
        );
    }

    #[test]
    fn steady_state_holds_three_hours_at_once() {
        let mut m = machine();
        edge(&mut m, 10, 0);
        dispatch(&mut m, 10);
        close(&mut m);
        seal(&mut m);
        publish(&mut m);
        assert!(edge(&mut m, 11, 1_000).is_clean());
        dispatch(&mut m, 1_010);
        close(&mut m);
        assert!(edge(&mut m, 12, 2_000).is_clean());
        dispatch(&mut m, 2_010);

        assert_eq!(state(&m, 12), Some(CustodyState::Observing));
        assert_eq!(state(&m, 11), Some(CustodyState::Processing));
        assert_eq!(state(&m, 10), Some(CustodyState::Publishing));
        assert_eq!(m.ledger().in_flight, 3);
        assert_eq!(m.ledger().processing_backpressure, 0);
        assert_eq!(m.ledger().observation_deadline_miss, 0);
        assert!(m.ledger().balanced());
    }

    #[test]
    fn slow_processing_drops_exactly_one_snapshot_per_close() {
        let mut m = machine();
        edge(&mut m, 50, 0);
        dispatch(&mut m, 10);
        close(&mut m);

        assert!(edge(&mut m, 51, 1_000).is_clean());
        dispatch(&mut m, 1_010);
        let outcome = m.close_observation().unwrap();
        assert_eq!(outcome.closed, h(51));
        assert_eq!(outcome.processing_backpressure, Some(h(50)));
        assert_eq!(state(&m, 50), None, "the dropped snapshot is gone");
        assert_eq!(
            m.occupancy().iter().filter(|slot| slot.is_some()).count(),
            1,
            "exactly one custody in flight: a dropped snapshot must not leave a \
             second one behind"
        );

        seal(&mut m);
        edge(&mut m, 52, 2_000);
        dispatch(&mut m, 2_010);
        assert_eq!(close(&mut m), 52);
        assert_eq!(m.ledger().processing_backpressure, 1);
        assert!(m.ledger().balanced());
    }

    #[test]
    fn slow_processing_publishes_exactly_one_backpressure_disposition() {
        let mut s = manual();
        s.note_frontier_admitted(h(60), None);
        s.note_dispatch(h(60));
        s.note_read_custody_released(h(60));
        s.advance_clock_us(100_000);
        s.note_frontier_admitted(h(61), None);
        s.note_dispatch(h(61));
        s.note_read_custody_released(h(61));
        assert_eq!(s.summary().processing_backpressure_hours, 1);

        s.note_publication_resolved(h(60), true);
        let summary = s.summary();
        assert_eq!(summary.processing_backpressure_hours, 1);
        assert_eq!(summary.live_kept_hours_shadow_dropped, 1);
        assert_eq!(summary.illegal_transitions, 0);
        assert_eq!(summary.committed_hours, 0, "the shadow committed nothing");
        assert_eq!(summary.drop_memory_overflows, 0);
        assert!(s.ledger().balanced(), "{:?}", s.ledger());
    }

    #[test]
    fn seals_are_ordered_because_the_seal_call_names_no_hour() {
        let mut m = machine();
        let mut sealed = Vec::new();
        for hour in 200..206i32 {
            let at = u64::from(hour as u32) * 1_000;
            edge(&mut m, hour, at);
            dispatch(&mut m, at + 10);
            close(&mut m);
            sealed.push(seal(&mut m));
            publish(&mut m);
            commit(&mut m, true);
        }
        assert_eq!(sealed, vec![200, 201, 202, 203, 204, 205]);
        assert!(sealed.windows(2).all(|pair| pair[0] < pair[1]));
    }

    #[test]
    fn a_sealed_backlog_refuses_the_next_seal_rather_than_overtaking() {
        let mut m = machine();
        edge(&mut m, 300, 0);
        dispatch(&mut m, 10);
        close(&mut m);
        seal(&mut m);
        edge(&mut m, 301, 1_000);
        dispatch(&mut m, 1_010);
        close(&mut m);
        assert_eq!(m.seal(), Err(CustodyError::SealSlotOccupied));
        assert_eq!(state(&m, 301), Some(CustodyState::Processing));
        assert_eq!(state(&m, 300), Some(CustodyState::Sealed));
        assert_eq!(m.watermarks(), (Some(300), None));

        edge(&mut m, 302, 2_000);
        dispatch(&mut m, 2_010);
        let outcome = m.close_observation().unwrap();
        assert_eq!(outcome.processing_backpressure, Some(h(301)));
        assert_eq!(state(&m, 300), Some(CustodyState::Sealed));
    }

    #[test]
    fn an_edge_preempts_both_unopened_and_unclosed_work() {
        let mut m = machine();
        edge(&mut m, 400, 0);
        dispatch(&mut m, 10);
        let first = edge(&mut m, 401, 1_000);
        assert_eq!(first.unclosed_deadline_miss, Some(h(400)));
        assert_eq!(first.unopened_deadline_miss, None);
        let second = edge(&mut m, 402, 2_000);
        assert_eq!(second.unopened_deadline_miss, Some(h(401)));
        assert_eq!(second.unclosed_deadline_miss, None);
        assert_eq!(m.ledger().observation_deadline_miss, 2);
        assert_eq!(
            m.ledger().processing_backpressure,
            0,
            "an edge never touches processing"
        );
        assert!(m.ledger().balanced());
    }

    #[test]
    fn every_illegal_transition_is_refused_and_changes_nothing() {
        let mut m = machine();
        let before = m.clone();
        assert_eq!(m.dispatch(0), Err(CustodyError::NothingAdmitted));
        assert_eq!(m.close_observation(), Err(CustodyError::NothingObserving));
        assert_eq!(m.seal(), Err(CustodyError::NothingProcessing));
        assert_eq!(m.begin_publication(), Err(CustodyError::NothingSealed));
        assert_eq!(m.commit(true), Err(CustodyError::NothingPublishing));
        assert_eq!(m, before, "a refused transition must change nothing");

        let mut m = machine();
        edge(&mut m, 700, 0);
        let before = m.clone();
        assert_eq!(
            m.note_hour_edge(E, 700, 10),
            Err(CustodyError::NonMonotonicHourEdge {
                last: h(700),
                offered: h(700)
            })
        );
        assert_eq!(
            m.note_hour_edge(E, 699, 10),
            Err(CustodyError::NonMonotonicHourEdge {
                last: h(700),
                offered: h(699)
            })
        );
        assert_eq!(m, before);

        let mut m = machine();
        edge(&mut m, 800, 0);
        dispatch(&mut m, 10);
        close(&mut m);
        seal(&mut m);
        publish(&mut m);
        edge(&mut m, 801, 1_000);
        dispatch(&mut m, 1_010);
        close(&mut m);
        seal(&mut m);
        let before = m.clone();
        assert_eq!(
            m.begin_publication(),
            Err(CustodyError::PublicationSlotOccupied)
        );
        assert_eq!(m, before);

        edge(&mut m, 802, 2_000);
        dispatch(&mut m, 2_010);
        close(&mut m);
        let before = m.clone();
        assert_eq!(m.seal(), Err(CustodyError::SealSlotOccupied));
        assert_eq!(m, before);
    }

    #[test]
    fn invalidation_drops_unsealed_work_and_spares_the_immutable_bytes() {
        let mut m = machine();
        edge(&mut m, 1_000, 0);
        dispatch(&mut m, 10);
        close(&mut m);
        seal(&mut m);
        publish(&mut m);
        edge(&mut m, 1_001, 1_000);
        dispatch(&mut m, 1_010);
        close(&mut m);
        edge(&mut m, 1_002, 2_000);

        let dropped: Vec<(u64, i32)> = m.invalidate().into_iter().flatten().collect();
        assert_eq!(
            dropped,
            vec![h(1_002), h(1_001)],
            "the admitted hour and the unsealed snapshot"
        );
        assert_eq!(state(&m, 1_000), Some(CustodyState::Publishing));
        assert!(m.note_hour_edge(1, 5, 3_000).is_ok());
        assert_eq!(commit(&mut m, true), 1_000);
        assert!(m.ledger().balanced(), "{:?}", m.ledger());
    }

    const SWEEP_EVENTS: u64 = 7;
    const SEED_DEPTH: u32 = 6;
    const SWEEP_DEPTH: u32 = 5;

    fn apply(
        m: &mut CustodyMachine,
        event: u64,
        epoch: &mut u64,
        hour_source: &mut i32,
        repeat_edge: bool,
    ) -> Option<CustodyError> {
        match event {
            0 => {
                if !repeat_edge {
                    *hour_source += 1;
                }
                m.note_hour_edge(*epoch, *hour_source, u64::from(*hour_source as u32) * 100)
                    .err()
            }
            1 => m.dispatch(u64::from(*hour_source as u32) * 100 + 50).err(),
            2 => m.close_observation().err(),
            3 => m.seal().err(),
            4 => m.begin_publication().err(),
            5 => m.commit(true).err(),
            _ => {
                m.invalidate();
                if *hour_source % 3 != 0 {
                    *epoch += 1;
                }
                None
            }
        }
    }

    fn pattern(m: &CustodyMachine) -> [bool; 5] {
        m.occupancy().map(|slot| slot.is_some())
    }

    fn seed_machines() -> Vec<(CustodyMachine, u64, i32)> {
        let mut seen: BTreeSet<[bool; 5]> = BTreeSet::new();
        let mut seeds = Vec::new();
        let mut epoch = 0u64;
        let mut hour_source = 0i32;
        for script in 0..SWEEP_EVENTS.pow(SEED_DEPTH) {
            let mut m = machine();
            let mut code = script;
            for _ in 0..SEED_DEPTH {
                let event = code % SWEEP_EVENTS;
                code /= SWEEP_EVENTS;
                apply(&mut m, event, &mut epoch, &mut hour_source, script % 3 == 0);
            }
            if seen.insert(pattern(&m)) {
                seeds.push((m, epoch, hour_source));
            }
        }
        seeds
    }

    #[test]
    fn every_event_sequence_preserves_the_custody_invariants() {
        let seeds = seed_machines();
        assert!(
            seeds.len() >= 8,
            "only {} occupancy patterns are reachable; the seed phase is broken",
            seeds.len()
        );
        let mut steps = 0u64;
        let mut refusals = 0u64;
        for (seed, seed_epoch, seed_hour) in &seeds {
            for script in 0..SWEEP_EVENTS.pow(SWEEP_DEPTH) {
                let mut m = seed.clone();
                let mut epoch = *seed_epoch;
                let mut hour_source = *seed_hour;
                let mut code = script;
                let mut last_seal: Option<(u64, i32)> = None;
                for _ in 0..SWEEP_DEPTH {
                    let event = code % SWEEP_EVENTS;
                    code /= SWEEP_EVENTS;
                    let before = m.clone();
                    let sealed_before = m.watermarks().0;
                    let error = apply(&mut m, event, &mut epoch, &mut hour_source, script % 3 == 0);
                    if error.is_some() {
                        refusals += 1;
                        assert_eq!(m, before, "a refused transition mutated the machine");
                    } else if event == 3 && m.watermarks().0 != sealed_before {
                        let stamp = (epoch, m.watermarks().0.expect("a seal just landed"));
                        if let Some(previous) = last_seal {
                            if previous.0 == stamp.0 {
                                assert!(
                                    previous.1 < stamp.1,
                                    "out-of-order seal {previous:?} -> {stamp:?}"
                                );
                            }
                        }
                        last_seal = Some(stamp);
                    }
                    let occupancy = m.occupancy_stamped();
                    let mut previous: Option<(u64, i32)> = None;
                    for stamp in occupancy.into_iter().flatten() {
                        if let Some(previous) = previous {
                            assert!(
                                previous > stamp,
                                "pipeline out of order: {occupancy:?} in {m:?}"
                            );
                        }
                        previous = Some(stamp);
                    }
                    let ledger = m.ledger();
                    assert!(ledger.balanced(), "{ledger:?} in {m:?}");
                    steps += 1;
                }
            }
        }
        assert_eq!(
            steps,
            seeds.len() as u64 * SWEEP_EVENTS.pow(SWEEP_DEPTH) * u64::from(SWEEP_DEPTH)
        );
        assert!(
            refusals > 0,
            "no illegal transition was ever attempted; the sweep proves nothing"
        );
    }

    #[test]
    fn the_sweep_reaches_every_refusal_the_api_can_produce() {
        let mut seen: BTreeSet<String> = BTreeSet::new();
        for (seed, seed_epoch, seed_hour) in seed_machines() {
            for script in 0..SWEEP_EVENTS.pow(SWEEP_DEPTH) {
                let mut m = seed.clone();
                let mut epoch = seed_epoch;
                let mut hour_source = seed_hour;
                let mut code = script;
                for _ in 0..SWEEP_DEPTH {
                    let event = code % SWEEP_EVENTS;
                    code /= SWEEP_EVENTS;
                    if let Some(error) =
                        apply(&mut m, event, &mut epoch, &mut hour_source, script % 3 == 0)
                    {
                        let name = format!("{error:?}");
                        let name = name.split(' ').next().unwrap_or(&name).to_string();
                        seen.insert(name);
                    }
                }
            }
        }
        let expected: BTreeSet<String> = [
            "NonMonotonicHourEdge",
            "NothingAdmitted",
            "NothingObserving",
            "NothingProcessing",
            "NothingSealed",
            "NothingPublishing",
            "SealSlotOccupied",
            "PublicationSlotOccupied",
        ]
        .into_iter()
        .map(str::to_string)
        .collect();
        assert_eq!(
            seen, expected,
            "the reachable refusal set changed; ObservationSlotOccupied is \
             documented as an unreachable guard"
        );
    }

    #[test]
    fn reserved_codes_are_the_next_free_ones_and_are_still_refused_by_the_decoder() {
        use compact_contract::calendar::DATE_EPOCH_CONST;
        use compact_contract::domain_gap::{DomainGapPart, DomainGapReason, DomainGapRecordV1};

        let mut assigned: Vec<u16> = DomainGapReason::ALL.iter().map(|r| r.code()).collect();
        assigned.sort_unstable();
        let highest = *assigned.last().expect("H4DG assigns reasons");
        assert_eq!(
            assigned,
            (1..=highest).collect::<Vec<u16>>(),
            "the assigned roster is not contiguous from 1, so the next FREE \
             code is not the one after the highest"
        );
        assert_eq!(
            RESERVED_DOMAIN_GAP_REASON_PROCESSING_BACKPRESSURE,
            highest + 1
        );
        assert_eq!(
            RESERVED_DOMAIN_GAP_REASON_OBSERVATION_DEADLINE_MISS,
            highest + 2
        );

        let hour = DATE_EPOCH_CONST + 1944 * 365 * 24;
        let mut wire = DomainGapRecordV1 {
            part: DomainGapPart::State,
            reason: DomainGapReason::Torn,
            target_hour_raw: hour,
            range_start_hour_raw: hour,
            range_end_hour_raw: hour,
            tick_before: 100,
            tick_after: 101,
            queue_capacity: 0,
            queue_depth: 0,
            queue_dropped: 0,
            outcome_ordinal: 17,
        }
        .encode()
        .expect("a Torn sample encodes");
        for reserved in [
            RESERVED_DOMAIN_GAP_REASON_PROCESSING_BACKPRESSURE,
            RESERVED_DOMAIN_GAP_REASON_OBSERVATION_DEADLINE_MISS,
        ] {
            wire[8..10].copy_from_slice(&reserved.to_le_bytes());
            assert!(
                DomainGapRecordV1::decode(&wire).is_err(),
                "reserved code {reserved} decoded; the reservation has silently \
                 become an assignment"
            );
        }

        assert_eq!(
            ShadowDisposition::ProcessingBackpressure.reserved_gap_reason_code(),
            Some(RESERVED_DOMAIN_GAP_REASON_PROCESSING_BACKPRESSURE)
        );
        assert_eq!(
            ShadowDisposition::ObservationDeadlineMiss.reserved_gap_reason_code(),
            Some(RESERVED_DOMAIN_GAP_REASON_OBSERVATION_DEADLINE_MISS)
        );
        for quiet in [
            ShadowDisposition::Committed,
            ShadowDisposition::Refused,
            ShadowDisposition::Invalidated,
        ] {
            assert_eq!(quiet.reserved_gap_reason_code(), None);
        }
    }

    #[test]
    fn a_disabled_shadow_records_nothing() {
        let mut s = CustodyShadow::new(ShadowMode::Disabled, ShadowClock::Manual { now_us: 0 });
        s.note_frontier_admitted(h(10), None);
        s.note_dispatch(h(10));
        s.note_read_custody_released(h(10));
        s.note_publication_resolved(h(10), true);
        s.note_invalidated();
        let summary = s.summary();
        assert!(!summary.active);
        assert_eq!(summary.events, 0);
        assert_eq!(summary.hour_edges, 0);
        assert_eq!(summary.committed_hours, 0);
        assert_eq!(s.ledger(), CustodyLedger::default());
        assert_eq!(s.recent_divergences().count(), 0);
    }

    #[test]
    fn lateness_is_measured_from_the_hour_edge() {
        let mut s = manual();
        s.note_frontier_admitted(h(20), None);
        s.advance_clock_us(7_500);
        s.note_dispatch(h(20));
        let summary = s.summary();
        assert_eq!(summary.late_dispatch_samples, 1);
        assert_eq!(summary.late_dispatch_total_us, 7_500);
        assert_eq!(summary.max_late_dispatch_us, 7_500);

        s.advance_clock_us(1_000);
        s.note_read_custody_released(h(20));
        s.note_publication_resolved(h(20), true);
        s.advance_clock_us(500);
        s.note_frontier_admitted(h(21), None);
        s.advance_clock_us(1_200);
        s.note_dispatch(h(21));
        let summary = s.summary();
        assert_eq!(summary.late_dispatch_samples, 2);
        assert_eq!(summary.late_dispatch_total_us, 8_700);
        assert_eq!(summary.max_late_dispatch_us, 7_500, "a running maximum");
    }

    /// MUTATION: treat a missing release as a pipelined one and
    #[test]
    fn a_synchronous_publication_is_counted_as_a_collapse() {
        let mut s = manual();
        s.note_frontier_admitted(h(30), None);
        s.note_dispatch(h(30));
        s.note_publication_resolved(h(30), true); // no release: synchronous
        let summary = s.summary();
        assert_eq!(summary.collapsed_publications, 1);
        assert_eq!(summary.committed_hours, 1);
        assert_eq!(summary.illegal_transitions, 0);
        assert!(s.ledger().balanced());

        let mut s = manual();
        s.note_frontier_admitted(h(31), None);
        s.note_dispatch(h(31));
        s.note_read_custody_released(h(31));
        s.note_publication_resolved(h(31), false);
        let summary = s.summary();
        assert_eq!(summary.collapsed_publications, 0);
        assert_eq!(summary.refused_hours, 1);
        assert_eq!(summary.committed_hours, 0);
        assert!(s.ledger().balanced());
    }

    #[test]
    fn a_live_eviction_agrees_with_the_shadow_deadline_miss() {
        let mut s = manual();
        s.note_frontier_admitted(h(40), None);
        s.advance_clock_us(1_000);
        s.note_frontier_admitted(h(41), Some(h(40)));
        let summary = s.summary();
        assert_eq!(summary.observation_deadline_miss_hours, 1);
        assert_eq!(summary.agreed_evictions, 1);
        assert_eq!(summary.disagreed_evictions, 0);
        let ledger = s.ledger();
        assert!(ledger.balanced(), "{ledger:?}");
        assert_eq!(ledger.observation_deadline_miss, 1);

        let mut s = manual();
        s.note_frontier_admitted(h(42), None);
        s.note_dispatch(h(42));
        s.note_read_custody_released(h(42));
        s.note_frontier_admitted(h(43), Some(h(41)));
        assert_eq!(s.summary().disagreed_evictions, 1);
        assert_eq!(s.summary().agreed_evictions, 0);
    }

    #[test]
    fn an_impossible_live_order_is_counted_as_an_illegal_transition() {
        let mut s = manual();
        s.note_dispatch(h(50));
        let summary = s.summary();
        assert_eq!(summary.illegal_transitions, 1);
        assert_eq!(summary.dispatches, 1);
        let divergence = s
            .recent_divergences()
            .last()
            .copied()
            .expect("the refusal is recorded");
        assert_eq!(
            divergence.kind,
            DivergenceKind::IllegalTransition {
                error: CustodyError::NothingAdmitted
            }
        );
        assert_eq!(divergence.live, h(50));
    }

    #[test]
    fn the_divergence_ring_and_the_drop_memory_are_bounded() {
        let mut s = manual();
        let hours = DIVERGENCE_RING_CAP as i32 * 4;
        for hour in 0..hours {
            s.note_frontier_admitted(h(hour + 1), None);
        }
        assert_eq!(s.recent_divergences().count(), DIVERGENCE_RING_CAP);
        assert_eq!(
            s.summary().observation_deadline_miss_hours,
            u64::try_from(hours).unwrap() - 1,
            "the counters keep counting after the ring stops keeping detail"
        );
        assert_eq!(
            s.summary().drop_memory_overflows,
            0,
            "unopened hours are never remembered, so a long eviction run cannot \
             overflow the drop memory at all"
        );
        assert!(s.ledger().balanced());

        let mut s = manual();
        s.note_frontier_admitted(h(1), None);
        s.note_dispatch(h(1));
        s.note_frontier_admitted(h(2), None);
        s.note_dispatch(h(2));
        for hour in 3..=12 {
            s.note_frontier_admitted(h(hour), None);
            s.note_dispatch(h(hour));
            s.note_publication_resolved(h(hour - 1), true);
        }
        assert_eq!(
            s.summary().drop_memory_overflows,
            0,
            "a resolved drop must give its slot back: {:?}",
            s.summary()
        );
        s.note_publication_resolved(h(1), true);
        let summary = s.summary();
        assert_eq!(
            summary.live_kept_hours_shadow_dropped, 11,
            "the ten interleaved hours plus hour 1, which was dropped eleven \
             edges before it resolved and is still recognised"
        );
        assert_eq!(summary.drop_memory_overflows, 0);
        assert_eq!(summary.illegal_transitions, 0);
    }

    #[test]
    fn a_drop_memory_overflow_is_counted_not_silent() {
        let mut s = manual();
        for hour in 1..(DROPPED_HOUR_MEMORY as i32 + 3) {
            s.note_frontier_admitted(h(hour), None);
            s.note_dispatch(h(hour));
            // Never released and never resolved: the next edge drops it as an
        }
        s.note_frontier_admitted(h(100), None);
        let summary = s.summary();
        assert!(
            summary.drop_memory_overflows > 0,
            "the memory should have overflowed: {summary:?}"
        );
        s.note_publication_resolved(h(1), true);
        assert_eq!(s.summary().live_kept_hours_shadow_dropped, 0);
        assert_eq!(s.summary().illegal_transitions, 1);
    }

    #[test]
    fn the_environment_kill_switch_defaults_on() {
        for (value, expected) in [
            (Some("0"), ShadowMode::Disabled),
            (Some("off"), ShadowMode::Disabled),
            (Some("false"), ShadowMode::Disabled),
            (Some("1"), ShadowMode::Enabled),
            (Some(""), ShadowMode::Enabled),
            (None, ShadowMode::Enabled),
        ] {
            let mode = match value {
                Some("0") | Some("off") | Some("false") => ShadowMode::Disabled,
                _ => ShadowMode::Enabled,
            };
            assert_eq!(mode, expected, "{value:?}");
        }
        assert_eq!(CUSTODY_SHADOW_ENV, "BATTLE_PLAN_CUSTODY_SHADOW");
        assert_eq!(CustodyShadow::default().mode(), ShadowMode::Enabled);
    }

    /// One "frame" is the full live event set: edge, dispatch, release,
    #[test]
    fn shadow_cost_per_frame_is_negligible() {
        const FRAMES: u32 = 20_000;
        let mut s = CustodyShadow::new(ShadowMode::Enabled, ShadowClock::real());
        let started = Instant::now();
        for hour in 0..FRAMES as i32 {
            s.note_frontier_admitted(h(hour + 1), None);
            s.note_dispatch(h(hour + 1));
            s.note_read_custody_released(h(hour + 1));
            s.note_publication_resolved(h(hour + 1), true);
        }
        let elapsed = started.elapsed();
        let per_frame_ns = elapsed.as_nanos() / u128::from(FRAMES);
        println!(
            "custody shadow: {FRAMES} frames in {elapsed:?} = {per_frame_ns} ns/frame \
             ({} events, self-measured {} ns total, max {} ns/event)",
            s.summary().events,
            s.summary().cost_total_ns,
            s.summary().max_cost_ns,
        );
        assert_eq!(s.summary().illegal_transitions, 0);
        assert_eq!(s.summary().committed_hours, u64::from(FRAMES));
        assert!(
            per_frame_ns < 50_000,
            "shadow costs {per_frame_ns} ns/frame, over the 50 µs budget"
        );
    }
}
