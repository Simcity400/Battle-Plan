use super::SegmentReason;

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

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FrameFacts {
    pub session_identity: String,
    pub epoch_token: String,
    pub hour_index: Option<i64>,
    pub reload_uncertain: bool,
    pub first_record_sequence: u64,
    pub first_byte_offset: u64,
    pub last_record_sequence: u64,
    pub end_byte_offset: u64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RollbackTransition {
    pub old_head_hour: i64,
    pub new_hour: i64,
    pub record_sequence: u64,
    pub byte_offset: u64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SealedSegment {
    pub segment_ordinal: u64,
    pub session_identity: String,
    pub epoch_token: String,
    pub start_reason: SegmentReason,
    pub stop_reason: SegmentReason,
    pub cutover_hour: Option<i64>,
    pub head_hour: Option<i64>,
    pub byte_bounds: (u64, u64),
    pub record_seq_bounds: (u64, u64),
    pub rollback_transition: Option<RollbackTransition>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum FrameRouting {
    Extend {
        ordinal: u64,
    },
    Cut {
        trigger: SegmentReason,
        sealed: Option<SealedSegment>,
        new_ordinal: u64,
    },
    UncertainSeal {
        sealed: Option<SealedSegment>,
    },
    HeldUncertain,
}

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

#[derive(Debug, Clone, Default)]
pub struct SegmentCutter {
    open: Option<OpenSegment>,
    next_ordinal: u64,
    held_epoch: Option<String>,
    last_admitted_extent: Option<(String, u64, u64)>,
}

impl SegmentCutter {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn is_uncertain(&self) -> bool {
        self.held_epoch.is_some()
    }

    pub fn open_ordinal(&self) -> Option<u64> {
        self.open.as_ref().map(|open| open.ordinal)
    }

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

    pub fn route(&mut self, frame: &FrameFacts) -> FrameRouting {
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

        if frame.reload_uncertain {
            let sealed = self.seal_open(SegmentReason::ReloadUncertain, None);
            self.held_epoch = Some(open_epoch.clone());
            return FrameRouting::UncertainSeal {
                sealed: Some(sealed),
            };
        }

        if frame.session_identity != open_session {
            let sealed = self.seal_open(SegmentReason::SessionStart, None);
            let ordinal = self.open_new(frame, SegmentReason::SessionStart);
            return FrameRouting::Cut {
                trigger: SegmentReason::SessionStart,
                sealed: Some(sealed),
                new_ordinal: ordinal,
            };
        }

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

        self.extend(frame);
        FrameRouting::Extend {
            ordinal: open_ordinal,
        }
    }

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
        assert_eq!(
            cutter.route(&frame("s0", "e0", 15)),
            FrameRouting::Extend { ordinal: 0 }
        );
        assert_eq!(
            cutter.route(&frame("s0", "e0", 15)),
            FrameRouting::Extend { ordinal: 0 }
        );
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
        cutter.route(&frame("s0", "e0", 12));
        let routing = cutter.route(&frame("s0", "e0", 8));
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
                assert_eq!(sealed.byte_bounds.0, 1000);
                assert_eq!(sealed.byte_bounds.1, 1300);
            }
            other => panic!("expected a rollback cut, got {other:?}"),
        }
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
        cutter.route(&frame("s0", "", 10));
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

        let mut still_uncertain = frame("s0", "e0", 12);
        still_uncertain.reload_uncertain = true;
        assert_eq!(cutter.route(&still_uncertain), FrameRouting::HeldUncertain);
        assert!(cutter.is_uncertain());

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
        let mut cutter = SegmentCutter::new();
        cutter.route(&frame("s0", "e0", 10));
        cutter.route(&frame("s0", "e0", 11));

        let mut uncertain = frame("s0", "e0", 12);
        uncertain.reload_uncertain = true;
        assert!(matches!(
            cutter.route(&uncertain),
            FrameRouting::UncertainSeal { sealed: Some(_) }
        ));
        assert!(cutter.is_uncertain());

        assert_eq!(
            cutter.route(&frame("s0", "e0", 13)),
            FrameRouting::HeldUncertain
        );
        assert!(cutter.is_uncertain(), "same-epoch frame must not resume");

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
        let mut cutter = SegmentCutter::new();
        cutter.route(&frame("s0", "e0", 10));
        cutter.route(&frame("s0", "e0", 11));

        let mut both = frame("s1", "e0", 12);
        both.reload_uncertain = true;
        match cutter.route(&both) {
            FrameRouting::UncertainSeal {
                sealed: Some(sealed),
            } => {
                assert_eq!(sealed.stop_reason, SegmentReason::ReloadUncertain);
                assert_eq!(sealed.session_identity, "s0");
                assert_eq!(sealed.head_hour, Some(11));
            }
            other => panic!("both-signals frame must be held under the seal, got {other:?}"),
        }
        assert!(cutter.is_uncertain());
        assert_eq!(cutter.open_ordinal(), None);
    }

    #[test]
    fn torn_first_frame_opens_an_empty_cutover_segment() {
        let mut cutter = SegmentCutter::new();
        let mut torn = frame("s0", "e0", 10);
        torn.hour_index = None;
        cutter.route(&torn);
        let sealed = cutter.finish().unwrap();
        assert_eq!(sealed.cutover_hour, None);
        assert_eq!(sealed.head_hour, None);
    }

    #[test]
    fn ordinals_are_scanned_from_zero_monotonically() {
        let mut cutter = SegmentCutter::new();
        cutter.route(&frame("s0", "e0", 10));
        cutter.route(&frame("s0", "e1", 11));
        cutter.route(&frame("s1", "e2", 12));
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
