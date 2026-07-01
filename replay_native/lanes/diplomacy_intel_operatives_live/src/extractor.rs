use crate::compact::{
    decode_payload, encode_payload, AgencyCapacityClusterRow, AgencyRow, CountryDiplomacyRow,
    CryptologyRow, DiplomacyIntelSnapshot, Domain53GateSummary, IntelMapRow, OperationAnchorRow,
    OperativeRow, OpinionRow, RelationFlagRow, StateEventRow, SubjectRow, WarRelationRow,
    DOMAIN_ID_PROPOSED, REL_KIND_GUARANTEE, STATE_EVENT_KIND_OBSERVED,
    STATE_EVENT_SOURCE_AGENCY_CAPACITY, STATE_EVENT_SOURCE_AGENCY_SCALAR,
    STATE_EVENT_SOURCE_COUNTRY_FACTION_GROUP, STATE_EVENT_SOURCE_CRYPTOLOGY,
    STATE_EVENT_SOURCE_OPERATIVE_LIFECYCLE, STATE_EVENT_SOURCE_RELATION_FLAG,
    STATE_EVENT_SOURCE_RELATION_OPINION, STATE_EVENT_SOURCE_SUBJECT_AUTONOMY,
    STATUS_INCOMING_ACTION_QUEUE_UNVALIDATED, STATUS_INTEL_MAP_RESISTED,
    STATUS_OPERATIONS_DETAIL_RESISTED, STATUS_WAR_LIFECYCLE_DETAIL_UNVALIDATED,
    STATUS_WAR_ROOT_UNVALIDATED,
};

pub const EXPECTED_HOI4_SHA256_870EBF: &str =
    "870ebf1f615262facca48a3b947775c41f0a5338f3068c40958209f5c41c65ad";

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
    | SOURCE_PROOF_WAR_LIFECYCLE_DETAIL_870EBF;

pub const CURRENT_VALIDATED_SOURCE_PROOFS: u64 = SOURCE_PROOF_BUILD_SHA_870EBF
    | SOURCE_PROOF_COUNTRY_ROOTS_870EBF
    | SOURCE_PROOF_RELATION_ROOTS_870EBF
    | SOURCE_PROOF_AGENCY_ROOT_870EBF
    | SOURCE_PROOF_CRYPTOLOGY_ROOT_870EBF
    | SOURCE_PROOF_OPERATIVE_ROOT_870EBF
    | SOURCE_PROOF_WAR_ROOT_870EBF;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct CurrentBuildRoot {
    pub name: &'static str,
    pub evidence: &'static str,
}

