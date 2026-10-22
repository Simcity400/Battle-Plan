//! Script event observations after native admission, with stack-independent
//! tap instructions. These sites do not reevaluate fire-once or option gates.

#[derive(Debug, Clone, Copy)]
pub struct Site {
    pub name: &'static str,
    pub rva: usize,
    pub sentinel: [u8; 16],
}

/// Common dispatch has passed world and fire-once gates. The definition is
/// RSI, recipient tag pointer R13, actor tag pointer in the frame's home slot,
/// and the bound scope pointer in its fifth-argument slot.
pub const ACCEPTED: Site = Site {
    name: "ScriptEventAccepted",
    rva: 0x00a0_2b53,
    sentinel: [
        0x4c, 0x8b, 0xbd, 0xb0, 0x01, 0, 0, 0x44, 0x0f, 0xb6, 0xa5, 0xb8, 0x01, 0, 0, 0x45,
    ],
};
pub const DISPATCH_ACTOR_HOME: usize = 0x1a0;
pub const DISPATCH_SCOPE_HOME: usize = 0x1b0;

/// Existing pending instance, or no pending instance for an optionless event.
/// EDI holds the native instance word; the definition and frame remain live.
pub const EXISTING_INSTANCE: Site = Site {
    name: "ScriptEventExistingInstance",
    rva: 0x00a0_2d79,
    sentinel: [
        0x48, 0x8b, 0x9d, 0xa0, 0x01, 0, 0, 0x49, 0x8b, 0xcd, 0xe8, 0x18, 0x29, 0x1a, 0, 0x48,
    ],
};

/// Selected nonnegative option after both native ordinal gates. RBX is the
/// event, RSI its command-owned scope, EDI the admitted option ordinal.
pub const OPTION: Site = Site {
    name: "ScriptEventOption",
    rva: 0x0116_ad92,
    sentinel: [
        0x48, 0x8b, 0x83, 0x68, 0x03, 0, 0, 0x48, 0x8b, 0xd6, 0x48, 0x8b, 0x0c, 0xf8, 0xe8, 0xbb,
    ],
};
/// A negative native option ordinal executes the event's immediate option.
pub const IMMEDIATE_OPTION: Site = Site {
    name: "ScriptEventImmediateOption",
    rva: 0x0116_ad4d,
    sentinel: [
        0x48, 0x8d, 0x8b, 0x18, 0x01, 0, 0, 0x48, 0x8b, 0xd6, 0xe8, 0x04, 0xcb, 0x3b, 0, 0x90,
    ],
};
pub const OPTION_COMMAND_VTABLE_RVA: usize = 0x0292_2860;
pub const OPTION_COMMAND_SCOPE: usize = 0x38;
pub const OPTION_COMMAND_INSTANCE: usize = 0x28;
pub const OPTION_COMMAND_ACTOR: usize = 0x34;

/// Registry insertion's original ABI has six integer/pointer arguments and
/// returns the actual native instance in the low word of RAX.
pub const REGISTER: Site = Site {
    name: "ScriptEventRegistered",
    rva: 0x001c_ab70,
    sentinel: [
        0x48, 0x89, 0x5c, 0x24, 0x10, 0x48, 0x89, 0x6c, 0x24, 0x18, 0x48, 0x89, 0x74, 0x24, 0x20,
        0x57,
    ],
};

pub const REMOVE_PENDING: Site = Site {
    name: "RemovePendingEvent",
    rva: 0x001e_b860,
    sentinel: [
        0x48, 0x89, 0x5c, 0x24, 0x18, 0x57, 0x48, 0x83, 0xec, 0x30, 0x48, 0x63, 0x81, 0x6c, 0x05, 0,
    ],
};
pub const RETIRE_PENDING: Site = Site {
    name: "RetirePendingEvent",
    rva: 0x001e_fd90,
    sentinel: [
        0x48, 0x89, 0x5c, 0x24, 0x08, 0x48, 0x89, 0x6c, 0x24, 0x10, 0x48, 0x89, 0x74, 0x24, 0x18,
        0x57,
    ],
};
pub const INSERT_FIRED: Site = Site {
    name: "InsertFiredEvent",
    rva: 0x001c_9d50,
    sentinel: [
        0x48, 0x89, 0x5c, 0x24, 0x18, 0x48, 0x89, 0x74, 0x24, 0x20, 0x57, 0x48, 0x83, 0xec, 0x20,
        0x48,
    ],
};

