mod domain44_compact;

use domain44_compact::{
    decode_payload, domain_id_rejection_reason, encode_payload, DiplomacyIntelSnapshot,
    Domain53GateSummary, DOMAIN_ID_PROPOSED, STATE_EVENT_SOURCE_AGENCY_CAPACITY,
    STATE_EVENT_SOURCE_AGENCY_SCALAR, STATE_EVENT_SOURCE_COUNTRY_FACTION_GROUP,
    STATE_EVENT_SOURCE_CRYPTOLOGY, STATE_EVENT_SOURCE_OPERATIVE_LIFECYCLE,
    STATE_EVENT_SOURCE_RELATION_FLAG, STATE_EVENT_SOURCE_RELATION_OPINION,
    STATE_EVENT_SOURCE_SUBJECT_AUTONOMY, STATUS_FACTION_GROUP_UNVALIDATED,
    STATUS_INCOMING_ACTION_QUEUE_UNVALIDATED, STATUS_INTEL_MAP_RESISTED,
    STATUS_OPERATIONS_DETAIL_RESISTED, STATUS_WAR_LIFECYCLE_DETAIL_UNVALIDATED,
    STATUS_WAR_ROOT_UNVALIDATED,
};

pub const EXPECTED_HOI4_SHA256_870EBF: &str =
    "870ebf1f615262facca48a3b947775c41f0a5338f3068c40958209f5c41c65ad";
pub const SLOT_READY_DOMAIN_ID: u16 = DOMAIN_ID_PROPOSED;

pub const SOURCE_PROOF_BUILD_SHA_870EBF: u64 = 1 << 0;
pub const SOURCE_PROOF_COUNTRY_ROOTS_870EBF: u64 = 1 << 1;
pub const SOURCE_PROOF_RELATION_ROOTS_870EBF: u64 = 1 << 2;
pub const SOURCE_PROOF_AGENCY_ROOT_870EBF: u64 = 1 << 3;
pub const SOURCE_PROOF_CRYPTOLOGY_ROOT_870EBF: u64 = 1 << 4;
pub const SOURCE_PROOF_OPERATIVE_ROOT_870EBF: u64 = 1 << 5;
pub const SOURCE_PROOF_WAR_ROOT_870EBF: u64 = 1 << 6;
pub const SOURCE_PROOF_OPERATION_DETAIL_870EBF: u64 = 1 << 7;
pub const SOURCE_PROOF_INTEL_MAP_PERCENTAGES_870EBF: u64 = 1 << 8;
pub const SOURCE_PROOF_INCOMING_ACTION_QUEUE_870EBF: u64 = 1 << 9;
pub const SOURCE_PROOF_WAR_LIFECYCLE_DETAIL_870EBF: u64 = 1 << 10;
pub const SOURCE_PROOF_FACTION_GROUP_870EBF: u64 = 1 << 11;

pub const KNOWN_SOURCE_PROOFS: u64 = SOURCE_PROOF_BUILD_SHA_870EBF
    | SOURCE_PROOF_COUNTRY_ROOTS_870EBF
    | SOURCE_PROOF_RELATION_ROOTS_870EBF
    | SOURCE_PROOF_AGENCY_ROOT_870EBF
    | SOURCE_PROOF_CRYPTOLOGY_ROOT_870EBF
    | SOURCE_PROOF_OPERATIVE_ROOT_870EBF
    | SOURCE_PROOF_WAR_ROOT_870EBF
    | SOURCE_PROOF_OPERATION_DETAIL_870EBF
    | SOURCE_PROOF_INTEL_MAP_PERCENTAGES_870EBF
    | SOURCE_PROOF_INCOMING_ACTION_QUEUE_870EBF
    | SOURCE_PROOF_WAR_LIFECYCLE_DETAIL_870EBF
    | SOURCE_PROOF_FACTION_GROUP_870EBF;

