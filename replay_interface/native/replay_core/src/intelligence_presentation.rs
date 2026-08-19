
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

pub const XUI14_SCHEMA: u32 = 8;
pub const XUI14_AUTHORITY: &str = "exact_a729_passive_all_country_intelligence";
pub const XUI14_EXECUTABLE_SHA256: &str =
    "870ebf1f615262facca48a3b947775c41f0a5338f3068c40958209f5c41c65ad";

pub const XUI14_PRODUCER_CONTRACT_MANIFEST_SHA256: &str =
    "25d5d795a12e54837cc82361c6a1475f2b77ea8731a69533dfb1446b4994a240";
pub const XUI14_EVIDENCE_MANIFEST_SHA256: &str =
    "a39656be95b17039e0a1cd516a24488ee567bc8caf1bf4fd2c92da9d8bf5459b";

#[derive(Debug, Clone, PartialEq, Eq, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct IntelligenceTextExact {
    pub authored_identity: String,
    pub final_text: String,
    pub final_text_sha256: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct IntelligenceTooltipExact {
    pub primary: IntelligenceTextExact,
    pub delayed: Option<IntelligenceTextExact>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum IntelligenceAssignmentKindExact {
    Idle,
    Mission,
    Operation,
    Network,
    Captured,
    Recruitable,
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct IntelligenceAssignmentExact {
    pub kind: IntelligenceAssignmentKindExact,
    pub assignment_identity: Option<String>,
    pub action_identity: Option<String>,
    pub target_country: Option<String>,
    pub target_country_tag_db_id: Option<u32>,
    pub target_state_id: Option<u32>,
    pub authored_subject_id: Option<String>,
    pub final_text: IntelligenceTextExact,
    pub tooltip: IntelligenceTooltipExact,
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct IntelligenceAgencyExact {
    pub agency_instance_id: String,
    pub created: bool,
    pub name: IntelligenceTextExact,
    pub icon_sprite: String,
    pub upgrade_progress_raw: i64,
    pub upgrade_progress_denominator_raw: i64,
    pub current_upgrade_id: Option<String>,
    pub max_operatives: u32,
    pub usable_slots: u32,
    pub defense_raw: i64,
    pub defense_denominator_raw: i64,
    pub capacity_text: IntelligenceTextExact,
    pub defense_text: IntelligenceTextExact,
    pub tooltip: IntelligenceTooltipExact,
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct IntelligenceUpgradeExact {
    pub source_ordinal: u32,
    pub stable_item_id: String,
    pub upgrade_id: String,
    pub branch_id: String,
    pub level: u32,
    pub icon_sprite: String,
    pub display: IntelligenceTextExact,
    pub tooltip: IntelligenceTooltipExact,
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct IntelligenceOperativeExact {
    pub source_ordinal: u32,
    pub stable_item_id: String,
    pub operative_id: String,
    pub name: IntelligenceTextExact,
    pub portrait_sprite: String,
    pub skill: u32,
    pub experience_raw: i64,
    pub experience_denominator_raw: i64,
    pub experience_text: IntelligenceTextExact,
    pub trait_ids: Vec<String>,
    pub trait_texts: Vec<IntelligenceTextExact>,
    pub nationality_tags: Vec<String>,
    pub nationality_texts: Vec<IntelligenceTextExact>,
    pub captured: bool,
    pub captured_by: Option<String>,
    pub captured_by_tag_db_id: Option<u32>,
    pub capture_end_date_raw: Option<i64>,
    pub recruitable_pool_member: bool,
    pub assignment: IntelligenceAssignmentExact,
    pub tooltip: IntelligenceTooltipExact,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum IntelligenceOperationStatusExact {
    Preparing,
    Prepared,
    Running,
    Finished,
    Aborted,
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct IntelligenceOperationExact {
    pub source_ordinal: u32,
    pub stable_item_id: String,
    pub operation_id: String,
    pub definition_id: String,
    pub target_country: Option<String>,
    pub target_state_id: Option<u32>,
    pub status: IntelligenceOperationStatusExact,
    pub phase_index: u32,
    pub phase_count: u32,
    pub progress_raw: i64,
    pub progress_denominator_raw: i64,
    pub start_hour: i64,
    pub duration_hours: u32,
    pub assigned_operative_ids: Vec<String>,
    pub icon_sprite: String,
    pub display: IntelligenceTextExact,
    pub status_text: IntelligenceTextExact,
    pub target_text: IntelligenceTextExact,
    pub tooltip: IntelligenceTooltipExact,
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct IntelligenceCipherExact {
    pub source_ordinal: u32,
    pub stable_item_id: String,
    pub target_country: String,
    pub target_country_tag_db_id: u32,
    pub decryption_points_raw: i64,
    pub progress_denominator_raw: i64,
    pub decrypting: bool,
    pub cipher_broken: bool,
    pub target_text: IntelligenceTextExact,
    pub progress_text: IntelligenceTextExact,
    pub flag_sprite: String,
    pub tooltip: IntelligenceTooltipExact,
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct IntelligenceLevelExact {
    pub source_ordinal: u32,
    pub stable_item_id: String,
    pub target_country: String,
    pub target_country_tag_db_id: u32,
    pub civilian_raw: i64,
    pub army_raw: i64,
    pub navy_raw: i64,
    pub air_raw: i64,
    pub scale_raw: i64,
    pub target_text: IntelligenceTextExact,
    pub civilian_text: IntelligenceTextExact,
    pub army_text: IntelligenceTextExact,
    pub navy_text: IntelligenceTextExact,
    pub air_text: IntelligenceTextExact,
    pub flag_sprite: String,
    pub tooltip: IntelligenceTooltipExact,
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct IntelligenceNetworkStateExact {
    pub state_id: u32,
    pub strength_raw: i64,
    pub strength_denominator_raw: i64,
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct IntelligenceNetworkExact {
    pub source_ordinal: u32,
    pub stable_item_id: String,
    pub network_id: String,
    pub target_country: String,
    pub target_country_tag_db_id: u32,
    pub operative_ids: Vec<String>,
    pub state_strengths: Vec<IntelligenceNetworkStateExact>,
    pub national_coverage_raw: i64,
    pub national_coverage_denominator_raw: i64,
    pub target_text: IntelligenceTextExact,
    pub coverage_text: IntelligenceTextExact,
    pub flag_sprite: String,
    pub tooltip: IntelligenceTooltipExact,
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct IntelligenceCountryPresentationExact {
    pub source_ordinal: u32,
    pub country_tag: String,
    pub tag_db_id: u32,
    pub source_record_identity_sha256: String,
    pub effective_controller_identity_sha256: String,
    pub effective_root_identity_sha256: String,
    pub expected_upgrade_count: u32,
    pub expected_operative_count: u32,
    pub expected_operation_count: u32,
    pub expected_cipher_count: u32,
    pub expected_intel_level_count: u32,
    pub expected_network_count: u32,
    pub agency: IntelligenceAgencyExact,
    pub upgrades: Vec<IntelligenceUpgradeExact>,
    pub operatives: Vec<IntelligenceOperativeExact>,
    pub operations: Vec<IntelligenceOperationExact>,
    pub ciphers: Vec<IntelligenceCipherExact>,
    pub intel_levels: Vec<IntelligenceLevelExact>,
    pub networks: Vec<IntelligenceNetworkExact>,
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct IntelligencePresentationExact {
    pub schema_version: u32,
    pub authority: String,
    pub executable_sha256: String,
    pub content_manifest_sha256: String,
    pub producer_manifest_sha256: String,
    pub evidence_manifest_sha256: String,
    pub frame_hour: u64,
    pub capture_sequence: u64,
    pub passive_all_country: bool,
    pub ui_open_independent: bool,
    pub recorder_pov_independent: bool,
    pub producer_complete: bool,
    pub expected_country_count: u32,
    pub previous_record_sha256: Option<String>,
    pub record_sha256: String,
    pub rows: Vec<IntelligenceCountryPresentationExact>,
}

pub fn effective_controller_identity_sha256(
    content_manifest_sha256: &str,
    country_tag: &str,
    tag_db_id: u32,
    source_record_identity_sha256: &str,
) -> String {
    format!(
        "{:x}",
        Sha256::digest(format!(
            "xui14-effective-controller-v1\0{content_manifest_sha256}\0{country_tag}\0{tag_db_id}\0{source_record_identity_sha256}"
        ))
    )
}

pub fn effective_root_identity_sha256(content_manifest_sha256: &str) -> String {
    format!(
        "{:x}",
        Sha256::digest(format!(
            "xui14-effective-roots-v1\0{content_manifest_sha256}\0interface/countryintelligenceagencyview.gui\0interface/countryintelledger.gui\0interface/operationoverview.gui\0interface/operativeleader.gui"
        ))
    )
}

pub fn valid_lower_sha256(value: &str) -> bool {
    value.len() == 64
        && value
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
}

pub fn text_valid(value: &IntelligenceTextExact, allow_empty: bool) -> bool {
    !value.authored_identity.trim().is_empty()
        && value.final_text.len() <= 1_048_576
        && (allow_empty || !value.final_text.is_empty())
        && format!("{:x}", Sha256::digest(value.final_text.as_bytes())) == value.final_text_sha256
}

pub fn tooltip_valid(value: &IntelligenceTooltipExact) -> bool {
    text_valid(&value.primary, false)
        && value
            .delayed
            .as_ref()
            .is_none_or(|text| text_valid(text, false))
}

pub fn record_sha256(value: &IntelligencePresentationExact) -> Option<String> {
    let mut canonical = value.clone();
    canonical.record_sha256.clear();
    serde_json::to_vec(&canonical)
        .ok()
        .map(|bytes| format!("{:x}", Sha256::digest(bytes)))
}
