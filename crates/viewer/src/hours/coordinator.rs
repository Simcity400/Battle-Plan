//! Event-driven coordinator adapter. It never seeks, waits on a slot, or parks
//! on pool/output admission. Accepted work retains its original epoch/credit.
use super::*;
use dispatch_plan::{DispatchPlan, HeldJob, PreparedTarget};
use flow::Ledger;
use viewer_foundation::builder_pool::{CapacityListener, TrySubmitError};

enum Turn {
    Progress,
    Park,
    Stop,
}
struct Reading {
    ticket: u64,
    key: (BranchId, Hour),
    requested: Option<trace::Stamp>,
    handle: SourceHandle,
}
// The production builder is monomorphized; tests substitute only work bodies,
// never the event/admission/output pump or its state transitions.
pub(super) trait BuildTarget: std::ops::DerefMut<Target = CoordinatorState> {
    fn prepare(&mut self, plan: DispatchPlan, current: Current) -> PreparedTarget;
    fn colour_work(&self, plan: &mut DispatchPlan, current: Current) -> Box<dyn FnOnce() + Send>;
}
struct History {
    ticket: u64,
    key: (BranchId, Hour),
    // None means pool-accepted; retain the credit until its terminal reply.
    work: Option<Box<dyn FnOnce() + Send>>,
}
struct Driver<B: BuildTarget = Coordinator> {
    core: B,
    ledger: Ledger<(BranchId, Hour)>,
    decode: Option<DispatchPlan>,
    reading: Option<Reading>,
    history: Option<History>,
    source: Arc<SourceMailbox>,
    targets: BTreeMap<u64, PreparedTarget>,
    slot_tickets: BTreeMap<(BranchId, Hour), u64>,
    sending: BTreeSet<u64>,
    capacity: CapacityListener,
    pool_blocked: bool,
    output_blocked: bool,
    started: Instant,
}

pub(super) fn run(core: Coordinator, inbox: Receiver<Event>) {
    let source = match &core.shared_decoder {
        Some(decoder) => decoder.client(),
        None => core.lane.as_ref().expect("campaign source").client(),
    };
    let mut driver = Driver::new(core, source);
    let mut next = None;
    loop {
        if let Some(event) = next.take()
            && !driver.event(event)
        {
            return;
        }
        // A poll reduces the complete available batch before choosing stable
        // ready tickets. Independent unavailable targets never head-of-line it.
        loop {
            match inbox.try_recv() {
                Ok(event) => {
                    if !driver.event(event) {
                        return;
                    }
                }
                Err(mpsc::TryRecvError::Empty) => break,
                Err(mpsc::TryRecvError::Disconnected) => return,
            }
        }
        match driver.turn() {
            Turn::Stop => return,
            Turn::Progress => {} // service inbox again before another admission
            Turn::Park => match inbox.recv() {
                Ok(event) => next = Some(event),
                Err(_) => return,
            },
        }
    }
}

impl<B: BuildTarget> Driver<B> {
    fn new(core: B, source: Arc<SourceMailbox>) -> Self {
        let events = core.events.clone();
        // Register before admission; callbacks only post nonblocking wakes.
        let capacity = core.pool.listen_capacity(move || {
            let _ = events.send(Event::PoolSpace);
        });
        Self {
            ledger: Ledger::new(core.parallel),
            core,
            source,
            capacity,
            decode: None,
            reading: None,
            history: None,
            targets: BTreeMap::new(),
            slot_tickets: BTreeMap::new(),
            sending: BTreeSet::new(),
            pool_blocked: false,
            output_blocked: false,
            started: Instant::now(),
        }
    }