pub const CURRENT_VALIDATED_SOURCE_PROOFS: u64 = SOURCE_PROOF_BUILD_SHA_870EBF
    | SOURCE_PROOF_COUNTRY_ROOTS_870EBF
    | SOURCE_PROOF_RELATION_ROOTS_870EBF
    | SOURCE_PROOF_AGENCY_ROOT_870EBF
    | SOURCE_PROOF_CRYPTOLOGY_ROOT_870EBF
    | SOURCE_PROOF_OPERATIVE_ROOT_870EBF
    | SOURCE_PROOF_WAR_ROOT_870EBF
    | SOURCE_PROOF_FACTION_GROUP_870EBF;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SlotAdmissionReport {
    pub ok: bool,
    pub source_proofs: u64,
    pub emitted_status_flags: u32,
    pub payload: Vec<u8>,
    pub gate_summary: Domain53GateSummary,
    pub errors: Vec<String>,
}

pub fn admit_slot_payload(
    mut snapshot: DiplomacyIntelSnapshot,
    source_proofs: u64,
) -> SlotAdmissionReport {
    let mut errors = Vec::new();
    if let Err(error) = admit_slot_domain_id(SLOT_READY_DOMAIN_ID) {
        errors.push(error);
    }
    let unknown_source_bits = source_proofs & !KNOWN_SOURCE_PROOFS;
    if unknown_source_bits != 0 {
        errors.push(format!(
            "source_proofs has unknown bits 0x{unknown_source_bits:X}"
        ));
    }

    if snapshot.row_count() != 0 && !has(source_proofs, SOURCE_PROOF_BUILD_SHA_870EBF) {
        errors.push(format!(
            "domain{SLOT_READY_DOMAIN_ID} rows require SOURCE_PROOF_BUILD_SHA_870EBF for {EXPECTED_HOI4_SHA256_870EBF}"
        ));
    }

    if !snapshot.countries.is_empty() {
        require(
            source_proofs,
            SOURCE_PROOF_COUNTRY_ROOTS_870EBF,
            "country diplomacy rows require SOURCE_PROOF_COUNTRY_ROOTS_870EBF",
            &mut errors,
        );
        require(
            source_proofs,
            SOURCE_PROOF_FACTION_GROUP_870EBF,
            "country diplomacy rows require SOURCE_PROOF_FACTION_GROUP_870EBF",
            &mut errors,
        );
    }
    if !snapshot.opinions.is_empty()
        || !snapshot.relation_flags.is_empty()
        || !snapshot.subjects.is_empty()
    {
        require(
            source_proofs,
            SOURCE_PROOF_RELATION_ROOTS_870EBF,
            "opinion/relation/subject rows require SOURCE_PROOF_RELATION_ROOTS_870EBF",
            &mut errors,
        );
    }
    if !snapshot.agencies.is_empty() || !snapshot.agency_capacity_clusters.is_empty() {
        require(
            source_proofs,
            SOURCE_PROOF_AGENCY_ROOT_870EBF,
            "agency rows/capacity clusters require SOURCE_PROOF_AGENCY_ROOT_870EBF",
            &mut errors,
        );
    }
    if !snapshot.cryptology.is_empty() {
        require(
            source_proofs,
            SOURCE_PROOF_CRYPTOLOGY_ROOT_870EBF,
            "cryptology rows require SOURCE_PROOF_CRYPTOLOGY_ROOT_870EBF",
            &mut errors,
        );
    }
    if !snapshot.operatives.is_empty() {
        require(
            source_proofs,
            SOURCE_PROOF_OPERATIVE_ROOT_870EBF,
            "operative rows require SOURCE_PROOF_OPERATIVE_ROOT_870EBF",
            &mut errors,
        );
    }
    if snapshot.state_events.iter().any(|row| {
        matches!(
            row.source_kind,
            STATE_EVENT_SOURCE_RELATION_FLAG
                | STATE_EVENT_SOURCE_RELATION_OPINION
                | STATE_EVENT_SOURCE_SUBJECT_AUTONOMY
        )
    }) {
        require(
            source_proofs,
            SOURCE_PROOF_RELATION_ROOTS_870EBF,
            "compact state events sourced from relation/opinion/subject rows require SOURCE_PROOF_RELATION_ROOTS_870EBF",
            &mut errors,
        );
    }
    if snapshot
        .state_events
        .iter()
        .any(|row| row.source_kind == STATE_EVENT_SOURCE_COUNTRY_FACTION_GROUP)
    {
        require(
            source_proofs,
            SOURCE_PROOF_COUNTRY_ROOTS_870EBF,
            "compact state events sourced from country faction rows require SOURCE_PROOF_COUNTRY_ROOTS_870EBF",
            &mut errors,
        );
        require(
            source_proofs,
            SOURCE_PROOF_FACTION_GROUP_870EBF,
            "compact state events sourced from country faction rows require SOURCE_PROOF_FACTION_GROUP_870EBF",
            &mut errors,
        );
    }
    if snapshot.state_events.iter().any(|row| {
        matches!(
            row.source_kind,
            STATE_EVENT_SOURCE_AGENCY_SCALAR | STATE_EVENT_SOURCE_AGENCY_CAPACITY
        )
    }) {
        require(
            source_proofs,
            SOURCE_PROOF_AGENCY_ROOT_870EBF,
            "compact state events sourced from agency rows require SOURCE_PROOF_AGENCY_ROOT_870EBF",
            &mut errors,
        );
    }
    if snapshot
        .state_events
        .iter()
        .any(|row| row.source_kind == STATE_EVENT_SOURCE_OPERATIVE_LIFECYCLE)
    {
        require(
            source_proofs,
            SOURCE_PROOF_OPERATIVE_ROOT_870EBF,
            "compact state events sourced from operative lifecycle rows require SOURCE_PROOF_OPERATIVE_ROOT_870EBF",
            &mut errors,
        );
    }
    if snapshot
        .state_events
        .iter()
        .any(|row| row.source_kind == STATE_EVENT_SOURCE_CRYPTOLOGY)
    {
        require(
            source_proofs,
            SOURCE_PROOF_CRYPTOLOGY_ROOT_870EBF,
            "compact state events sourced from cryptology rows require SOURCE_PROOF_CRYPTOLOGY_ROOT_870EBF",
            &mut errors,
        );
    }

    let has_country_war_projection = snapshot
        .countries
        .iter()
        .any(|row| row.at_war_count != 0 || row.casualties != 0);
    if has(source_proofs, SOURCE_PROOF_WAR_ROOT_870EBF) {
        snapshot.status_flags &= !STATUS_WAR_ROOT_UNVALIDATED;
    } else {
        snapshot.status_flags |= STATUS_WAR_ROOT_UNVALIDATED;
        if !snapshot.wars.is_empty() {
            errors.push("war rows require SOURCE_PROOF_WAR_ROOT_870EBF".to_string());
        }
        if has_country_war_projection {
            errors.push(
                "country at-war/casualty projections require SOURCE_PROOF_WAR_ROOT_870EBF"
                    .to_string(),
            );
        }
    }
    if has(source_proofs, SOURCE_PROOF_WAR_LIFECYCLE_DETAIL_870EBF) {
        snapshot.status_flags &= !STATUS_WAR_LIFECYCLE_DETAIL_UNVALIDATED;
    } else {
        snapshot.status_flags |= STATUS_WAR_LIFECYCLE_DETAIL_UNVALIDATED;
    }

    let has_faction_rows = !snapshot.countries.is_empty()
        || snapshot
            .state_events
            .iter()
            .any(|row| row.source_kind == STATE_EVENT_SOURCE_COUNTRY_FACTION_GROUP);
    if has(source_proofs, SOURCE_PROOF_FACTION_GROUP_870EBF) {
        snapshot.status_flags &= !STATUS_FACTION_GROUP_UNVALIDATED;
    } else {
        snapshot.status_flags |= STATUS_FACTION_GROUP_UNVALIDATED;
        if has_faction_rows {
            errors
                .push("country faction rows require SOURCE_PROOF_FACTION_GROUP_870EBF".to_string());
        }
    }

    if has(source_proofs, SOURCE_PROOF_INCOMING_ACTION_QUEUE_870EBF) {
        snapshot.status_flags &= !STATUS_INCOMING_ACTION_QUEUE_UNVALIDATED;
    } else {
        snapshot.status_flags |= STATUS_INCOMING_ACTION_QUEUE_UNVALIDATED;
    }

    let has_resisted_operations = snapshot
        .operation_anchors
        .iter()
        .any(|row| row.detail_resisted);
    let has_unresisted_operations = snapshot
        .operation_anchors
        .iter()
        .any(|row| !row.detail_resisted);
    if has_resisted_operations || !has(source_proofs, SOURCE_PROOF_OPERATION_DETAIL_870EBF) {
        snapshot.status_flags |= STATUS_OPERATIONS_DETAIL_RESISTED;
    }
    if has_unresisted_operations {
        require(
            source_proofs,
            SOURCE_PROOF_OPERATION_DETAIL_870EBF,
            "unresisted operation rows require SOURCE_PROOF_OPERATION_DETAIL_870EBF",
            &mut errors,
        );
    }

    let has_resisted_intel_map = snapshot.intel_map.iter().any(|row| row.resisted);
    let has_unresisted_intel_map = snapshot.intel_map.iter().any(|row| !row.resisted);
    if has_resisted_intel_map || !has(source_proofs, SOURCE_PROOF_INTEL_MAP_PERCENTAGES_870EBF) {
        snapshot.status_flags |= STATUS_INTEL_MAP_RESISTED;
    }
    if has_unresisted_intel_map {
        require(
            source_proofs,
            SOURCE_PROOF_INTEL_MAP_PERCENTAGES_870EBF,
            "unresisted intel-map rows require SOURCE_PROOF_INTEL_MAP_PERCENTAGES_870EBF",
            &mut errors,
        );
    }

    let emitted_status_flags = snapshot.status_flags;
    let payload = encode_payload(&snapshot);
    let gate_summary = decode_payload(0, &payload);
    errors.extend(gate_summary.parse_errors.iter().cloned());
    let ok = errors.is_empty();

    SlotAdmissionReport {
        ok,
        source_proofs,
        emitted_status_flags,
        payload,
        gate_summary,
        errors,
    }
}

