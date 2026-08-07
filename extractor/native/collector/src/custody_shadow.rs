//! Processing-custody state machine, run in SHADOW MODE beside the live
//! scheduler (capture-floor stage S3).
//!
//! # What this is
//!
//! The adopted capture-floor plan replaces today's read-then-publish sweep with
//! a custody pipeline whose phases each hold exactly one hour:
//!
//! ```text
//!   OBSERVING(H) -> PROCESSING(H) -> SEALED(H) -> PUBLISHING(H) -> COMMITTED(H)
//! ```
//!
//! In steady state hour `H` is being observed while `H-1` is processed and
//! `H-2` is published. Observation is the only phase that touches the game, it
//! is aligned to the hour edge, and it PREEMPTS compute: when the edge arrives
//! and a downstream phase is still busy, the oldest UNSEALED snapshot is
//! dropped with a typed `ProcessingBackpressure` disposition rather than the
//! observation being delayed. Work that was admitted but never opened by the
//! next boundary is dropped as `ObservationDeadlineMiss`. Both are losses the
//! end state must NAME, which is why they are dispositions and not silence.
//!
//! t35 is why the observation phase must both shrink and align to the edge:
//! frames that started less than 10 ms late still mislabelled ~19.8 domains on
//! average, and frames 10-20 ms late mislabelled ~41.8
//! (`runs/s0_census/t35/hour_label_audit.txt`).
//!
//! # Shadow mode
//!
//! Nothing here changes what the recorder captures. [`CustodyShadow`] is driven
//! by the events the live [`crate::read_only_scheduler::ReadOnlyScheduler`]
//! already produces and answers one question per frame: *what would the end
//! state have done here?* The answers reach the wire only as cumulative
//! `rs_shadow_*` diagnostic units on the reserved probe domain.
//!
//! ## What the live events can and cannot tell it
//!
//! The mapping from live events to phases is exact for three of the five
//! phases and a PROXY for the other two, and the difference is stated wherever
//! a counter depends on it:
//!
//! | phase | live event that enters it | exact? |
//! |---|---|---|
//! | OBSERVING | `take_current_request` | yes -- the bracket starts there |
//! | PROCESSING | `release_read_custody` | yes -- the bytes are frozen there |
//! | SEALED / PUBLISHING / COMMITTED | the sink outcome | PROXY |
//!
//! Today's recorder has no separate compute phase, so there is no event that
//! says "the end state's processing of hour H finished". The shadow therefore
//! leaves PROCESSING when the frame's SINK OUTCOME lands, which means an hour
//! counted as still-unsealed may in truth have been held up by the sink rather
//! than by compute. [`CustodyShadowSummary::processing_backpressure_hours`]
//! spells out what that makes it and what it does not.
//!
//! ## Why it cannot change a decision -- and what it DOES cost
//!
//! The claim is about DECISIONS, not about wall-clock, and the distinction is
//! load-bearing. What is structural:
//!
//! 1. Every method of [`CustodyShadow`] and [`CustodyMachine`] takes `&mut
//!    self` plus `Copy` scalars. Neither type is ever handed a
//!    `&ReadOnlyScheduler`, so neither can read or write one field of the live
//!    scheduler. It learns only what the scheduler chooses to tell it.
//! 2. Every notification returns `()`. There is no value a live decision could
//!    branch on even if it wanted to.
//! 3. Every notification is made AFTER the live decision has been computed, at
//!    the end of the method that computed it.
//!
//! `read_only_scheduler`'s
//! `the_shadow_cannot_change_a_dispatch_decision_even_when_it_is_hostile`
//! drives the same script through a shadow-off scheduler, a shadow-on
//! scheduler, and a scheduler whose shadow destroys its own state inside every
//! notification, and requires all three to produce identical decisions,
//! telemetry, ledgers and control ranges.
//!
//! WHAT IT DOES COST, stated rather than waved away. The notifications run
//! synchronously inside the scheduler's own methods, which in production means
//! inside the observer/sweep mutex, and the probe record grows by ~20 diagnostic
//! units. Measured: 316-588 ns per frame over 20 000 frames
//! (`shadow_cost_per_frame_is_negligible`), against a 95 ms hour and a sweep
//! that today takes ~86.6 ms of it. So the shadow can in principle delay a
//! decision through time, and the budget it can delay it by is under a
//! microsecond per frame -- five orders of magnitude below the margin any
//! scheduler decision turns on. The kill switch exists for anyone who wants
//! that to be zero.
//!
//! # Reserved wire codes (NOT emitted by this lane)
//!
//! The two new dispositions will eventually be `H4DG` gap reasons. This lane
//! does not put them on the wire -- it publishes through the diagnostic channel
//! only -- but the codes are RESERVED here so the cutover lane does not
//! improvise a number. See
//! [`RESERVED_DOMAIN_GAP_REASON_PROCESSING_BACKPRESSURE`] and
//! [`RESERVED_DOMAIN_GAP_REASON_OBSERVATION_DEADLINE_MISS`].

use std::collections::VecDeque;
use std::time::Instant;

/// `ProcessingBackpressure`, reserved as the ninth `H4DG` gap reason.
///
/// The eight assigned codes are 1..=8 in
/// `compact_contract::record_semantics`; this is the next free one. It is
/// RESERVED, not assigned: nothing encodes it yet, and
/// `compact_contract::domain_gap::DomainGapReason` still refuses it, which is
/// the law that an unassigned code rejects at the decoder. The reservation
/// exists so the cutover lane extends the enum at this number instead of
/// picking one, and
/// `reserved_codes_are_the_next_free_ones_and_are_still_refused_by_the_decoder`
/// fails if either code is taken by something else first.
///
/// SEMANTICS the cutover must preserve: the hour was observed and its snapshot
/// existed, but compute capacity was exhausted and the snapshot was dropped
/// before it could be sealed. This is a COMPUTE loss -- the read itself was
/// clean and inside its own hour -- which is what separates it from `TORN`,
/// from `HOUR_LABEL_MISMATCH`, and from the queue eviction `BACKPRESSURE`
/// already names.
pub const RESERVED_DOMAIN_GAP_REASON_PROCESSING_BACKPRESSURE: u16 = 9;

/// `ObservationDeadlineMiss`, reserved as the tenth `H4DG` gap reason.
///
/// SEMANTICS the cutover must preserve: the hour was admitted and its
/// observation never COMPLETED inside its own hour -- either it was never
/// opened, or it was opened and had not closed when the next edge arrived. No
/// snapshot of that hour exists, so unlike `ProcessingBackpressure` there is
/// nothing that was thrown away; the world was simply never read at that label.
/// It is distinct from `BACKPRESSURE` because a deadline, not a queue, refused
/// it: the live scheduler's eviction happens because a NEWER frontier arrived,
/// while this happens because THIS hour ran out.
pub const RESERVED_DOMAIN_GAP_REASON_OBSERVATION_DEADLINE_MISS: u16 = 10;

