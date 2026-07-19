//! Pre-fold **segment formation** (design `STITCHING_DESIGN.md` §5).
//!
//! The live stream driver must stage exactly one logical frame — all `DOMAIN_DATA`
//! records for one game hour plus its matching `FRAME_END` — and decide its segment
//! membership **before** applying it to any [`WorldFold`](crate::world_fold::WorldFold)
//! (§5.1). This module is the pure decision core the driver drives:
//! [`SegmentCutter::route`] takes the staged frame's identity / epoch / hour facts
//! and returns whether the frame extends the currently open segment or **cuts** a
//! new one, detecting all four §5.2 cut triggers and producing the §5.3 rollback
//! cut-transaction record.
//!
//! It owns no I/O and no fold: the caller performs the fsync / index-and-byte seal /
//! fresh-`WorldFold` construction the routing prescribes (per §5.3 the caller
//! spins up [`WorldFold::new_file_backed_for_segment`](crate::world_fold::WorldFold::new_file_backed_for_segment)
//! for the new ordinal so no accumulator crosses the cut). Keeping the decision core
//! I/O-free is what makes every trigger unit-testable offline.
//!
//! ## Determinism
//!
//! Segment ordinals are scanned from zero in observation order (§8 rule 2) — never a
//! PID, wall clock, mtime, or thread order. The `session-local` ordinal this cutter
//! assigns is the segment's in-session sequence; the finalize planner
//! ([`plan_stitch`](super::plan_stitch)) assigns the canonical cross-session global
//! `segment_id` from `(play_order, in_session_ordinal)`. For a single-session
//! recording (one recorder attachment with in-process reloads) the two coincide.
//!
//! ## What the driver still owns (interfaces left for the live-loop lane)
//!
//! * staging the frame bytes and computing [`FrameFacts`] (record framing already
//!   lives in [`crate::stream_fold`]);
//! * fsync + byte/index seal of the sealed segment, and constructing the fresh
//!   per-segment fold;
//! * retaining reload-uncertain / torn bytes as Failed coverage and building the
//!   final [`SegmentInput`](super::SegmentInput) coverage maps from fold results;
//! * the epoch/baseline completeness gate that decides whether the new segment may
//!   publish a snapshot (§5.3 step 6).

use super::SegmentReason;

/// Structural rejection at the dormant live-driver boundary. Invalid frame
/// facts are retained by the caller as failed raw evidence and are never routed
/// into a fold or allowed to mutate cutter state.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum FormationInputError {
    EmptySessionIdentity,
    InvertedRecordBounds { first: u64, last: u64 },
    EmptyByteExtent { start: u64, end: u64 },
    RecordSequenceRegression { previous_last: u64, next_first: u64 },
    ByteExtentOverlap { previous_end: u64, next_start: u64 },
}

impl std::fmt::Display for FormationInputError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::EmptySessionIdentity => write!(f, "frame has no process/session identity"),
            Self::InvertedRecordBounds { first, last } => {
                write!(f, "frame record bounds are inverted: {first}..={last}")
            }
            Self::EmptyByteExtent { start, end } => {
                write!(f, "frame byte extent is empty/inverted: [{start}, {end})")
            }
            Self::RecordSequenceRegression {
                previous_last,
                next_first,
            } => write!(
                f,
                "frame record sequence overlaps/regresses: previous last {previous_last}, next first {next_first}"
            ),
            Self::ByteExtentOverlap {
                previous_end,
                next_start,
            } => write!(
                f,
                "frame byte extent overlaps/regresses: previous end {previous_end}, next start {next_start}"
            ),
        }
    }
}

impl std::error::Error for FormationInputError {}

