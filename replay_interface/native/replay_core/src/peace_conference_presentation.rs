//! Strict passive XUI08 peace-conference presentation carrier.
//!
//! Ordinary d97 preserves the small registered-object/lifecycle witness, but
//! it cannot carry the controller-resolved participant, allocation, terminal
//! result, final-text, and ordering graph.  This optional replay-file carrier
//! is the exact producer/consumer seam for that graph.  It is sampled without
//! opening the game window and is independent of recorder POV.

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

pub const XUI08_SCHEMA_VERSION: u32 = 3;
pub const XUI08_AUTHORITY: &str = "exact_a729_passive_global_peace_conference";
pub const XUI08_EXECUTABLE_SHA256: &str =
    "870ebf1f615262facca48a3b947775c41f0a5338f3068c40958209f5c41c65ad";

// Hashes of committed Replay-side contract/evidence manifests.  Recordings
// cannot authorize themselves by supplying arbitrary syntax-shaped hashes.
pub const XUI08_PRODUCER_CONTRACT_MANIFEST_SHA256: &str =
    "2d54f7ee3610af7f8aff97268c0c56e71674e408329daaaa6f6132e523a1f08c";
pub const XUI08_EVIDENCE_MANIFEST_SHA256: &str =
    "c531145bd016c548bf1641f68cc542300fbaa333c2bdec174ccfd380078f7253";

#[derive(Debug, Clone, PartialEq, Eq, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct PeaceObjectReferenceExact {
    pub object_type: u32,
    pub object_id: u32,
}