pub fn admit_slot_domain_id(domain_id: u16) -> Result<(), String> {
    domain_id_rejection_reason(domain_id).map_or(Ok(()), Err)
}

fn require(source_proofs: u64, flag: u64, message: &str, errors: &mut Vec<String>) {
    if !has(source_proofs, flag) {
        errors.push(message.to_string());
    }
}

fn has(source_proofs: u64, flag: u64) -> bool {
    source_proofs & flag != 0
}

#[cfg(test)]
mod tests {
    use super::*;
    use domain44_compact::{
        stable_hash32, AgencyCapacityClusterRow, AgencyRow, CountryDiplomacyRow, CryptologyRow,
        DiplomacyIntelSnapshot, IntelMapRow, OperationAnchorRow, OperativeRow, OpinionRow,
        RelationFlagRow, StateEventRow, SubjectRow, WarRelationRow, REL_KIND_GUARANTEE,
        RESERVED_OCCUPIED_DOMAIN_IDS, STATE_EVENT_KIND_OBSERVED,
    };

    #[test]
    fn slot_ready_accepts_reserved_domain_53() {
        assert_eq!(SLOT_READY_DOMAIN_ID, 53);
        assert!(admit_slot_domain_id(SLOT_READY_DOMAIN_ID).is_ok());
    }