    fn event(&mut self, event: Event) -> bool {
        match event {
            Event::Shutdown => return false,
            Event::PoolSpace => self.pool_blocked = false,
            Event::ResultSpace => {
                // Clear coalescing before retry: a subsequent UI receive must
                // be able to post another wake, including across a Full result.
                self.core.result_space.store(false, Ordering::Release);
                self.output_blocked = false;
            }
            Event::Want => {
                let requested = self
                    .core
                    .requested
                    .lock()
                    .unwrap_or_else(|p| p.into_inner())
                    .take();
                if let Some((epoch, want, posted)) = requested {
                    self.core.profile_want = posted;
                    self.core.profile_received =
                        trace::now(viewer_foundation::profile_writer::profiling());
                    self.core.set_want(epoch, want);
                    self.slot_tickets
                        .retain(|key, _| self.core.slots.contains_key(key));
                    self.retarget();
                }
            }
            Event::Frame { ticket, key, reply } => {
                if !self
                    .reading
                    .as_ref()
                    .is_some_and(|r| r.ticket == ticket && r.key == key)
                {
                    return true;
                }
                if self.history.is_some() {
                    return true; // the matching source reply was already consumed
                }
                let reading = self.reading.take().expect("matching request");
                self.retarget();
                if self.ledger.get(ticket).is_none_or(|flight| flight.retired) {
                    self.ledger.source_finished(ticket);
                    self.retarget();
                    return true;
                }
                let Some(plan) = &mut self.decode else {
                    return true;
                };
                assert_eq!(plan.ticket, ticket);
                let (current, sample) = match reply {
                    source_mailbox::Reply::Ready { value, sample } => (value, sample),
                    source_mailbox::Reply::Cancelled => {
                        assert!(self.ledger.source_finished(ticket));
                        return true;
                    }
                    source_mailbox::Reply::Stopped => (
                        Current::Gap {
                            branch: key.0,
                            hour: key.1,
                            reason: LANE_STOPPED.to_owned(),
                        },
                        None,
                    ),
                };
                if key.1 < plan.hour {
                    plan.trace
                        .decoded_step(key.0, key.1, reading.requested, sample);
                    if plan.colour.as_ref().is_some_and(Result::is_err) {
                        // Preserve the first colour refusal, but retain the same
                        // native-source traversal and endpoint error precedence.
                        assert!(self.ledger.source_finished(ticket));
                        plan.next_source += 1;
                    } else {
                        let work = self.core.colour_work(plan, current);
                        self.history = Some(History {
                            ticket,
                            key,
                            work: Some(work),
                        });
                        self.reading = Some(reading);
                    }
                } else {
                    plan.trace.decoded(key.0, key.1, reading.requested, sample);
                    assert!(self.ledger.source_finished(ticket));
                    let plan = self.decode.take().expect("endpoint");
                    let key = (plan.want.branch, plan.hour);
                    let prepared = self.core.prepare(plan, current);
                    let reused = match &prepared {
                        PreparedTarget::Ready(_) => u8::MAX,
                        PreparedTarget::Parts(pending, _) => {
                            self.slot_tickets.insert(key, ticket);
                            if pending.received == 0 {
                                0
                            } else {
                                0b1111_1000
                            }
                        }
                    };
                    assert!(if reused == u8::MAX {
                        self.ledger.gap(ticket)
                    } else {
                        self.ledger.decoded(ticket, reused)
                    });
                    self.targets.insert(ticket, prepared);
                }
            }
            Event::Colour {
                ticket,
                key,
                folded,
            } => {
                if !self.history.as_ref().is_some_and(|history| {
                    history.ticket == ticket && history.key == key && history.work.is_none()
                }) {
                    return true;
                }
                self.history = None;
                self.reading = None;
                assert!(self.ledger.source_finished(ticket));
                self.retarget();
                if let Some(plan) = &mut self.decode {
                    assert_eq!(plan.ticket, ticket);
                    plan.folded(*folded);
                }
            }
            Event::Part {
                ticket,
                epoch,
                branch,
                hour,
                name,
                timing,
                children,
                part,
            } => {
                let valid = self
                    .ledger
                    .get(ticket)
                    .is_some_and(|f| f.epoch == epoch && f.key == (branch, hour));
                if valid && self.ledger.completed(ticket, part_rank(&part)) {
                    let Some(PreparedTarget::Parts(pending, _)) = self.targets.get_mut(&ticket)
                    else {
                        return false;
                    };
                    pending.take(ticket, name, timing, children, *part);
                }
            }
        }
        true
    }

