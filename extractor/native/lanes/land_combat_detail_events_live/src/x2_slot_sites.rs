use production_hook_receipt::{
    admit_engine_owned_slot_site_roster, AdmittedEngineOwnedSlotRoster, EngineOwnedSlotSiteRoster,
    ExpectedEngineOwnedFunctionPointerSlot, OwningStructureIdentity, OwningStructureKind,
    SlotEnumerationCompleteness, SlotEnumerationEvidence, A729_EXE_SHA256,
};

pub const SLOT_ENUMERATION_JOB_ID: &str = "20260729T122806Z-d55-slot-xref-recheck-f88d65f3";
pub const SLOT_ENUMERATION_MANIFEST_SHA256: [u8; 32] =
    hex32("7a742a987297f518e1813add8b58fed26bb745992f148ac8c117e2f26d26a432");

pub const DIVISION_WIDTH_LEAF_TARGET_RVA: u64 = 0x00C6_E270;
pub const DIVISION_WIDTH_LEAF_SLOT_RVA: u64 = 0x0294_3C40;
pub const DIVISION_WRAPPER_VTABLE_RVA: u64 = 0x0294_3AD0;
pub const DIVISION_WRAPPER_VTABLE_ENTRY_COUNT: u32 = 59;
pub const DIVISION_WIDTH_LEAF_VTABLE_ENTRY_INDEX: u32 = 46;
pub const DIVISION_WIDTH_LEAF_UNWIND_RVA: u64 = 0x0363_7D30;

pub const PARTICIPANT_LIFECYCLE_TARGET_RVA: u64 = 0x0129_9850;
pub const PARTICIPANT_LIFECYCLE_PRIMARY_SLOT_RVA: u64 = 0x0299_14C8;
pub const PARTICIPANT_LIFECYCLE_PRIMARY_VTABLE_RVA: u64 = 0x0299_13C0;
pub const PARTICIPANT_LIFECYCLE_SECONDARY_SLOT_RVA: u64 = 0x029A_5920;
pub const PARTICIPANT_LIFECYCLE_SECONDARY_VTABLE_RVA: u64 = 0x029A_5818;
pub const PARTICIPANT_LIFECYCLE_VTABLE_ENTRY_COUNT: u32 = 34;
pub const PARTICIPANT_LIFECYCLE_VTABLE_ENTRY_INDEX: u32 = 33;
pub const PARTICIPANT_LIFECYCLE_UNWIND_RVA: u64 = 0x0367_0E14;

static DIVISION_WIDTH_LEAF_ENUMERATION: SlotEnumerationEvidence = SlotEnumerationEvidence {
    evidence_job_id: SLOT_ENUMERATION_JOB_ID,
    evidence_artifact_sha256: SLOT_ENUMERATION_MANIFEST_SHA256,
    target_function_rva: DIVISION_WIDTH_LEAF_TARGET_RVA,
    slot_rvas: &[DIVISION_WIDTH_LEAF_SLOT_RVA],
    direct_call_bypasses: &[],
    unwind_metadata_rvas: &[DIVISION_WIDTH_LEAF_UNWIND_RVA],
    completeness: SlotEnumerationCompleteness::Complete,
};

static PARTICIPANT_LIFECYCLE_ENUMERATION: SlotEnumerationEvidence = SlotEnumerationEvidence {
    evidence_job_id: SLOT_ENUMERATION_JOB_ID,
    evidence_artifact_sha256: SLOT_ENUMERATION_MANIFEST_SHA256,
    target_function_rva: PARTICIPANT_LIFECYCLE_TARGET_RVA,
    slot_rvas: &[
        PARTICIPANT_LIFECYCLE_PRIMARY_SLOT_RVA,
        PARTICIPANT_LIFECYCLE_SECONDARY_SLOT_RVA,
    ],
    direct_call_bypasses: &[],
    unwind_metadata_rvas: &[PARTICIPANT_LIFECYCLE_UNWIND_RVA],
    completeness: SlotEnumerationCompleteness::Complete,
};

static DIVISION_WIDTH_LEAF_SITES: [ExpectedEngineOwnedFunctionPointerSlot; 1] =
    [ExpectedEngineOwnedFunctionPointerSlot {
        logical_name: "gameplay.d55.division_width_leaf.wrapper_vtable_slot",
        module_sha256: A729_EXE_SHA256,
        slot_rva: DIVISION_WIDTH_LEAF_SLOT_RVA,
        owner: OwningStructureIdentity {
            kind: OwningStructureKind::Vtable,
            base_rva: DIVISION_WRAPPER_VTABLE_RVA,
            entry_count: DIVISION_WRAPPER_VTABLE_ENTRY_COUNT,
            entry_index: DIVISION_WIDTH_LEAF_VTABLE_ENTRY_INDEX,
        },
        expected_original_target_rva: DIVISION_WIDTH_LEAF_TARGET_RVA,
        enumeration: &DIVISION_WIDTH_LEAF_ENUMERATION,
    }];

