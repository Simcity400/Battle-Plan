//! The per-identity attach state machine (design §4).
//!
//! Each detected `(pid, creation_filetime)` gets its own `CandidateState`. The
//! states and their legal transitions are exactly the table in the design; this
//! module encodes them as data so the coordinator cannot silently invent an
//! illegal jump (e.g. reaching `Recording` without a heartbeat, which the
//! self-refutation section calls out as the original bug class).

use std::fmt;

/// The lifecycle states from design §4. Ordered roughly by progress; terminal
/// states are grouped at the end.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Hash)]
pub enum CandidateState {
    /// Just observed; identity query pending.
    Detected,
    /// Identity/modset evidence missing or ambiguous. Fail-closed holding state.
    WaitingForIdentity,
    /// Identity resolved but no injectable visible window yet.
    WaitingForWindow,
    /// Staging a fresh DLL directory, control file, and pipe.
    Preparing,
    /// Pipe listening barrier satisfied; installing the hook exactly once.
    Injecting,
    /// Hook installed; waiting for a session-matching heartbeat.
    Handshaking,
    /// Collector resident and proven; waiting for a structurally valid frame.
    WaitingForWorld,
    /// A valid session frame arrived; capture is live.
    Recording,
    /// Folding/packing/verifying the replay.
    Finalizing,
    /// Verified replay produced. Terminal (success).
    Complete,
    /// Any hard failure. Terminal; NO uncertain reinjection (design §5, §8).
    Failed,
    /// Process exited before we ever attached. Terminal (design §8 closure).
    ProcessExitedBeforeAttach,
}

impl CandidateState {
    /// Terminal states never transition again. Every detected identity MUST end
    /// in one of these (design §8: "Every detected identity must end
    /// `Complete`, `Failed`, or `ProcessExitedBeforeAttach`" — `Complete` here
    /// subsumes the finalized-success path).
    pub fn is_terminal(self) -> bool {
        matches!(
            self,
            CandidateState::Complete
                | CandidateState::Failed
                | CandidateState::ProcessExitedBeforeAttach
        )
    }

    /// True once a collector DLL has (or may have) been injected into the live
    /// process. Past this line a second injection would crash the game
    /// (SINGLE-INJECTION LAW), so the coordinator must never start a second
    /// attempt for the same identity while it is in — or past — one of these.
    pub fn injection_committed(self) -> bool {
        matches!(
            self,
            CandidateState::Injecting
                | CandidateState::Handshaking
                | CandidateState::WaitingForWorld
                | CandidateState::Recording
                | CandidateState::Finalizing
                | CandidateState::Complete
        )
    }

    /// Human label for the tray/window card (design §4 "Visible result").
    pub fn visible_label(self) -> &'static str {
        match self {
            CandidateState::Detected => "the game detected",
            CandidateState::WaitingForIdentity => "Resolving game identity",
            CandidateState::WaitingForWindow => "the game is still starting",
            CandidateState::Preparing => "Preparing recorder",
            CandidateState::Injecting => "Attaching",
            CandidateState::Handshaking => "Waiting for collector proof",
            CandidateState::WaitingForWorld => "Attached; waiting for game/save",
            CandidateState::Recording => "Recording",
            CandidateState::Finalizing => "Saving replay",
            CandidateState::Complete => "Replay saved",
            CandidateState::Failed => "Attach failed",
            CandidateState::ProcessExitedBeforeAttach => "Game closed before attach",
        }
    }

    /// The legal forward transitions from this state. A process-exit or hard
    /// failure edge is allowed from any non-terminal state and is applied
    /// through [`CandidateState::on_process_exit`] / [`CandidateState::fail`],
    /// NOT this table, so keep this to the "happy path plus documented retries".
    pub fn legal_next(self) -> &'static [CandidateState] {
        use CandidateState::*;
        match self {
            Detected => &[WaitingForIdentity, WaitingForWindow],
            // identity can resolve straight to window-wait, or bounce back while
            // evidence is still ambiguous (self-loop is allowed via retry).
            WaitingForIdentity => &[WaitingForIdentity, WaitingForWindow],
            WaitingForWindow => &[WaitingForWindow, Preparing],
            Preparing => &[Injecting],
            Injecting => &[Handshaking],
            Handshaking => &[WaitingForWorld],
            WaitingForWorld => &[Recording],
            Recording => &[Finalizing],
            Finalizing => &[Complete],
            Complete | Failed | ProcessExitedBeforeAttach => &[],
        }
    }

    /// Whether `next` is a legal *forward* transition from `self`.
    pub fn can_advance_to(self, next: CandidateState) -> bool {
        self.legal_next().contains(&next)
    }

    /// The state a live candidate moves to when its process is observed gone.
    /// Before injection this is a clean "exited before attach"; after injection
    /// commit it is a `Finalizing`/`Failed` decision the coordinator makes with
    /// the worker's prefix status — so this only answers the pre-attach case.
    pub fn on_process_exit(self) -> Option<CandidateState> {
        if self.is_terminal() {
            None
        } else if self.injection_committed() {
            // Post-injection exit is handled by the worker-finalization path,
            // not here; returning None signals "not a simple pre-attach exit".
            None
        } else {
            Some(CandidateState::ProcessExitedBeforeAttach)
        }
    }
}

