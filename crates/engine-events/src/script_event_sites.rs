
use engine_sites::Rva;
#[derive(Debug, Clone, Copy)]
pub struct Site {
    pub name: &'static str,
    pub rva: Rva,
}

pub const ACCEPTED: Site = Site {
    name: "ScriptEventAccepted",
    rva: Rva(0x00a5_b163),
};
pub const DISPATCH_ACTOR_HOME: usize = 0x1a0;
pub const DISPATCH_SCOPE_HOME: usize = 0x1b0;

pub const EXISTING_INSTANCE: Site = Site {
    name: "ScriptEventExistingInstance",
    rva: Rva(0x00a5_b389),
};

pub const OPTION: Site = Site {
    name: "ScriptEventOption",
    rva: Rva(0x0120_c6c2),
};
pub const IMMEDIATE_OPTION: Site = Site {
    name: "ScriptEventImmediateOption",
    rva: Rva(0x0120_c67d),
};
pub const OPTION_COMMAND_VTABLE_RVA: Rva = Rva(0x029b_6ce0);
pub const OPTION_COMMAND_SCOPE: usize = 0x38;
pub const OPTION_COMMAND_INSTANCE: usize = 0x28;
pub const OPTION_COMMAND_ACTOR: usize = 0x34;

pub const REGISTER: Site = Site {
    name: "ScriptEventRegistered",
    rva: Rva(0x001e_1650),
};

pub const REMOVE_PENDING: Site = Site {
    name: "RemovePendingEvent",
    rva: Rva(0x0020_1bd0),
};
pub const RETIRE_PENDING: Site = Site {
    name: "RetirePendingEvent",
    rva: Rva(0x0020_62b0),
};
pub const INSERT_FIRED: Site = Site {
    name: "InsertFiredEvent",
    rva: Rva(0x001e_0810),
};

pub const REGISTRY_ENTRIES: [Site; 4] = [REGISTER, REMOVE_PENDING, RETIRE_PENDING, INSERT_FIRED];
pub const DELAYED_SCHEDULE: Site = Site {
    name: "ScheduleDelayedEvent",
    rva: Rva(0x0071_83d0),
};
pub const DELAYED_DESTROY: Site = Site {
    name: "DestroyDelayedEvent",
    rva: Rva(0x0071_6ae0),
};
pub const DELAYED_PASS: Site = Site {
    name: "DelayedEventPass",
    rva: Rva(0x0074_4760),
};
pub const DELAYED_FIRE_BOUNDARY: Site = Site {
    name: "DelayedEventFireBoundary",
    rva: Rva(0x0074_4856),
};
pub const DELAYED_RETIRE: Site = Site {
    name: "RetireDelayedEvent",
    rva: Rva(0x0074_4912),
};
pub const DELAYED_TAPS: [Site; 2] = [DELAYED_FIRE_BOUNDARY, DELAYED_RETIRE];
pub const DELAYED_ENTRIES: [Site; 3] = [DELAYED_SCHEDULE, DELAYED_DESTROY, DELAYED_PASS];

pub const ERASE_FIRED: Site = Site {
    name: "EraseFiredEvent",
    rva: Rva(0x001f_061f),
};

pub const MISSION_APPENDED: Site = Site {
    name: "MissionAppended",
    rva: Rva(0x0077_fa2b),
};
pub const MISSION_REACTIVATED: Site = Site {
    name: "MissionReactivated",
    rva: Rva(0x0077_fa7d),
};
pub const ACTIVATE_MISSION: Site = Site {
    name: "ActivateMission",
    rva: Rva(0x0076_dfd0),
};
pub const ACTIVATE_TARGETED: Site = Site {
    name: "ActivateTargetedDecision",
    rva: Rva(0x0076_de20),
};