    #[test]
    fn slot_ready_rejects_occupied_lower_domain_ids() {
        for (domain_id, owner) in RESERVED_OCCUPIED_DOMAIN_IDS {
            let error = admit_slot_domain_id(*domain_id)
                .expect_err("occupied lower domain id should be rejected");
            assert!(
                error.contains(&domain_id.to_string()) && error.contains(owner),
                "{error}"
            );
        }
    }

    #[test]
    fn current_validated_roots_admit_badged_gap_payload() {
        let report = admit_slot_payload(gap_badged_snapshot(), CURRENT_VALIDATED_SOURCE_PROOFS);

        assert!(report.ok, "{:?}", report.errors);
        assert_eq!(report.emitted_status_flags & STATUS_WAR_ROOT_UNVALIDATED, 0);
        assert!(report.emitted_status_flags & STATUS_WAR_LIFECYCLE_DETAIL_UNVALIDATED != 0);
        assert!(report.emitted_status_flags & STATUS_INCOMING_ACTION_QUEUE_UNVALIDATED != 0);
        assert!(report.emitted_status_flags & STATUS_OPERATIONS_DETAIL_RESISTED != 0);
        assert!(report.emitted_status_flags & STATUS_INTEL_MAP_RESISTED != 0);
        assert_eq!(
            report.emitted_status_flags & STATUS_FACTION_GROUP_UNVALIDATED,
            0
        );
        assert_eq!(report.gate_summary.parse_errors.len(), 0);
        assert_eq!(
            report
                .gate_summary
                .data_units_total
                .get("agency_capacity_cluster_rows")
                .copied(),
            Some(1)
        );
        assert_eq!(
            report
                .gate_summary
                .data_units_total
                .get("war_relation_rows")
                .copied(),
            Some(0)
        );
        assert_eq!(
            report
                .gate_summary
                .data_units_total
                .get("compact_state_event_rows")
                .copied(),
            Some(8)
        );
        assert_eq!(
            report
                .gate_summary
                .data_units_total
                .get("compact_state_event_relation_opinion_rows")
                .copied(),
            Some(1)
        );
        assert_eq!(
            report
                .gate_summary
                .data_units_total
                .get("compact_state_event_country_faction_group_rows")
                .copied(),
            Some(1)
        );
    }