pub const REGISTRY_ENTRIES: [Site; 4] = [REGISTER, REMOVE_PENDING, RETIRE_PENDING, INSERT_FIRED];
pub const DELAYED_SCHEDULE: Site = Site {
    name: "ScheduleDelayedEvent",
    rva: 0x006c_3d40,
    sentinel: [
        0x48, 0x89, 0x5c, 0x24, 0x10, 0x48, 0x89, 0x6c, 0x24, 0x18, 0x48, 0x89, 0x74, 0x24, 0x20,
        0x57,
    ],
};
pub const DELAYED_DESTROY: Site = Site {
    name: "DestroyDelayedEvent",
    rva: 0x006c_2450,
    sentinel: [
        0x48, 0x89, 0x5c, 0x24, 0x08, 0x57, 0x48, 0x83, 0xec, 0x20, 0x48, 0x8b, 0xf9, 0x8b, 0xda,
        0x48,
    ],
};
/// Delayed pass entry: anchor once, then observe only sparse cursor boundaries.
pub const DELAYED_PASS: Site = Site {
    name: "DelayedEventPass",
    rva: 0x006e_fe10,
    sentinel: [
        0x48, 0x89, 0x5c, 0x24, 0x10, 0x48, 0x89, 0x6c, 0x24, 0x18, 0x48, 0x89, 0x74, 0x24, 0x20,
        0x57,
    ],
};
/// A timer reached its fire evaluation after decrement. RSI is its native
/// ordinal, RBX the timer and RDI the owner; quiet timers never reach this site.
pub const DELAYED_FIRE_BOUNDARY: Site = Site {
    name: "DelayedEventFireBoundary",
    rva: 0x006e_ff06,
    sentinel: [
        0x4c, 0x8b, 0x7b, 0x08, 0x48, 0x8b, 0xcb, 0xe8, 0xee, 0x24, 0xb3, 0xff, 0x4c, 0x8b, 0xe0,
        0x48,
    ],
};
/// Native expiry/invalid-entry removal, before its swap-removal stores.
pub const DELAYED_RETIRE: Site = Site {
    name: "RetireDelayedEvent",
    rva: 0x006e_ffc2,
    sentinel: [
        0x48, 0x8b, 0x97, 0x90, 0x12, 0, 0, 0x48, 0x63, 0x87, 0x9c, 0x12, 0, 0, 0x48, 0x8b,
    ],
};
pub const DELAYED_TAPS: [Site; 2] = [DELAYED_FIRE_BOUNDARY, DELAYED_RETIRE];
pub const DELAYED_ENTRIES: [Site; 3] = [DELAYED_SCHEDULE, DELAYED_DESTROY, DELAYED_PASS];

/// Replacement unlinked one fired-set node; RSI is the world and RDI the
/// still-live old definition. No-match replacement bypasses this count store.
/// A witness only: the fired-match branch staged for
/// [`decisions::REPLACE_EVENT_RETIREMENT_CALL`].
pub const ERASE_FIRED: Site = Site {
    name: "EraseFiredEvent",
    rva: 0x001d_9f7f,
    sentinel: [
        0xff, 0x8e, 0x38, 0x05, 0, 0, 0x48, 0x63, 0x86, 0x6c, 0x05, 0, 0, 0x8b, 0xd3, 0x85,
    ],
};