pub const CURRENT_BUILD_ROOTS_870EBF: &[CurrentBuildRoot] = &[
    CurrentBuildRoot {
        name: "game_state",
        evidence: "u64[module+0x3316E90]",
    },
    CurrentBuildRoot {
        name: "country_database",
        evidence: "u64[module+0x33189C8]",
    },
    CurrentBuildRoot {
        name: "country_table",
        evidence: "game_state+0x310 table, game_state+0x31C count, country vtable module+0x27D0E90",
    },
    CurrentBuildRoot {
        name: "relations",
        evidence: "country+0xF88 -> manager, manager+0x08 -> table, relation+0x300 opinion",
    },
    CurrentBuildRoot {
        name: "subject_relation",
        evidence: "CSubjectRelation module+0x2949F18, overlord +0x10, subject +0x14, autonomy +0x58/+0x60",
    },
    CurrentBuildRoot {
        name: "agency",
        evidence: "country+0xFC0 -> CIntelligenceAgency, vtable module+0x296B338, capacity raw cluster agency+0xE4/+0xF0/+0xF4 with +0xE4==+0xF4 warm invariant",
    },
    CurrentBuildRoot {
        name: "cryptology",
        evidence: "agency+0x120 -> CCryptology module+0x29A6540; records manager+0x28 stride 0x38, state vtable module+0x29A64F0",
    },
    CurrentBuildRoot {
        name: "operative_identity",
        evidence: "COperativeLeader vtable/classdesc module+0x293F790, identity+0x110 lifecycle flag cohorts {47,31,63}, owner identity+0x118, skill/XP stats path",
    },
    CurrentBuildRoot {
        name: "war_relation_rows",
        evidence: "global row pool with tag ids +0x00/+0x04, date vtable +0x08 in {CGregorianDate module+0x2702190, CGameDate module+0x2702388}, casualties +0x40/+0x48 i32, echo +0x68/+0x6C; v13 warm validation 850/850",
    },
];

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct ExtractedDomain53Rows {
    pub elapsed_wall_ms: u64,
    pub changed_rows: Option<u32>,
    pub countries: Vec<CountryDiplomacyRow>,
    pub wars: Vec<WarRelationRow>,
    pub opinions: Vec<OpinionRow>,
    pub relation_flags: Vec<RelationFlagRow>,
    pub subjects: Vec<SubjectRow>,
    pub agencies: Vec<AgencyRow>,
    pub agency_capacity_clusters: Vec<AgencyCapacityClusterRow>,
    pub operatives: Vec<OperativeRow>,
    pub cryptology: Vec<CryptologyRow>,
    pub operation_anchors: Vec<OperationAnchorRow>,
    pub intel_map: Vec<IntelMapRow>,
    pub state_events: Vec<StateEventRow>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Domain53ExtractionReport {
    pub domain_id: u16,
    pub source_proofs: u64,
    pub snapshot: DiplomacyIntelSnapshot,
    pub payload: Vec<u8>,
    pub gate_summary: Domain53GateSummary,
    pub errors: Vec<String>,
}

impl Domain53ExtractionReport {
    pub fn ok(&self) -> bool {
        self.errors.is_empty()
    }
}

pub fn build_domain53_payload(
    rows: ExtractedDomain53Rows,
    source_proofs: u64,
) -> Domain53ExtractionReport {
    let mut snapshot = DiplomacyIntelSnapshot {
        elapsed_wall_ms: rows.elapsed_wall_ms,
        status_flags: 0,
        changed_rows: rows.changed_rows,
        countries: rows.countries,
        wars: rows.wars,
        opinions: rows.opinions,
        relation_flags: rows.relation_flags,
        subjects: rows.subjects,
        agencies: rows.agencies,
        agency_capacity_clusters: rows.agency_capacity_clusters,
        operatives: rows.operatives,
        cryptology: rows.cryptology,
        operation_anchors: rows.operation_anchors,
        intel_map: rows.intel_map,
        state_events: rows.state_events,
    };
    let mut errors = Vec::new();

    let unknown_source_bits = source_proofs & !KNOWN_SOURCE_PROOFS;
    if unknown_source_bits != 0 {
        errors.push(format!(
            "domain{DOMAIN_ID_PROPOSED} source_proofs has unknown bits 0x{unknown_source_bits:X}"
        ));
    }
    if snapshot.row_count() != 0 && !has(source_proofs, SOURCE_PROOF_BUILD_SHA_870EBF) {
        errors.push(format!(
            "domain{DOMAIN_ID_PROPOSED} rows require SOURCE_PROOF_BUILD_SHA_870EBF for {EXPECTED_HOI4_SHA256_870EBF}"
        ));
    }

    require_rows(
        !snapshot.countries.is_empty(),
        source_proofs,
        SOURCE_PROOF_COUNTRY_ROOTS_870EBF,
        "country diplomacy rows require SOURCE_PROOF_COUNTRY_ROOTS_870EBF",
        &mut errors,
    );
    require_rows(
        !snapshot.opinions.is_empty()
            || !snapshot.relation_flags.is_empty()
            || !snapshot.subjects.is_empty(),
        source_proofs,
        SOURCE_PROOF_RELATION_ROOTS_870EBF,
        "opinion/relation/subject rows require SOURCE_PROOF_RELATION_ROOTS_870EBF",
        &mut errors,
    );
    require_rows(
        !snapshot.agencies.is_empty() || !snapshot.agency_capacity_clusters.is_empty(),
        source_proofs,
        SOURCE_PROOF_AGENCY_ROOT_870EBF,
        "agency rows/capacity clusters require SOURCE_PROOF_AGENCY_ROOT_870EBF",
        &mut errors,
    );
    require_rows(
        !snapshot.operatives.is_empty(),
        source_proofs,
        SOURCE_PROOF_OPERATIVE_ROOT_870EBF,
        "operative rows require SOURCE_PROOF_OPERATIVE_ROOT_870EBF",
        &mut errors,
    );
    require_rows(
        !snapshot.cryptology.is_empty(),
        source_proofs,
        SOURCE_PROOF_CRYPTOLOGY_ROOT_870EBF,
        "cryptology rows require SOURCE_PROOF_CRYPTOLOGY_ROOT_870EBF",
        &mut errors,
    );
    require_rows(
        snapshot.state_events.iter().any(|row| {
            matches!(
                row.source_kind,
                STATE_EVENT_SOURCE_RELATION_FLAG
                    | STATE_EVENT_SOURCE_RELATION_OPINION
                    | STATE_EVENT_SOURCE_SUBJECT_AUTONOMY
            )
        }),
        source_proofs,
        SOURCE_PROOF_RELATION_ROOTS_870EBF,
        "compact state events sourced from relation/opinion/subject rows require SOURCE_PROOF_RELATION_ROOTS_870EBF",
        &mut errors,
    );
    require_rows(
        snapshot
            .state_events
            .iter()
            .any(|row| row.source_kind == STATE_EVENT_SOURCE_COUNTRY_FACTION_GROUP),
        source_proofs,
        SOURCE_PROOF_COUNTRY_ROOTS_870EBF,
        "compact state events sourced from country faction rows require SOURCE_PROOF_COUNTRY_ROOTS_870EBF",
        &mut errors,
    );
    require_rows(
        snapshot.state_events.iter().any(|row| {
            matches!(
                row.source_kind,
                STATE_EVENT_SOURCE_AGENCY_SCALAR | STATE_EVENT_SOURCE_AGENCY_CAPACITY
            )
        }),
        source_proofs,
        SOURCE_PROOF_AGENCY_ROOT_870EBF,
        "compact state events sourced from agency rows require SOURCE_PROOF_AGENCY_ROOT_870EBF",
        &mut errors,
    );
    require_rows(
        snapshot
            .state_events
            .iter()
            .any(|row| row.source_kind == STATE_EVENT_SOURCE_OPERATIVE_LIFECYCLE),
        source_proofs,
        SOURCE_PROOF_OPERATIVE_ROOT_870EBF,
        "compact state events sourced from operative lifecycle rows require SOURCE_PROOF_OPERATIVE_ROOT_870EBF",
        &mut errors,
    );
    require_rows(
        snapshot
            .state_events
            .iter()
            .any(|row| row.source_kind == STATE_EVENT_SOURCE_CRYPTOLOGY),
        source_proofs,
        SOURCE_PROOF_CRYPTOLOGY_ROOT_870EBF,
        "compact state events sourced from cryptology rows require SOURCE_PROOF_CRYPTOLOGY_ROOT_870EBF",
        &mut errors,
    );

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
    if has_resisted_operations {
        snapshot.status_flags |= STATUS_OPERATIONS_DETAIL_RESISTED;
    }
    require_rows(
        has_unresisted_operations,
        source_proofs,
        SOURCE_PROOF_OPERATION_DETAIL_870EBF,
        "unresisted operation rows require SOURCE_PROOF_OPERATION_DETAIL_870EBF",
        &mut errors,
    );

    let has_resisted_intel_map = snapshot.intel_map.iter().any(|row| row.resisted);
    let has_unresisted_intel_map = snapshot.intel_map.iter().any(|row| !row.resisted);
    if has_resisted_intel_map {
        snapshot.status_flags |= STATUS_INTEL_MAP_RESISTED;
    }
    require_rows(
        has_unresisted_intel_map,
        source_proofs,
        SOURCE_PROOF_INTEL_MAP_PERCENTAGES_870EBF,
        "unresisted intel-map rows require SOURCE_PROOF_INTEL_MAP_PERCENTAGES_870EBF",
        &mut errors,
    );

    let payload = encode_payload(&snapshot);
    let gate_summary = decode_payload(0, &payload);
    errors.extend(gate_summary.parse_errors.iter().cloned());

    Domain53ExtractionReport {
        domain_id: DOMAIN_ID_PROPOSED,
        source_proofs,
        snapshot,
        payload,
        gate_summary,
        errors,
    }
}

pub fn proof_fixture_rows() -> ExtractedDomain53Rows {
    ExtractedDomain53Rows {
        elapsed_wall_ms: 3_600_000,
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
            target_tag_db_id: 3,
            opinion: -35,
        }],
        relation_flags: vec![RelationFlagRow {
            src_tag_db_id: 1,
            target_tag_db_id: 3,
            flag_kind: REL_KIND_GUARANTEE,
            directed: true,
        }],
        subjects: vec![SubjectRow {
            overlord_tag_db_id: 1,
            subject_tag_db_id: 22,
            autonomy_state_hash: crate::compact::stable_hash32("autonomy_puppet"),
            autonomy_ratio_raw: 40_000,
        }],
        agencies: vec![AgencyRow {
            owner_tag_db_id: 1,
            max_operatives: 3,
            usable_slots: 3,
            operative_count: 3,
            cryptology_target_count: 1,
            defense_raw: 600_000,
            upgrade_progress_raw: 2_400_000,
            name_hash: crate::compact::stable_hash64("Abwehr"),
            icon_hash: crate::compact::stable_hash64("GFX_intelligence_agency_logo_ger"),
        }],
        agency_capacity_clusters: vec![AgencyCapacityClusterRow {
            owner_tag_db_id: 2,
            value_0xe4: 9,
            value_0xf0: 10,
            value_0xf4: 9,
            proof_hash: crate::compact::stable_hash32("agency_capacity_offsets_e4_f0_f4"),
        }],
        operatives: vec![
            OperativeRow {
                owner_tag_db_id: 1,
                op_id: 11_387,
                detected: false,
                captured: false,
                skill: Some(2),
                experience_raw: Some(71_372_000),
                name_hash: crate::compact::stable_hash64("Johann Hein"),
                state_hash: crate::compact::stable_hash32("lifeflag_47"),
                lifecycle_flag: Some(47),
            },
            OperativeRow {
                owner_tag_db_id: 1,
                op_id: 11_388,
                detected: false,
                captured: false,
                skill: Some(1),
                experience_raw: Some(12_400_000),
                name_hash: crate::compact::stable_hash64("Pool Cohort"),
                state_hash: crate::compact::stable_hash32("lifeflag_31"),
                lifecycle_flag: Some(31),
            },
            OperativeRow {
                owner_tag_db_id: 1,
                op_id: 11_389,
                detected: false,
                captured: false,
                skill: Some(1),
                experience_raw: Some(0),
                name_hash: crate::compact::stable_hash64("Lifecycle Cohort"),
                state_hash: crate::compact::stable_hash32("lifeflag_63"),
                lifecycle_flag: Some(63),
            },
        ],
        cryptology: vec![CryptologyRow {
            owner_tag_db_id: 1,
            target_tag_db_id: 3,
            amount_raw: 960_000_000,
            decrypting: true,
        }],
        operation_anchors: vec![OperationAnchorRow {
            owner_tag_db_id: 1,
            operation_id: 231_592,
            target_tag_db_id: Some(3),
            slot_count: 2,
            phase_count: 3,
            detail_resisted: true,
        }],
        intel_map: vec![IntelMapRow {
            owner_tag_db_id: 1,
            target_tag_db_id: 3,
            civilian_raw: 0,
            army_raw: 0,
            navy_raw: 0,
            air_raw: 0,
            resisted: true,
        }],
        state_events: validated_state_event_rows(),
    }
}