    #[test]
    fn unresolved_operation_and_intel_gaps_are_flagged_when_rows_are_absent() {
        let mut snapshot = gap_badged_snapshot();
        snapshot.operation_anchors.clear();
        snapshot.intel_map.clear();

        let report = admit_slot_payload(snapshot, CURRENT_VALIDATED_SOURCE_PROOFS);

        assert!(report.ok, "{:?}", report.errors);
        assert!(report.emitted_status_flags & STATUS_OPERATIONS_DETAIL_RESISTED != 0);
        assert!(report.emitted_status_flags & STATUS_INTEL_MAP_RESISTED != 0);
        assert_eq!(
            report
                .gate_summary
                .data_units_total
                .get("operation_anchor_rows")
                .copied(),
            Some(0)
        );
        assert_eq!(
            report
                .gate_summary
                .data_units_total
                .get("intel_map_rows")
                .copied(),
            Some(0)
        );
    }

    #[test]
    fn rejects_war_rows_without_current_war_source_proof() {
        let mut snapshot = gap_badged_snapshot();
        snapshot.wars.push(WarRelationRow {
            first_tag_db_id: 1,
            second_tag_db_id: 2,
            first_casualties: 0,
            second_casualties: 0,
        });

        let report = admit_slot_payload(
            snapshot,
            CURRENT_VALIDATED_SOURCE_PROOFS & !SOURCE_PROOF_WAR_ROOT_870EBF,
        );

        assert!(!report.ok);
        assert!(report
            .errors
            .iter()
            .any(|error| error.contains("war rows require SOURCE_PROOF_WAR_ROOT_870EBF")));
    }