/// Automatic mission activation after its native append has returned. RBP
/// owns the decision status, RSI the definition and R12D the admitted days.
pub const MISSION_APPENDED: Site = Site {
    name: "MissionAppended",
    rva: 0x0072_ac1b,
    sentinel: [
        0x48, 0x8b, 0xd6, 0x48, 0x8b, 0xcd, 0xe8, 0x6a, 0xe8, 0xfe, 0xff, 0xc6, 0x85, 0xc0, 0x01, 0,
    ],
};
/// The automatic writer found a retained mission and selected reactivation.
/// Its next member stores set the days and native active lifecycle code. A
/// witness only, staged for [`decisions::AUTO_MISSION_ACTIVATION`].
pub const MISSION_REACTIVATED: Site = Site {
    name: "MissionReactivated",
    rva: 0x0072_ac6d,
    sentinel: [
        0x44, 0x89, 0x67, 0x18, 0x45, 0x33, 0xf6, 0x44, 0x89, 0x77, 0x1c, 0xeb, 0xb6, 0x48, 0x8d,
        0x0d,
    ],
};
/// Scripted mission activation handles both insertion and reactivation;
/// returning normally proves its native member stores/append completed.
pub const ACTIVATE_MISSION: Site = Site {
    name: "ActivateMission",
    rva: 0x0071_9210,
    sentinel: [
        0x48, 0x89, 0x5c, 0x24, 0x10, 0x48, 0x89, 0x6c, 0x24, 0x18, 0x56, 0x57, 0x41, 0x54, 0x41,
        0x56,
    ],
};
/// Targeted activation's helper returns after the new instance was appended
/// to the native ordinary or timed targeted container.
pub const ACTIVATE_TARGETED: Site = Site {
    name: "ActivateTargetedDecision",
    rva: 0x0071_9060,
    sentinel: [
        0x48, 0x89, 0x5c, 0x24, 0x10, 0x48, 0x89, 0x6c, 0x24, 0x18, 0x48, 0x89, 0x74, 0x24, 0x20,
        0x57,
    ],
};