/// The staged logical frame's identity / epoch / coverage facts, computed by the
/// live driver before the frame is applied to any fold (§5.1). A `hour_index` of
/// `None` marks a torn / incomplete / epoch-uncertain frame that never produced a
/// matching `FRAME_END`: it is retained Failed and can never open visible history.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FrameFacts {
    /// Session / process-start identity token. A bare PID is **not** one (§3): the
    /// driver forms this from process-start identity, not the PID alone.
    pub session_identity: String,
    /// Verified save-epoch token for this frame. Empty while the epoch is not yet
    /// verified (an unverified epoch never triggers a clean [`SegmentReason::EpochChange`]
    /// cut; reload-uncertainty is the signal that seals instead — §5.2 #3).
    pub epoch_token: String,
    /// The frame's closed game `hour_index`, or `None` for a torn frame.
    pub hour_index: Option<i64>,
    /// The collector reported that reload may have begun at/just before this frame
    /// (§5.2 #3). Seals the prior segment immediately, even if the new epoch is not
    /// yet identified.
    pub reload_uncertain: bool,
    /// Decoded record sequence of the frame's FIRST record.
    pub first_record_sequence: u64,
    /// Decoded byte offset (half-open start) of the frame's FIRST record.
    pub first_byte_offset: u64,
    /// Decoded record sequence of the frame's LAST record (its `FRAME_END`).
    pub last_record_sequence: u64,
    /// Decoded byte offset (half-open end) immediately after the frame's last record.
    pub end_byte_offset: u64,
}

/// The observed backward-time transition recorded when a rollback cuts a segment
/// (§5.3 step 3): `(old_hour, new_hour, record_sequence, byte_offset)`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RollbackTransition {
    pub old_head_hour: i64,
    pub new_hour: i64,
    /// Decoded record sequence of the first record of the backward frame.
    pub record_sequence: u64,
    /// Decoded byte offset of the first record of the backward frame.
    pub byte_offset: u64,
}

/// A segment sealed by a cut or by session end. The caller fsyncs and seals the raw
/// stream extent at these bounds and records the reasons in segment provenance.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SealedSegment {
    /// Session-local (in-session) ordinal, scanned from zero (§8 rule 2).
    pub segment_ordinal: u64,
    pub session_identity: String,
    pub epoch_token: String,
    pub start_reason: SegmentReason,
    pub stop_reason: SegmentReason,
    /// First observed owned hour, or `None` for an empty recovery segment (§3).
    pub cutover_hour: Option<i64>,
    /// Greatest complete observed hour, or `None` for an empty recovery segment.
    pub head_hour: Option<i64>,
    /// Half-open decoded byte extent `[start, end)` of the segment's records. On a
    /// contiguous stream `end` equals the triggering frame's `first_byte_offset`
    /// (§5.3 step 2: seal at the byte immediately before the backward frame).
    pub byte_bounds: (u64, u64),
    /// Inclusive decoded record-sequence bounds `[start, end]` of the segment.
    pub record_seq_bounds: (u64, u64),
    /// Present only for a [`SegmentReason::TimeRollback`] stop (§5.3 step 3).
    pub rollback_transition: Option<RollbackTransition>,
}

/// The routing decision for one staged frame.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum FrameRouting {
    /// The frame extends the currently open segment (a same-hour observation or a
    /// forward jump — neither cuts, §5.2 note). `ordinal` is the open segment's.
    Extend { ordinal: u64 },
    /// A cut trigger fired. `sealed` is the prior open segment sealed at its stop
    /// reason (`None` on the very first frame); the staged frame is applied to the
    /// NEW segment's fresh fold (`new_ordinal`). `trigger` is the new segment's
    /// start reason and equals the prior segment's stop reason for that cut.
    Cut {
        trigger: SegmentReason,
        sealed: Option<SealedSegment>,
        new_ordinal: u64,
    },
    /// Reload may have begun (§5.2 #3): the prior segment is sealed immediately
    /// (`None` if none was open) and the cutter enters an uncertainty hold. The
    /// staged frame is Failed and opens no visible segment.
    UncertainSeal { sealed: Option<SealedSegment> },
    /// A frame observed while the cutter is in the reload-uncertainty hold (§5.3
    /// step 6): retained Failed by the driver, opens no visible history until a new
    /// verified epoch resumes.
    HeldUncertain,
}

/// One open segment's running accumulation. Byte/record end bounds advance with
/// every extended frame so a later cut can seal the exact extent.
#[derive(Debug, Clone)]
struct OpenSegment {
    ordinal: u64,
    session_identity: String,
    epoch_token: String,
    start_reason: SegmentReason,
    cutover_hour: Option<i64>,
    last_complete_hour: Option<i64>,
    start_record_sequence: u64,
    start_byte_offset: u64,
    last_record_sequence: u64,
    last_byte_end: u64,
}