pub const SCRIPTED_MISSION_APPENDED: Site = Site {
    name: "ScriptedMissionAppended",
    rva: Rva(0x0076_e082),
};
pub const SCRIPTED_MISSION_REACTIVATED: Site = Site {
    name: "ScriptedMissionReactivated",
    rva: Rva(0x0076_e0b0),
};
pub const TAPS: [Site; 9] = [
    ACCEPTED,
    EXISTING_INSTANCE,
    OPTION,
    IMMEDIATE_OPTION,
    MISSION_APPENDED,
    MISSION_REACTIVATED,
    SCRIPTED_MISSION_APPENDED,
    SCRIPTED_MISSION_REACTIVATED,
    ERASE_FIRED,
];

pub const DECISION_EXPIRED: Site = Site {
    name: "DecisionExpired",
    rva: Rva(0x0077_f79a),
};
pub const TARGETED_MISSION_REENABLED: Site = Site {
    name: "TargetedMissionReenabled",
    rva: Rva(0x0077_ef1a),
};
pub const REMOVE_DECISION: Site = Site {
    name: "RemoveSelectedDecision",
    rva: Rva(0x0077_d520),
};
pub const REMOVE_MISSION: Site = Site {
    name: "RemoveSelectedMission",
    rva: Rva(0x0077_d840),
};
pub const MISSION_COMPLETED: Site = Site {
    name: "MissionCompleted",
    rva: Rva(0x0077_f3e9),
};
pub const MISSION_TIMED_OUT: Site = Site {
    name: "MissionTimedOut",
    rva: Rva(0x0077_f4b8),
};
pub const TARGETED_DECISION_REENABLED: Site = Site {
    name: "TargetedDecisionReenabled",
    rva: Rva(0x0077_ed46),
};
pub const DECISION_COOLDOWN_FINISHED: Site = Site {
    name: "DecisionCooldownFinished",
    rva: Rva(0x0077_7f51),
};
pub const TAKEN_DECISION_REMOVED: Site = Site {
    name: "TakenDecisionRemoved",
    rva: Rva(0x007802c5),
};
pub const REFRESH_COOLDOWN_FINISHED: Site = Site {
    name: "RefreshCooldownFinished",
    rva: Rva(0x0077f59d),
};
pub const MISSION_REMOVED: Site = Site {
    name: "MissionRemoved",
    rva: Rva(0x0077f23a),
};
pub const MISSION_CANCELLED: Site = Site {
    name: "MissionCancelled",
    rva: Rva(0x0077f371),
};
pub const TARGETED_DECISION_REMOVED: Site = Site {
    name: "TargetedDecisionRemoved",
    rva: Rva(0x0077edca),
};
pub const TARGETED_MISSION_REMOVED: Site = Site {
    name: "TargetedMissionRemoved",
    rva: Rva(0x0077eebf),
};
pub const TARGETED_MISSION_CANCELLED: Site = Site {
    name: "TargetedMissionCancelled",
    rva: Rva(0x0077efe9),
};
pub const TARGETED_MISSION_COMPLETED: Site = Site {
    name: "TargetedMissionCompleted",
    rva: Rva(0x0077f03f),
};
pub const TARGETED_MISSION_TIMED_OUT: Site = Site {
    name: "TargetedMissionTimedOut",
    rva: Rva(0x0077f079),
};
pub const TARGETED_DURATION_CHANGED: Site = Site {
    name: "TargetedDurationChanged",
    rva: Rva(0x0077_f0ed),
};
pub const LIFECYCLE_TAPS: [Site; 16] = [
    DECISION_EXPIRED,
    TARGETED_MISSION_REENABLED,
    MISSION_COMPLETED,
    MISSION_TIMED_OUT,
    TARGETED_DECISION_REENABLED,
    DECISION_COOLDOWN_FINISHED,
    TAKEN_DECISION_REMOVED,
    REFRESH_COOLDOWN_FINISHED,
    MISSION_REMOVED,
    MISSION_CANCELLED,
    TARGETED_DECISION_REMOVED,
    TARGETED_MISSION_REMOVED,
    TARGETED_MISSION_CANCELLED,
    TARGETED_MISSION_COMPLETED,
    TARGETED_MISSION_TIMED_OUT,
    TARGETED_DURATION_CHANGED,
];
pub const CLEAR_COOLDOWN_DECISION: Site = Site {
    name: "ClearCooldownDecision",
    rva: Rva(0x0077d5f0),
};
pub const MISSION_DISPLAY_DURATION: Site = Site {
    name: "MissionDisplayDuration",
    rva: Rva(0x0077e200),
};
pub const LIFECYCLE_ENTRIES: [Site; 4] = [
    REMOVE_DECISION,
    REMOVE_MISSION,
    CLEAR_COOLDOWN_DECISION,
    MISSION_DISPLAY_DURATION,
];