fn validated_state_event_rows() -> Vec<StateEventRow> {
    vec![
        StateEventRow {
            owner_tag_db_id: 1,
            target_tag_db_id: Some(3),
            event_kind: STATE_EVENT_KIND_OBSERVED,
            source_kind: STATE_EVENT_SOURCE_RELATION_FLAG,
            object_hash: crate::compact::stable_hash32("relation_flag_guarantee"),
            value_hash: crate::compact::stable_hash32("directed_true"),
            value_raw: REL_KIND_GUARANTEE as i64,
        },
        StateEventRow {
            owner_tag_db_id: 1,
            target_tag_db_id: Some(22),
            event_kind: STATE_EVENT_KIND_OBSERVED,
            source_kind: STATE_EVENT_SOURCE_SUBJECT_AUTONOMY,
            object_hash: crate::compact::stable_hash32("autonomy_puppet"),
            value_hash: crate::compact::stable_hash32("autonomy_ratio_40000"),
            value_raw: 40_000,
        },
        StateEventRow {
            owner_tag_db_id: 1,
            target_tag_db_id: Some(3),
            event_kind: STATE_EVENT_KIND_OBSERVED,
            source_kind: STATE_EVENT_SOURCE_RELATION_OPINION,
            object_hash: crate::compact::stable_hash32("relation_opinion"),
            value_hash: crate::compact::stable_hash32("opinion_neg35"),
            value_raw: -35,
        },
        StateEventRow {
            owner_tag_db_id: 1,
            target_tag_db_id: None,
            event_kind: STATE_EVENT_KIND_OBSERVED,
            source_kind: STATE_EVENT_SOURCE_COUNTRY_FACTION_GROUP,
            object_hash: crate::compact::stable_hash32("country_faction_group"),
            value_hash: crate::compact::stable_hash32("faction_group_42"),
            value_raw: 42,
        },
        StateEventRow {
            owner_tag_db_id: 1,
            target_tag_db_id: None,
            event_kind: STATE_EVENT_KIND_OBSERVED,
            source_kind: STATE_EVENT_SOURCE_AGENCY_SCALAR,
            object_hash: crate::compact::stable_hash32("agency_upgrade_progress_raw"),
            value_hash: crate::compact::stable_hash32("agency_upgrade_progress_2400000"),
            value_raw: 2_400_000,
        },
        StateEventRow {
            owner_tag_db_id: 1,
            target_tag_db_id: None,
            event_kind: STATE_EVENT_KIND_OBSERVED,
            source_kind: STATE_EVENT_SOURCE_OPERATIVE_LIFECYCLE,
            object_hash: crate::compact::stable_hash32("operative_11387_lifecycle"),
            value_hash: crate::compact::stable_hash32("lifeflag_47"),
            value_raw: 47,
        },
        StateEventRow {
            owner_tag_db_id: 2,
            target_tag_db_id: None,
            event_kind: STATE_EVENT_KIND_OBSERVED,
            source_kind: STATE_EVENT_SOURCE_AGENCY_CAPACITY,
            object_hash: crate::compact::stable_hash32("agency_capacity_offsets_e4_f0_f4"),
            value_hash: crate::compact::stable_hash32("agency_capacity_9_10_9"),
            value_raw: 10,
        },
        StateEventRow {
            owner_tag_db_id: 1,
            target_tag_db_id: Some(3),
            event_kind: STATE_EVENT_KIND_OBSERVED,
            source_kind: STATE_EVENT_SOURCE_CRYPTOLOGY,
            object_hash: crate::compact::stable_hash32("cryptology_amount_raw"),
            value_hash: crate::compact::stable_hash32("cryptology_amount_960000000"),
            value_raw: 960_000_000,
        },
    ]
}