/// The pre-fold segment-formation state machine (design §5). Feed it one staged
/// [`FrameFacts`] per logical frame in observation order; seal the tail with
/// [`SegmentCutter::finish`].
#[derive(Debug, Clone, Default)]
pub struct SegmentCutter {
    open: Option<OpenSegment>,
    next_ordinal: u64,
    /// The reload-uncertainty hold. `Some(epoch)` means the cutter is holding after a
    /// reload-uncertain seal, retaining the sealed segment's (old, trusted) epoch
    /// identity: visible history resumes only when a frame carries a *genuinely
    /// different* verified epoch, so a same-epoch late frame during the hold stays
    /// Failed (§5.2 #3 / §5.3 step 6, fail-closed). The held value is empty when there
    /// was no prior trusted epoch (a reload-uncertain very-first frame), in which case
    /// any verified epoch resumes. `None` means no hold is active.
    held_epoch: Option<String>,
    /// Last structurally admitted frame extent. Sequence/byte monotonicity is
    /// enforced within one session identity before routing; a new process/session
    /// may legitimately start a fresh physical stream at sequence/offset zero.
    last_admitted_extent: Option<(String, u64, u64)>,
}

impl SegmentCutter {
    pub fn new() -> Self {
        Self::default()
    }

    /// True while the cutter is holding reload uncertainty (no visible segment open).
    pub fn is_uncertain(&self) -> bool {
        self.held_epoch.is_some()
    }

    /// The open segment's session-local ordinal, if one is open.
    pub fn open_ordinal(&self) -> Option<u64> {
        self.open.as_ref().map(|open| open.ordinal)
    }

    /// Validate and route one staged frame at the production boundary.
    ///
    /// Unlike [`Self::route`], this rejects impossible source bounds before any
    /// state mutation. The unchecked method remains for already-validated callers
    /// and compatibility with the pure historical test surface; live composition
    /// must use this checked seam.
    pub fn route_checked(
        &mut self,
        frame: &FrameFacts,
    ) -> Result<FrameRouting, FormationInputError> {
        self.validate_frame(frame)?;
        let routing = self.route(frame);
        self.last_admitted_extent = Some((
            frame.session_identity.clone(),
            frame.last_record_sequence,
            frame.end_byte_offset,
        ));
        Ok(routing)
    }

    fn validate_frame(&self, frame: &FrameFacts) -> Result<(), FormationInputError> {
        if frame.session_identity.is_empty() {
            return Err(FormationInputError::EmptySessionIdentity);
        }
        if frame.first_record_sequence > frame.last_record_sequence {
            return Err(FormationInputError::InvertedRecordBounds {
                first: frame.first_record_sequence,
                last: frame.last_record_sequence,
            });
        }
        if frame.first_byte_offset >= frame.end_byte_offset {
            return Err(FormationInputError::EmptyByteExtent {
                start: frame.first_byte_offset,
                end: frame.end_byte_offset,
            });
        }
        if let Some((session_identity, previous_last, previous_end)) = &self.last_admitted_extent {
            if session_identity == &frame.session_identity {
                if frame.first_record_sequence <= *previous_last {
                    return Err(FormationInputError::RecordSequenceRegression {
                        previous_last: *previous_last,
                        next_first: frame.first_record_sequence,
                    });
                }
                if frame.first_byte_offset < *previous_end {
                    return Err(FormationInputError::ByteExtentOverlap {
                        previous_end: *previous_end,
                        next_start: frame.first_byte_offset,
                    });
                }
            }
        }
        Ok(())
    }