    fn retarget(&mut self) {
        let cancelled = retarget_ledger(
            &mut self.ledger,
            &self.core,
            self.core.want.as_ref(),
            self.core.epoch,
        );
        for ticket in cancelled {
            let key = self.targets.get(&ticket).map(|target| match target {
                PreparedTarget::Parts(pending, _) => match &pending.current {
                    Current::Frame(f) => (f.branch, f.hour),
                    Current::Gap { branch, hour, .. } => (*branch, *hour),
                },
                PreparedTarget::Ready(built) => (built.branch, built.hour),
            });
            // Only wholly unaccepted work can reach this branch. Dropping its
            // captured Fill guards terminates slots, never cancels a pool job.
            self.targets.remove(&ticket);
            self.sending.remove(&ticket);
            if self.decode.as_ref().is_some_and(|p| p.ticket == ticket) {
                self.decode = None;
            }
            if let Some(key) = key
                && self.slot_tickets.get(&key) == Some(&ticket)
            {
                self.slot_tickets.remove(&key);
                self.core.slots.remove(&key);
            }
        }
        if let Some(reading) = &self.reading
            && self.ledger.obsolete_reads().contains(&reading.ticket)
        {
            reading.handle.cancel();
            if self
                .history
                .as_ref()
                .is_some_and(|history| history.work.is_some())
            {
                // The fold was not accepted. Drop its one frame before returning
                // the source credit; an accepted fold instead acknowledges itself.
                let ticket = reading.ticket;
                self.history = None;
                self.reading = None;
                assert!(self.ledger.source_finished(ticket));
                self.retarget();
            }
        }
    }

    fn turn(&mut self) -> Turn {
        let mut progress = false;
        for ticket in self.ledger.terminal_retired() {
            self.targets.remove(&ticket);
            self.sending.remove(&ticket);
            assert!(self.ledger.release(ticket));
            progress = true;
        }
        self.retarget();
        // First assemble every ready target, regardless of output admission.
        // In particular a later decode/Full pool cannot postpone this work.
        for ticket in self.ledger.ready() {
            if matches!(self.targets.get(&ticket), Some(PreparedTarget::Parts(_, _))) {
                let PreparedTarget::Parts(pending, jobs) =
                    self.targets.remove(&ticket).expect("ready payload")
                else {
                    unreachable!()
                };
                assert!(jobs.is_empty());
                let f = self.ledger.get(ticket).expect("ready credit");
                let built =
                    self.core
                        .finish_pending(f.epoch, f.key.0, f.key.1, *pending, self.started);
                self.targets
                    .insert(ticket, PreparedTarget::Ready(Box::new(built)));
                progress = true;
            }
        }
        if !self.output_blocked {
            for ticket in self.ledger.ready() {
                let Some(PreparedTarget::Ready(built)) = self.targets.remove(&ticket) else {
                    unreachable!()
                };
                let key = (built.branch, built.hour);
                let (built_epoch, usable) = (built.epoch, built.layers.is_ok());
                let trace = built.trace.clone();
                if self.sending.insert(ticket) {
                    trace.sending();
                }
                match self.core.outbox.try_send(*built) {
                    Ok(()) => {
                        trace.sent();
                        let order = self.core.sends;
                        self.core.sent.insert(key, order);
                        self.core.note_sent(built_epoch, key, order, usable);
                        self.core.sends += 1;
                        self.sending.remove(&ticket);
                        assert!(self.ledger.release(ticket));
                        progress = true;
                    }
                    Err(mpsc::TrySendError::Full(built)) => {
                        self.targets
                            .insert(ticket, PreparedTarget::Ready(Box::new(built)));
                        self.output_blocked = true;
                        break;
                    }
                    Err(mpsc::TrySendError::Disconnected(_)) => return Turn::Stop,
                }
            }
        }
        if !self.pool_blocked
            && let Some((ticket, part)) = self.ledger.next_admission()
        {
            let before = self.capacity.acknowledge(); // ALWAYS before trying
            let Some(PreparedTarget::Parts(pending, jobs)) = self.targets.get_mut(&ticket) else {
                return Turn::Stop;
            };
            let HeldJob { name, work } = jobs.pop_front().expect("held part");
            assert_eq!(lap_rank(name) - 1, part);
            match self.core.pool.try_submit_status(work) {
                Ok(()) => {
                    assert!(self.ledger.accepted(ticket, part));
                    pending.trace.accepted(name);
                    return Turn::Progress;
                }
                Err(TrySubmitError::Full(work)) => {
                    jobs.push_front(HeldJob { name, work });
                    // A dequeue between acknowledge and Full must retry after
                    // inbox service, even if its coalesced wake was consumed.
                    self.pool_blocked = self.capacity.generation() == before;
                    if !self.pool_blocked {
                        return Turn::Progress;
                    }
                }
                Err(TrySubmitError::Disconnected(work)) => {
                    drop(work);
                    return Turn::Stop;
                }
            }
        }
        if !self.pool_blocked
            && let Some(history) = &mut self.history
            && let Some(work) = history.work.take()
        {
            let before = self.capacity.acknowledge();
            match self.core.pool.try_submit_status(work) {
                Ok(()) => return Turn::Progress,
                Err(TrySubmitError::Full(work)) => {
                    history.work = Some(work);
                    self.pool_blocked = self.capacity.generation() == before;
                    if !self.pool_blocked {
                        return Turn::Progress;
                    }
                }
                Err(TrySubmitError::Disconnected(work)) => {
                    drop(work);
                    return Turn::Stop;
                }
            }
        }
        if self.reading.is_none()
            && let Some(plan) = &self.decode
        {
            let ticket = plan.ticket;
            let key = (plan.want.branch, plan.next_source);
            let requested = trace::now(plan.trace.enabled());
            let events = self.core.events.clone();
            assert!(self.ledger.source_started(ticket));
            let handle = self.source.post(ticket, key, move |ticket, reply| {
                let _ = events.send(Event::Frame { ticket, key, reply });
            });
            self.reading = Some(Reading {
                ticket,
                key,
                requested,
                handle,
            });
            return Turn::Progress;
        }
        if self.ledger.can_plan()
            && let Some(hour) = self.next_hour()
        {
            let plan = self.core.plan(hour);
            let dependencies = neighbour_slots(&self.core.slots, plan.want.branch, hour)
                .into_iter()
                .filter_map(|(key, _)| self.slot_tickets.get(&key).copied())
                .filter(|ticket| self.ledger.get(*ticket).is_some())
                .collect();
            assert!(self.ledger.reserve(
                plan.ticket,
                plan.epoch,
                (plan.want.branch, hour),
                dependencies
            ));
            self.decode = Some(plan);
            return Turn::Progress;
        }
        if progress { Turn::Progress } else { Turn::Park }
    }