static PARTICIPANT_LIFECYCLE_SITES: [ExpectedEngineOwnedFunctionPointerSlot; 2] = [
    ExpectedEngineOwnedFunctionPointerSlot {
        logical_name: "gameplay.d55.participant_lifecycle_loss_writer.primary_vtable_slot",
        module_sha256: A729_EXE_SHA256,
        slot_rva: PARTICIPANT_LIFECYCLE_PRIMARY_SLOT_RVA,
        owner: OwningStructureIdentity {
            kind: OwningStructureKind::Vtable,
            base_rva: PARTICIPANT_LIFECYCLE_PRIMARY_VTABLE_RVA,
            entry_count: PARTICIPANT_LIFECYCLE_VTABLE_ENTRY_COUNT,
            entry_index: PARTICIPANT_LIFECYCLE_VTABLE_ENTRY_INDEX,
        },
        expected_original_target_rva: PARTICIPANT_LIFECYCLE_TARGET_RVA,
        enumeration: &PARTICIPANT_LIFECYCLE_ENUMERATION,
    },
    ExpectedEngineOwnedFunctionPointerSlot {
        logical_name: "gameplay.d55.participant_lifecycle_loss_writer.secondary_vtable_slot",
        module_sha256: A729_EXE_SHA256,
        slot_rva: PARTICIPANT_LIFECYCLE_SECONDARY_SLOT_RVA,
        owner: OwningStructureIdentity {
            kind: OwningStructureKind::Vtable,
            base_rva: PARTICIPANT_LIFECYCLE_SECONDARY_VTABLE_RVA,
            entry_count: PARTICIPANT_LIFECYCLE_VTABLE_ENTRY_COUNT,
            entry_index: PARTICIPANT_LIFECYCLE_VTABLE_ENTRY_INDEX,
        },
        expected_original_target_rva: PARTICIPANT_LIFECYCLE_TARGET_RVA,
        enumeration: &PARTICIPANT_LIFECYCLE_ENUMERATION,
    },
];

pub const DIVISION_WIDTH_LEAF_ROSTER: EngineOwnedSlotSiteRoster = EngineOwnedSlotSiteRoster {
    target_logical_name: "gameplay.d55.division_width_leaf",
    target_function_rva: DIVISION_WIDTH_LEAF_TARGET_RVA,
    sites: &DIVISION_WIDTH_LEAF_SITES,
};

pub const PARTICIPANT_LIFECYCLE_ROSTER: EngineOwnedSlotSiteRoster = EngineOwnedSlotSiteRoster {
    target_logical_name: "gameplay.d55.participant_lifecycle_loss_writer",
    target_function_rva: PARTICIPANT_LIFECYCLE_TARGET_RVA,
    sites: &PARTICIPANT_LIFECYCLE_SITES,
};

pub fn admit_division_width_leaf_slots() -> Result<AdmittedEngineOwnedSlotRoster, String> {
    admit_engine_owned_slot_site_roster(DIVISION_WIDTH_LEAF_ROSTER)
        .map_err(|error| format!("D55 division width leaf slot roster refused: {error}"))
}

pub fn admit_participant_lifecycle_slots() -> Result<AdmittedEngineOwnedSlotRoster, String> {
    admit_engine_owned_slot_site_roster(PARTICIPANT_LIFECYCLE_ROSTER)
        .map_err(|error| format!("D55 participant lifecycle slot roster refused: {error}"))
}

const fn hex32(value: &str) -> [u8; 32] {
    let bytes = value.as_bytes();
    assert!(bytes.len() == 64, "digest literal must be 64 hex chars");
    let mut out = [0u8; 32];
    let mut index = 0;
    while index < 32 {
        out[index] = hex_nibble(bytes[index * 2]) << 4 | hex_nibble(bytes[index * 2 + 1]);
        index += 1;
    }
    out
}