    #[test]
    fn rejects_country_war_projection_without_current_war_source_proof() {
        let mut snapshot = gap_badged_snapshot();
        snapshot.countries[0].at_war_count = 1;

        let report = admit_slot_payload(
            snapshot,
            CURRENT_VALIDATED_SOURCE_PROOFS & !SOURCE_PROOF_WAR_ROOT_870EBF,
        );

        assert!(!report.ok);
        assert!(report.errors.iter().any(|error| error
            .contains("country at-war/casualty projections require SOURCE_PROOF_WAR_ROOT_870EBF")));
    }

    #[test]
    fn current_war_source_proof_allows_war_rows() {
        let mut snapshot = gap_badged_snapshot();
        snapshot.wars.push(WarRelationRow {
            first_tag_db_id: 1,
            second_tag_db_id: 2,
            first_casualties: 0,
            second_casualties: 0,
        });
        snapshot.countries[0].at_war_count = 1;

        let report = admit_slot_payload(
            snapshot,
            CURRENT_VALIDATED_SOURCE_PROOFS | SOURCE_PROOF_WAR_ROOT_870EBF,
        );

        assert!(report.ok, "{:?}", report.errors);
        assert_eq!(report.emitted_status_flags & STATUS_WAR_ROOT_UNVALIDATED, 0);
        assert!(report.emitted_status_flags & STATUS_WAR_LIFECYCLE_DETAIL_UNVALIDATED != 0);
    }

    #[test]
    fn war_lifecycle_detail_remains_badged_with_current_root_only() {
        let mut snapshot = gap_badged_snapshot();
        snapshot.wars.push(WarRelationRow {
            first_tag_db_id: 1,
            second_tag_db_id: 2,
            first_casualties: 0,
            second_casualties: 0,
        });
        snapshot.countries[0].at_war_count = 1;

        let report = admit_slot_payload(snapshot, CURRENT_VALIDATED_SOURCE_PROOFS);

        assert!(report.ok, "{:?}", report.errors);
        assert_eq!(report.emitted_status_flags & STATUS_WAR_ROOT_UNVALIDATED, 0);
        assert!(report.emitted_status_flags & STATUS_WAR_LIFECYCLE_DETAIL_UNVALIDATED != 0);
    }

    #[test]
    fn rejects_unresisted_operation_without_operation_detail_source_proof() {
        let mut snapshot = gap_badged_snapshot();
        snapshot.operation_anchors[0].detail_resisted = false;

        let report = admit_slot_payload(snapshot, CURRENT_VALIDATED_SOURCE_PROOFS);

        assert!(!report.ok);
        assert!(report.errors.iter().any(|error| error
            .contains("unresisted operation rows require SOURCE_PROOF_OPERATION_DETAIL_870EBF")));
    }

    #[test]
    fn rejects_unresisted_intel_map_without_percentage_source_proof() {
        let mut snapshot = gap_badged_snapshot();
        snapshot.intel_map[0].resisted = false;
        snapshot.intel_map[0].civilian_raw = 1;

        let report = admit_slot_payload(snapshot, CURRENT_VALIDATED_SOURCE_PROOFS);

        assert!(!report.ok);
        assert!(report.errors.iter().any(|error| error.contains(
            "unresisted intel-map rows require SOURCE_PROOF_INTEL_MAP_PERCENTAGES_870EBF"
        )));
    }

    #[test]
    fn rejects_unknown_source_proof_bits() {
        let report = admit_slot_payload(
            gap_badged_snapshot(),
            CURRENT_VALIDATED_SOURCE_PROOFS | (1 << 63),
        );

        assert!(!report.ok);
        assert!(report
            .errors
            .iter()
            .any(|error| error.contains("source_proofs has unknown bits")));
    }