/// The kill switch. `0`, `off` or `false` disables the shadow.
pub const CUSTODY_SHADOW_ENV: &str = "BATTLE_PLAN_CUSTODY_SHADOW";

/// The bounded ring of recent per-frame divergences kept for inspection.
///
/// Bounded because the shadow runs for the whole capture and an unbounded log
/// would be a memory leak with a diagnostic excuse. The cumulative counters in
/// [`CustodyShadowSummary`] are what reach the wire; this is the detail a test
/// or a debugger reads.
const DIVERGENCE_RING_CAP: usize = 64;

/// How many just-dropped hours the shadow remembers.
///
/// Needed because the live scheduler goes on to publish an hour the shadow has
/// already dispositioned -- that disagreement IS the S3 measurement -- and
/// without this the later live event would be miscounted as the shadow's own
/// model breaking.
///
/// Two things keep this small honestly, and neither is "the live lane only
/// holds one request":
///
/// 1. Only DISPATCHED hours are remembered (see
///    [`CustodyShadow::note_disposition`]). An hour the live scheduler never
///    handed to a reader is one it can never resolve either, so remembering it
///    would only crowd the ring -- and a long eviction run would then flush the
///    one hour that still mattered.
/// 2. An entry is REMOVED the moment the live scheduler resolves it
///    ([`CustodyShadow::classify_as_live_kept`]), so the ring holds only drops
///    that are still outstanding rather than every drop of the capture. Without
///    that, four unrelated later drops would evict a still-resolvable entry --
///    reachable, because a clock boundary clears an in-flight request without
///    resolving it while an earlier publication is still outstanding.
///
/// And when the ring does overflow, the eviction is COUNTED
/// ([`CustodyShadowSummary::drop_memory_overflows`]) rather than silently
/// turning a later live resolution into a fabricated illegal transition.
const DROPPED_HOUR_MEMORY: usize = 8;

/// Which custody phase an hour is in. `Admitted` is the pre-custody holding
/// slot -- the frontier the observer has seen and the end state has not yet
/// opened -- and is what makes "unopened work at the next boundary" nameable.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CustodyState {
    Admitted,
    Observing,
    Processing,
    Sealed,
    Publishing,
}

/// One hour inside the pipeline.
///
/// IDENTITY IS `(epoch, hour_raw)`, never the hour alone. A campaign reload
/// restarts the game clock, so the same raw hour recurs in a later epoch while
/// an earlier one may still be resolving; an hour-only lookup would then match
/// the wrong custody and commit it. The epoch is the LIVE scheduler's own
/// (`ClockStamp::epoch`), supplied with every notification, so the two sides
/// cannot drift into disagreeing about which hour is which.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct HourCustody {
    pub hour_raw: i32,
    /// The live scheduler's epoch for this hour.
    pub epoch: u64,
    /// When the hour edge that admitted this hour was observed, in micros since
    /// the shadow's origin. This is the instant an EDGE-ALIGNED dispatch would
    /// have started the bracket, and so the baseline every lateness delta is
    /// measured from.
    ///
    /// A LOWER BOUND on the true edge: the observer polls, so the counter may
    /// have turned over up to one poll period earlier. Lateness computed from
    /// it therefore understates rather than flatters, which is the same
    /// direction `lw_hour_age_us` is built to err in.
    pub edge_at_us: u64,
}

/// How an hour left the pipeline. Exactly one per admitted hour.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ShadowDisposition {
    /// Published and resolved by its sink outcome.
    Committed,
    /// Resolved, but the frame was refused rather than published.
    Refused,
    /// Compute capacity was exhausted at an hour edge and this snapshot was the
    /// oldest unsealed one.
    ProcessingBackpressure,
    /// The observation never completed inside its own hour.
    ObservationDeadlineMiss,
    /// A clock-unknown or reload boundary invalidated the hour before it
    /// resolved. Not a custody failure -- the world it described stopped being
    /// the world.
    Invalidated,
}

impl ShadowDisposition {
    /// The `H4DG` gap reason this disposition WILL carry once the cutover lane
    /// lands. `None` for the two dispositions that already have a wire
    /// vocabulary (a committed hour publishes a frame; a refused one is named
    /// by `H4FR`) and for `Invalidated`, which the live scheduler already
    /// covers with `ClockUnreadable` / `ReloadBaseline` ranges.
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

/// Why a transition was refused. Every refusal leaves the machine EXACTLY as it
/// was; there is no partially applied transition.
///
/// Each variant names the SOURCE slot being empty or the DESTINATION slot being
/// full, in that order of checking: a caller that asks to close an observation
/// that does not exist must be told that, not told about the slot downstream.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CustodyError {
    /// An hour edge at or before the last one, compared as `(epoch, hour)`.
    ///
    /// Within an epoch the hour must advance. ACROSS epochs it need not -- a
    /// reload restarts the clock, and the higher epoch is what orders the two.
    /// Comparing bare hours would refuse every reload; comparing nothing would
    /// let a stale observation re-admit an hour already in custody.
    NonMonotonicHourEdge { last: (u64, i32), offered: (u64, i32) },
    /// Dispatch with nothing admitted.
    NothingAdmitted,
    /// Dispatch while an observation is still open.
    ///
    /// UNREACHABLE through this API, and deliberately kept: every edge clears
    /// `observing`, and only an edge can re-admit, so an occupied observation
    /// slot can never coexist with an admitted hour. The guard is what makes
    /// that unreachability a property of the code rather than of the current
    /// call order. `the_sweep_reaches_every_refusal_the_api_can_produce` pins
    /// it as unreached.
    ObservationSlotOccupied,
    /// Close with no observation open.
    NothingObserving,
    /// Seal with nothing in processing.
    NothingProcessing,
    /// Seal while an earlier sealed frame has not moved to publication. THE
    /// ORDERED-SEAL BACKSTOP: the newer hour stays in processing and becomes
    /// `ProcessingBackpressure` at the next edge, rather than overtaking.
    SealSlotOccupied,
    /// Begin publication with nothing sealed.
    NothingSealed,
    /// Begin publication while an earlier frame is still publishing.
    PublicationSlotOccupied,
    /// Commit with nothing publishing.
    NothingPublishing,
}

/// What an hour edge did to the hours already in the pipeline.
///
/// The two fields cannot both be set at one edge, and that is a property of the
/// slots rather than an untested corner: `admitted` is filled only by an edge
/// and EMPTIED by the dispatch that fills `observing`, so an edge finds at most
/// one of them occupied.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct EdgeOutcome {
    /// Admitted, never dispatched. Identified as `(epoch, hour)`.
    pub unopened_deadline_miss: Option<(u64, i32)>,
    /// Dispatched, still open when its own hour ended.
    pub unclosed_deadline_miss: Option<(u64, i32)>,
}