const fn hex_nibble(value: u8) -> u8 {
    match value {
        b'0'..=b'9' => value - b'0',
        b'a'..=b'f' => value - b'a' + 10,
        _ => panic!("digest literal must be lowercase hex"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use production_hook_receipt::{
        admit_engine_owned_function_pointer_slot_site, EngineOwnedSlotAdmissionError,
        EngineOwnedSlotRosterAdmissionError, ENGINE_OWNED_SLOT_WIDTH_BYTES,
    };

    #[test]
    fn both_rosters_admit_and_their_owners_derive_their_slots() {
        let leaf = admit_division_width_leaf_slots().expect("leaf roster is complete");
        assert_eq!(leaf.sites().len(), 1);
        assert_eq!(leaf.target_function_rva(), DIVISION_WIDTH_LEAF_TARGET_RVA);

        let lifecycle =
            admit_participant_lifecycle_slots().expect("lifecycle roster covers both slots");
        assert_eq!(lifecycle.sites().len(), 2);
        assert_eq!(
            lifecycle
                .sites()
                .iter()
                .map(|site| site.expected().slot_rva)
                .collect::<Vec<_>>(),
            vec![
                PARTICIPANT_LIFECYCLE_PRIMARY_SLOT_RVA,
                PARTICIPANT_LIFECYCLE_SECONDARY_SLOT_RVA
            ]
        );
        for site in leaf.sites().iter().chain(lifecycle.sites()) {
            let expected = site.expected();
            assert_eq!(
                expected.owner.derived_slot_rva(),
                Some(expected.slot_rva),
                "{} owner must derive its own slot",
                expected.logical_name
            );
            assert_eq!(expected.slot_rva % ENGINE_OWNED_SLOT_WIDTH_BYTES, 0);
            assert_eq!(expected.module_sha256, A729_EXE_SHA256);
        }
    }

    #[test]
    fn declared_addresses_match_the_recovered_a729_evidence() {
        assert_eq!(DIVISION_WIDTH_LEAF_TARGET_RVA, 0xC6_E270);
        assert_eq!(DIVISION_WIDTH_LEAF_SLOT_RVA, 0x294_3C40);
        assert_eq!(PARTICIPANT_LIFECYCLE_TARGET_RVA, 0x129_9850);
        assert_eq!(PARTICIPANT_LIFECYCLE_PRIMARY_SLOT_RVA, 0x299_14C8);
        assert_eq!(PARTICIPANT_LIFECYCLE_SECONDARY_SLOT_RVA, 0x29A_5920);
        assert_eq!(
            0x1_4000_0000u64 + DIVISION_WIDTH_LEAF_SLOT_RVA,
            0x1_4294_3C40
        );
        assert_eq!(
            0x1_4000_0000u64 + PARTICIPANT_LIFECYCLE_PRIMARY_SLOT_RVA,
            0x1_4299_14C8
        );
        assert_eq!(
            0x1_4000_0000u64 + PARTICIPANT_LIFECYCLE_SECONDARY_SLOT_RVA,
            0x1_429A_5920
        );
    }

    #[test]
    fn declared_targets_are_the_minhook_entries_being_replaced() {
        assert_eq!(
            DIVISION_WIDTH_LEAF_TARGET_RVA as usize,
            crate::x2_carriers::DIVISION_WIDTH_LEAF_RVA
        );
        assert_eq!(
            DIVISION_WRAPPER_VTABLE_RVA as usize,
            crate::x2_carriers::DIVISION_WRAPPER_VTABLE_RVA
        );
        let contract = crate::x2_carriers::AUTHENTICATED_LOSS_WRITER
            .expect("D55 has an authenticated loss-writer contract");
        assert_eq!(
            PARTICIPANT_LIFECYCLE_TARGET_RVA as usize,
            contract.participant_lifecycle.writer_rva
        );
    }

    #[test]
    fn covering_only_one_lifecycle_slot_is_refused() {
        static PARTIAL: [ExpectedEngineOwnedFunctionPointerSlot; 1] =
            [PARTICIPANT_LIFECYCLE_SITES[0]];
        assert_eq!(
            admit_engine_owned_slot_site_roster(EngineOwnedSlotSiteRoster {
                sites: &PARTIAL,
                ..PARTICIPANT_LIFECYCLE_ROSTER
            }),
            Err(
                EngineOwnedSlotRosterAdmissionError::SlotCoverageIncomplete {
                    uncovered_slot_rva: PARTICIPANT_LIFECYCLE_SECONDARY_SLOT_RVA
                }
            )
        );
    }

    #[test]
    fn recording_a_direct_call_bypass_disqualifies_the_site() {
        use production_hook_receipt::DirectCallBypass;
        static BYPASSED: SlotEnumerationEvidence = SlotEnumerationEvidence {
            direct_call_bypasses: &[DirectCallBypass {
                caller_function_rva: 0x129_9610,
                call_site_rva: 0x129_96C4,
            }],
            ..DIVISION_WIDTH_LEAF_ENUMERATION
        };
        let mut site = DIVISION_WIDTH_LEAF_SITES[0];
        site.enumeration = &BYPASSED;
        assert!(matches!(
            admit_engine_owned_function_pointer_slot_site(site),
            Err(EngineOwnedSlotAdmissionError::UncoveredDirectCallBypass { .. })
        ));
    }

    #[test]
    fn an_unconverged_census_disqualifies_its_sites() {
        static UNCONVERGED: SlotEnumerationEvidence = SlotEnumerationEvidence {
            completeness: SlotEnumerationCompleteness::Incomplete,
            ..PARTICIPANT_LIFECYCLE_ENUMERATION
        };
        for index in 0..PARTICIPANT_LIFECYCLE_SITES.len() {
            let mut site = PARTICIPANT_LIFECYCLE_SITES[index];
            site.enumeration = &UNCONVERGED;
            assert_eq!(
                admit_engine_owned_function_pointer_slot_site(site),
                Err(EngineOwnedSlotAdmissionError::EnumerationIncomplete)
            );
        }
    }
}