impl fmt::Display for CandidateState {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{self:?}")
    }
}

/// Stable machine-readable reason codes for the journal and UI (design §8).
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum ReasonCode {
    Detected,
    IdentityUnresolved,
    ModsetUnresolved,
    UserDirAmbiguous,
    WrongUserOrSession,
    IdentityMismatchAtInject,
    NoVisibleWindow,
    Preparing,
    PayloadHashMismatch,
    PipeBarrierTimeout,
    Injected,
    HeartbeatTimeout,
    HeartbeatMismatch,
    CollectorAttached,
    FirstFrameValid,
    RecordingStarted,
    ProcessExited,
    PipeBrokenWhileAlive,
    DiskFailure,
    FinalizeVerifyFailed,
    ReplayComplete,
    RecoveredNonterminalReinjectionBlocked,
    ExplicitStop,
}

impl ReasonCode {
    pub fn as_str(self) -> &'static str {
        match self {
            ReasonCode::Detected => "detected",
            ReasonCode::IdentityUnresolved => "identity_unresolved",
            ReasonCode::ModsetUnresolved => "modset_unresolved",
            ReasonCode::UserDirAmbiguous => "user_dir_ambiguous",
            ReasonCode::WrongUserOrSession => "wrong_user_or_session",
            ReasonCode::IdentityMismatchAtInject => "identity_mismatch_at_inject",
            ReasonCode::NoVisibleWindow => "no_visible_window",
            ReasonCode::Preparing => "preparing",
            ReasonCode::PayloadHashMismatch => "payload_hash_mismatch",
            ReasonCode::PipeBarrierTimeout => "pipe_barrier_timeout",
            ReasonCode::Injected => "injected",
            ReasonCode::HeartbeatTimeout => "heartbeat_timeout",
            ReasonCode::HeartbeatMismatch => "heartbeat_mismatch",
            ReasonCode::CollectorAttached => "collector_attached",
            ReasonCode::FirstFrameValid => "first_frame_valid",
            ReasonCode::RecordingStarted => "recording_started",
            ReasonCode::ProcessExited => "process_exited",
            ReasonCode::PipeBrokenWhileAlive => "pipe_broken_while_alive",
            ReasonCode::DiskFailure => "disk_failure",
            ReasonCode::FinalizeVerifyFailed => "finalize_verify_failed",
            ReasonCode::ReplayComplete => "replay_complete",
            ReasonCode::RecoveredNonterminalReinjectionBlocked => {
                "recovered_nonterminal_reinjection_blocked"
            }
            ReasonCode::ExplicitStop => "explicit_stop",
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn terminals_do_not_advance() {
        for s in [
            CandidateState::Complete,
            CandidateState::Failed,
            CandidateState::ProcessExitedBeforeAttach,
        ] {
            assert!(s.is_terminal());
            assert!(s.legal_next().is_empty());
            assert_eq!(s.on_process_exit(), None);
        }
    }

    #[test]
    fn recording_unreachable_without_full_ladder() {
        // The design's prime bug class: reaching Recording without heartbeat/
        // frame proof. Prove there is no single legal edge that skips the
        // Handshaking -> WaitingForWorld -> Recording ladder.
        assert!(!CandidateState::Injecting.can_advance_to(CandidateState::Recording));
        assert!(!CandidateState::Preparing.can_advance_to(CandidateState::WaitingForWorld));
        assert!(!CandidateState::Handshaking.can_advance_to(CandidateState::Recording));
        assert!(CandidateState::WaitingForWorld.can_advance_to(CandidateState::Recording));
    }

    #[test]
    fn injection_commit_line_is_correct() {
        // Pre-injection states are safe to abandon; committed states are not.
        assert!(!CandidateState::Detected.injection_committed());
        assert!(!CandidateState::WaitingForIdentity.injection_committed());
        assert!(!CandidateState::WaitingForWindow.injection_committed());
        assert!(!CandidateState::Preparing.injection_committed());
        assert!(CandidateState::Injecting.injection_committed());
        assert!(CandidateState::Handshaking.injection_committed());
        assert!(CandidateState::Recording.injection_committed());
    }

    #[test]
    fn pre_attach_exit_is_clean_terminal() {
        assert_eq!(
            CandidateState::WaitingForWindow.on_process_exit(),
            Some(CandidateState::ProcessExitedBeforeAttach)
        );
        // Post-injection exit is NOT a simple pre-attach terminal (handled by
        // worker finalization instead).
        assert_eq!(CandidateState::Recording.on_process_exit(), None);
    }

    #[test]
    fn full_happy_ladder_is_connected() {
        let ladder = [
            CandidateState::Detected,
            CandidateState::WaitingForWindow,
            CandidateState::Preparing,
            CandidateState::Injecting,
            CandidateState::Handshaking,
            CandidateState::WaitingForWorld,
            CandidateState::Recording,
            CandidateState::Finalizing,
            CandidateState::Complete,
        ];
        for pair in ladder.windows(2) {
            assert!(
                pair[0].can_advance_to(pair[1]),
                "no legal edge {} -> {}",
                pair[0],
                pair[1]
            );
        }
    }
}