pub const SCRIPTED_MISSION_APPENDED: Site = Site {
    name: "ScriptedMissionAppended",
    rva: 0x0071_92c2,
    sentinel: [
        0x49, 0x8b, 0xd6, 0x49, 0x8b, 0xcf, 0xe8, 0xc3, 1, 0, 0, 0x41, 0xc6, 0x87, 0xc0, 1,
    ],
};
/// Scripted reactivation of an existing mission: a witness only, staged for
/// [`ACTIVATE_MISSION`], which publishes the stored days and state.
pub const SCRIPTED_MISSION_REACTIVATED: Site = Site {
    name: "ScriptedMissionReactivated",
    rva: 0x0071_92f0,
    sentinel: [
        0xc7, 0x47, 0x1c, 0, 0, 0, 0, 0xeb, 0xdc, 0xcc, 0xcc, 0xcc, 0xcc, 0xcc, 0xcc, 0xcc,
    ],
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

/// Ordinary expiry after native taken-membership removal, including empty
/// remove effects. RBX is the definition and RDI the owning status.
pub const DECISION_EXPIRED: Site = Site {
    name: "DecisionExpired",
    rva: 0x0072_a98a,
    sentinel: [
        0x48, 0x8b, 0x57, 8, 0x48, 0x8b, 0xcb, 0x48, 0x83, 0xc2, 8, 0xe8, 0xa6, 0x3d, 0xff, 0xff,
    ],
};
/// Timed targeted cooldown has expired; the next native store reenables the
/// selected instance. RBX is the instance and RBP owns the status.
pub const TARGETED_MISSION_REENABLED: Site = Site {
    name: "TargetedMissionReenabled",
    rva: 0x0072_a10a,
    sentinel: [
        0xc7, 0x43, 0x28, 0, 0, 0, 0, 0xe9, 0xcd, 1, 0, 0, 0x48, 0x8b, 3, 0x48,
    ],
};
pub const REMOVE_DECISION: Site = Site {
    name: "RemoveSelectedDecision",
    rva: 0x0072_8750,
    sentinel: [
        0x48, 0x89, 0x5c, 0x24, 8, 0x57, 0x48, 0x83, 0xec, 0x20, 0x48, 0x8b, 0xf9, 0x45, 0x33, 0xc0,
    ],
};
pub const REMOVE_MISSION: Site = Site {
    name: "RemoveSelectedMission",
    rva: 0x0072_8a70,
    sentinel: [
        0x40, 0x55, 0x56, 0x57, 0x41, 0x54, 0x41, 0x57, 0x48, 0x83, 0xec, 0x20, 0x4c, 0x63, 0xb9,
        0xac,
    ],
};
/// Timed mission result is stored before execution. RSI owns the status and
/// RBX the selected timed instance; effects may subsequently reactivate it.
pub const MISSION_COMPLETED: Site = Site {
    name: "MissionCompleted",
    rva: 0x0072_a5d9,
    sentinel: [
        0x48, 0x8b, 0x6e, 8, 0x48, 0x8b, 0x4b, 0x10, 0x48, 0x8b, 0xd5, 0xe8, 0xc7, 8, 0xff, 0xff,
    ],
};
pub const MISSION_TIMED_OUT: Site = Site {
    name: "MissionTimedOut",
    rva: 0x0072_a6a8,
    sentinel: [
        0x48, 0x8b, 0x17, 0x48, 0x8b, 0x4b, 0x10, 0xe8, 0xdc, 0x1d, 0xff, 0xff, 0x83, 0x7b, 0x1c, 1,
    ],
};
/// Ordinary targeted cooldown has finished. R12D is the native zero result;
/// RSI owns the status and RBX the selected instance. The complete store and
/// its terminal jump are one admitted relocation.
pub const TARGETED_DECISION_REENABLED: Site = Site {
    name: "TargetedDecisionReenabled",
    rva: 0x0072_9f36,
    sentinel: [
        0x44, 0x89, 0x63, 0x28, 0xe9, 0x97, 0, 0, 0, 0x48, 0x8b, 0x56, 8, 0x48, 0x8b, 0xcb,
    ],
};
/// Ordinary cooldown membership was removed. R8 is the selected, still-live
/// cooldown and RBP its status owner, immediately before native destruction.
pub const DECISION_COOLDOWN_FINISHED: Site = Site {
    name: "DecisionCooldownFinished",
    rva: 0x0072_3181,
    sentinel: [
        0x49, 0x8b, 0x00, 0xba, 1, 0, 0, 0, 0x49, 0x8b, 0xc8, 0xff, 0x10, 0x44, 0x8b, 0x45,
    ],
};
/// Taken membership is gone, before optional effects; RDI definition, RSI status.
pub const TAKEN_DECISION_REMOVED: Site = Site {
    name: "TakenDecisionRemoved",
    rva: 0x0072b4b5,
    sentinel: [
        0x48, 0x8b, 0x56, 0x08, 0x48, 0x83, 0xc2, 0x08, 0x48, 0x8b, 0xcf, 0xe8, 0x7b, 0x32, 0xff,
        0xff,
    ],
};
/// Refresh removed its selected cooldown; R8 instance and R14 status.
pub const REFRESH_COOLDOWN_FINISHED: Site = Site {
    name: "RefreshCooldownFinished",
    rva: 0x0072a78d,
    sentinel: [
        0x49, 0x8b, 0x00, 0xba, 0x01, 0x00, 0x00, 0x00, 0x49, 0x8b, 0xc8, 0xff, 0x10, 0x45, 0x8b,
        0x46,
    ],
};
/// The reached terminal store follows the post-effect reactivation check. RBX instance, RSI status.
pub const MISSION_REMOVED: Site = Site {
    name: "MissionRemoved",
    rva: 0x0072a42a,
    sentinel: [
        0xc7, 0x43, 0x1c, 0x04, 0x00, 0x00, 0x00, 0xff, 0x10, 0x48, 0x8d, 0x88, 0x40, 0x03, 0x00,
        0x00,
    ],
};
/// Cancellation stores its terminal lifecycle; days were stored immediately before. RBX instance, RSI status.
pub const MISSION_CANCELLED: Site = Site {
    name: "MissionCancelled",
    rva: 0x0072a561,
    sentinel: [
        0xc7, 0x43, 0x1c, 0x04, 0x00, 0x00, 0x00, 0xff, 0x10, 0x48, 0x8d, 0x88, 0x40, 0x03, 0x00,
        0x00,
    ],
};
/// After actual state and duration stores; RBX instance, RSI status.
pub const TARGETED_DECISION_REMOVED: Site = Site {
    name: "TargetedDecisionRemoved",
    rva: 0x00729fba,
    sentinel: [
        0x48, 0x8b, 0xcb, 0x48, 0x8b, 0x03, 0xff, 0x10, 0x48, 0x8d, 0x88, 0x40, 0x03, 0x00, 0x00,
        0xe8,
    ],
};
/// After actual state and duration stores; RBX instance, RBP status.
pub const TARGETED_MISSION_REMOVED: Site = Site {
    name: "TargetedMissionRemoved",
    rva: 0x0072a0af,
    sentinel: [
        0x48, 0x8b, 0xcb, 0x48, 0x8b, 0x03, 0xff, 0x10, 0x48, 0x8d, 0x88, 0x40, 0x03, 0x00, 0x00,
        0xe8,
    ],
};
/// Actual cancellation duration store and terminal jump; state is already stored. RBX instance, RBP status.
pub const TARGETED_MISSION_CANCELLED: Site = Site {
    name: "TargetedMissionCancelled",
    rva: 0x0072a1d9,
    sentinel: [
        0x89, 0x43, 0x30, 0xe9, 0x02, 0x01, 0x00, 0x00, 0x48, 0x8b, 0x43, 0x10, 0x48, 0x8d, 0x75,
        0x08,
    ],
};
/// Actual admitted completion lifecycle in ECX; RBX instance, RBP status. Effects have not run.
pub const TARGETED_MISSION_COMPLETED: Site = Site {
    name: "TargetedMissionCompleted",
    rva: 0x0072a22f,
    sentinel: [
        0x89, 0x4b, 0x28, 0x48, 0x8b, 0x55, 0x08, 0x48, 0x8b, 0x4b, 0x10, 0xe8, 0xb1, 0x18, 0xff,
        0xff,
    ],
};
/// Actual timeout lifecycle in ECX; RBX instance, RBP status. Effects have not run.
pub const TARGETED_MISSION_TIMED_OUT: Site = Site {
    name: "TargetedMissionTimedOut",
    rva: 0x0072a269,
    sentinel: [
        0x89, 0x4b, 0x28, 0x48, 0x8b, 0x16, 0x48, 0x8b, 0x4b, 0x10, 0xe8, 0xb8, 0x1e, 0xff, 0xff,
        0x83,
    ],
};
/// Common boundary after a final duration store or the reactivation bypass.
/// RBX is the instance and RBP the status. State zero proves the bypass and
/// emits no duration result. All incoming branches target this boundary,
/// never the interior of its relocated register-only instruction.
pub const TARGETED_DURATION_CHANGED: Site = Site {
    name: "TargetedDurationChanged",
    rva: 0x0072_a2dd,
    sentinel: [
        0x41, 0xbf, 0x01, 0x00, 0x00, 0x00, 0x49, 0xff, 0xc4, 0x4d, 0x3b, 0xe5, 0x0f, 0x8c, 0x61,
        0xfd,
    ],
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
/// Explicit clear of matching re-enable/removal timers and taken membership; status, definition ABI.
pub const CLEAR_COOLDOWN_DECISION: Site = Site {
    name: "ClearCooldownDecision",
    rva: 0x00728820,
    sentinel: [
        0x40, 0x53, 0x55, 0x56, 0x48, 0x83, 0xec, 0x20, 0x8b, 0x59, 0x64, 0x48, 0x8b, 0xea, 0x83,
        0xeb,
    ],
};
/// Final ordinary mission display-duration helper; selected instance and owner country-tag pointer ABI.
pub const MISSION_DISPLAY_DURATION: Site = Site {
    name: "MissionDisplayDuration",
    rva: 0x007293f0,
    sentinel: [
        0x48, 0x89, 0x5c, 0x24, 0x08, 0x57, 0x48, 0x83, 0xec, 0x20, 0x83, 0x79, 0x1c, 0x01, 0x48,
        0x8b,
    ],
};
pub const LIFECYCLE_ENTRIES: [Site; 4] = [
    REMOVE_DECISION,
    REMOVE_MISSION,
    CLEAR_COOLDOWN_DECISION,
    MISSION_DISPLAY_DURATION,
];

/// Unconditional timeout add follows these five bytes; RDI selected mission, R15 scope, EAX signed days.
pub const EXTEND_MISSION_TIMEOUT: Site = Site {
    name: "ExtendMissionTimeout",
    rva: 0x0034976c,
    sentinel: [
        0x8b, 0xd0, 0x48, 0x8b, 0xcf, 0xe8, 0x9a, 0xfb, 0x3c, 0x00, 0x48, 0x8b, 0x7c, 0x24, 0x68,
        0x48,
    ],
};
/// Unconditional removal-timer add follows these six bytes; RAX selected timer, RSI scope, RBX effect.
pub const EXTEND_DECISION_REMOVAL: Site = Site {
    name: "ExtendDecisionRemoval",
    rva: 0x003497e6,
    sentinel: [
        0x8b, 0x53, 0x78, 0x48, 0x8b, 0xc8, 0xe8, 0x0f, 0xfb, 0x3c, 0x00, 0x48, 0x8b, 0x5c, 0x24,
        0x30,
    ],
};
/// Descriptors only, no longer installed: these taps ran before the scalar
/// add. [`decisions::EXTEND_MISSION_TIMEOUT_CALL`] and
/// [`decisions::EXTEND_DECISION_REMOVAL_CALL`] enclose it instead.
pub const DURATION_TAPS: [Site; 2] = [EXTEND_MISSION_TIMEOUT, EXTEND_DECISION_REMOVAL];

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn duration_sites_end_at_the_unconditional_helper_call_boundary() {
        assert_eq!(EXTEND_MISSION_TIMEOUT.sentinel[5], 0xe8);
        assert_eq!(EXTEND_DECISION_REMOVAL.sentinel[6], 0xe8);
        assert_eq!(
            DURATION_TAPS.map(|site| site.name),
            ["ExtendMissionTimeout", "ExtendDecisionRemoval"]
        );
        assert!(
            LIFECYCLE_ENTRIES
                .iter()
                .any(|site| site.name == "ClearCooldownDecision")
        );
        assert!(
            LIFECYCLE_ENTRIES
                .iter()
                .any(|site| site.name == "MissionDisplayDuration")
        );
    }
    #[test]
    fn installed_taps_use_one_complete_frame_independent_move() {
        for site in [ACCEPTED, EXISTING_INSTANCE, OPTION, IMMEDIATE_OPTION] {
            assert!(matches!(site.sentinel[0], 0x48 | 0x4c));
            assert!(matches!(site.sentinel[1], 0x8b | 0x8d));
            assert_eq!(site.sentinel[2] >> 6, 2);
            assert_ne!(site.sentinel[2] & 7, 4);
        }
    }
}

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
    /// Static family/site label; the observer supplies the failing stage and cause.
    pub fn label(self) -> &'static str {
        let failure = self.failure();
        failure
            .split_once(" hook")
            .map_or(failure, |(label, _)| label)
    }
}