    #[test]
    fn rejects_state_events_without_matching_source_proof() {
        let mut snapshot = gap_badged_snapshot();
        snapshot.operatives.clear();
        snapshot
            .state_events
            .retain(|row| row.source_kind == STATE_EVENT_SOURCE_OPERATIVE_LIFECYCLE);

        let report = admit_slot_payload(
            snapshot,
            CURRENT_VALIDATED_SOURCE_PROOFS & !SOURCE_PROOF_OPERATIVE_ROOT_870EBF,
        );

        assert!(!report.ok);
        assert!(report
            .errors
            .iter()
            .any(|error| error.contains("compact state events sourced from operative lifecycle")));
    }

    #[test]
    fn rejects_country_faction_rows_without_faction_group_source_proof() {
        let report = admit_slot_payload(
            gap_badged_snapshot(),
            CURRENT_VALIDATED_SOURCE_PROOFS & !SOURCE_PROOF_FACTION_GROUP_870EBF,
        );

        assert!(!report.ok);
        assert!(report
            .errors
            .iter()
            .any(|error| error.contains("SOURCE_PROOF_FACTION_GROUP_870EBF")));
        assert!(report.emitted_status_flags & STATUS_FACTION_GROUP_UNVALIDATED != 0);
    }

    #[test]
    fn rejects_faction_group_state_event_without_country_source_proof() {
        let mut snapshot = gap_badged_snapshot();
        snapshot.countries.clear();
        snapshot
            .state_events
            .retain(|row| row.source_kind == STATE_EVENT_SOURCE_COUNTRY_FACTION_GROUP);

        let report = admit_slot_payload(
            snapshot,
            CURRENT_VALIDATED_SOURCE_PROOFS & !SOURCE_PROOF_COUNTRY_ROOTS_870EBF,
        );

        assert!(!report.ok);
        assert!(report
            .errors
            .iter()
            .any(|error| error.contains("compact state events sourced from country faction")));
    }