impl EdgeOutcome {
    pub fn is_clean(&self) -> bool {
        *self == Self::default()
    }
}

/// What closing an observation did.
///
/// THE PROCESSING-BACKPRESSURE SITE. A snapshot enters processing at the close,
/// and if the previous one has not sealed by then it is the oldest unsealed
/// snapshot in the pipeline and is dropped to make room. That is the plan's
/// "observation preempts processing": the newly observed hour is never made to
/// wait, and the loss is named instead of being paid for in lateness.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct CloseOutcome {
    /// The `(epoch, hour)` that entered processing.
    pub closed: (u64, i32),
    /// The oldest unsealed snapshot, dropped so the close never waits.
    pub processing_backpressure: Option<(u64, i32)>,
}

/// Closed accounting for every hour the machine admitted.
///
/// Same contract as `read_only_scheduler::HourLedger`: each disposition is
/// counted at the exact site where it happens, and [`Self::balanced`] states
/// the identity that makes the set exhaustive, so a drop site added later
/// without a counter fails a test instead of widening an unexplained set.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct CustodyLedger {
    pub admitted: u64,
    pub committed: u64,
    pub refused: u64,
    pub processing_backpressure: u64,
    pub observation_deadline_miss: u64,
    pub invalidated: u64,
    /// Hours still held by some phase right now.
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

/// The custody pipeline itself. Pure: no clock, no I/O, no scheduler.
///
/// # Out-of-order sealing is UNCONSTRUCTIBLE
///
/// Only [`Self::note_hour_edge`] names an hour. [`Self::dispatch`],
/// [`Self::close_observation`], [`Self::seal`], [`Self::begin_publication`] and
/// [`Self::commit`] each act on the single occupant of their source slot and
/// RETURN the hour they acted on. There is no argument through which a caller
/// could ask for a different hour, so "seal H+1 before H" is not a transition
/// this API can express -- it is not rejected at runtime, it cannot be written.
/// The returned hour is what lets the shadow compare the machine against the
/// live scheduler without the machine ever taking the live scheduler's word for
/// which hour it is on.
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

    /// The epoch of the most recent edge, if any.
    pub fn epoch(&self) -> Option<u64> {
        self.last_edge.map(|(epoch, _)| epoch)
    }

    /// Which phase holds `(epoch, hour_raw)`, if any. The slots are disjoint by
    /// construction, so at most one answer exists.
    ///
    /// The epoch is part of the key, not decoration: a reload restarts the
    /// clock, so hour H can be in flight in epoch 5 while epoch 4's hour H is
    /// still resolving, and an hour-only match would resolve the wrong one.
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

    /// The hours the pipeline holds, newest phase first, for diagnostics.
    pub fn occupancy(&self) -> [Option<i32>; 5] {
        [
            self.admitted.map(|c| c.hour_raw),
            self.observing.map(|c| c.hour_raw),
            self.processing.map(|c| c.hour_raw),
            self.sealed.map(|c| c.hour_raw),
            self.publishing.map(|c| c.hour_raw),
        ]
    }

    /// The same, stamped with each hour's epoch.
    ///
    /// The ORDER INVARIANT is stated on these pairs and not on bare hours: a
    /// reload restarts the counter, so an hour left in a downstream phase from
    /// the old epoch can legitimately carry a HIGHER hour number than the hour
    /// being admitted now. `(epoch, hour)` is the pair that is genuinely
    /// ordered, and comparing bare hours across a reload would report a defect
    /// that is not one. (An hour left in a downstream phase from the old epoch
    /// is exactly what `invalidate` spares.)
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

    /// The hour edge: the ONLY event that admits work, and the site of the
    /// `ObservationDeadlineMiss` disposition.
    ///
    /// Two things can be stale when an edge arrives, and neither may survive it:
    ///
    /// 1. Work admitted and never opened. Its hour is over; there is nothing to
    ///    observe any more.
    /// 2. An observation still open when its hour ended. Its bracket would
    ///    otherwise span the boundary, which the plan refuses outright.
    ///
    /// PROCESSING IS NOT TOUCHED HERE. Hour `H-1` being processed while `H` is
    /// observed IS the steady state, not an overload; the pipeline only runs
    /// out of room when a new snapshot needs the processing slot, which happens
    /// at the close. See [`Self::close_observation`].
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

    /// Open the observation of the admitted hour. Returns the hour opened, as
    /// its  identity, and how late that open is against the edge
    /// that admitted it.
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

    /// Close the observation: the snapshot is frozen and moves to processing.
    ///
    /// If the previous snapshot is still there it has failed to seal before a
    /// fresh one needed the slot, it is the OLDEST UNSEALED snapshot in the
    /// pipeline, and it is dropped as `ProcessingBackpressure` -- because the
    /// alternative is making the fresh, correctly-bracketed snapshot wait for
    /// compute, and a delayed observation is the one failure the whole
    /// architecture exists to remove.
    ///
    /// Dropping the OLDER one is also what keeps sealing ordered: the dropped
    /// hour never sealed, so nothing overtakes anything.
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

    /// Seal whatever is in processing. Takes no hour: see the type comment.
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

    /// Resolve the publishing hour. `accepted` distinguishes a published frame
    /// from a refused one; both END custody, which is the property the ledger
    /// depends on.
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

    /// A clock-unknown, reload, or teardown boundary. Returns the hours
    /// dropped, in slot order [admitted, observing, processing].
    ///
    /// SEALED and PUBLISHING are deliberately untouched, for the same reason
    /// `ReadOnlyScheduler::gap_all_pending` leaves `publishing` alone: those
    /// bytes were already read, inside their own bracket, and are on their way
    /// to the sink. A later boundary says nothing about them.
    ///
    /// The EDGE WATERMARK IS KEPT. It used to be cleared here, on the theory
    /// that a reload needs to admit a lower hour -- but the live scheduler
    /// stamps a reload with a NEW EPOCH, and `(epoch, hour)` already orders
    /// that correctly. Clearing the watermark would instead have let a stale
    /// observation re-admit an hour inside the same epoch.
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

    /// The last hour each ordered watermark reached, for tests and diagnostics.
    pub fn watermarks(&self) -> (Option<i32>, Option<i32>) {
        (
            self.last_sealed.map(|(_, hour)| hour),
            self.last_committed.map(|(_, hour)| hour),
        )
    }
}

/// Whether the shadow runs at all.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ShadowMode {
    /// Default. Every notification is processed.
    Enabled,
    /// Kill switch. Every notification returns immediately; the machine is
    /// never stepped and no clock is read.
    Disabled,
}

