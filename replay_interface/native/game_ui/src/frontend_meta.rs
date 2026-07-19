//! XUI26 production seam for frontend/meta/dev panels and DLC music chrome.
//!
//! Layout and assets are admitted from the effective VFS. Dynamic values are
//! admitted only as typed, complete snapshots with provenance. Missing live,
//! backend, host-local, or developer inputs produce an explicit unavailable
//! binding; the replay never invents values or contacts a service implicitly.

use serde::Deserialize;
use std::collections::{BTreeMap, BTreeSet};

const CONTRACT_JSON: &str =
    include_str!("../../../re_evidence/rendering_laws/XUI26/promotion_contract.json");

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum FrontendAuthority {
    ReplayPayload,
    ExternalService,
    LiveSession,
    HostConfig,
    HostRuntime,
    DeveloperRuntime,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FrontendBindingSnapshot {
    pub slot_id: String,
    pub authority: FrontendAuthority,
    pub source_record_id: String,
    pub complete: bool,
    pub payload: FrontendBindingPayload,
}

/// Slot-exact values consumed by frontend/meta rendering.  Collection order is
/// retained because several of these panels render producer order verbatim.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum FrontendBindingPayload {
    AccountSummary {
        account_id: String,
        display_name: String,
        signed_in: bool,
    },
    ScenarioSetup {
        scenario_id: String,
        bookmark_id: String,
        country_tag: String,
        game_rules_sha256: String,
    },
    SessionDirectory {
        session_ids: Vec<String>,
    },
    FriendsRoster {
        account_ids_in_display_order: Vec<String>,
    },
    LoadProgress {
        stage_id: String,
        completed_units: u64,
        total_units: u64,
    },
    TutorialState {
        tutorial_id: String,
        step_id: String,
        completed_step_ids: Vec<String>,
    },
    AchievementState {
        achievement_ids_in_display_order: Vec<String>,
        unlocked_ids: BTreeSet<String>,
        progress_milli: BTreeMap<String, u32>,
        platform_validated: bool,
    },
    BackendAccount {
        account_id: String,
        display_name: String,
        authenticated: bool,
    },
    BrowserDocument {
        document_id: String,
        content_sha256: String,
        scroll_y_bits: u32,
    },
    PopsLogin {
        account_id: String,
        phase_id: String,
        error_code: Option<String>,
    },
    SubscriptionState {
        offer_id: String,
        active: bool,
        entitlement_ids: BTreeSet<String>,
    },
    SettingsSnapshot {
        settings_file_sha256: String,
        assignments: BTreeMap<String, String>,
    },
    MessageSettingsSnapshot {
        settings_file_sha256: String,
        assignments: BTreeMap<String, String>,
    },
    MessageFilterSnapshot {
        settings_file_sha256: String,
        assignments: BTreeMap<String, String>,
    },
    SavedGameRules {
        rules_sha256: String,
        assignments: BTreeMap<String, String>,
    },
    PlayerAiPreferences {
        assignments: BTreeMap<String, String>,
    },
    LiveLobby {
        player_keys_in_slot_order: Vec<String>,
        country_tags_in_slot_order: Vec<String>,
    },
    HotjoinState {
        event_record_ids: Vec<String>,
    },
    MatchmakingState {
        phase_id: String,
        selected_session_id: Option<String>,
    },
    MatchmakingChat {
        event_record_ids: Vec<String>,
    },
    NudgeRuntime {
        mode_id: String,
        selected_domain_keys: Vec<String>,
        dirty: bool,
    },
    AiTraceRuntime {
        event_record_ids: Vec<String>,
    },
    MusicPlayerState {
        track_id: Option<String>,
        elapsed_ms: u64,
        duration_ms: u64,
        playing: bool,
        shuffle: bool,
        volume_f32_bits: u32,
        station_ids_in_order: Vec<String>,
        selected_station_id: Option<String>,
    },
}

