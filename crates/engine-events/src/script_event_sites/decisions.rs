use super::Site;
use engine_sites::Rva;

pub const fn call_span_len(site: Site) -> usize {
    if site.rva.0 == REACTIVATE_TARGETED_DAYS.rva.0 {
        REACTIVATE_TARGETED_STATE.rva.0 - site.rva.0
    } else if site.rva.0 == REQUEST_TARGETED_TIMED.rva.0 {
        REQUEST_TARGETED.rva.0 - site.rva.0
    } else {
        16
    }
}
pub const TIMED_PASS: Site = Site {
    name: "DecisionTimedPass",
    rva: Rva(0x0077f130),
};
pub const TARGETED_PASS: Site = Site {
    name: "DecisionTargetedPass",
    rva: Rva(0x0077ecd0),
};
pub const TARGETED_TIMED_PASS: Site = Site {
    name: "DecisionTargetedTimedPass",
    rva: Rva(0x0077ee10),
};
pub const REMOVAL_PASS: Site = Site {
    name: "DecisionRemovalPass",
    rva: Rva(0x0077f660),
};
pub const TAKEN_PASS: Site = Site {
    name: "DecisionTakenPass",
    rva: Rva(0x00780140),
};
pub const ERASE_TIMER_RANGE: Site = Site {
    name: "DecisionEraseTimerRange",
    rva: Rva(0x0077d750),
};
pub const RESET: Site = Site {
    name: "DecisionReset",
    rva: Rva(0x0076ef60),
};
pub const RESTORE: Site = Site {
    name: "DecisionRestore",
    rva: Rva(0x0077bbc0),
};
pub const RANDOM_INCREMENT: Site = Site {
    name: "DecisionRandomIncrement",
    rva: Rva(0x0077ffe0),
};
pub const COUNTRY_CANDIDATE: Site = Site {
    name: "DecisionCountryCandidate",
    rva: Rva(0x0076a610),
};
pub const STATE_CANDIDATES: Site = Site {
    name: "DecisionStateCandidates",
    rva: Rva(0x0076f400),
};
pub const GLOBAL_STATES: Site = Site {
    name: "DecisionGlobalStates",
    rva: Rva(0x0076cef0),
};
pub const OWNED_STATES: Site = Site {
    name: "DecisionOwnedStates",
    rva: Rva(0x0076d180),
};
pub const CONTROLLED_STATES: Site = Site {
    name: "DecisionControlledStates",
    rva: Rva(0x0076d310),
};
pub const RELATED_STATES: Site = Site {
    name: "DecisionRelatedStates",
    rva: Rva(0x0076d4a0),
};
pub const FILTER_CANDIDATES: Site = Site {
    name: "DecisionFilterCandidates",
    rva: Rva(0x0077a4e0),
};
pub const IGNORE_TOKEN: Site = Site {
    name: "DecisionIgnoreToken",
    rva: Rva(0x01581a10),
};
pub const UNIGNORE_TOKEN: Site = Site {
    name: "DecisionUnignoreToken",
    rva: Rva(0x01581ea0),
};
pub const APPEND_TAKEN: Site = Site {
    name: "DecisionAppendTaken",
    rva: Rva(0x0077d9b7),
};
pub const APPEND_REMOVAL: Site = Site {
    name: "DecisionAppendRemoval",
    rva: Rva(0x0077da64),
};
pub const TRANSFER_REENABLE: Site = Site {
    name: "DecisionTransferReenable",
    rva: Rva(0x007802f7),
};
pub const APPEND_REENABLE: Site = Site {
    name: "DecisionAppendReenable",
    rva: Rva(0x00780336),
};
pub const EXPIRY_TRANSFER: Site = Site {
    name: "DecisionExpiryTransfer",
    rva: Rva(0x0077f7d3),
};
pub const CLEAN_TARGETED: Site = Site {
    name: "DecisionCleanTargeted",
    rva: Rva(0x007805fe),
};
pub const CLEAN_TARGETED_TIMED: Site = Site {
    name: "DecisionCleanTargetedTimed",
    rva: Rva(0x0078060e),
};
pub const REACTIVATE_TARGETED_DAYS: Site = Site {
    name: "DecisionReactivateTargetedDays",
    rva: Rva(0x003606db),
};
pub const REACTIVATE_TARGETED_STATE: Site = Site {
    name: "DecisionReactivateTargetedState",
    rva: Rva(0x003606e5),
};
pub const CONSOLE_DAYS: Site = Site {
    name: "DecisionConsoleDays",
    rva: Rva(0x00272008),
};
pub const CONSOLE_ALL_DAYS: Site = Site {
    name: "DecisionConsoleAllDays",
    rva: Rva(0x0027208d),
};
pub const IGNORE_ALL_SELECTED: Site = Site {
    name: "DecisionIgnoreAllSelected",
    rva: Rva(0x011e0dcc),
};
pub const REQUEST_TARGETED_TIMED: Site = Site {
    name: "DecisionRequestTargetedTimed",
    rva: Rva(0x00377989),
};
pub const REQUEST_TARGETED: Site = Site {
    name: "DecisionRequestTargeted",
    rva: Rva(0x00377990),
};
pub const HOURLY_REENABLE_BEGIN: Site = Site {
    name: "DecisionHourlyReenableBegin",
    rva: Rva(0x00777f07),
};
pub const REFRESH_REENABLE_BEGIN: Site = Site {
    name: "DecisionRefreshReenableBegin",
    rva: Rva(0x0077f542),
};
pub const TIMED_CANCEL_CUT: Site = Site {
    name: "DecisionTimedCancelCut",
    rva: Rva(0x0077f295),
};
pub const TIMED_REMOVE_CUT: Site = Site {
    name: "DecisionTimedRemoveCut",
    rva: Rva(0x0077f2ca),
};
pub const TIMED_REMOVE_ALTERNATE_CUT: Site = Site {
    name: "DecisionTimedRemoveAlternateCut",
    rva: Rva(0x0077f305),
};
pub const TIMED_ACTIVE_CANCEL_CUT: Site = Site {
    name: "DecisionTimedActiveCancelCut",
    rva: Rva(0x0077f344),
};
pub const TARGETED_CANCEL_CUT: Site = Site {
    name: "DecisionTargetedCancelCut",
    rva: Rva(0x0077ed67),
};
pub const TARGETED_REMOVE_CUT: Site = Site {
    name: "DecisionTargetedRemoveCut",
    rva: Rva(0x0077ed97),
};
pub const TARGETED_TIMED_CANCEL_CUT: Site = Site {
    name: "DecisionTargetedTimedCancelCut",
    rva: Rva(0x0077ef4b),
};
pub const TARGETED_TIMED_REMOVE_CUT: Site = Site {
    name: "DecisionTargetedTimedRemoveCut",
    rva: Rva(0x0077ef75),
};
pub const TARGETED_TIMED_ACTIVE_CANCEL_CUT: Site = Site {
    name: "DecisionTargetedTimedActiveCancelCut",
    rva: Rva(0x0077efb9),
};
pub const TIMED_RETIRED: Site = Site {
    name: "DecisionTimedRetired",
    rva: Rva(0x0077f1bf),
};
pub const EXPLICIT_TIMED_RETIRED: Site = Site {
    name: "DecisionExplicitTimedRetired",
    rva: Rva(0x0077d8ac),
};
pub const REMOVAL_EXPIRED_CUT: Site = Site {
    name: "DecisionRemovalExpiredCut",
    rva: Rva(0x0077f770),
};
pub const REMOVAL_RETIRED: Site = Site {
    name: "DecisionRemovalRetired",
    rva: Rva(0x0077f7fb),
};
pub const TIMED_SELECTION_RESULT: Site = Site {
    name: "DecisionTimedSelectionResult",
    rva: Rva(0x0077de4b),
};
pub const TARGETED_SELECTION_RESULT: Site = Site {
    name: "DecisionTargetedSelectionResult",
    rva: Rva(0x0077dd79),
};
pub const TARGETED_IGNORE_RESULT: Site = Site {
    name: "DecisionTargetedIgnoreResult",
    rva: Rva(0x011e138f),
};
pub const AUTO_MISSION_ACTIVATION: Site = Site {
    name: "AutoMissionActivation",
    rva: Rva(0x0077f850),
};
pub const TIMED_SELECTION: Site = Site {
    name: "TimedDecisionSelection",
    rva: Rva(0x0077ddc0),
};
pub const TARGETED_IGNORE_COMMAND: Site = Site {
    name: "TargetedIgnoreCommand",
    rva: Rva(0x011e1280),
};
pub const IGNORE_ALL_COMMAND: Site = Site {
    name: "IgnoreAllDecisionsCommand",
    rva: Rva(0x011e0d10),
};
pub const TARGETED_REMOVAL_REQUEST: Site = Site {
    name: "TargetedRemovalRequest",
    rva: Rva(0x00377860),
};
pub const CANDIDATE_FINALIZATION: Site = Site {
    name: "DecisionCandidateFinalization",
    rva: Rva(0x00780440),
};
pub const EXTEND_MISSION_TIMEOUT_CALL: Site = Site {
    name: "ExtendMissionTimeoutCall",
    rva: Rva(0x003623e1),
};
pub const EXTEND_DECISION_REMOVAL_CALL: Site = Site {
    name: "ExtendDecisionRemovalCall",
    rva: Rva(0x0036245c),
};
pub const REPLACE_EVENT_RETIREMENT_CALL: Site = Site {
    name: "ReplaceEventRetirementCall",
    rva: Rva(0x00a5a6a7),
};
pub const ENTRIES: [Site; 27] = [
    TIMED_PASS,
    TARGETED_PASS,
    TARGETED_TIMED_PASS,
    REMOVAL_PASS,
    TAKEN_PASS,
    ERASE_TIMER_RANGE,
    RESET,
    RESTORE,
    RANDOM_INCREMENT,
    COUNTRY_CANDIDATE,
    STATE_CANDIDATES,
    GLOBAL_STATES,
    OWNED_STATES,
    CONTROLLED_STATES,
    RELATED_STATES,
    FILTER_CANDIDATES,
    IGNORE_TOKEN,
    UNIGNORE_TOKEN,
    HOURLY_UPDATE,
    REFRESH_UPDATE,
    AUTO_MISSION_ACTIVATION,
    super::ACTIVATE_MISSION,
    TIMED_SELECTION,
    TARGETED_IGNORE_COMMAND,
    IGNORE_ALL_COMMAND,
    TARGETED_REMOVAL_REQUEST,
    CANDIDATE_FINALIZATION,
];
pub const CALLS: [Site; 17] = [
    APPEND_TAKEN,
    APPEND_REMOVAL,
    TRANSFER_REENABLE,
    APPEND_REENABLE,
    EXPIRY_TRANSFER,
    CLEAN_TARGETED,
    CLEAN_TARGETED_TIMED,
    REACTIVATE_TARGETED_DAYS,
    REACTIVATE_TARGETED_STATE,
    CONSOLE_DAYS,
    CONSOLE_ALL_DAYS,
    IGNORE_ALL_SELECTED,
    REQUEST_TARGETED_TIMED,
    REQUEST_TARGETED,
    EXTEND_MISSION_TIMEOUT_CALL,
    EXTEND_DECISION_REMOVAL_CALL,
    REPLACE_EVENT_RETIREMENT_CALL,
];
pub const TAPS: [Site; 20] = [
    HOURLY_REENABLE_BEGIN,
    REFRESH_REENABLE_BEGIN,
    TIMED_CANCEL_CUT,
    TIMED_REMOVE_CUT,
    TIMED_REMOVE_ALTERNATE_CUT,
    TIMED_ACTIVE_CANCEL_CUT,
    TARGETED_CANCEL_CUT,
    TARGETED_REMOVE_CUT,
    TARGETED_TIMED_CANCEL_CUT,
    TARGETED_TIMED_REMOVE_CUT,
    TARGETED_TIMED_ACTIVE_CANCEL_CUT,
    TIMED_RETIRED,
    EXPLICIT_TIMED_RETIRED,
    REMOVAL_EXPIRED_CUT,
    REMOVAL_RETIRED,
    TIMED_SELECTION_RESULT,
    TARGETED_SELECTION_RESULT,
    TARGETED_IGNORE_RESULT,
    CLEAR_CANDIDATES,
    TAKEN_TIMER_RETIRED,
];
pub const HOURLY_UPDATE: Site = Site {
    name: "DecisionHourlyUpdate",
    rva: Rva(0x777e30),
};
pub const REFRESH_UPDATE: Site = Site {
    name: "DecisionRefreshUpdate",
    rva: Rva(0x77f510),
};
pub const CLEAR_CANDIDATES: Site = Site {
    name: "DecisionClearCandidates",
    rva: Rva(0x007807df),
};

pub const TAKEN_TIMER_RETIRED: Site = Site {
    name: "DecisionTakenTimerRetired",
    rva: Rva(0x0078037c),
};