/// Where the shadow's micros come from.
///
/// Production uses [`ShadowClock::Real`]. Tests use [`ShadowClock::Manual`] so
/// a lateness assertion is an exact number rather than a wall-clock race --
/// this module contains no sleep, no retry and no "eventually".
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

    /// Move a manual clock forward. A no-op on the real clock, which cannot be
    /// moved and must not pretend it can.
    pub fn advance_us(&mut self, delta: u64) {
        if let Self::Manual { now_us } = self {
            *now_us = now_us.saturating_add(delta);
        }
    }
}

/// What the shadow disagreed with the live scheduler about, on one frame.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DivergenceKind {
    /// The live scheduler dispatched, released, or resolved an hour that is not
    /// the one the machine holds in the corresponding phase. A nonzero count
    /// means the shadow's model of the live event order is WRONG, and every
    /// other number it publishes should be read with that in mind.
    HourMismatch { shadow: Option<(u64, i32)> },
    /// The end state would have dropped this hour here. THE S3 MEASUREMENT.
    WouldDrop { disposition: ShadowDisposition },
    /// The live capture published (or refused) an hour the shadow had already
    /// dropped. The other half of the same measurement, seen from the live
    /// side: this hour is in today's stream and would not be in the end
    /// state's.
    LiveKeptWhatShadowDropped { disposition: ShadowDisposition },
    /// The live scheduler published an hour synchronously -- it never released
    /// read custody, so observation, processing, sealing and publication all
    /// happened in one un-overlapped span. The end state would have overlapped
    /// them. Not an error: it is today's behaviour, counted so the shadow's
    /// pipeline depth is never read as a claim about the live one.
    CollapsedPublication,
    /// The machine refused the transition the live event implied.
    IllegalTransition { error: CustodyError },
}

/// One recorded divergence: what the machine was doing, and what the live
/// scheduler did instead.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct FrameDivergence {
    /// The `(epoch, hour)` the live scheduler named.
    pub live: (u64, i32),
    pub shadow_state: Option<CustodyState>,
    pub kind: DivergenceKind,
    /// Micros between the hour edge and the live dispatch, when the divergence
    /// was raised at a dispatch. Zero otherwise.
    pub late_dispatch_us: u64,
}

/// The cumulative shadow accounting that reaches the wire.
///
/// Every field is a running TOTAL or a running MAXIMUM and is never reset
/// within a capture, which is what makes the `rs_shadow_*` probe units legal
/// under `compact_contract::diagnostic_channel::CUMULATIVE_UNITS` (folded by
/// MAX; each probe carries the accounting as of that record).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct CustodyShadowSummary {
    pub active: bool,
    pub events: u64,
    pub hour_edges: u64,
    pub dispatches: u64,
    pub committed_hours: u64,
    pub refused_hours: u64,
    /// Snapshots the end state would have dropped because a fresh one needed
    /// the processing slot.
    ///
    /// A PROXY, and the one counter here that is not a direct observation.
    /// "Unsealed" is read off the LIVE publication boundary -- an hour occupies
    /// the processing slot from `release_read_custody` until its sink outcome
    /// lands -- because today's recorder has no separate compute phase and
    /// therefore no event that says when the end state's processing would have
    /// finished. So a slow SINK reads here as slow processing.
    ///
    /// It is still worth publishing, for two reasons. It is a strict UPPER
    /// bound on the compute-caused drops (the end state's processing is a
    /// subset of the interval measured), and on the live scheduler's ordering
    /// it is expected to be ZERO or near it -- two unresolved publications are
    /// terminal, so the only way a second snapshot closes over an unsealed one
    /// is the mixed synchronous order. A nonzero value on a real capture is
    /// therefore worth reading; a zero one says the compute custody is not
    /// where today's loss is.
    pub processing_backpressure_hours: u64,
    pub observation_deadline_miss_hours: u64,
    pub invalidated_hours: u64,
    pub illegal_transitions: u64,
    pub hour_mismatches: u64,
    pub collapsed_publications: u64,
    /// Hours the live capture resolved after the shadow had dropped them.
    pub live_kept_hours_shadow_dropped: u64,
    pub late_dispatch_samples: u64,
    pub late_dispatch_total_us: u64,
    pub max_late_dispatch_us: u64,
    /// Live evictions the end state would ALSO have dropped (as an
    /// `ObservationDeadlineMiss`), and the ones it would have kept. The second
    /// number is the one that says the architecture change buys hours.
    pub agreed_evictions: u64,
    pub disagreed_evictions: u64,
    /// Drops the bounded memory had to forget while they were still
    /// unresolved. Nonzero means `live_kept_hours_shadow_dropped` UNDERCOUNTS
    /// by up to this much, and that some of `illegal_transitions` is this
    /// forgetting rather than a model failure. Published so neither number is
    /// ever read as exact when it is not.
    pub drop_memory_overflows: u64,
    /// Time spent inside the shadow itself. One extra clock read per
    /// notification pays for this; it is what makes the per-frame cost claim a
    /// field measurement instead of a bench extrapolation.
    pub cost_total_ns: u64,
    pub max_cost_ns: u64,
}

/// The observer that sits beside the live scheduler.
#[derive(Clone, Debug)]
pub struct CustodyShadow {
    mode: ShadowMode,
    clock: ShadowClock,
    machine: CustodyMachine,
    summary: CustodyShadowSummary,
    recent: VecDeque<FrameDivergence>,
    /// Hours the machine dropped and the live scheduler has NOT yet resolved,
    /// so a later live event about one of them is classified as a disagreement
    /// rather than as the shadow's own model breaking. Keyed by `(epoch, hour)`
    /// and emptied on resolution. See [`DROPPED_HOUR_MEMORY`].
    dropped: VecDeque<((u64, i32), ShadowDisposition)>,
    /// Test-only: make the shadow perturb its own state as violently as it can,
    /// inside the same critical section the live scheduler decides in. Dispatch
    /// must be bit-identical anyway.
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

    /// The production default, with the environment kill switch applied.
    ///
    /// `BATTLE_PLAN_CUSTODY_SHADOW=0` turns it off. Read ONCE, when the scheduler is
    /// built, so no capture can change mode halfway and publish two
    /// incomparable halves under one set of counters.
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

    /// Remember a drop the live scheduler may still resolve. An overflow is
    /// COUNTED, because forgetting silently is what would turn a later live
    /// resolution into a fabricated illegal transition.
    fn remember_drop(&mut self, live: (u64, i32), disposition: ShadowDisposition) {
        if self.dropped.len() == DROPPED_HOUR_MEMORY {
            self.dropped.pop_front();
            self.summary.drop_memory_overflows =
                self.summary.drop_memory_overflows.saturating_add(1);
        }
        self.dropped.push_back((live, disposition));
    }

    /// Take the remembered drop for `live`, if there is one. TAKE, not peek:
    /// an hour the live scheduler has resolved can never be resolved again, so
    /// holding the entry would only crowd out a drop that is still outstanding.
    fn take_dropped_disposition(&mut self, live: (u64, i32)) -> Option<ShadowDisposition> {
        let index = self
            .dropped
            .iter()
            .rposition(|(candidate, _)| *candidate == live)?;
        self.dropped.remove(index).map(|(_, disposition)| disposition)
    }

