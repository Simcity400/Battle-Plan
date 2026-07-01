#[path = "scripted_state_compact.rs"]
mod compact;

pub const EXPECTED_SHA_870EBF: &str =
    "870ebf1f615262facca48a3b947775c41f0a5338f3068c40958209f5c41c65ad";
pub const RESERVED_DOMAIN_ID: u16 = compact::DOMAIN_ID;
pub const OCCUPIED_LOWER_DOMAIN_IDS: &[(u16, &str)] = &[
    (44, "supply_logistics_attrition_live"),
    (45, "focus_research_decisions_depth_live"),
    (46, "air_live_orchestrator_reserved"),
    (47, "characters_live_orchestrator_reserved"),
    (48, "construction_live_orchestrator_reserved"),
    (49, "naval_combat_live_orchestrator_reserved"),
    (50, "naval_combat_members_live_orchestrator_reserved"),
];

pub const CGAMESTATE_GLOBAL_RVA: usize = 0x3316E90;
pub const CGAMESTATE_COUNTRY_TABLE: usize = 0x310;
pub const CGAMESTATE_COUNTRY_COUNT: usize = 0x31C;
pub const COUNTRY_VTABLE_RVA: usize = 0x27D0E90;
pub const COUNTRY_TAG_DB_ID: usize = 0x08;
pub const COUNTRY_VARIABLES: usize = 0x218;
pub const COUNTRY_FLAGS: usize = 0x230;

pub const CGAMESTATE_GLOBAL_FLAGS: usize = 0x258;
pub const CGAMESTATE_GLOBAL_VARIABLES: usize = 0x980;

pub const CGAMESTATE_CPROVINCE_ARRAY: usize = 0x2B0;
pub const CGAMESTATE_CPROVINCE_COUNT: usize = 0x2BC;
pub const CPROVINCE_VTABLE_RVA: usize = 0x295AF68;
pub const CPROVINCE_STATE_PTR: usize = 0xC0;
pub const STATE_VTABLE_RVA: usize = 0x29206C0;
pub const STATE_ID: usize = 0x58;
pub const STATE_FLAGS: usize = 0x540;
pub const STATE_VARIABLES: usize = 0x7F8;

pub const FLAG_ROWS: usize = 0x08;
pub const FLAG_PACKED_SLOT_ENTRY_COUNT: usize = 0x10;
pub const FLAG_ROW_STRIDE: usize = 0x30;
pub const FLAG_TOKEN_ID: usize = 0x08;
pub const FLAG_DATE_HOUR: usize = 0x18;
pub const FLAG_VALUE: usize = 0x28;