impl FrontendBindingPayload {
    fn slot_id(&self) -> &'static str {
        match self {
            Self::AccountSummary { .. } => "account_summary",
            Self::ScenarioSetup { .. } => "scenario_setup",
            Self::SessionDirectory { .. } => "session_directory",
            Self::FriendsRoster { .. } => "friends_roster",
            Self::LoadProgress { .. } => "load_progress",
            Self::TutorialState { .. } => "tutorial_state",
            Self::AchievementState { .. } => "achievement_state",
            Self::BackendAccount { .. } => "backend_account",
            Self::BrowserDocument { .. } => "browser_document",
            Self::PopsLogin { .. } => "pops_login",
            Self::SubscriptionState { .. } => "subscription_state",
            Self::SettingsSnapshot { .. } => "settings_snapshot",
            Self::MessageSettingsSnapshot { .. } => "message_settings_snapshot",
            Self::MessageFilterSnapshot { .. } => "message_filter_snapshot",
            Self::SavedGameRules { .. } => "saved_game_rules",
            Self::PlayerAiPreferences { .. } => "player_ai_preferences",
            Self::LiveLobby { .. } => "live_lobby",
            Self::HotjoinState { .. } => "hotjoin_state",
            Self::MatchmakingState { .. } => "matchmaking_state",
            Self::MatchmakingChat { .. } => "matchmaking_chat",
            Self::NudgeRuntime { .. } => "nudge_runtime",
            Self::AiTraceRuntime { .. } => "ai_trace_runtime",
            Self::MusicPlayerState { .. } => "music_player_state",
        }
    }

    fn is_valid(&self) -> bool {
        fn sha256(value: &str) -> bool {
            value.len() == 64 && value.bytes().all(|byte| byte.is_ascii_hexdigit())
        }
        fn unique(values: &[String]) -> bool {
            values.iter().collect::<BTreeSet<_>>().len() == values.len()
        }
        match self {
            Self::ScenarioSetup {
                game_rules_sha256, ..
            }
            | Self::BrowserDocument {
                content_sha256: game_rules_sha256,
                ..
            }
            | Self::SettingsSnapshot {
                settings_file_sha256: game_rules_sha256,
                ..
            }
            | Self::MessageSettingsSnapshot {
                settings_file_sha256: game_rules_sha256,
                ..
            }
            | Self::MessageFilterSnapshot {
                settings_file_sha256: game_rules_sha256,
                ..
            }
            | Self::SavedGameRules {
                rules_sha256: game_rules_sha256,
                ..
            } => sha256(game_rules_sha256),
            Self::SessionDirectory { session_ids } => unique(session_ids),
            Self::FriendsRoster {
                account_ids_in_display_order,
            } => unique(account_ids_in_display_order),
            Self::LoadProgress {
                completed_units,
                total_units,
                ..
            } => *total_units > 0 && completed_units <= total_units,
            Self::AchievementState {
                achievement_ids_in_display_order,
                unlocked_ids,
                progress_milli,
                ..
            } => {
                unique(achievement_ids_in_display_order)
                    && unlocked_ids
                        .iter()
                        .all(|id| achievement_ids_in_display_order.contains(id))
                    && progress_milli.iter().all(|(id, value)| {
                        achievement_ids_in_display_order.contains(id) && *value <= 1000
                    })
            }
            Self::LiveLobby {
                player_keys_in_slot_order,
                country_tags_in_slot_order,
            } => {
                player_keys_in_slot_order.len() == country_tags_in_slot_order.len()
                    && unique(player_keys_in_slot_order)
            }
            Self::MusicPlayerState {
                track_id,
                elapsed_ms,
                duration_ms,
                volume_f32_bits,
                station_ids_in_order,
                selected_station_id,
                ..
            } => {
                let volume = f32::from_bits(*volume_f32_bits);
                unique(station_ids_in_order)
                    && volume.is_finite()
                    && (0.0..=1.0).contains(&volume)
                    && (*duration_ms == 0 || elapsed_ms <= duration_ms)
                    && track_id
                        .as_ref()
                        .map_or(*elapsed_ms == 0, |id| !id.is_empty())
                    && selected_station_id
                        .as_ref()
                        .map_or(true, |id| station_ids_in_order.contains(id))
            }
            _ => true,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AdmittedFrontendBinding {
    pub slot_id: String,
    pub source_record_id: String,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct UnavailableFrontendBinding {
    pub slot_id: String,
    pub reason_code: String,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FrontendComposition {
    pub family_id: String,
    pub source_file: String,
    pub frozen_chrome_admitted: bool,
    pub admitted_bindings: Vec<AdmittedFrontendBinding>,
    pub unavailable_bindings: Vec<UnavailableFrontendBinding>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FrontendAdmissionError {
    ContractInvalid,
    UnknownFamily,
    SourceDigestMismatch,
    UnknownBindingSlot,
    DuplicateBindingSlot,
    BindingAuthorityMismatch,
    MissingBindingProvenance,
    IncompleteBindingSnapshot,
    BindingPayloadMismatch,
    InvalidBindingPayload,
}

#[derive(Deserialize)]
struct Contract {
    schema: String,
    authority: ContractAuthority,
    summary: ContractSummary,
    families: Vec<ContractFamily>,
}

#[derive(Deserialize)]
struct ContractAuthority {
    program: String,
    image_base: String,
    exe_sha256: String,
}

#[derive(Deserialize)]
struct ContractSummary {
    families: usize,
    base_frontend_meta_dev_families: usize,
    dlc_music_families: usize,
    window_definitions: usize,
}

#[derive(Deserialize)]
struct ContractFamily {
    family_id: String,
    source_file: String,
    sha256: String,
    window_count: usize,
    dynamic_bindings: Vec<ContractBinding>,
}

#[derive(Deserialize)]
struct ContractBinding {
    slot_id: String,
    authority: FrontendAuthority,
    unavailable_reason_code: String,
}

/// Admits one XUI26 panel family against the frozen game-file identity.
///
/// Missing dynamic inputs are a successful, explicit fail-closed result. A
/// supplied but malformed or incorrectly sourced input rejects the request.
pub fn admit_frontend_composition(
    family_id: &str,
    source_sha256: &str,
    snapshots: &[FrontendBindingSnapshot],
) -> Result<FrontendComposition, FrontendAdmissionError> {
    let contract = contract()?;
    let family = contract
        .families
        .iter()
        .find(|row| row.family_id == family_id)
        .ok_or(FrontendAdmissionError::UnknownFamily)?;
    if source_sha256 != family.sha256 {
        return Err(FrontendAdmissionError::SourceDigestMismatch);
    }

    let expected = family
        .dynamic_bindings
        .iter()
        .map(|row| (row.slot_id.as_str(), row))
        .collect::<BTreeMap<_, _>>();
    let mut supplied = BTreeMap::new();
    for snapshot in snapshots {
        let rule = expected
            .get(snapshot.slot_id.as_str())
            .ok_or(FrontendAdmissionError::UnknownBindingSlot)?;
        if supplied
            .insert(snapshot.slot_id.as_str(), snapshot)
            .is_some()
        {
            return Err(FrontendAdmissionError::DuplicateBindingSlot);
        }
        if snapshot.authority != rule.authority {
            return Err(FrontendAdmissionError::BindingAuthorityMismatch);
        }
        if snapshot.source_record_id.trim().is_empty() {
            return Err(FrontendAdmissionError::MissingBindingProvenance);
        }
        if !snapshot.complete {
            return Err(FrontendAdmissionError::IncompleteBindingSnapshot);
        }
        if snapshot.payload.slot_id() != snapshot.slot_id {
            return Err(FrontendAdmissionError::BindingPayloadMismatch);
        }
        if !snapshot.payload.is_valid() {
            return Err(FrontendAdmissionError::InvalidBindingPayload);
        }
    }

    let mut admitted_bindings = Vec::new();
    let mut unavailable_bindings = Vec::new();
    for rule in &family.dynamic_bindings {
        if let Some(snapshot) = supplied.get(rule.slot_id.as_str()) {
            admitted_bindings.push(AdmittedFrontendBinding {
                slot_id: rule.slot_id.clone(),
                source_record_id: snapshot.source_record_id.clone(),
            });
        } else {
            unavailable_bindings.push(UnavailableFrontendBinding {
                slot_id: rule.slot_id.clone(),
                reason_code: rule.unavailable_reason_code.clone(),
            });
        }
    }
    Ok(FrontendComposition {
        family_id: family.family_id.clone(),
        source_file: family.source_file.clone(),
        frozen_chrome_admitted: true,
        admitted_bindings,
        unavailable_bindings,
    })
}

pub fn frozen_frontend_family_ids() -> Result<Vec<String>, FrontendAdmissionError> {
    Ok(contract()?
        .families
        .into_iter()
        .map(|row| row.family_id)
        .collect())
}

/// Exact XUI26 source-file allowlist for production window composition.
/// Callers still need XUI01 definition admission and XUI02 open state; this
/// function prevents generic static-window composition from becoming a route
/// around DATA_BLOCKED packet ownership.
pub fn frozen_frontend_source_files() -> Result<BTreeSet<String>, FrontendAdmissionError> {
    let contract = contract()?;
    Ok(contract
        .families
        .into_iter()
        .map(|family| family.source_file)
        .collect())
}

fn contract() -> Result<Contract, FrontendAdmissionError> {
    let contract: Contract =
        serde_json::from_str(CONTRACT_JSON).map_err(|_| FrontendAdmissionError::ContractInvalid)?;
    let ids = contract
        .families
        .iter()
        .map(|row| row.family_id.as_str())
        .collect::<BTreeSet<_>>();
    let slots_valid = contract.families.iter().all(|family| {
        let slots = family
            .dynamic_bindings
            .iter()
            .map(|row| row.slot_id.as_str())
            .collect::<BTreeSet<_>>();
        slots.len() == family.dynamic_bindings.len()
            && family.window_count > 0
            && family.sha256.len() == 64
    });
    if contract.schema != "xui26-frontend-meta-contract-v1"
        || contract.authority.program != "hoi4.exe 1.19.2.0.a729"
        || contract.authority.image_base != "0x140000000"
        || contract.authority.exe_sha256
            != "870ebf1f615262facca48a3b947775c41f0a5338f3068c40958209f5c41c65ad"
        || contract.summary.families != 28
        || contract.summary.base_frontend_meta_dev_families != 25
        || contract.summary.dlc_music_families != 3
        || contract.summary.window_definitions != 232
        || contract.families.len() != 28
        || ids.len() != 28
        || !slots_valid
    {
        return Err(FrontendAdmissionError::ContractInvalid);
    }
    Ok(contract)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::definition_admission::{
        load_effective_definition_catalog, ResolvedGuiDefinitionFile,
    };
    use snapshot::sha256::Sha256;
    use std::fs;
    use std::path::PathBuf;

    fn sha(family: &str) -> String {
        contract()
            .unwrap()
            .families
            .into_iter()
            .find(|row| row.family_id == family)
            .unwrap()
            .sha256
    }

    #[test]
    fn xui26_contract_covers_all_28_distinct_families() {
        let ids = frozen_frontend_family_ids().unwrap();
        assert_eq!(ids.len(), 28);
        assert!(ids.contains(&"frontend_main".to_string()));
        assert!(ids.contains(&"ai_trace".to_string()));
        assert!(ids.contains(&"mtg_music".to_string()));
        assert!(ids.contains(&"sabaton_music_vol2".to_string()));
    }

    #[test]
    fn missing_external_data_is_explicit_not_fabricated() {
        let composition =
            admit_frontend_composition("backend_account", &sha("backend_account"), &[]).unwrap();
        assert!(composition.frozen_chrome_admitted);
        assert!(composition.admitted_bindings.is_empty());
        assert_eq!(composition.unavailable_bindings.len(), 1);
        assert_eq!(
            composition.unavailable_bindings[0].reason_code,
            "paradox_backend_unavailable"
        );
    }

    #[test]
    fn exact_typed_snapshot_is_admitted() {
        let snapshot = FrontendBindingSnapshot {
            slot_id: "scenario_setup".to_string(),
            authority: FrontendAuthority::ReplayPayload,
            source_record_id: "replay:scenario:0".to_string(),
            complete: true,
            payload: FrontendBindingPayload::ScenarioSetup {
                scenario_id: "1936".into(),
                bookmark_id: "gathering_storm".into(),
                country_tag: "GER".into(),
                game_rules_sha256: "a".repeat(64),
            },
        };
        let composition = admit_frontend_composition(
            "frontend_game_setup",
            &sha("frontend_game_setup"),
            &[snapshot],
        )
        .unwrap();
        assert_eq!(composition.admitted_bindings.len(), 1);
        assert!(composition.unavailable_bindings.is_empty());
    }

    #[test]
    fn wrong_authority_incomplete_and_unproven_snapshots_reject() {
        let digest = sha("hotjoin");
        let mut snapshot = FrontendBindingSnapshot {
            slot_id: "hotjoin_state".to_string(),
            authority: FrontendAuthority::LiveSession,
            source_record_id: "replay:session:4".to_string(),
            complete: true,
            payload: FrontendBindingPayload::HotjoinState {
                event_record_ids: vec!["session:event:4".into()],
            },
        };
        assert_eq!(
            admit_frontend_composition("hotjoin", &digest, &[snapshot.clone()]),
            Err(FrontendAdmissionError::BindingAuthorityMismatch)
        );
        snapshot.authority = FrontendAuthority::ReplayPayload;
        snapshot.complete = false;
        assert_eq!(
            admit_frontend_composition("hotjoin", &digest, &[snapshot.clone()]),
            Err(FrontendAdmissionError::IncompleteBindingSnapshot)
        );
        snapshot.complete = true;
        snapshot.source_record_id.clear();
        assert_eq!(
            admit_frontend_composition("hotjoin", &digest, &[snapshot]),
            Err(FrontendAdmissionError::MissingBindingProvenance)
        );
    }

    #[test]
    fn source_drift_and_unknown_inputs_reject() {
        assert_eq!(
            admit_frontend_composition("credits", &"0".repeat(64), &[]),
            Err(FrontendAdmissionError::SourceDigestMismatch)
        );
        let unknown = FrontendBindingSnapshot {
            slot_id: "invented".to_string(),
            authority: FrontendAuthority::ExternalService,
            source_record_id: "bad:1".to_string(),
            complete: true,
            payload: FrontendBindingPayload::AiTraceRuntime {
                event_record_ids: vec![],
            },
        };
        assert_eq!(
            admit_frontend_composition("credits", &sha("credits"), &[unknown]),
            Err(FrontendAdmissionError::UnknownBindingSlot)
        );
    }

    #[test]
    fn slot_payload_kind_mismatch_rejects() {
        let snapshot = FrontendBindingSnapshot {
            slot_id: "scenario_setup".into(),
            authority: FrontendAuthority::ReplayPayload,
            source_record_id: "replay:scenario:0".into(),
            complete: true,
            payload: FrontendBindingPayload::TutorialState {
                tutorial_id: "tutorial".into(),
                step_id: "step_1".into(),
                completed_step_ids: vec![],
            },
        };
        assert_eq!(
            admit_frontend_composition(
                "frontend_game_setup",
                &sha("frontend_game_setup"),
                &[snapshot]
            ),
            Err(FrontendAdmissionError::BindingPayloadMismatch)
        );
    }

    #[test]
    fn structurally_invalid_typed_payload_rejects() {
        let snapshot = FrontendBindingSnapshot {
            slot_id: "load_progress".into(),
            authority: FrontendAuthority::HostRuntime,
            source_record_id: "replay-ui:loader:4".into(),
            complete: true,
            payload: FrontendBindingPayload::LoadProgress {
                stage_id: "parse_definitions".into(),
                completed_units: 11,
                total_units: 10,
            },
        };
        assert_eq!(
            admit_frontend_composition("load_screen", &sha("load_screen"), &[snapshot]),
            Err(FrontendAdmissionError::InvalidBindingPayload)
        );
    }

    #[test]
    #[ignore = "production verification supplies BATTLE_PLAN_GAME_ROOT"]
    fn installed_xui26_files_hash_parse_and_admit_232_windows() {
        let root = PathBuf::from(std::env::var("BATTLE_PLAN_GAME_ROOT").or_else(|_| std::env::var("HOI4_GAME_ROOT")).expect("BATTLE_PLAN_GAME_ROOT"));
        let contract = contract().unwrap();
        let mut resolved = Vec::new();
        for family in &contract.families {
            let path = root.join(family.source_file.replace('/', "\\"));
            let bytes = fs::read(&path).unwrap();
            assert_eq!(Sha256::hex(&bytes), family.sha256, "{}", family.family_id);
            resolved.push(ResolvedGuiDefinitionFile::new(
                family.source_file.clone(),
                path,
            ));
        }
        let catalog = load_effective_definition_catalog(resolved).unwrap();
        assert_eq!(catalog.files.len(), 28);
        assert_eq!(catalog.total_window_count(), 232);
        assert_eq!(catalog.admitted_windows.len(), 232);
        assert!(catalog.rejected_windows.is_empty());
    }
}