    /// Route one staged frame, mutating the open-segment state.
    pub fn route(&mut self, frame: &FrameFacts) -> FrameRouting {
        // (1) Uncertainty hold: only a GENUINELY DIFFERENT verified epoch on a complete
        // frame that is not itself reload-uncertain resumes visible history (§5.3 step
        // 6). The hold retains the sealed segment's (old, trusted) epoch identity, so a
        // same-epoch late frame observed after the reload-uncertain seal stays Failed
        // (fail-closed) instead of opening a new visible segment — it does not resume
        // until a genuinely changed verified epoch is seen. An empty held epoch means
        // there was no prior trusted epoch, so any verified epoch resumes.
        if let Some(held_epoch) = self.held_epoch.clone() {
            if !frame.epoch_token.is_empty()
                && frame.epoch_token != held_epoch
                && frame.hour_index.is_some()
                && !frame.reload_uncertain
            {
                self.held_epoch = None;
                let ordinal = self.open_new(frame, SegmentReason::EpochChange);
                return FrameRouting::Cut {
                    trigger: SegmentReason::EpochChange,
                    sealed: None,
                    new_ordinal: ordinal,
                };
            }
            return FrameRouting::HeldUncertain;
        }

        // (2) Very first frame: open the first segment (or hold if it is itself
        // reload-uncertain).
        let Some((open_session, open_epoch, open_last_hour, open_ordinal)) =
            self.open.as_ref().map(|open| {
                (
                    open.session_identity.clone(),
                    open.epoch_token.clone(),
                    open.last_complete_hour,
                    open.ordinal,
                )
            })
        else {
            if frame.reload_uncertain {
                // No prior trusted epoch to compare against: hold with an empty held
                // epoch so any later verified epoch resumes visible history.
                self.held_epoch = Some(String::new());
                return FrameRouting::UncertainSeal { sealed: None };
            }
            let ordinal = self.open_new(frame, SegmentReason::SessionStart);
            return FrameRouting::Cut {
                trigger: SegmentReason::SessionStart,
                sealed: None,
                new_ordinal: ordinal,
            };
        };

        // (3) Reload may have begun (§5.2 #3): seal immediately and enter the
        // uncertainty hold. This is evaluated BEFORE the session-identity trigger
        // because it is the more conservative, fail-closed state: a frame carrying BOTH
        // a reload-uncertain signal AND a new session identity must be held Failed under
        // the seal, never admitted to a fresh visible segment for folding. Seal at the
        // OLD trusted epoch so the hold resumes only when a genuinely different verified
        // epoch appears.
        if frame.reload_uncertain {
            let sealed = self.seal_open(SegmentReason::ReloadUncertain, None);
            self.held_epoch = Some(open_epoch.clone());
            return FrameRouting::UncertainSeal {
                sealed: Some(sealed),
            };
        }

        // (4) New session / process-start identity (§5.2 #1).
        if frame.session_identity != open_session {
            let sealed = self.seal_open(SegmentReason::SessionStart, None);
            let ordinal = self.open_new(frame, SegmentReason::SessionStart);
            return FrameRouting::Cut {
                trigger: SegmentReason::SessionStart,
                sealed: Some(sealed),
                new_ordinal: ordinal,
            };
        }

        // (5) Verified save-epoch token changed (§5.2 #2). Both tokens must be
        // verified (non-empty); an unverified token never forces a clean cut.
        if !frame.epoch_token.is_empty()
            && !open_epoch.is_empty()
            && frame.epoch_token != open_epoch
        {
            let sealed = self.seal_open(SegmentReason::EpochChange, None);
            let ordinal = self.open_new(frame, SegmentReason::EpochChange);
            return FrameRouting::Cut {
                trigger: SegmentReason::EpochChange,
                sealed: Some(sealed),
                new_ordinal: ordinal,
            };
        }

        // (6) Backward hour = rollback (§5.2 #4 / §5.3). `== head` is NOT a rollback
        // (§5.2): a same-hour observation stays in the segment.
        if let (Some(hour), Some(prev)) = (frame.hour_index, open_last_hour) {
            if hour < prev {
                let transition = RollbackTransition {
                    old_head_hour: prev,
                    new_hour: hour,
                    record_sequence: frame.first_record_sequence,
                    byte_offset: frame.first_byte_offset,
                };
                let sealed = self.seal_open(SegmentReason::TimeRollback, Some(transition));
                let ordinal = self.open_new(frame, SegmentReason::TimeRollback);
                return FrameRouting::Cut {
                    trigger: SegmentReason::TimeRollback,
                    sealed: Some(sealed),
                    new_ordinal: ordinal,
                };
            }
        }

        // (7) Same hour or forward jump: stays in the open segment. A forward jump
        // leaves an explicit coverage gap the driver records; the cutter does not
        // invent it.
        self.extend(frame);
        FrameRouting::Extend {
            ordinal: open_ordinal,
        }
    }