fn require_rows(
    present: bool,
    source_proofs: u64,
    required: u64,
    message: &str,
    errors: &mut Vec<String>,
) {
    if present && !has(source_proofs, required) {
        errors.push(message.to_string());
    }
}

fn has(source_proofs: u64, required: u64) -> bool {
    source_proofs & required != 0
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn current_validated_sources_emit_badged_domain53_payload() {
        let report = build_domain53_payload(proof_fixture_rows(), CURRENT_VALIDATED_SOURCE_PROOFS);

        assert!(report.ok(), "{:?}", report.errors);
        assert_eq!(report.domain_id, 53);
        assert_eq!(report.gate_summary.data_rows_total, 20);
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
                .get("operative_lifecycle_rows")
                .copied(),
            Some(3)
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
        assert_eq!(
            report.gate_summary.status_flags_or & STATUS_WAR_ROOT_UNVALIDATED,
            0
        );
        assert_ne!(
            report.gate_summary.status_flags_or & STATUS_INCOMING_ACTION_QUEUE_UNVALIDATED,
            0
        );
        assert_ne!(
            report.gate_summary.status_flags_or & STATUS_WAR_LIFECYCLE_DETAIL_UNVALIDATED,
            0
        );
        assert_ne!(
            report.gate_summary.status_flags_or & STATUS_OPERATIONS_DETAIL_RESISTED,
            0
        );
        assert_ne!(
            report.gate_summary.status_flags_or & STATUS_INTEL_MAP_RESISTED,
            0
        );
        assert_eq!(
            report
                .gate_summary
                .data_units_total
                .get("war_relation_rows")
                .copied(),
            Some(0)
        );
    }

    #[test]
    fn rejects_country_war_projection_without_current_war_root() {
        let mut rows = proof_fixture_rows();
        rows.countries[0].at_war_count = 1;

        let report = build_domain53_payload(
            rows,
            CURRENT_VALIDATED_SOURCE_PROOFS & !SOURCE_PROOF_WAR_ROOT_870EBF,
        );

        assert!(!report.ok());
        assert!(report
            .errors
            .iter()
            .any(|error| error.contains("country at-war/casualty projections")));
    }

    #[test]
    fn rejects_war_rows_without_current_war_root() {
        let mut rows = proof_fixture_rows();
        rows.wars.push(WarRelationRow {
            first_tag_db_id: 1,
            second_tag_db_id: 3,
            first_casualties: 100,
            second_casualties: 200,
        });

        let report = build_domain53_payload(
            rows,
            CURRENT_VALIDATED_SOURCE_PROOFS & !SOURCE_PROOF_WAR_ROOT_870EBF,
        );

        assert!(!report.ok());
        assert!(report
            .errors
            .iter()
            .any(|error| error.contains("war rows require SOURCE_PROOF_WAR_ROOT_870EBF")));
    }

    #[test]
    fn badges_war_lifecycle_detail_until_source_proof_exists() {
        let mut rows = proof_fixture_rows();
        rows.wars.push(WarRelationRow {
            first_tag_db_id: 1,
            second_tag_db_id: 3,
            first_casualties: 100,
            second_casualties: 200,
        });

        let report = build_domain53_payload(rows, CURRENT_VALIDATED_SOURCE_PROOFS);

        assert!(report.ok(), "{:?}", report.errors);
        assert_ne!(
            report.gate_summary.status_flags_or & STATUS_WAR_LIFECYCLE_DETAIL_UNVALIDATED,
            0
        );
        assert_eq!(
            report.gate_summary.status_flags_or & STATUS_WAR_ROOT_UNVALIDATED,
            0
        );
    }

    #[test]
    fn rejects_unresisted_operation_without_operation_detail_root() {
        let mut rows = proof_fixture_rows();
        rows.operation_anchors[0].detail_resisted = false;

        let report = build_domain53_payload(rows, CURRENT_VALIDATED_SOURCE_PROOFS);

        assert!(!report.ok());
        assert!(report
            .errors
            .iter()
            .any(|error| error.contains("unresisted operation rows require")));
    }

    #[test]
    fn rejects_unresisted_intel_map_without_percentage_root() {
        let mut rows = proof_fixture_rows();
        rows.intel_map[0].resisted = false;
        rows.intel_map[0].civilian_raw = 10_000;

        let report = build_domain53_payload(rows, CURRENT_VALIDATED_SOURCE_PROOFS);

        assert!(!report.ok());
        assert!(report
            .errors
            .iter()
            .any(|error| error.contains("unresisted intel-map rows require")));
    }

    #[test]
    fn rejects_state_events_without_matching_source_proof() {
        let mut rows = proof_fixture_rows();
        rows.operatives.clear();
        rows.state_events
            .retain(|row| row.source_kind == STATE_EVENT_SOURCE_OPERATIVE_LIFECYCLE);

        let report = build_domain53_payload(
            rows,
            CURRENT_VALIDATED_SOURCE_PROOFS & !SOURCE_PROOF_OPERATIVE_ROOT_870EBF,
        );

        assert!(!report.ok());
        assert!(report
            .errors
            .iter()
            .any(|error| error.contains("compact state events sourced from operative lifecycle")));
    }

    #[test]
    fn rejects_faction_group_state_event_without_country_source_proof() {
        let mut rows = proof_fixture_rows();
        rows.countries.clear();
        rows.state_events
            .retain(|row| row.source_kind == STATE_EVENT_SOURCE_COUNTRY_FACTION_GROUP);

        let report = build_domain53_payload(
            rows,
            CURRENT_VALIDATED_SOURCE_PROOFS & !SOURCE_PROOF_COUNTRY_ROOTS_870EBF,
        );

        assert!(!report.ok());
        assert!(report
            .errors
            .iter()
            .any(|error| error.contains("compact state events sourced from country faction")));
    }

    #[test]
    fn root_manifest_pins_current_870ebf_subset() {
        let evidence = CURRENT_BUILD_ROOTS_870EBF
            .iter()
            .map(|root| root.evidence)
            .collect::<Vec<_>>()
            .join("\n");

        assert!(evidence.contains("module+0x3316E90"));
        assert!(evidence.contains("module+0x296B338"));
        assert!(evidence.contains("module+0x29A6540"));
        assert!(evidence.contains("module+0x293F790"));
        assert!(evidence.contains("module+0x2702388"));
        assert_ne!(
            CURRENT_VALIDATED_SOURCE_PROOFS & SOURCE_PROOF_WAR_ROOT_870EBF,
            0
        );
    }

    #[test]
    fn intentional_omissions_do_not_set_partial_or_source_failure_bits() {
        let report = build_domain53_payload(proof_fixture_rows(), CURRENT_VALIDATED_SOURCE_PROOFS);

        assert!(report.ok(), "{:?}", report.errors);
        assert_eq!(
            report.gate_summary.status_flags_or
                & (crate::compact::STATUS_SOURCE_UNAVAILABLE | crate::compact::STATUS_PARTIAL_DATA),
            0
        );
        assert_ne!(
            report.gate_summary.status_flags_or
                & (STATUS_INCOMING_ACTION_QUEUE_UNVALIDATED
                    | STATUS_OPERATIONS_DETAIL_RESISTED
                    | STATUS_INTEL_MAP_RESISTED
                    | STATUS_WAR_LIFECYCLE_DETAIL_UNVALIDATED),
            0
        );
        assert_eq!(
            report.gate_summary.status_flags_or & STATUS_WAR_ROOT_UNVALIDATED,
            0
        );
    }
}