pub const DELAYED_EVENTS_BLOCKER_KIND: &str = "missing_positive_save_fixture";
pub const DELAYED_EVENTS_REJECTED_PROXY: &str = "saved_event_target";

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RootStatus {
    Validated,
    UnresolvedFailClosed,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct RootContract {
    pub name: &'static str,
    pub scope_kind: u32,
    pub owner_root: &'static str,
    pub offset: usize,
    pub target_rtti: &'static str,
    pub status: RootStatus,
}

pub fn validated_root_contracts() -> Vec<RootContract> {
    vec![
        RootContract {
            name: "country_variables",
            scope_kind: compact::SCOPE_COUNTRY,
            owner_root: "CCountry",
            offset: COUNTRY_VARIABLES,
            target_rtti: ".?AVCVariables@@",
            status: RootStatus::Validated,
        },
        RootContract {
            name: "country_flags",
            scope_kind: compact::SCOPE_COUNTRY,
            owner_root: "CCountry",
            offset: COUNTRY_FLAGS,
            target_rtti: ".?AVCFlagManager@@",
            status: RootStatus::Validated,
        },
        RootContract {
            name: "global_variables",
            scope_kind: compact::SCOPE_GLOBAL,
            owner_root: "CGameState",
            offset: CGAMESTATE_GLOBAL_VARIABLES,
            target_rtti: ".?AVCVariables@@",
            status: RootStatus::Validated,
        },
        RootContract {
            name: "global_flags",
            scope_kind: compact::SCOPE_GLOBAL,
            owner_root: "CGameState",
            offset: CGAMESTATE_GLOBAL_FLAGS,
            target_rtti: ".?AVCFlagManager@@",
            status: RootStatus::Validated,
        },
        RootContract {
            name: "state_variables",
            scope_kind: compact::SCOPE_STATE,
            owner_root: "CState",
            offset: STATE_VARIABLES,
            target_rtti: ".?AVCVariables@@",
            status: RootStatus::Validated,
        },
        RootContract {
            name: "state_flags",
            scope_kind: compact::SCOPE_STATE,
            owner_root: "CState",
            offset: STATE_FLAGS,
            target_rtti: ".?AVCFlagManager@@",
            status: RootStatus::Validated,
        },
    ]
}

pub fn unresolved_root_contracts() -> Vec<RootContract> {
    vec![
        RootContract {
            name: "delayed_events",
            scope_kind: compact::SCOPE_GLOBAL,
            owner_root: "unknown",
            offset: 0,
            target_rtti: "",
            status: RootStatus::UnresolvedFailClosed,
        },
        RootContract {
            name: "scripted_gui_open_state",
            scope_kind: compact::SCOPE_GLOBAL,
            owner_root: "unknown",
            offset: 0,
            target_rtti: "",
            status: RootStatus::UnresolvedFailClosed,
        },
    ]
}

pub fn required_fail_closed_status_flags() -> u32 {
    compact::STATUS_PARTIAL_DATA
        | compact::STATUS_FLAG_NAMES_PARTIAL
        | compact::STATUS_DELAYED_EVENTS_ROOT_UNRESOLVED
        | compact::STATUS_SCRIPTED_GUI_ROOT_UNRESOLVED
}

pub fn domain_id_is_available_for_lane(domain_id: u16) -> bool {
    domain_id == RESERVED_DOMAIN_ID
        && !OCCUPIED_LOWER_DOMAIN_IDS
            .iter()
            .any(|(occupied, _)| *occupied == domain_id)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn packet_names_all_validated_runtime_roots() {
        assert_eq!(RESERVED_DOMAIN_ID, 51);
        let roots = validated_root_contracts();
        assert_eq!(roots.len(), 6);
        assert!(roots.iter().any(|root| root.name == "country_variables"
            && root.offset == COUNTRY_VARIABLES
            && root.scope_kind == compact::SCOPE_COUNTRY));
        assert!(roots.iter().any(|root| root.name == "global_flags"
            && root.offset == CGAMESTATE_GLOBAL_FLAGS
            && root.scope_kind == compact::SCOPE_GLOBAL));
        assert!(roots.iter().any(|root| root.name == "state_variables"
            && root.offset == STATE_VARIABLES
            && root.scope_kind == compact::SCOPE_STATE));
        assert!(roots
            .iter()
            .all(|root| root.status == RootStatus::Validated));
    }

    #[test]
    fn packet_keeps_unresolved_roots_fail_closed() {
        let unresolved = unresolved_root_contracts();
        assert_eq!(unresolved.len(), 2);
        assert!(unresolved
            .iter()
            .any(|root| root.name == "delayed_events"
                && root.status == RootStatus::UnresolvedFailClosed));
        assert!(unresolved
            .iter()
            .any(|root| root.name == "scripted_gui_open_state"
                && root.status == RootStatus::UnresolvedFailClosed));
        let flags = required_fail_closed_status_flags();
        assert_ne!(flags & compact::STATUS_DELAYED_EVENTS_ROOT_UNRESOLVED, 0);
        assert_ne!(flags & compact::STATUS_SCRIPTED_GUI_ROOT_UNRESOLVED, 0);
        assert_ne!(flags & compact::STATUS_FLAG_NAMES_PARTIAL, 0);
    }

    #[test]
    fn packet_rejects_occupied_lower_domain_ids() {
        assert!(domain_id_is_available_for_lane(51));
        for (domain_id, _owner) in OCCUPIED_LOWER_DOMAIN_IDS {
            assert!(!domain_id_is_available_for_lane(*domain_id));
        }
        assert_eq!(
            OCCUPIED_LOWER_DOMAIN_IDS[0],
            (44, "supply_logistics_attrition_live")
        );
        assert_eq!(
            OCCUPIED_LOWER_DOMAIN_IDS[1],
            (45, "focus_research_decisions_depth_live")
        );
        assert_eq!(
            OCCUPIED_LOWER_DOMAIN_IDS[6],
            (50, "naval_combat_members_live_orchestrator_reserved")
        );
    }

    #[test]
    fn packet_keeps_delayed_events_blocked_without_positive_fixture() {
        assert_eq!(DELAYED_EVENTS_BLOCKER_KIND, "missing_positive_save_fixture");
        assert_eq!(DELAYED_EVENTS_REJECTED_PROXY, "saved_event_target");

        let delayed = unresolved_root_contracts()
            .into_iter()
            .find(|root| root.name == "delayed_events")
            .expect("delayed_events unresolved root contract");
        assert_eq!(delayed.owner_root, "unknown");
        assert_eq!(delayed.offset, 0);
        assert_eq!(delayed.status, RootStatus::UnresolvedFailClosed);
        assert_ne!(
            required_fail_closed_status_flags() & compact::STATUS_DELAYED_EVENTS_ROOT_UNRESOLVED,
            0
        );
    }
}