    /// Seal the currently open segment at session end (§5.2 [`SegmentReason::SessionEnd`]).
    /// Returns `None` if nothing is open (e.g. the tail was reload-uncertain).
    pub fn finish(&mut self) -> Option<SealedSegment> {
        if self.open.is_some() {
            Some(self.seal_open(SegmentReason::SessionEnd, None))
        } else {
            None
        }
    }

    fn open_new(&mut self, frame: &FrameFacts, start_reason: SegmentReason) -> u64 {
        let ordinal = self.next_ordinal;
        self.next_ordinal += 1;
        self.open = Some(OpenSegment {
            ordinal,
            session_identity: frame.session_identity.clone(),
            epoch_token: frame.epoch_token.clone(),
            start_reason,
            cutover_hour: frame.hour_index,
            last_complete_hour: frame.hour_index,
            start_record_sequence: frame.first_record_sequence,
            start_byte_offset: frame.first_byte_offset,
            last_record_sequence: frame.last_record_sequence,
            last_byte_end: frame.end_byte_offset,
        });
        ordinal
    }

    fn extend(&mut self, frame: &FrameFacts) {
        if let Some(open) = self.open.as_mut() {
            if let Some(hour) = frame.hour_index {
                if open.cutover_hour.is_none() {
                    open.cutover_hour = Some(hour);
                }
                open.last_complete_hour =
                    Some(open.last_complete_hour.map_or(hour, |prev| prev.max(hour)));
            }
            open.last_record_sequence = frame.last_record_sequence;
            open.last_byte_end = frame.end_byte_offset;
        }
    }