    fn peek_dropped_disposition(&self, live: (u64, i32)) -> Option<ShadowDisposition> {
        self.dropped
            .iter()
            .rev()
            .find(|(candidate, _)| *candidate == live)
            .map(|(_, disposition)| *disposition)
    }

    /// Count one terminal disposition, and -- for the two the end state invents
    /// -- record the divergence and, if the live scheduler could still resolve
    /// the hour, remember it.
    ///
    /// `opened` is that condition: an hour the live scheduler never handed to a
    /// reader can never be published or refused later, so remembering it would
    /// only crowd the bounded ring. See [`DROPPED_HOUR_MEMORY`].
    fn note_disposition(
        &mut self,
        live: (u64, i32),
        disposition: ShadowDisposition,
        opened: bool,
    ) {
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

    /// True when the live event is about an hour the shadow has already
    /// dropped, which is a disagreement rather than a broken model.
    ///
    /// `resolving` distinguishes the event that ENDS the live scheduler's
    /// interest in the hour (its publication outcome) from one that merely
    /// mentions it: only the former counts the disagreement and forgets the
    /// entry, so one dropped hour is counted once however many live events it
    /// takes to resolve.
    fn classify_as_live_kept(&mut self, live: (u64, i32), resolving: bool) -> bool {
        if !resolving {
            return self.peek_dropped_disposition(live).is_some();
        }
        let Some(disposition) = self.take_dropped_disposition(live) else {
            return false;
        };
        self.summary.live_kept_hours_shadow_dropped =
            self.summary.live_kept_hours_shadow_dropped.saturating_add(1);
        self.record(FrameDivergence {
            live,
            shadow_state: None,
            kind: DivergenceKind::LiveKeptWhatShadowDropped { disposition },
            late_dispatch_us: 0,
        });
        true
    }

    /// Open one notification: bump the event counter and take the entry stamp.
    /// `None` when the shadow is off, which is the ONLY branch the disabled
    /// path executes.
    fn begin(&mut self) -> Option<u64> {
        if !self.enabled() {
            return None;
        }
        self.summary.events = self.summary.events.saturating_add(1);
        Some(self.clock.now_us())
    }

    /// Close one notification, charging it to the cost counters.
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
            // Maximal self-perturbation, inside the same critical section the
            // live scheduler decides in. If any live decision read shadow
            // state, the differential test would see it.
            self.machine.invalidate();
            self.summary.events = self.summary.events.saturating_mul(3).saturating_add(7);
            self.recent.clear();
            self.dropped.clear();
        }
    }

    // --- notifications, one per live scheduler event ----------------------

    /// The observer admitted a new frontier: THE HOUR EDGE. `evicted` is the
    /// pending frontier the live scheduler dropped to make room, if any.
    ///
    /// The two are one notification because they are one live event
    /// (`ReadOnlyScheduler::enqueue_request`), and splitting them would make
    /// the agreement classification depend on call order.
    pub fn note_frontier_admitted(&mut self, live: (u64, i32), evicted: Option<(u64, i32)>) {
        let Some(entered) = self.begin() else { return };
        self.summary.hour_edges = self.summary.hour_edges.saturating_add(1);
        match self.machine.note_hour_edge(live.0, live.1, entered) {
            Ok(outcome) => {
                if let Some(stale) = outcome.unopened_deadline_miss {
                    // Never opened, so the live scheduler cannot resolve it
                    // later either: counted, not remembered.
                    self.note_disposition(stale, ShadowDisposition::ObservationDeadlineMiss, false);
                }
                if let Some(stale) = outcome.unclosed_deadline_miss {
                    self.note_disposition(stale, ShadowDisposition::ObservationDeadlineMiss, true);
                }
                // The live eviction and the shadow's unopened deadline miss are
                // the same hour whenever the two models agree. When they do not,
                // the live scheduler threw away an hour the end state would have
                // kept -- which is a result, not an error.
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

    /// A reader took the request: the live bracket starts HERE.
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

    /// The reader finished: bytes frozen, hour handed to processing.
    ///
    /// An hour the shadow already dropped is tolerated here and counted once,
    /// at its resolution -- counting it twice would double-report one
    /// disagreement.
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

    /// The sink outcome landed. Walks the remaining phases in order.
    ///
    /// On the SYNCHRONOUS path there was no release, so the hour is still being
    /// observed here and the whole tail runs in one step. That is counted as a
    /// [`DivergenceKind::CollapsedPublication`] rather than silently modelled as
    /// a pipeline, because the pipeline is exactly the thing that does not exist
    /// yet.
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

    /// A clock-unknown, reload, or teardown boundary.
    pub fn note_invalidated(&mut self) {
        let Some(entered) = self.begin() else { return };
        // `invalidate` returns [admitted, observing, processing]. Only the last
        // two were ever handed to a reader, so only those can still be resolved
        // by the live scheduler and only those are worth remembering.
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

    /// The epoch almost every test runs in. A reload is the only reason to use
    /// another one, and the two tests that do say so.
    const E: u64 = 0;

    fn manual() -> CustodyShadow {
        CustodyShadow::new(ShadowMode::Enabled, ShadowClock::Manual { now_us: 0 })
    }

    fn machine() -> CustodyMachine {
        CustodyMachine::new()
    }

    /// `(epoch, hour)` in the default epoch, which is what every notification
    /// and every slot is keyed by.
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

    /// Close and require that nothing was evicted. Most tests are about a
    /// pipeline that keeps up; the ones that are not check the outcome.
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

    /// One clean hour, phase by phase, with the hours the API returns pinned.
    ///
    /// MUTATION: make `close_observation` leave `self.observing` set and the
    /// next `note_hour_edge` raises a deadline miss instead of a clean edge.
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

    /// CUSTODY IDENTITY IS `(epoch, hour)`, and the raw hour alone is not
    /// enough.
    ///
    /// A campaign reload restarts the game clock, so the same raw hour recurs
    /// in a later epoch. If the lookup matched on the hour alone, the older
    /// epoch's still-unresolved custody and the newer epoch's live one would be
    /// the same key -- and resolving the old one would commit the new one,
    /// silently, without either truthfulness counter moving.
    ///
    /// MUTATION: drop the `custody.epoch == epoch` conjunct from `state_of` and
    /// the last two assertions fail: hour 500 of epoch 1 is committed by a
    /// resolution that belonged to epoch 0.
    #[test]
    fn the_same_raw_hour_in_two_epochs_is_two_different_custodies() {
        let mut m = machine();
        // Epoch 0 walks hour 500 into processing and leaves it there.
        m.note_hour_edge(0, 500, 0).unwrap();
        m.dispatch(10).unwrap();
        m.close_observation().unwrap();
        assert_eq!(m.state_of(0, 500), Some(CustodyState::Processing));

        // A reload: epoch 1, and the clock is back at hour 500.
        m.note_hour_edge(1, 500, 1_000).unwrap();
        assert_eq!(m.state_of(1, 500), Some(CustodyState::Admitted));
        assert_eq!(
            m.state_of(0, 500),
            Some(CustodyState::Processing),
            "the old epoch's custody is still its own"
        );
        // And the two are ordered by the pair, so the pipeline invariant holds
        // even though the raw hours are equal.
        assert_eq!(
            m.occupancy_stamped(),
            [Some((1, 500)), None, Some((0, 500)), None, None]
        );

        // A shadow resolving epoch 0's hour 500 must not touch epoch 1's.
        let mut s = manual();
        s.note_frontier_admitted((0, 500), None);
        s.note_dispatch((0, 500));
        s.note_read_custody_released((0, 500));
        s.note_invalidated(); // the reload boundary
        s.note_frontier_admitted((1, 500), None);
        s.note_dispatch((1, 500));
        // The live scheduler now resolves the OLD hour 500, which its
        // publication custody survived the boundary with.
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

    /// THE STEADY STATE the plan specifies: H observing, H-1 processing, H-2
    /// publishing, all three at once, with no disposition raised.
    ///
    /// MUTATION: put the `ProcessingBackpressure` eviction back on the hour
    /// edge instead of the close, and the third edge destroys hour 11 -- the
    /// steady state stops being reachable at all.
    #[test]
    fn steady_state_holds_three_hours_at_once() {
        let mut m = machine();
        // H-2 walks to publishing.
        edge(&mut m, 10, 0);
        dispatch(&mut m, 10);
        close(&mut m);
        seal(&mut m);
        publish(&mut m);
        // H-1 walks to processing.
        assert!(edge(&mut m, 11, 1_000).is_clean());
        dispatch(&mut m, 1_010);
        close(&mut m);
        // H is observed.
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

    /// THE PLAN'S FIRST DISPROOF CASE. A processing phase that never finishes
    /// must cost EXACTLY ONE `ProcessingBackpressure` each time a new snapshot
    /// needs the slot, and must never leave a second unsealed custody behind.
    ///
    /// MUTATION: let the close push instead of replace (a queue rather than a
    /// slot) and the "exactly one custody" assertion fails; drop the counter at
    /// the eviction site and the ledger stops balancing.
    #[test]
    fn slow_processing_drops_exactly_one_snapshot_per_close() {
        let mut m = machine();
        edge(&mut m, 50, 0);
        dispatch(&mut m, 10);
        close(&mut m);

        // Hour 51 is observed while 50 is still unsealed. The EDGE is clean --
        // that is the steady state -- and the CLOSE is what runs out of room.
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

        // And once 51 seals, the next close evicts nothing.
        seal(&mut m);
        edge(&mut m, 52, 2_000);
        dispatch(&mut m, 2_010);
        assert_eq!(close(&mut m), 52);
        assert_eq!(m.ledger().processing_backpressure, 1);
        assert!(m.ledger().balanced());
    }

    /// The same case driven through the SHADOW, which is where the "exactly
    /// one, never a second" claim has to hold for the published counters.
    #[test]
    fn slow_processing_publishes_exactly_one_backpressure_disposition() {
        let mut s = manual();
        s.note_frontier_admitted(h(60), None);
        s.note_dispatch(h(60));
        s.note_read_custody_released(h(60));
        // Hour 61 is observed and closes while 60's frame has not resolved.
        s.advance_clock_us(100_000);
        s.note_frontier_admitted(h(61), None);
        s.note_dispatch(h(61));
        s.note_read_custody_released(h(61));
        assert_eq!(s.summary().processing_backpressure_hours, 1);

        // The live capture goes on to publish hour 60 anyway. That is the
        // disagreement the stage exists to measure, and it must not be counted
        // as a second backpressure or as a broken model.
        s.note_publication_resolved(h(60), true);
        let summary = s.summary();
        assert_eq!(summary.processing_backpressure_hours, 1);
        assert_eq!(summary.live_kept_hours_shadow_dropped, 1);
        assert_eq!(summary.illegal_transitions, 0);
        assert_eq!(summary.committed_hours, 0, "the shadow committed nothing");
        assert_eq!(summary.drop_memory_overflows, 0);
        assert!(s.ledger().balanced(), "{:?}", s.ledger());
    }

    /// THE PLAN'S SECOND DISPROOF CASE, stated as strongly as a test can state
    /// it: the seal API takes no hour, so an out-of-order seal has no
    /// expression. What remains testable is that the hours it DOES seal come
    /// out in order however the caller interleaves the pipeline -- which
    /// `every_event_sequence_preserves_the_custody_invariants` then proves for
    /// every reachable interleaving.
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

    /// An ordered-seal backlog pushes back UP the pipeline instead of letting a
    /// later hour overtake an earlier one.
    ///
    /// MUTATION: let `seal` overwrite an occupied `sealed` slot and hour 300 is
    /// silently destroyed while 301 publishes first.
    #[test]
    fn a_sealed_backlog_refuses_the_next_seal_rather_than_overtaking() {
        let mut m = machine();
        edge(&mut m, 300, 0);
        dispatch(&mut m, 10);
        close(&mut m);
        seal(&mut m);
        // 300 sits sealed and unpublished. 301 observes and closes.
        edge(&mut m, 301, 1_000);
        dispatch(&mut m, 1_010);
        close(&mut m);
        assert_eq!(m.seal(), Err(CustodyError::SealSlotOccupied));
        assert_eq!(state(&m, 301), Some(CustodyState::Processing));
        assert_eq!(state(&m, 300), Some(CustodyState::Sealed));
        assert_eq!(m.watermarks(), (Some(300), None));

        // The refused seal leaves 301 unsealed, so the NEXT close is the one
        // that pays -- and 300, whose bytes are already frozen, is untouched.
        edge(&mut m, 302, 2_000);
        dispatch(&mut m, 2_010);
        let outcome = m.close_observation().unwrap();
        assert_eq!(outcome.processing_backpressure, Some(h(301)));
        assert_eq!(state(&m, 300), Some(CustodyState::Sealed));
    }

    /// Preemption at the edge, in both of its shapes.
    ///
    /// MUTATION: drop the `observing` arm from `note_hour_edge` and the
    /// unclosed observation survives its own hour -- a bracket spanning the
    /// boundary, which the plan refuses outright.
    #[test]
    fn an_edge_preempts_both_unopened_and_unclosed_work() {
        let mut m = machine();
        // Hour 400 admitted, dispatched, never closed.
        edge(&mut m, 400, 0);
        dispatch(&mut m, 10);
        // Hour 401 arrives: 400 missed its close deadline.
        let first = edge(&mut m, 401, 1_000);
        assert_eq!(first.unclosed_deadline_miss, Some(h(400)));
        assert_eq!(first.unopened_deadline_miss, None);
        // Hour 402 arrives: 401 was never opened at all.
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

    /// Every illegal transition is refused with its own typed error AND leaves
    /// the machine byte-identical. A refusal that half-applied would be worse
    /// than no refusal.
    ///
    /// MUTATION: move the `self.observing = None` in `close_observation` above
    /// its `NothingObserving` guard and the state-unchanged assertion fails.
    #[test]
    fn every_illegal_transition_is_refused_and_changes_nothing() {
        // Empty machine: every advancing call has nothing to act on.
        let mut m = machine();
        let before = m.clone();
        assert_eq!(m.dispatch(0), Err(CustodyError::NothingAdmitted));
        assert_eq!(m.close_observation(), Err(CustodyError::NothingObserving));
        assert_eq!(m.seal(), Err(CustodyError::NothingProcessing));
        assert_eq!(m.begin_publication(), Err(CustodyError::NothingSealed));
        assert_eq!(m.commit(true), Err(CustodyError::NothingPublishing));
        assert_eq!(m, before, "a refused transition must change nothing");

        // Repeated and backwards edges, INSIDE an epoch. Across epochs a lower
        // hour is legal, which the reload test above covers.
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

        // Occupied destination slots.
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

    /// Invalidation drops the unsealed hours and leaves the immutable ones,
    /// exactly as `ReadOnlyScheduler::gap_all_pending` leaves `publishing`
    /// alone.
    ///
    /// MUTATION: add `self.publishing` to the invalidate loop and the surviving
    /// publication assertion fails -- and a frame already on its way to the sink
    /// would be double-reported.
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
        // The next epoch may carry a LOWER hour -- that is what a reload is --
        // and the pair ordering is what makes it legal.
        assert!(m.note_hour_edge(1, 5, 3_000).is_ok());
        assert_eq!(commit(&mut m, true), 1_000);
        assert!(m.ledger().balanced(), "{:?}", m.ledger());
    }

    // --- the exhaustive sweep --------------------------------------------

    /// The event alphabet the sweeps enumerate.
    const SWEEP_EVENTS: u64 = 7;
    /// Depth of the seed phase, and of the sweep from each seed.
    const SEED_DEPTH: u32 = 6;
    const SWEEP_DEPTH: u32 = 5;

    /// One step. `hour_source` is shared so every edge in a run is distinct;
    /// `repeat_edge` re-offers the last hour, which is the only way to exercise
    /// the non-monotonic refusal.
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
                // A boundary. The live scheduler stamps a new epoch at a reload
                // or a clock outage, so the sweep does too -- and every third
                // one does NOT, which is the teardown-flush shape.
                m.invalidate();
                if *hour_source % 3 != 0 {
                    *epoch += 1;
                }
                None
            }
        }
    }

    /// Which phases are occupied, as the abstraction the reachable-state search
    /// is over. Errors depend on occupancy and on nothing else, so two machines
    /// with the same pattern refuse the same things.
    fn pattern(m: &CustodyMachine) -> [bool; 5] {
        m.occupancy().map(|slot| slot.is_some())
    }

    /// One machine per reachable occupancy pattern, each built by a REAL
    /// script rather than by constructing state directly.
    ///
    /// This is what lets a depth-5 sweep see refusals that need eight or nine
    /// steps to set up: the seeds carry the first six. Nothing here asserts;
    /// the seeds are inputs to the two sweeps below.
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
                apply(
                    &mut m,
                    event,
                    &mut epoch,
                    &mut hour_source,
                    script % 3 == 0,
                );
            }
            if seen.insert(pattern(&m)) {
                seeds.push((m, epoch, hour_source));
            }
        }
        seeds
    }

    /// EXHAUSTIVE: every event sequence of depth 5 from every reachable
    /// occupancy pattern, asserting the invariants that make the machine sound.
    ///
    /// This is what makes "every legal transition, every illegal one refused" a
    /// proof rather than a sample. After every single step it checks
    ///
    /// * the pipeline is strictly ordered by `(epoch, hour)` -- the newest
    ///   phase holds the newest custody -- which also means no custody occupies
    ///   two phases,
    /// * the ledger balances,
    /// * an `Err` return left the machine byte-identical,
    ///
    /// and per sequence, that seals are monotonic within an epoch.
    ///
    /// The order invariant is stated on `(epoch, hour)` because a reload
    /// restarts the counter: an hour left in a downstream phase from the old
    /// epoch legitimately carries a higher NUMBER than the hour admitted now.
    ///
    /// MUTATION: remove the `sealed.is_some()` guard from `seal` and the
    /// pipeline-order assertion fails; make `note_hour_edge` forget to count a
    /// deadline miss and the ledger identity fails.
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
                    let error = apply(
                        &mut m,
                        event,
                        &mut epoch,
                        &mut hour_source,
                        script % 3 == 0,
                    );
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

    /// Which refusals the API can actually produce, rebuilt from the sweep
    /// rather than transcribed.
    ///
    /// [`CustodyError::ObservationSlotOccupied`] is the one that never fires:
    /// every edge clears `observing`, and only an edge admits, so an occupied
    /// observation slot can never coexist with an admitted hour. The guard is
    /// kept because deleting it would make that unreachability a property of
    /// the current call order instead of the code. This test states which side
    /// of the line each error is on, so a change that moves one has to say so.
    ///
    /// MUTATION: delete the `sealed.is_some()` guard and `SealSlotOccupied`
    /// disappears from the reachable set.
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
                    if let Some(error) = apply(
                        &mut m,
                        event,
                        &mut epoch,
                        &mut hour_source,
                        script % 3 == 0,
                    ) {
                        // Canonical name only: the stamps inside
                        // `NonMonotonicHourEdge` are script detail.
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

    /// The reserved codes are the next two free `H4DG` reasons, sit above a
    /// CONTIGUOUS assigned roster so there is no hole to fall into, and are
    /// still refused by the decoder.
    ///
    /// What this establishes and what it does not: the decoder rejection is a
    /// property of the CONTRACT, and it is what the law "an unassigned code
    /// rejects at the decoder" asks for. That no producer emits them is a
    /// property of this commit, and the evidence for it is that
    /// `RESERVED_DOMAIN_GAP_REASON_*` are plain `u16` constants with no
    /// `DomainGapReason` conversion anywhere -- there is no expression that
    /// puts either number into an `H4DG` encoder. The cutover lane is expected
    /// to replace the rejection half of this test with a round-trip, at these
    /// exact numbers.
    #[test]
    fn reserved_codes_are_the_next_free_ones_and_are_still_refused_by_the_decoder() {
        use compact_contract::calendar::DATE_EPOCH_CONST;
        use compact_contract::domain_gap::{DomainGapPart, DomainGapReason, DomainGapRecordV1};

        let mut assigned: Vec<u16> = DomainGapReason::ALL.iter().map(|r| r.code()).collect();
        assigned.sort_unstable();
        let highest = *assigned.last().expect("H4DG assigns reasons");
        // CONTIGUITY, not just "above the maximum": a roster with a hole in it
        // would make "the next free code" ambiguous, and the cutover lane would
        // have two defensible answers.
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

        // A wire payload carrying either reserved code is refused today.
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

        // And every disposition that will need a code has exactly one.
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

    /// The disabled shadow is inert: no clock read, no machine step, no
    /// counter but its own inactive declaration.
    ///
    /// MUTATION: drop the `enabled()` check from `begin` and every counter here
    /// becomes nonzero.
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

    /// The lateness delta is the edge-to-dispatch distance, exactly.
    ///
    /// MUTATION: measure from the dispatch instead of the edge and the number
    /// becomes zero.
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

    /// The synchronous publication path is NAMED, not modelled away.
    ///
    /// MUTATION: treat a missing release as a pipelined one and
    /// `collapsed_publications` stays zero while the shadow silently claims a
    /// pipeline the recorder does not have.
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

        // The pipelined path does not count as a collapse, and a refusal still
        // ends custody.
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

    /// A live eviction and the shadow's deadline miss are the same event, so
    /// they are classified as agreement rather than counted twice.
    ///
    /// `disagreed_evictions` is expected to stay ZERO on any real capture --
    /// the live scheduler only ever evicts a frontier no reader took, which is
    /// exactly the shadow's unopened case. It is published anyway because it is
    /// the cheapest witness that the shadow's model of the eviction site is
    /// still the live one; the second half below is what proves the classifier
    /// does not simply agree with everything.
    ///
    /// MUTATION: raise a second disposition inside the eviction path and the
    /// ledger identity double-counts the hour.
    #[test]
    fn a_live_eviction_agrees_with_the_shadow_deadline_miss() {
        let mut s = manual();
        s.note_frontier_admitted(h(40), None);
        // Hour 41 arrives before anything took 40: the live scheduler evicts 40.
        s.advance_clock_us(1_000);
        s.note_frontier_admitted(h(41), Some(h(40)));
        let summary = s.summary();
        assert_eq!(summary.observation_deadline_miss_hours, 1);
        assert_eq!(summary.agreed_evictions, 1);
        assert_eq!(summary.disagreed_evictions, 0);
        let ledger = s.ledger();
        assert!(ledger.balanced(), "{ledger:?}");
        assert_eq!(ledger.observation_deadline_miss, 1);

        // An eviction the shadow's edge did NOT raise as an unopened miss is a
        // disagreement, and is reported as one instead of being folded into
        // agreement.
        let mut s = manual();
        s.note_frontier_admitted(h(42), None);
        s.note_dispatch(h(42));
        s.note_read_custody_released(h(42));
        s.note_frontier_admitted(h(43), Some(h(41)));
        assert_eq!(s.summary().disagreed_evictions, 1);
        assert_eq!(s.summary().agreed_evictions, 0);
    }

    /// A live event the machine cannot accept is COUNTED, with its typed
    /// reason, instead of being silently dropped -- otherwise a shadow whose
    /// model of the live order had drifted would keep publishing confident
    /// numbers.
    #[test]
    fn an_impossible_live_order_is_counted_as_an_illegal_transition() {
        let mut s = manual();
        s.note_dispatch(h(50)); // nothing was ever admitted
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

    /// The two bounded memories are bounded, and the drop memory holds only
    /// UNRESOLVED drops -- so the entry that still matters is not flushed by
    /// later ones, and an overflow that does happen is counted rather than
    /// silently turning a later resolution into a fake illegal transition.
    ///
    /// MUTATION: peek instead of taking in `classify_as_live_kept` and the
    /// eighth-hour assertion below fails: the resolved entries never leave, so
    /// hour 1 is evicted and its resolution is miscounted.
    #[test]
    fn the_divergence_ring_and_the_drop_memory_are_bounded() {
        let mut s = manual();
        let hours = DIVERGENCE_RING_CAP as i32 * 4;
        for hour in 0..hours {
            s.note_frontier_admitted(h(hour + 1), None);
            // Never dispatched: every edge after the first raises a miss.
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

        // A dispatched hour dropped long before its live resolution is still
        // recognised, because every RESOLVED entry LEAVES the memory.
        //
        // Hour 1 is dropped early and resolved last. Between the two, ten more
        // hours are each dropped and then resolved. If a resolved entry stayed
        // in the memory, those ten would fill it and evict hour 1 -- which is
        // exactly what the `take`, rather than a peek, is for.
        let mut s = manual();
        s.note_frontier_admitted(h(1), None);
        s.note_dispatch(h(1));
        s.note_frontier_admitted(h(2), None); // drops hour 1, unclosed
        s.note_dispatch(h(2));
        for hour in 3..=12 {
            s.note_frontier_admitted(h(hour), None); // drops hour-1, unclosed
            s.note_dispatch(h(hour));
            // ...and the live scheduler resolves the hour just dropped.
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

    /// An overflow of the drop memory is COUNTED, so an undercount of
    /// `live_kept_hours_shadow_dropped` can never be silent.
    ///
    /// MUTATION: drop the counter from `remember_drop` and this fails while the
    /// undercount below still happens.
    #[test]
    fn a_drop_memory_overflow_is_counted_not_silent() {
        let mut s = manual();
        // One dispatched hour that is dropped and never resolved, then more
        // dispatched-and-dropped hours than the memory holds.
        for hour in 1..(DROPPED_HOUR_MEMORY as i32 + 3) {
            s.note_frontier_admitted(h(hour), None);
            s.note_dispatch(h(hour));
            // Never released and never resolved: the next edge drops it as an
            // unclosed observation, which IS remembered.
        }
        s.note_frontier_admitted(h(100), None);
        let summary = s.summary();
        assert!(
            summary.drop_memory_overflows > 0,
            "the memory should have overflowed: {summary:?}"
        );
        // And the forgotten hour's later resolution is an illegal transition,
        // which is exactly what the overflow counter warns a reader about.
        s.note_publication_resolved(h(1), true);
        assert_eq!(s.summary().live_kept_hours_shadow_dropped, 0);
        assert_eq!(s.summary().illegal_transitions, 1);
    }

    /// The environment kill switch, in both directions.
    #[test]
    fn the_environment_kill_switch_defaults_on() {
        // The parse is asserted against a table rather than against the process
        // environment, which every test in this binary shares.
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

    /// PER-FRAME COST. Reported as a number, asserted against the lane's 50 µs
    /// budget with orders of magnitude of headroom, so a loaded machine cannot
    /// make it flap.
    ///
    /// One "frame" is the full live event set: edge, dispatch, release,
    /// resolution -- four notifications, six machine transitions.
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