pub const EXTEND_MISSION_TIMEOUT: Site = Site {
    name: "ExtendMissionTimeout",
    rva: Rva(0x003623dc),
};
pub const EXTEND_DECISION_REMOVAL: Site = Site {
    name: "ExtendDecisionRemoval",
    rva: Rva(0x00362456),
};
pub const DURATION_TAPS: [Site; 2] = [EXTEND_MISSION_TIMEOUT, EXTEND_DECISION_REMOVAL];

pub mod decisions;

impl Site {
    pub fn failure(self) -> &'static str {
        match self.name {
            "ScriptEventAccepted" => {
                "Event feed / ScriptEventAccepted hook sentinel, construction or publication failed"
            }
            "ScriptEventExistingInstance" => {
                "Event feed / ScriptEventExistingInstance hook sentinel, construction or publication failed"
            }
            "ScriptEventOption" => {
                "Event feed / ScriptEventOption hook sentinel, construction or publication failed"
            }
            "ScriptEventImmediateOption" => {
                "Event feed / ScriptEventImmediateOption hook sentinel, construction or publication failed"
            }
            "ScriptEventRegistered" => {
                "Event feed / ScriptEventRegistered hook sentinel, construction or publication failed"
            }
            "RemovePendingEvent" => {
                "Event feed / RemovePendingEvent hook sentinel, construction or publication failed"
            }
            "RetirePendingEvent" => {
                "Event feed / RetirePendingEvent hook sentinel, construction or publication failed"
            }
            "InsertFiredEvent" => {
                "Event feed / InsertFiredEvent hook sentinel, construction or publication failed"
            }
            "ScheduleDelayedEvent" => {
                "Event feed / ScheduleDelayedEvent hook sentinel, construction or publication failed"
            }
            "DestroyDelayedEvent" => {
                "Event feed / DestroyDelayedEvent hook sentinel, construction or publication failed"
            }
            "DelayedEventPass" => {
                "Event feed / DelayedEventPass hook sentinel, construction or publication failed"
            }
            "DelayedEventFireBoundary" => {
                "Event feed / DelayedEventFireBoundary hook sentinel, construction or publication failed"
            }
            "RetireDelayedEvent" => {
                "Event feed / RetireDelayedEvent hook sentinel, construction or publication failed"
            }
            "EraseFiredEvent" => {
                "Event feed / EraseFiredEvent hook sentinel, construction or publication failed"
            }
            "ScriptEventOptionExecution" => {
                "Event feed / ScriptEventOptionExecution hook sentinel, construction or publication failed"
            }
            "MissionAppended" => {
                "Event feed / MissionAppended hook sentinel, construction or publication failed"
            }
            "MissionReactivated" => {
                "Event feed / MissionReactivated hook sentinel, construction or publication failed"
            }
            "ActivateMission" => {
                "Event feed / ActivateMission hook sentinel, construction or publication failed"
            }
            "ActivateTargetedDecision" => {
                "Event feed / ActivateTargetedDecision hook sentinel, construction or publication failed"
            }
            "ScriptedMissionAppended" => {
                "Event feed / ScriptedMissionAppended hook sentinel, construction or publication failed"
            }
            "ScriptedMissionReactivated" => {
                "Event feed / ScriptedMissionReactivated hook sentinel, construction or publication failed"
            }
            "DecisionExpired" => {
                "Event feed / DecisionExpired hook sentinel, construction or publication failed"
            }
            "TargetedMissionReenabled" => {
                "Event feed / TargetedMissionReenabled hook sentinel, construction or publication failed"
            }
            "RemoveSelectedDecision" => {
                "Event feed / RemoveSelectedDecision hook sentinel, construction or publication failed"
            }
            "RemoveSelectedMission" => {
                "Event feed / RemoveSelectedMission hook sentinel, construction or publication failed"
            }
            "MissionCompleted" => {
                "Event feed / MissionCompleted hook sentinel, construction or publication failed"
            }
            "MissionTimedOut" => {
                "Event feed / MissionTimedOut hook sentinel, construction or publication failed"
            }
            "TargetedDecisionReenabled" => {
                "Event feed / TargetedDecisionReenabled hook sentinel, construction or publication failed"
            }
            "DecisionCooldownFinished" => {
                "Event feed / DecisionCooldownFinished hook sentinel, construction or publication failed"
            }
            "TakenDecisionRemoved" => {
                "Event feed / TakenDecisionRemoved hook sentinel, construction or publication failed"
            }
            "RefreshCooldownFinished" => {
                "Event feed / RefreshCooldownFinished hook sentinel, construction or publication failed"
            }
            "MissionRemoved" => {
                "Event feed / MissionRemoved hook sentinel, construction or publication failed"
            }
            "MissionCancelled" => {
                "Event feed / MissionCancelled hook sentinel, construction or publication failed"
            }
            "TargetedDecisionRemoved" => {
                "Event feed / TargetedDecisionRemoved hook sentinel, construction or publication failed"
            }
            "TargetedMissionRemoved" => {
                "Event feed / TargetedMissionRemoved hook sentinel, construction or publication failed"
            }
            "TargetedMissionCancelled" => {
                "Event feed / TargetedMissionCancelled hook sentinel, construction or publication failed"
            }
            "TargetedMissionCompleted" => {
                "Event feed / TargetedMissionCompleted hook sentinel, construction or publication failed"
            }
            "TargetedMissionTimedOut" => {
                "Event feed / TargetedMissionTimedOut hook sentinel, construction or publication failed"
            }
            "TargetedDurationChanged" => {
                "Event feed / TargetedDurationChanged hook sentinel, construction or publication failed"
            }
            "ClearCooldownDecision" => {
                "Event feed / ClearCooldownDecision hook sentinel, construction or publication failed"
            }
            "MissionDisplayDuration" => {
                "Event feed / MissionDisplayDuration hook sentinel, construction or publication failed"
            }
            "ExtendMissionTimeout" => {
                "Event feed / ExtendMissionTimeout hook sentinel, construction or publication failed"
            }
            "ExtendDecisionRemoval" => {
                "Event feed / ExtendDecisionRemoval hook sentinel, construction or publication failed"
            }
            "DecisionTimedPass" => {
                "Event feed / DecisionTimedPass hook sentinel, construction or publication failed"
            }
            "DecisionTargetedPass" => {
                "Event feed / DecisionTargetedPass hook sentinel, construction or publication failed"
            }
            "DecisionTargetedTimedPass" => {
                "Event feed / DecisionTargetedTimedPass hook sentinel, construction or publication failed"
            }
            "DecisionRemovalPass" => {
                "Event feed / DecisionRemovalPass hook sentinel, construction or publication failed"
            }
            "DecisionTakenPass" => {
                "Event feed / DecisionTakenPass hook sentinel, construction or publication failed"
            }
            "DecisionEraseTimerRange" => {
                "Event feed / DecisionEraseTimerRange hook sentinel, construction or publication failed"
            }
            "DecisionReset" => {
                "Event feed / DecisionReset hook sentinel, construction or publication failed"
            }
            "DecisionRestore" => {
                "Event feed / DecisionRestore hook sentinel, construction or publication failed"
            }
            "DecisionRandomIncrement" => {
                "Event feed / DecisionRandomIncrement hook sentinel, construction or publication failed"
            }
            "DecisionCountryCandidate" => {
                "Event feed / DecisionCountryCandidate hook sentinel, construction or publication failed"
            }
            "DecisionStateCandidates" => {
                "Event feed / DecisionStateCandidates hook sentinel, construction or publication failed"
            }
            "DecisionGlobalStates" => {
                "Event feed / DecisionGlobalStates hook sentinel, construction or publication failed"
            }
            "DecisionOwnedStates" => {
                "Event feed / DecisionOwnedStates hook sentinel, construction or publication failed"
            }
            "DecisionControlledStates" => {
                "Event feed / DecisionControlledStates hook sentinel, construction or publication failed"
            }
            "DecisionRelatedStates" => {
                "Event feed / DecisionRelatedStates hook sentinel, construction or publication failed"
            }
            "DecisionFilterCandidates" => {
                "Event feed / DecisionFilterCandidates hook sentinel, construction or publication failed"
            }
            "DecisionRebuildAllowed" => {
                "Event feed / DecisionRebuildAllowed hook sentinel, construction or publication failed"
            }
            "DecisionRebuildTargets" => {
                "Event feed / DecisionRebuildTargets hook sentinel, construction or publication failed"
            }
            "DecisionIgnoreToken" => {
                "Event feed / DecisionIgnoreToken hook sentinel, construction or publication failed"
            }
            "DecisionUnignoreToken" => {
                "Event feed / DecisionUnignoreToken hook sentinel, construction or publication failed"
            }
            "DecisionAppendTaken" => {
                "Event feed / DecisionAppendTaken hook sentinel, construction or publication failed"
            }
            "DecisionAppendRemoval" => {
                "Event feed / DecisionAppendRemoval hook sentinel, construction or publication failed"
            }
            "DecisionTransferReenable" => {
                "Event feed / DecisionTransferReenable hook sentinel, construction or publication failed"
            }
            "DecisionAppendReenable" => {
                "Event feed / DecisionAppendReenable hook sentinel, construction or publication failed"
            }
            "DecisionExpiryTransfer" => {
                "Event feed / DecisionExpiryTransfer hook sentinel, construction or publication failed"
            }
            "DecisionCleanTargeted" => {
                "Event feed / DecisionCleanTargeted hook sentinel, construction or publication failed"
            }
            "DecisionCleanTargetedTimed" => {
                "Event feed / DecisionCleanTargetedTimed hook sentinel, construction or publication failed"
            }
            "DecisionReactivateTargetedDays" => {
                "Event feed / DecisionReactivateTargetedDays hook sentinel, construction or publication failed"
            }
            "DecisionReactivateTargetedState" => {
                "Event feed / DecisionReactivateTargetedState hook sentinel, construction or publication failed"
            }
            "DecisionConsoleDays" => {
                "Event feed / DecisionConsoleDays hook sentinel, construction or publication failed"
            }
            "DecisionConsoleAllDays" => {
                "Event feed / DecisionConsoleAllDays hook sentinel, construction or publication failed"
            }
            "DecisionIgnoreAllSelected" => {
                "Event feed / DecisionIgnoreAllSelected hook sentinel, construction or publication failed"
            }
            "DecisionRequestTargetedTimed" => {
                "Event feed / DecisionRequestTargetedTimed hook sentinel, construction or publication failed"
            }
            "DecisionRequestTargeted" => {
                "Event feed / DecisionRequestTargeted hook sentinel, construction or publication failed"
            }
            "DecisionHourlyReenableBegin" => {
                "Event feed / DecisionHourlyReenableBegin hook sentinel, construction or publication failed"
            }
            "DecisionRefreshReenableBegin" => {
                "Event feed / DecisionRefreshReenableBegin hook sentinel, construction or publication failed"
            }
            "DecisionTimedCancelCut" => {
                "Event feed / DecisionTimedCancelCut hook sentinel, construction or publication failed"
            }
            "DecisionTimedRemoveCut" => {
                "Event feed / DecisionTimedRemoveCut hook sentinel, construction or publication failed"
            }
            "DecisionTimedRemoveAlternateCut" => {
                "Event feed / DecisionTimedRemoveAlternateCut hook sentinel, construction or publication failed"
            }
            "DecisionTimedActiveCancelCut" => {
                "Event feed / DecisionTimedActiveCancelCut hook sentinel, construction or publication failed"
            }
            "DecisionTargetedCancelCut" => {
                "Event feed / DecisionTargetedCancelCut hook sentinel, construction or publication failed"
            }
            "DecisionTargetedRemoveCut" => {
                "Event feed / DecisionTargetedRemoveCut hook sentinel, construction or publication failed"
            }
            "DecisionTargetedTimedCancelCut" => {
                "Event feed / DecisionTargetedTimedCancelCut hook sentinel, construction or publication failed"
            }
            "DecisionTargetedTimedRemoveCut" => {
                "Event feed / DecisionTargetedTimedRemoveCut hook sentinel, construction or publication failed"
            }
            "DecisionTargetedTimedActiveCancelCut" => {
                "Event feed / DecisionTargetedTimedActiveCancelCut hook sentinel, construction or publication failed"
            }
            "DecisionTimedRetired" => {
                "Event feed / DecisionTimedRetired hook sentinel, construction or publication failed"
            }
            "DecisionExplicitTimedRetired" => {
                "Event feed / DecisionExplicitTimedRetired hook sentinel, construction or publication failed"
            }
            "DecisionRemovalExpiredCut" => {
                "Event feed / DecisionRemovalExpiredCut hook sentinel, construction or publication failed"
            }
            "DecisionRemovalRetired" => {
                "Event feed / DecisionRemovalRetired hook sentinel, construction or publication failed"
            }
            "DecisionTimedSelectionResult" => {
                "Event feed / DecisionTimedSelectionResult hook sentinel, construction or publication failed"
            }
            "DecisionTargetedSelectionResult" => {
                "Event feed / DecisionTargetedSelectionResult hook sentinel, construction or publication failed"
            }
            "DecisionTargetedIgnoreResult" => {
                "Event feed / DecisionTargetedIgnoreResult hook sentinel, construction or publication failed"
            }
            "DecisionHourlyUpdate" => {
                "Event feed / DecisionHourlyUpdate hook sentinel, construction or publication failed"
            }
            "DecisionRefreshUpdate" => {
                "Event feed / DecisionRefreshUpdate hook sentinel, construction or publication failed"
            }
            "DecisionClearCandidates" => {
                "Event feed / DecisionClearCandidates hook sentinel, construction or publication failed"
            }
            "DecisionCancellationEffects" => "Event feed / DecisionCancellationEffects hook failed",
            "DecisionRemovalEffects" => "Event feed / DecisionRemovalEffects hook failed",
            "TargetedDecisionCancellationEffects" => {
                "Event feed / TargetedDecisionCancellationEffects hook failed"
            }
            "TargetedDecisionRemovalEffects" => {
                "Event feed / TargetedDecisionRemovalEffects hook failed"
            }
            "TargetedDecisionExecuted" => "Event feed / TargetedDecisionExecuted hook failed",
            "DecisionTakenTimerRetired" => "Event feed / DecisionTakenTimerRetired hook failed",
            _ => "Event feed / selected decision effect hook failed",
        }
    }
}

impl Site {
    pub fn label(self) -> &'static str {
        let failure = self.failure();
        failure
            .split_once(" hook")
            .map_or(failure, |(label, _)| label)
    }
}