    fn next_hour(&self) -> Option<Hour> {
        let want = self.core.want.as_ref()?;
        let range = self.core.playback.range(want.branch)?;
        self.core
            .wanted_hours(want, range)
            .into_iter()
            .find(|hour| {
                let key = (want.branch, *hour);
                !self.core.sent.contains_key(&key)
                    && !want.have.contains(hour)
                    && !self.ledger.contains(self.core.epoch, &key)
            })
    }
}
// A ready result must drain even when every part was reused.
fn started(flight: &flow::Flight<(BranchId, Hour)>) -> bool {
    flight.accepted_any() || flight.phase == flow::Phase::Ready
}

pub(super) fn retarget_ledger(
    ledger: &mut Ledger<(BranchId, Hour)>,
    core: &CoordinatorState,
    want: Option<&Want>,
    epoch: u64,
) -> Vec<u64> {
    let keep: BTreeSet<_> = want
        .map(|want| {
            let range = core
                .playback
                .range(want.branch)
                .unwrap_or((want.hour, want.hour));
            core.wanted_hours(want, range)
                .into_iter()
                .map(|hour| (want.branch, hour))
                .collect()
        })
        .unwrap_or_default();
    match want.filter(|want| want.grid.is_some()) {
        Some(want) => {
            let facts = want.grid_facts(
                ledger
                    .flights()
                    .filter(|(_, flight)| flight.epoch == epoch)
                    .map(|(_, flight)| (flight.key, started(flight))),
                core.usable_sent.iter().copied().filter(|key| {
                    core.sent
                        .get(key)
                        .is_some_and(|order| *order >= want.received)
                }),
            );
            ledger.retarget_flights(|_, flight| {
                flight.epoch == epoch
                    && want.keeps_flight(flight.key, started(flight), &keep, &facts)
            })
        }
        None => ledger
            .retarget(|e, key| e == epoch && want.is_some_and(|w| w.keeps_pending(*key, &keep))),
    }
}

impl<B: BuildTarget> Drop for Driver<B> {
    fn drop(&mut self) {
        if let Some(reading) = &self.reading {
            reading.handle.cancel();
        }
        // Do not clear accepted Pending.alive. Accepted jobs, including still
        // queued jobs, run to completion and fill their captured slots.
    }
}

#[cfg(test)]
mod tests;
