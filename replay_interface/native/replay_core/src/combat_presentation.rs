
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

pub const XUI20_SCHEMA_VERSION: u32 = 1;
pub const XUI20_AUTHORITY: &str = "exact_a729_passive_global_combat_presentation";
pub const XUI20_EXECUTABLE_SHA256: &str =
    "870ebf1f615262facca48a3b947775c41f0a5338f3068c40958209f5c41c65ad";
pub const XUI20_PRODUCER_CONTRACT_MANIFEST_SHA256: &str =
    "25434ecaa4eca68a9c8e577a717365a09573669fa21b56177a95e02aaf8ee498";
pub const XUI20_EVIDENCE_MANIFEST_SHA256: &str =
    "e516f8d3ea39c4ebb43ae278bd99885b3e7dc99abec854a9a2e20263476ad8e0";

pub const XUI20_LAND_COMBAT_ROOT: &str = "landcombatview";
pub const XUI20_NAVAL_COMBAT_ROOT: &str = "navalcombatview";
pub const XUI20_COMBAT_LOG_ROOT: &str = "combat_log_view";
pub const XUI20_NAVAL_LOSSES_ROOT: &str = "navallosseswindow";
pub const XUI20_NAVAL_RESULTS_ROOT: &str = "navalcombatresultswindow";
pub const XUI20_DYNAMIC_TEMPLATE_NAMES: &[&str] = &[
    "reserve_combat_entry",
    "division_combat_defender_entry",
    "division_combat_attacker_entry",
    "global_modifier_icon_entry",
    "modifier_icon_entry",
    "navalcombat_ship_entry",
    "combat_log_combat_tab",
    "combat_log_equipment_tab",
    "combat_log_template_tab",
    "combat_log_enemy_loss_item",
    "combat_log_loss_item",
    "combat_log_div_template_item",
    "combat_log_combat_item",
    "equipment_archetype_entry",
    "accident_equipment_archetype_entry",
    "naval_losses_sunk_ship_entry",
    "navallosses_dropdown_item",
    "navallosses_name_header",
    "naval_combat_results_attacker_survivor_entry",
    "naval_combat_results_defender_survivor_entry",
    "naval_combat_results_defender_lost_line_entry",
    "naval_combat_results_attacker_lost_line_entry",
    "naval_combat_results_attacker_lost_ship_entry",
    "naval_combat_results_defender_lost_ship_entry",
    "naval_combat_results_defender_air_lost_line_entry",
    "naval_combat_results_attacker_air_lost_line_entry",
    "naval_combat_results_attacker_lost_air_entry",
    "naval_combat_results_defender_lost_air_entry",
    "naval_combat_results_attacker_air_entry",
    "naval_combat_results_defender_air_entry",
];

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Deserialize, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum CombatPanelKindExact {
    LandCombat,
    NavalCombat,
    CombatLog,
    NavalLosses,
    NavalResults,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Deserialize, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum CombatLogTabExact {
    Combats,
    Equipment,
    Templates,
}

impl CombatPanelKindExact {
    pub fn root_name(self) -> &'static str {
        match self {
            Self::LandCombat => XUI20_LAND_COMBAT_ROOT,
            Self::NavalCombat => XUI20_NAVAL_COMBAT_ROOT,
            Self::CombatLog => XUI20_COMBAT_LOG_ROOT,
            Self::NavalLosses => XUI20_NAVAL_LOSSES_ROOT,
            Self::NavalResults => XUI20_NAVAL_RESULTS_ROOT,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct CombatElementPathStepExact {
    pub child_ordinal: u32,
    pub authored_name: String,
}

pub type CombatElementPathExact = Vec<CombatElementPathStepExact>;

#[derive(Debug, Clone, PartialEq, Eq, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct CombatFinalTextExact {
    pub authored_identity: String,
    pub final_text: String,
    pub final_text_sha256: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct CombatTooltipExact {
    pub primary: CombatFinalTextExact,
    pub delayed: Option<CombatFinalTextExact>,
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize, Serialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum CombatElementBindingExact {
    Text {
        path: CombatElementPathExact,
        value: CombatFinalTextExact,
        tooltip: Option<CombatTooltipExact>,
    },
    Sprite {
        path: CombatElementPathExact,
        sprite_name: String,
        tooltip: Option<CombatTooltipExact>,
    },
    Texture {
        path: CombatElementPathExact,
        texture_path: String,
        tooltip: Option<CombatTooltipExact>,
    },
    Visible {
        path: CombatElementPathExact,
        visible: bool,
        tooltip: Option<CombatTooltipExact>,
    },
    Progress {
        path: CombatElementPathExact,
        numerator_raw: i64,
        denominator_raw: i64,
        tooltip: Option<CombatTooltipExact>,
    },
    Color {
        path: CombatElementPathExact,
        rgba: [u8; 4],
        tooltip: Option<CombatTooltipExact>,
    },
    Frame {
        path: CombatElementPathExact,
        frame_index: u32,
        tooltip: Option<CombatTooltipExact>,
    },
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct CombatScopedBindingExact {
    pub combat_log_tab: Option<CombatLogTabExact>,
    pub binding: CombatElementBindingExact,
}

impl CombatElementBindingExact {
    pub fn path(&self) -> &CombatElementPathExact {
        match self {
            Self::Text { path, .. }
            | Self::Sprite { path, .. }
            | Self::Texture { path, .. }
            | Self::Visible { path, .. }
            | Self::Progress { path, .. }
            | Self::Color { path, .. }
            | Self::Frame { path, .. } => path,
        }
    }

    pub fn tooltip(&self) -> Option<&CombatTooltipExact> {
        match self {
            Self::Text { tooltip, .. }
            | Self::Sprite { tooltip, .. }
            | Self::Texture { tooltip, .. }
            | Self::Visible { tooltip, .. }
            | Self::Progress { tooltip, .. }
            | Self::Color { tooltip, .. }
            | Self::Frame { tooltip, .. } => tooltip.as_ref(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct CombatDynamicRowExact {
    pub source_ordinal: u32,
    pub parent_row_ordinal: Option<u32>,
    pub layout_ordinal: u32,
    pub stable_identity_sha256: String,
    pub parent_path: CombatElementPathExact,
    pub template_name: String,
    pub effective_template_identity_sha256: String,
    pub position_px: [i32; 2],
    pub scroll_channel: CombatScrollChannelExact,
    pub combat_log_tab: Option<CombatLogTabExact>,
    pub expected_binding_count: u32,
    pub bindings: Vec<CombatScopedBindingExact>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum CombatScrollChannelExact {
    None,
    Primary,
    Secondary,
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct CombatPanelPresentationExact {
    pub source_ordinal: u32,
    pub panel_kind: CombatPanelKindExact,
    pub owner_country_tag: Option<String>,
    pub combat_id: Option<i64>,
    pub combat_generation: Option<i64>,
    pub source_record_identity_sha256: String,
    pub effective_root_identity_sha256: String,
    pub close_control_path: CombatElementPathExact,
    pub primary_scroll_viewport_path: Option<CombatElementPathExact>,
    pub primary_scroll_max_px: i32,
    pub primary_scroll_wheel_px: i32,
    pub secondary_scroll_viewport_path: Option<CombatElementPathExact>,
    pub secondary_scroll_max_px: i32,
    pub secondary_scroll_wheel_px: i32,
    pub expected_root_binding_count: u32,
    pub expected_dynamic_row_count: u32,
    pub root_bindings: Vec<CombatScopedBindingExact>,
    pub dynamic_rows: Vec<CombatDynamicRowExact>,
    pub complete: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct CombatPresentationExact {
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
    pub expected_land_combat_count: u32,
    pub expected_naval_combat_count: u32,
    pub expected_combat_log_country_count: u32,
    pub expected_naval_losses_country_count: u32,
    pub expected_naval_result_count: u32,
    pub expected_panel_count: u32,
    pub previous_record_sha256: Option<String>,
    pub record_sha256: String,
    pub panels: Vec<CombatPanelPresentationExact>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CombatPresentationExactView {
    pub status: crate::ViewStatus,
    pub presentation: Option<CombatPresentationExact>,
}

pub fn valid_lower_sha256(value: &str) -> bool {
    value.len() == 64
        && value
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
}

pub fn final_text_sha256(value: &str) -> String {
    format!("{:x}", Sha256::digest(value.as_bytes()))
}

pub fn record_sha256(value: &CombatPresentationExact) -> Option<String> {
    let mut canonical = value.clone();
    canonical.record_sha256.clear();
    serde_json::to_vec(&canonical)
        .ok()
        .map(|bytes| format!("{:x}", Sha256::digest(bytes)))
}

pub fn effective_root_identity_sha256(
    content_manifest_sha256: &str,
    panel_kind: CombatPanelKindExact,
) -> String {
    format!(
        "{:x}",
        Sha256::digest(format!(
            "xui20-effective-root-v1\0{content_manifest_sha256}\0{}",
            panel_kind.root_name()
        ))
    )
}

pub fn effective_template_identity_sha256(
    content_manifest_sha256: &str,
    template_name: &str,
) -> String {
    format!(
        "{:x}",
        Sha256::digest(format!(
            "xui20-effective-template-v1\0{content_manifest_sha256}\0{}",
            template_name.to_ascii_lowercase()
        ))
    )
}

pub fn presentation_identity(panel: &CombatPanelPresentationExact) -> String {
    format!(
        "{:?}:{}:{}:{}",
        panel.panel_kind,
        panel.owner_country_tag.as_deref().unwrap_or(""),
        panel.combat_id.map_or_else(String::new, |value| value.to_string()),
        panel
            .combat_generation
            .map_or_else(String::new, |value| value.to_string())
    )
}