    fn gap_badged_snapshot() -> DiplomacyIntelSnapshot {
        DiplomacyIntelSnapshot {
            elapsed_wall_ms: 1_000,
            status_flags: 0,
            changed_rows: None,
            countries: vec![CountryDiplomacyRow {
                tag_db_id: 1,
                faction_group: 42,
                at_war_count: 0,
                casualties: 0,
            }],
            wars: Vec::new(),
            opinions: vec![OpinionRow {
                src_tag_db_id: 1,
                target_tag_db_id: 2,
                opinion: 25,
            }],
            relation_flags: vec![RelationFlagRow {
                src_tag_db_id: 1,
                target_tag_db_id: 2,
                flag_kind: REL_KIND_GUARANTEE,
                directed: true,
            }],
            subjects: vec![SubjectRow {
                overlord_tag_db_id: 1,
                subject_tag_db_id: 3,
                autonomy_state_hash: 0x1234,
                autonomy_ratio_raw: 50_000,
            }],
            agencies: vec![AgencyRow {
                owner_tag_db_id: 1,
                max_operatives: 4,
                usable_slots: 3,
                operative_count: 2,
                cryptology_target_count: 1,
                defense_raw: 12,
                upgrade_progress_raw: 75_000,
                name_hash: 0xAAAA,
                icon_hash: 0xBBBB,
            }],
            agency_capacity_clusters: vec![AgencyCapacityClusterRow {
                owner_tag_db_id: 2,
                value_0xe4: 9,
                value_0xf0: 10,
                value_0xf4: 9,
                proof_hash: stable_hash32("agency_capacity_offsets_e4_f0_f4"),
            }],
            operatives: vec![OperativeRow {
                owner_tag_db_id: 1,
                op_id: 10,
                detected: false,
                captured: false,
                skill: Some(2),
                experience_raw: Some(40_000),
                name_hash: 0xCCCC,
                state_hash: 0xDDDD,
                lifecycle_flag: Some(47),
            }],
            cryptology: vec![CryptologyRow {
                owner_tag_db_id: 1,
                target_tag_db_id: 2,
                amount_raw: 10_000,
                decrypting: true,
            }],
            operation_anchors: vec![OperationAnchorRow {
                owner_tag_db_id: 1,
                operation_id: 99,
                target_tag_db_id: Some(2),
                slot_count: 1,
                phase_count: 0,
                detail_resisted: true,
            }],
            intel_map: vec![IntelMapRow {
                owner_tag_db_id: 1,
                target_tag_db_id: 2,
                civilian_raw: 0,
                army_raw: 0,
                navy_raw: 0,
                air_raw: 0,
                resisted: true,
            }],
            state_events: vec![
                StateEventRow {
                    owner_tag_db_id: 1,
                    target_tag_db_id: Some(2),
                    event_kind: STATE_EVENT_KIND_OBSERVED,
                    source_kind: STATE_EVENT_SOURCE_RELATION_FLAG,
                    object_hash: stable_hash32("relation_flag_guarantee"),
                    value_hash: stable_hash32("directed_true"),
                    value_raw: REL_KIND_GUARANTEE as i64,
                },
                StateEventRow {
                    owner_tag_db_id: 1,
                    target_tag_db_id: Some(3),
                    event_kind: STATE_EVENT_KIND_OBSERVED,
                    source_kind: STATE_EVENT_SOURCE_SUBJECT_AUTONOMY,
                    object_hash: stable_hash32("autonomy_puppet"),
                    value_hash: stable_hash32("autonomy_ratio_50000"),
                    value_raw: 50_000,
                },
                StateEventRow {
                    owner_tag_db_id: 1,
                    target_tag_db_id: Some(2),
                    event_kind: STATE_EVENT_KIND_OBSERVED,
                    source_kind: STATE_EVENT_SOURCE_RELATION_OPINION,
                    object_hash: stable_hash32("relation_opinion"),
                    value_hash: stable_hash32("opinion_25"),
                    value_raw: 25,
                },
                StateEventRow {
                    owner_tag_db_id: 1,
                    target_tag_db_id: None,
                    event_kind: STATE_EVENT_KIND_OBSERVED,
                    source_kind: STATE_EVENT_SOURCE_COUNTRY_FACTION_GROUP,
                    object_hash: stable_hash32("country_faction_group"),
                    value_hash: stable_hash32("faction_group_42"),
                    value_raw: 42,
                },
                StateEventRow {
                    owner_tag_db_id: 1,
                    target_tag_db_id: None,
                    event_kind: STATE_EVENT_KIND_OBSERVED,
                    source_kind: STATE_EVENT_SOURCE_AGENCY_SCALAR,
                    object_hash: stable_hash32("agency_upgrade_progress_raw"),
                    value_hash: stable_hash32("agency_upgrade_progress_75000"),
                    value_raw: 75_000,
                },
                StateEventRow {
                    owner_tag_db_id: 1,
                    target_tag_db_id: None,
                    event_kind: STATE_EVENT_KIND_OBSERVED,
                    source_kind: STATE_EVENT_SOURCE_OPERATIVE_LIFECYCLE,
                    object_hash: stable_hash32("operative_10_lifecycle"),
                    value_hash: stable_hash32("lifeflag_47"),
                    value_raw: 47,
                },
                StateEventRow {
                    owner_tag_db_id: 2,
                    target_tag_db_id: None,
                    event_kind: STATE_EVENT_KIND_OBSERVED,
                    source_kind: STATE_EVENT_SOURCE_AGENCY_CAPACITY,
                    object_hash: stable_hash32("agency_capacity_offsets_e4_f0_f4"),
                    value_hash: stable_hash32("agency_capacity_9_10_9"),
                    value_raw: 10,
                },
                StateEventRow {
                    owner_tag_db_id: 1,
                    target_tag_db_id: Some(2),
                    event_kind: STATE_EVENT_KIND_OBSERVED,
                    source_kind: STATE_EVENT_SOURCE_CRYPTOLOGY,
                    object_hash: stable_hash32("cryptology_amount_raw"),
                    value_hash: stable_hash32("cryptology_amount_10000"),
                    value_raw: 10_000,
                },
            ],
        }
    }
}