    fn seal_open(
        &mut self,
        stop_reason: SegmentReason,
        rollback_transition: Option<RollbackTransition>,
    ) -> SealedSegment {
        let open = self
            .open
            .take()
            .expect("seal_open requires an open segment");
        SealedSegment {
            segment_ordinal: open.ordinal,
            session_identity: open.session_identity,
            epoch_token: open.epoch_token,
            start_reason: open.start_reason,
            stop_reason,
            cutover_hour: open.cutover_hour,
            head_hour: open.last_complete_hour,
            byte_bounds: (open.start_byte_offset, open.last_byte_end),
            record_seq_bounds: (open.start_record_sequence, open.last_record_sequence),
            rollback_transition,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A complete frame in `session`/`epoch` at `hour`, self-consistent record/byte
    /// bounds derived from the hour so a stream of frames stays contiguous.
    fn frame(session: &str, epoch: &str, hour: i64) -> FrameFacts {
        let seq = hour as u64 * 2;
        let byte = hour as u64 * 100;
        FrameFacts {
            session_identity: session.into(),
            epoch_token: epoch.into(),
            hour_index: Some(hour),
            reload_uncertain: false,
            first_record_sequence: seq,
            first_byte_offset: byte,
            last_record_sequence: seq + 1,
            end_byte_offset: byte + 100,
        }
    }

    #[test]
    fn first_frame_opens_a_session_start_segment() {
        let mut cutter = SegmentCutter::new();
        let routing = cutter.route(&frame("s0", "e0", 10));
        assert_eq!(
            routing,
            FrameRouting::Cut {
                trigger: SegmentReason::SessionStart,
                sealed: None,
                new_ordinal: 0,
            }
        );
        assert_eq!(cutter.open_ordinal(), Some(0));
    }

    #[test]
    fn forward_and_same_hour_frames_extend_without_cutting() {
        let mut cutter = SegmentCutter::new();
        cutter.route(&frame("s0", "e0", 10));
        // Forward jump: same segment (the gap is the driver's to record).
        assert_eq!(
            cutter.route(&frame("s0", "e0", 15)),
            FrameRouting::Extend { ordinal: 0 }
        );
        // Same hour: NOT a rollback (§5.2), stays in the segment.
        assert_eq!(
            cutter.route(&frame("s0", "e0", 15)),
            FrameRouting::Extend { ordinal: 0 }
        );
        // Head advanced to 15, still one open segment.
        let sealed = cutter.finish().unwrap();
        assert_eq!(sealed.cutover_hour, Some(10));
        assert_eq!(sealed.head_hour, Some(15));
        assert_eq!(sealed.stop_reason, SegmentReason::SessionEnd);
    }

    #[test]
    fn backward_hour_cuts_a_rollback_segment_with_transition() {
        let mut cutter = SegmentCutter::new();
        cutter.route(&frame("s0", "e0", 10));
        cutter.route(&frame("s0", "e0", 11));
        cutter.route(&frame("s0", "e0", 12)); // head = 12
        let routing = cutter.route(&frame("s0", "e0", 8)); // rollback to hour 8
        match routing {
            FrameRouting::Cut {
                trigger,
                sealed: Some(sealed),
                new_ordinal,
            } => {
                assert_eq!(trigger, SegmentReason::TimeRollback);
                assert_eq!(new_ordinal, 1, "rollback allocates the next ordinal");
                assert_eq!(sealed.segment_ordinal, 0);
                assert_eq!(sealed.stop_reason, SegmentReason::TimeRollback);
                assert_eq!(sealed.cutover_hour, Some(10));
                assert_eq!(sealed.head_hour, Some(12));
                let transition = sealed.rollback_transition.expect("rollback transition");
                assert_eq!(transition.old_head_hour, 12);
                assert_eq!(transition.new_hour, 8);
                // The seal ends at the byte immediately before the backward frame
                // (contiguous stream): last applied frame (hour 12) ended at 1300,
                // which equals the backward frame's first byte offset (8 -> 800)?
                // No: they are only equal on a truly contiguous stream. Here the
                // helper derives bytes from the hour, so assert the seal used the
                // segment's own accumulated extent.
                assert_eq!(sealed.byte_bounds.0, 1000); // hour 10 start
                assert_eq!(sealed.byte_bounds.1, 1300); // hour 12 end
            }
            other => panic!("expected a rollback cut, got {other:?}"),
        }
        // The new segment starts fresh at hour 8.
        let tail = cutter.finish().unwrap();
        assert_eq!(tail.segment_ordinal, 1);
        assert_eq!(tail.cutover_hour, Some(8));
        assert_eq!(tail.head_hour, Some(8));
    }

    #[test]
    fn epoch_change_cuts_a_new_segment() {
        let mut cutter = SegmentCutter::new();
        cutter.route(&frame("s0", "e0", 10));
        let routing = cutter.route(&frame("s0", "e1", 11));
        match routing {
            FrameRouting::Cut {
                trigger,
                sealed: Some(sealed),
                new_ordinal,
            } => {
                assert_eq!(trigger, SegmentReason::EpochChange);
                assert_eq!(new_ordinal, 1);
                assert_eq!(sealed.stop_reason, SegmentReason::EpochChange);
                assert_eq!(sealed.epoch_token, "e0");
                assert!(sealed.rollback_transition.is_none());
            }
            other => panic!("expected an epoch-change cut, got {other:?}"),
        }
    }

    #[test]
    fn unverified_epoch_token_does_not_force_a_clean_cut() {
        let mut cutter = SegmentCutter::new();
        // Epoch not yet verified (empty token).
        cutter.route(&frame("s0", "", 10));
        // Still-empty epoch on a forward frame is not an EpochChange cut.
        assert_eq!(
            cutter.route(&frame("s0", "", 11)),
            FrameRouting::Extend { ordinal: 0 }
        );
    }

    #[test]
    fn new_session_identity_cuts_a_new_segment() {
        let mut cutter = SegmentCutter::new();
        cutter.route(&frame("s0", "e0", 10));
        let routing = cutter.route(&frame("s1", "e0", 11));
        match routing {
            FrameRouting::Cut {
                trigger,
                sealed: Some(sealed),
                new_ordinal,
            } => {
                assert_eq!(trigger, SegmentReason::SessionStart);
                assert_eq!(new_ordinal, 1);
                assert_eq!(sealed.session_identity, "s0");
                assert_eq!(sealed.stop_reason, SegmentReason::SessionStart);
            }
            other => panic!("expected a session-start cut, got {other:?}"),
        }
    }

    #[test]
    fn reload_uncertain_seals_and_holds_until_a_new_verified_epoch() {
        let mut cutter = SegmentCutter::new();
        cutter.route(&frame("s0", "e0", 10));
        cutter.route(&frame("s0", "e0", 11));

        // Reload may have begun: seal immediately and hold.
        let mut uncertain = frame("s0", "e0", 12);
        uncertain.reload_uncertain = true;
        match cutter.route(&uncertain) {
            FrameRouting::UncertainSeal {
                sealed: Some(sealed),
            } => {
                assert_eq!(sealed.stop_reason, SegmentReason::ReloadUncertain);
                assert_eq!(sealed.head_hour, Some(11));
            }
            other => panic!("expected an uncertain seal, got {other:?}"),
        }
        assert!(cutter.is_uncertain());

        // A frame observed while still uncertain (no new verified epoch, or itself
        // uncertain) is held Failed and opens no visible segment.
        let mut still_uncertain = frame("s0", "e0", 12);
        still_uncertain.reload_uncertain = true;
        assert_eq!(cutter.route(&still_uncertain), FrameRouting::HeldUncertain);
        assert!(cutter.is_uncertain());

        // A new verified epoch on a complete frame resumes visible history.
        match cutter.route(&frame("s0", "e1", 8)) {
            FrameRouting::Cut {
                trigger,
                sealed,
                new_ordinal,
            } => {
                assert_eq!(trigger, SegmentReason::EpochChange);
                assert!(sealed.is_none(), "prior segment was already sealed");
                assert_eq!(new_ordinal, 1);
            }
            other => panic!("expected a resume cut, got {other:?}"),
        }
        assert!(!cutter.is_uncertain());
    }

    #[test]
    fn same_epoch_frame_during_hold_stays_failed_until_a_changed_epoch() {
        // Finding 3: the reload-uncertainty hold must retain the sealed epoch identity.
        // A same-epoch late frame observed after the seal must stay Failed (HeldUncertain)
        // and NOT open a new visible segment; only a genuinely CHANGED verified epoch
        // resumes.
        let mut cutter = SegmentCutter::new();
        cutter.route(&frame("s0", "e0", 10));
        cutter.route(&frame("s0", "e0", 11));

        // Reload may have begun: seal at the old trusted epoch e0 and hold.
        let mut uncertain = frame("s0", "e0", 12);
        uncertain.reload_uncertain = true;
        assert!(matches!(
            cutter.route(&uncertain),
            FrameRouting::UncertainSeal { sealed: Some(_) }
        ));
        assert!(cutter.is_uncertain());

        // A COMPLETE, non-uncertain, SAME-epoch (e0) frame arrives during the hold. It
        // is the same sealed epoch, so it stays Failed — this is the regression: before
        // the fix any nonempty epoch resumed, opening a spurious visible segment here.
        assert_eq!(
            cutter.route(&frame("s0", "e0", 13)),
            FrameRouting::HeldUncertain
        );
        assert!(cutter.is_uncertain(), "same-epoch frame must not resume");

        // Only a genuinely CHANGED verified epoch (e1) resumes visible history.
        match cutter.route(&frame("s0", "e1", 8)) {
            FrameRouting::Cut {
                trigger,
                sealed,
                new_ordinal,
            } => {
                assert_eq!(trigger, SegmentReason::EpochChange);
                assert!(sealed.is_none(), "prior segment was already sealed");
                assert_eq!(new_ordinal, 1);
            }
            other => panic!("expected a resume cut on the changed epoch, got {other:?}"),
        }
        assert!(!cutter.is_uncertain());
    }

    #[test]
    fn frame_with_both_reload_uncertain_and_new_session_is_held_not_admitted() {
        // Finding 4: reload-uncertainty is evaluated BEFORE session identity. A frame
        // carrying BOTH a new session identity AND a reload-uncertain signal must be
        // held Failed under the uncertainty seal — not routed into a fresh visible
        // SessionStart segment for folding.
        let mut cutter = SegmentCutter::new();
        cutter.route(&frame("s0", "e0", 10));
        cutter.route(&frame("s0", "e0", 11));

        let mut both = frame("s1", "e0", 12); // new session identity...
        both.reload_uncertain = true; // ...AND reload-uncertain
        match cutter.route(&both) {
            FrameRouting::UncertainSeal {
                sealed: Some(sealed),
            } => {
                // The prior segment is sealed as ReloadUncertain (the conservative
                // reason), NOT SessionStart.
                assert_eq!(sealed.stop_reason, SegmentReason::ReloadUncertain);
                assert_eq!(sealed.session_identity, "s0");
                assert_eq!(sealed.head_hour, Some(11));
            }
            other => panic!("both-signals frame must be held under the seal, got {other:?}"),
        }
        assert!(cutter.is_uncertain());
        // Nothing was opened for the both-signals frame: no visible segment is live.
        assert_eq!(cutter.open_ordinal(), None);
    }

    #[test]
    fn torn_first_frame_opens_an_empty_cutover_segment() {
        let mut cutter = SegmentCutter::new();
        let mut torn = frame("s0", "e0", 10);
        torn.hour_index = None; // never closed
        cutter.route(&torn);
        let sealed = cutter.finish().unwrap();
        // A frame with no hour contributes no cutover/head (§3).
        assert_eq!(sealed.cutover_hour, None);
        assert_eq!(sealed.head_hour, None);
    }

    #[test]
    fn ordinals_are_scanned_from_zero_monotonically() {
        let mut cutter = SegmentCutter::new();
        cutter.route(&frame("s0", "e0", 10)); // ordinal 0
        cutter.route(&frame("s0", "e1", 11)); // ordinal 1 (epoch change)
        cutter.route(&frame("s1", "e2", 12)); // ordinal 2 (session change)
        assert_eq!(cutter.open_ordinal(), Some(2));
        assert_eq!(cutter.finish().unwrap().segment_ordinal, 2);
    }

    #[test]
    fn checked_route_rejects_impossible_bounds_without_mutating_state() {
        let mut cutter = SegmentCutter::new();
        let mut invalid = frame("s0", "e0", 10);
        invalid.last_record_sequence = invalid.first_record_sequence - 1;
        assert!(matches!(
            cutter.route_checked(&invalid),
            Err(FormationInputError::InvertedRecordBounds { .. })
        ));
        assert_eq!(cutter.open_ordinal(), None);

        invalid = frame("s0", "e0", 10);
        invalid.end_byte_offset = invalid.first_byte_offset;
        assert!(matches!(
            cutter.route_checked(&invalid),
            Err(FormationInputError::EmptyByteExtent { .. })
        ));
        assert_eq!(cutter.open_ordinal(), None);
    }

    #[test]
    fn checked_route_rejects_same_session_overlap_and_accepts_next_clean_frame() {
        let mut cutter = SegmentCutter::new();
        let first = FrameFacts {
            session_identity: "s0".into(),
            epoch_token: "e0".into(),
            hour_index: Some(10),
            reload_uncertain: false,
            first_record_sequence: 10,
            first_byte_offset: 100,
            last_record_sequence: 11,
            end_byte_offset: 200,
        };
        assert!(cutter.route_checked(&first).is_ok());

        let mut overlap = first.clone();
        overlap.hour_index = Some(11);
        overlap.first_record_sequence = 11;
        overlap.last_record_sequence = 12;
        overlap.first_byte_offset = 199;
        overlap.end_byte_offset = 250;
        assert!(matches!(
            cutter.route_checked(&overlap),
            Err(FormationInputError::RecordSequenceRegression { .. })
        ));

        let clean = FrameFacts {
            hour_index: Some(11),
            first_record_sequence: 12,
            last_record_sequence: 13,
            first_byte_offset: 200,
            end_byte_offset: 300,
            ..first
        };
        assert_eq!(
            cutter.route_checked(&clean).unwrap(),
            FrameRouting::Extend { ordinal: 0 }
        );
    }

    #[test]
    fn checked_route_allows_new_session_to_reset_physical_stream_bounds() {
        let mut cutter = SegmentCutter::new();
        let first = FrameFacts {
            session_identity: "s0".into(),
            epoch_token: "e0".into(),
            hour_index: Some(10),
            reload_uncertain: false,
            first_record_sequence: 100,
            first_byte_offset: 1_000,
            last_record_sequence: 101,
            end_byte_offset: 1_100,
        };
        cutter.route_checked(&first).unwrap();
        let reset = FrameFacts {
            session_identity: "s1".into(),
            epoch_token: "e1".into(),
            hour_index: Some(11),
            reload_uncertain: false,
            first_record_sequence: 0,
            first_byte_offset: 0,
            last_record_sequence: 1,
            end_byte_offset: 100,
        };
        assert!(matches!(
            cutter.route_checked(&reset).unwrap(),
            FrameRouting::Cut {
                trigger: SegmentReason::SessionStart,
                new_ordinal: 1,
                ..
            }
        ));
    }
}
