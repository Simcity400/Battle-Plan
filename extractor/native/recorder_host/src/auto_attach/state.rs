use std::fmt;

#[derive(Clone, Copy, PartialEq, Eq, Debug, Hash)]
pub enum CandidateState {
    Detected,
    WaitingForIdentity,
    WaitingForWindow,
    Preparing,
    Injecting,
    Handshaking,
    WaitingForWorld,
    Recording,
    Finalizing,
    Complete,
    Failed,
    ProcessExitedBeforeAttach,
}

impl CandidateState {
    pub fn is_terminal(self) -> bool {
        matches!(
            self,
            CandidateState::Complete
                | CandidateState::Failed
                | CandidateState::ProcessExitedBeforeAttach
        )
    }

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

    pub fn legal_next(self) -> &'static [CandidateState] {
        use CandidateState::*;
        match self {
            Detected => &[WaitingForIdentity, WaitingForWindow],
            WaitingForIdentity => &[WaitingForIdentity, WaitingForWindow, Preparing],
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

    pub fn can_advance_to(self, next: CandidateState) -> bool {
        self.legal_next().contains(&next)
    }

    pub fn on_process_exit(self) -> Option<CandidateState> {
        if self.is_terminal() {
            None
        } else if self.injection_committed() {
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

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum ReasonCode {
    Detected,
    IdentityUnresolved,
    UnsupportedGameBuild,
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
    CollectorUnloadedRearmed,
    ExplicitStop,
}

impl ReasonCode {
    pub fn as_str(self) -> &'static str {
        match self {
            ReasonCode::Detected => "detected",
            ReasonCode::IdentityUnresolved => "identity_unresolved",
            ReasonCode::UnsupportedGameBuild => {
                crate::game_build::UnsupportedGameBuild::TERMINAL_OUTCOME
            }
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
            ReasonCode::CollectorUnloadedRearmed => "collector_unloaded_rearmed",
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
        assert!(!CandidateState::Injecting.can_advance_to(CandidateState::Recording));
        assert!(!CandidateState::Preparing.can_advance_to(CandidateState::WaitingForWorld));
        assert!(!CandidateState::Handshaking.can_advance_to(CandidateState::Recording));
        assert!(CandidateState::WaitingForWorld.can_advance_to(CandidateState::Recording));
    }

    #[test]
    fn headless_identity_may_advance_directly_to_preparing() {
        assert!(CandidateState::WaitingForIdentity.can_advance_to(CandidateState::Preparing));
    }

    #[test]
    fn injection_commit_line_is_correct() {
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