impl PeaceObjectReferenceExact {
    pub fn valid(&self) -> bool {
        self.object_type != 0 || self.object_id != 0
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct PeaceFinalTextExact {
    pub authored_identity: String,
    pub raw_value: String,
    pub final_text: String,
    pub final_text_sha256: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct PeaceTooltipExact {
    pub primary: PeaceFinalTextExact,
    pub delayed: Option<PeaceFinalTextExact>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum PeacePhaseExact {
    Demands,
    Resolution,
    Completed,
}

impl PeacePhaseExact {
    pub fn token(self) -> &'static str {
        match self {
            Self::Demands => "demands",
            Self::Resolution => "resolution",
            Self::Completed => "completed",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum PeaceParticipantRoleExact {
    Victor,
    Beneficiary,
    Defeated,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum PeaceActionKindExact {
    TakeState,
    Liberate,
    Puppet,
    ForceGovernment,
    TakeNavy,
    DemilitarizeZone,
    ResourceRights,
    WarReparation,
    DismantleIndustry,
}

impl PeaceActionKindExact {
    pub fn token(self) -> &'static str {
        match self {
            Self::TakeState => "take_states",
            Self::Liberate => "liberate",
            Self::Puppet => "puppet",
            Self::ForceGovernment => "force_government",
            Self::TakeNavy => "take_navy",
            Self::DemilitarizeZone => "demilitarize_zone",
            Self::ResourceRights => "resource_rights",
            Self::WarReparation => "war_reparation",
            Self::DismantleIndustry => "dismantle_industry",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct PeaceParticipantPresentationExact {
    pub source_ordinal: u32,
    pub stable_participant_id: String,
    pub country_reference: PeaceObjectReferenceExact,
    pub country_tag: String,
    pub country_tag_db_id: u32,
    /// Exact effective-content texture selected for the small country flag.
    pub flag_texture_path: String,
    pub role: PeaceParticipantRoleExact,
    pub country_display: PeaceFinalTextExact,
    pub score: PeaceFinalTextExact,
    pub score_ratio: PeaceFinalTextExact,
    pub expected_allocation_count: u32,
    pub expected_result_count: u32,
    /// Exact producer-owned history membership and row order.
    pub allocation_ids: Vec<String>,
    pub result_ids: Vec<String>,
    pub bid_count_display: PeaceFinalTextExact,
    pub contested_bid_count_display: PeaceFinalTextExact,
    pub tooltip: PeaceTooltipExact,
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct PeaceAllocationPresentationExact {
    pub source_ordinal: u32,
    /// Refresh-scoped producer identity, never a process pointer.
    pub stable_allocation_id: String,
    pub turn: u32,
    pub action_kind: PeaceActionKindExact,
    pub actor_participant_id: String,
    pub beneficiary_country_tag: Option<String>,
    pub beneficiary_country_tag_db_id: Option<u32>,
    pub target_country_tag: Option<String>,
    pub target_country_tag_db_id: Option<u32>,
    pub expected_state_count: u32,
    pub expected_ship_count: u32,
    pub expected_contender_count: u32,
    pub target_state_ids: Vec<u32>,
    pub target_ship_ids: Vec<String>,
    pub contested_by_participant_ids: Vec<String>,
    pub action_display: PeaceFinalTextExact,
    pub target_display: PeaceFinalTextExact,
    pub cost: PeaceFinalTextExact,
    pub bid: PeaceFinalTextExact,
    pub status: PeaceFinalTextExact,
    pub popup_bid_count_display: PeaceFinalTextExact,
    pub tooltip: PeaceTooltipExact,
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct PeaceAllocationGroupPresentationExact {
    pub source_ordinal: u32,
    pub stable_group_id: String,
    pub expected_allocation_count: u32,
    /// Exact producer-owned membership and row order. Replay never regroups by
    /// country, text identity, action kind, state set, or any other heuristic.
    pub allocation_ids: Vec<String>,
    pub flag_texture_path: Option<String>,
    pub target_display: PeaceFinalTextExact,
    pub available_state_count_display: PeaceFinalTextExact,
    pub wargoal_count_display: PeaceFinalTextExact,
    pub contested_state_count_display: PeaceFinalTextExact,
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct PeaceContestedPopupRowPresentationExact {
    pub source_ordinal: u32,
    pub stable_popup_row_id: String,
    pub participant_id: String,
    pub allocation_id: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct PeaceContestedPopupPresentationExact {
    pub source_ordinal: u32,
    pub stable_popup_id: String,
    pub selected_allocation_id: String,
    pub expected_row_count: u32,
    pub rows: Vec<PeaceContestedPopupRowPresentationExact>,
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct PeaceResultPresentationExact {
    pub source_ordinal: u32,
    pub stable_result_id: String,
    /// Present only when the producer observed the exact engine join.
    pub allocation_id: Option<String>,
    pub action_kind: PeaceActionKindExact,
    pub actor_country_tag: String,
    pub actor_country_tag_db_id: u32,
    pub beneficiary_country_tag: Option<String>,
    pub beneficiary_country_tag_db_id: Option<u32>,
    pub target_country_tag: Option<String>,
    pub target_country_tag_db_id: Option<u32>,
    pub expected_state_count: u32,
    pub expected_ship_count: u32,
    pub target_state_ids: Vec<u32>,
    pub target_ship_ids: Vec<String>,
    pub action_display: PeaceFinalTextExact,
    pub target_display: PeaceFinalTextExact,
    pub outcome: PeaceFinalTextExact,
    pub tooltip: PeaceTooltipExact,
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize, Serialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum PeaceConferencePresentationObservationExact {
    /// The game state holds no peace conference, so there is no panel to
    /// present. The mirror of
    /// [`crate::world::PeaceConferenceObservationExact::NoConferenceInProgress`],
    /// and it pairs only with it.
    ///
    /// This carrier's admission requires a same-frame d97 lifecycle witness. On
    /// the ordinary hour that witness IS the measured-empty kind, so without
    /// this variant an XUI08 record could never be admitted on nearly any hour
    /// of any capture, whatever it said — the pairing match would fall through
    /// to its refusing arm. Gap map GP-F15.
    ///
    /// Empty struct form for the same reason as its d97 mirror: it keeps
    /// `deny_unknown_fields`, which a unit variant in an internally tagged enum
    /// would not.
    NoConferenceInProgress {},
    Inactive {
        refresh_sequence: u64,
    },
    Active {
        refresh_sequence: u64,
        refresh_epoch: u64,
        conference_reference: PeaceObjectReferenceExact,
        phase: PeacePhaseExact,
        turn: u32,
        remaining_score_turns: u32,
        current_participant_id: Option<String>,
        expected_available_state_count: u32,
        expected_contested_state_count: u32,
        expected_participant_count: u32,
        expected_allocation_count: u32,
        expected_allocation_group_count: u32,
        expected_contested_popup_count: u32,
        expected_result_count: u32,
        available_state_ids: Vec<u32>,
        contested_state_ids: Vec<u32>,
        current_participant_display: PeaceFinalTextExact,
        phase_display: PeaceFinalTextExact,
        current_score: PeaceFinalTextExact,
        score_this_turn: PeaceFinalTextExact,
        remaining_turns_display: PeaceFinalTextExact,
        participants: Vec<PeaceParticipantPresentationExact>,
        allocations: Vec<PeaceAllocationPresentationExact>,
        allocation_groups: Vec<PeaceAllocationGroupPresentationExact>,
        contested_popups: Vec<PeaceContestedPopupPresentationExact>,
        results: Vec<PeaceResultPresentationExact>,
    },
}

/// Complete passive global transaction for one canonical replay hour.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct PeaceConferencePresentationExact {
    pub schema_version: u32,
    pub authority: String,
    pub executable_sha256: String,
    pub content_manifest_sha256: String,
    pub producer_manifest_sha256: String,
    pub evidence_manifest_sha256: String,
    pub frame_hour: u64,
    pub capture_sequence: u64,
    pub passive_global: bool,
    pub ui_open_independent: bool,
    pub recorder_pov_independent: bool,
    pub producer_complete: bool,
    pub previous_record_sha256: Option<String>,
    pub record_sha256: String,
    pub observation: PeaceConferencePresentationObservationExact,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PeaceConferencePresentationExactView {
    pub status: crate::world::ViewStatus,
    pub presentation: Option<PeaceConferencePresentationExact>,
}

pub fn text_sha256(text: &str) -> String {
    format!("{:x}", Sha256::digest(text.as_bytes()))
}

pub fn participant_identity(reference: &PeaceObjectReferenceExact) -> String {
    format!(
        "participant:{}:{}",
        reference.object_type, reference.object_id
    )
}

pub fn allocation_identity(
    conference: &PeaceObjectReferenceExact,
    refresh_sequence: u64,
    source_ordinal: u32,
) -> String {
    format!(
        "allocation:{}:{}:{}:{}",
        conference.object_type, conference.object_id, refresh_sequence, source_ordinal
    )
}

pub fn result_identity(
    conference: &PeaceObjectReferenceExact,
    refresh_sequence: u64,
    source_ordinal: u32,
) -> String {
    format!(
        "result:{}:{}:{}:{}",
        conference.object_type, conference.object_id, refresh_sequence, source_ordinal
    )
}

pub fn allocation_group_identity(
    conference: &PeaceObjectReferenceExact,
    refresh_sequence: u64,
    source_ordinal: u32,
) -> String {
    format!(
        "allocation-group:{}:{}:{}:{}",
        conference.object_type, conference.object_id, refresh_sequence, source_ordinal
    )
}

pub fn contested_popup_identity(selected_allocation_id: &str) -> String {
    format!("contested-popup:{selected_allocation_id}")
}

pub fn contested_popup_row_identity(selected_allocation_id: &str, source_ordinal: u32) -> String {
    format!("contested-popup-row:{selected_allocation_id}:{source_ordinal}")
}

pub fn canonical_record_sha256(value: &PeaceConferencePresentationExact) -> Option<String> {
    let mut canonical = value.clone();
    canonical.record_sha256.clear();
    serde_json::to_vec(&canonical)
        .ok()
        .map(|bytes| format!("{:x}", Sha256::digest(bytes)))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn committed_manifests_are_compile_time_bound_to_admission_constants() {
        let producer =
            include_bytes!("../../../re_evidence/rendering_laws/XUI08/producer_contract.json");
        let evidence =
            include_bytes!("../../../re_evidence/rendering_laws/XUI08/evidence_manifest.json");
        assert_eq!(
            format!("{:x}", Sha256::digest(producer)),
            XUI08_PRODUCER_CONTRACT_MANIFEST_SHA256
        );
        assert_eq!(
            format!("{:x}", Sha256::digest(evidence)),
            XUI08_EVIDENCE_MANIFEST_SHA256
        );

        let producer: serde_json::Value =
            serde_json::from_slice(producer).expect("committed XUI08 producer manifest JSON");
        let evidence: serde_json::Value =
            serde_json::from_slice(evidence).expect("committed XUI08 evidence manifest JSON");
        assert_eq!(producer["carrier_schema"], XUI08_SCHEMA_VERSION);
        assert_eq!(producer["authority"], XUI08_AUTHORITY);
        assert_eq!(producer["executable_sha256"], XUI08_EXECUTABLE_SHA256);
        assert_eq!(evidence["packet"], "XUI08");
        assert_eq!(evidence["executable"]["sha256"], XUI08_EXECUTABLE_SHA256);
        assert_eq!(
            evidence["jobs"].as_array().map(Vec::len),
            Some(9),
            "every audited Ghidra job must remain individually hash-bound"
        );
        let function_artifacts = evidence["function_artifacts"]
            .as_array()
            .expect("address-specific XUI08 function artifact bindings");
        assert_eq!(function_artifacts.len(), 10);
        let expected_addresses = [
            "0x14184eb30",
            "0x141851210",
            "0x141854540",
            "0x141854ea0",
            "0x141851d70",
            "0x141e387a0",
            "0x140b6ff40",
            "0x140e2e250",
            "0x140e3c2b0",
            "0x140e23170",
        ];
        assert_eq!(
            function_artifacts
                .iter()
                .map(|entry| entry["address"].as_str().unwrap())
                .collect::<Vec<_>>(),
            expected_addresses
        );
        assert!(function_artifacts.iter().all(|entry| {
            ["decompile", "instructions"].iter().all(|kind| {
                entry[*kind]["function_artifact_sha256"]
                    .as_str()
                    .is_some_and(|sha| sha.len() == 64)
                    && entry[*kind]["bytes"]
                        .as_u64()
                        .is_some_and(|bytes| bytes > 0)
            })
        }));
    }

    fn inactive() -> PeaceConferencePresentationExact {
        PeaceConferencePresentationExact {
            schema_version: XUI08_SCHEMA_VERSION,
            authority: XUI08_AUTHORITY.into(),
            executable_sha256: XUI08_EXECUTABLE_SHA256.into(),
            content_manifest_sha256: "1".repeat(64),
            producer_manifest_sha256: XUI08_PRODUCER_CONTRACT_MANIFEST_SHA256.into(),
            evidence_manifest_sha256: XUI08_EVIDENCE_MANIFEST_SHA256.into(),
            frame_hour: 9,
            capture_sequence: 4,
            passive_global: true,
            ui_open_independent: true,
            recorder_pov_independent: true,
            producer_complete: true,
            previous_record_sha256: Some("2".repeat(64)),
            record_sha256: String::new(),
            observation: PeaceConferencePresentationObservationExact::Inactive {
                refresh_sequence: 7,
            },
        }
    }

    #[test]
    fn stable_identities_are_pointer_free_and_refresh_scoped() {
        let conference = PeaceObjectReferenceExact {
            object_type: 13,
            object_id: 72,
        };
        assert_eq!(participant_identity(&conference), "participant:13:72");
        assert_eq!(
            allocation_identity(&conference, 5, 2),
            "allocation:13:72:5:2"
        );
        assert_ne!(
            allocation_identity(&conference, 5, 2),
            allocation_identity(&conference, 6, 2)
        );
        assert_eq!(result_identity(&conference, 5, 2), "result:13:72:5:2");
        assert_eq!(
            allocation_group_identity(&conference, 5, 2),
            "allocation-group:13:72:5:2"
        );
        assert_eq!(
            contested_popup_identity("allocation:13:72:5:2"),
            "contested-popup:allocation:13:72:5:2"
        );
        assert_eq!(
            contested_popup_row_identity("allocation:13:72:5:2", 1),
            "contested-popup-row:allocation:13:72:5:2:1"
        );
    }

    #[test]
    fn record_hash_excludes_only_the_self_hash_field() {
        let mut carrier = inactive();
        let first = canonical_record_sha256(&carrier).expect("canonical PCS3 hash");
        carrier.record_sha256 = first.clone();
        assert_eq!(
            canonical_record_sha256(&carrier).as_deref(),
            Some(first.as_str())
        );
        carrier.capture_sequence += 1;
        assert_ne!(
            canonical_record_sha256(&carrier).as_deref(),
            Some(first.as_str())
        );
    }

    #[test]
    fn every_authenticated_action_widget_has_a_distinct_wire_token() {
        let tokens = [
            PeaceActionKindExact::TakeState,
            PeaceActionKindExact::Liberate,
            PeaceActionKindExact::Puppet,
            PeaceActionKindExact::ForceGovernment,
            PeaceActionKindExact::TakeNavy,
            PeaceActionKindExact::DemilitarizeZone,
            PeaceActionKindExact::ResourceRights,
            PeaceActionKindExact::WarReparation,
            PeaceActionKindExact::DismantleIndustry,
        ]
        .into_iter()
        .map(PeaceActionKindExact::token)
        .collect::<std::collections::BTreeSet<_>>();
        assert_eq!(tokens.len(), 9);
    }
}
