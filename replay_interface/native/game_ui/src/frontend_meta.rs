
use crate::authored_role_index::{AuthenticatedGuiRoleIndex, StrictAuthoredPanelAdmission};
use crate::window_manager_law::{
    ReplayAuthenticatedElementSemantic, ReplayElementPointerAuthority,
};
use portable_content_manifest::{ContentLayerKind, UsedContentKey};
use replay_core::ValidatedContentAdmission;
use serde::Deserialize;
use snapshot::sha256::Sha256;
use std::collections::{BTreeMap, BTreeSet};

const CONTRACT_JSON: &str =
    include_str!("../../../re_evidence/rendering_laws/XUI26/promotion_contract.json");
const CONTRACT_SHA256: &str = "dc08317ca30f726e725ef237e1c00b851b9736b17c302616432f0051e1c4714b";

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
    slot_id: String,
    authority: FrontendAuthority,
    source_record_id: String,
    complete: bool,
    frame_hour: Option<i64>,
    payload: FrontendBindingPayload,
    collections: Vec<replay_core::Xui26ReplayCollection>,
}

impl FrontendBindingSnapshot {
    /// The payload field this snapshot carries but the panel cannot present,
    /// or `None` when every field reaches it. A family whose snapshot answers
    /// `Some` is not a panel the viewer can open.
    pub fn unroutable_field(&self) -> Option<&'static str> {
        self.payload.unroutable_field()
    }

    /// The hour's typed frontend DATA for one family.
    ///
    /// What the carrier no longer brings is presentation: no resolved element
    /// semantics, no row template identity and no collection host id. Those are
    /// derived at composition time from the effective `.gui` this same
    /// admission already authenticates.
    pub fn from_replay_carrier(
        carrier: &replay_core::Xui26ReplayFamilyCarrier,
        admission: &ValidatedContentAdmission,
        source_content: &UsedContentKey,
        localisation_language: &str,
        view: &replay_core::Xui26FrontendView,
    ) -> Result<Self, FrontendAdmissionError> {
        if view.status != replay_core::ViewStatus::Present
            || view.product_modset_sha256.as_deref() != Some(admission.product_modset_sha256())
            || view.localisation_language.as_deref() != Some(localisation_language)
            || carrier.source_content_sha256 != source_content.payload_sha256
        {
            return Err(FrontendAdmissionError::BindingProvenanceMismatch);
        }
        let payload = match &carrier.payload {
            replay_core::Xui26ReplayPayload::ScenarioSetup {
                scenario_id,
                bookmark_id,
                country_tag,
                game_rules_sha256,
                bookmark_ids_in_order,
                country_tags_in_order,
                game_rule_ids_in_order,
            } => FrontendBindingPayload::ScenarioSetup {
                scenario_id: scenario_id.clone(),
                bookmark_id: bookmark_id.clone(),
                country_tag: country_tag.clone(),
                game_rules_sha256: game_rules_sha256.clone(),
                bookmark_ids_in_order: bookmark_ids_in_order.clone(),
                country_tags_in_order: country_tags_in_order.clone(),
                game_rule_ids_in_order: game_rule_ids_in_order.clone(),
            },
            replay_core::Xui26ReplayPayload::TutorialState {
                tutorial_id,
                step_id,
                completed_step_ids,
            } => FrontendBindingPayload::TutorialState {
                tutorial_id: tutorial_id.clone(),
                step_id: step_id.clone(),
                completed_step_ids: completed_step_ids.clone(),
            },
            replay_core::Xui26ReplayPayload::SettingsSnapshot {
                settings_file_sha256,
                assignments,
            } => FrontendBindingPayload::SettingsSnapshot {
                settings_file_sha256: settings_file_sha256.clone(),
                assignments: assignments.clone(),
            },
            replay_core::Xui26ReplayPayload::MessageSettingsSnapshot {
                settings_file_sha256,
                assignments,
            } => FrontendBindingPayload::MessageSettingsSnapshot {
                settings_file_sha256: settings_file_sha256.clone(),
                assignments: assignments.clone(),
            },
            replay_core::Xui26ReplayPayload::MessageFilterSnapshot {
                settings_file_sha256,
                assignments,
            } => FrontendBindingPayload::MessageFilterSnapshot {
                settings_file_sha256: settings_file_sha256.clone(),
                assignments: assignments.clone(),
            },
            replay_core::Xui26ReplayPayload::SavedGameRules {
                rules_sha256,
                assignments,
            } => FrontendBindingPayload::SavedGameRules {
                rules_sha256: rules_sha256.clone(),
                assignments: assignments.clone(),
            },
            replay_core::Xui26ReplayPayload::PlayerAiPreferences { assignments } => {
                FrontendBindingPayload::PlayerAiPreferences {
                    assignments: assignments.clone(),
                }
            }
            replay_core::Xui26ReplayPayload::LiveLobby {
                player_keys_in_slot_order,
                country_tags_in_slot_order,
            } => FrontendBindingPayload::LiveLobby {
                player_keys_in_slot_order: player_keys_in_slot_order.clone(),
                country_tags_in_slot_order: country_tags_in_slot_order.clone(),
            },
            replay_core::Xui26ReplayPayload::HotjoinState { event_record_ids } => {
                FrontendBindingPayload::HotjoinState {
                    event_record_ids: event_record_ids.clone(),
                }
            }
            replay_core::Xui26ReplayPayload::MatchmakingChat { event_record_ids } => {
                FrontendBindingPayload::MatchmakingChat {
                    event_record_ids: event_record_ids.clone(),
                }
            }
            replay_core::Xui26ReplayPayload::AiTraceRuntime { event_record_ids } => {
                FrontendBindingPayload::AiTraceRuntime {
                    event_record_ids: event_record_ids.clone(),
                }
            }
        };
        Ok(Self {
            slot_id: carrier.slot_id.clone(),
            authority: FrontendAuthority::ReplayPayload,
            source_record_id: carrier.source_record_id.clone(),
            complete: carrier.complete,
            frame_hour: Some(view.frame_hour),
            payload,
            collections: carrier.collections.clone(),
        })
    }
}

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
        bookmark_ids_in_order: Vec<String>,
        country_tags_in_order: Vec<String>,
        game_rule_ids_in_order: Vec<String>,
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

/// A dynamic slot whose payload carries a value the panel has no proven route
/// for is UNAVAILABLE, and its reason code says which field. The alternative -
/// admitting it - reports the slot as available while the recorded value goes
/// nowhere and the authored placeholder stands in its place.
pub const PAYLOAD_FIELD_HAS_NO_PRESENTATION_ROUTE: &str = "payload_field_has_no_presentation_route";

impl FrontendBindingPayload {
    /// The first payload field this layer can carry but cannot PRESENT, or
    /// `None` when every field reaches the panel.
    ///
    /// A field reaches the panel only through a collection group whose authored
    /// host and row template the XUI26 collection law proves. A digest is
    /// provenance rather than something the panel shows, so it does not need a
    /// route. Everything else - a selection, a step, a settings assignment set
    /// with no authored group, a per-row country tag - has none today.
    pub fn unroutable_field(&self) -> Option<&'static str> {
        match self {
            Self::ScenarioSetup { .. } => Some("scenario/bookmark/country selection"),
            Self::TutorialState { .. } => Some("tutorial and step"),
            Self::SettingsSnapshot { .. } => Some("settings assignments"),
            Self::MessageSettingsSnapshot { .. } => Some("message settings assignments"),
            Self::PlayerAiPreferences { .. } => Some("player AI preference assignments"),
            Self::LiveLobby { .. } => Some("per-slot country tags"),
            // The group is built from the assignment KEYS alone, so two
            // recordings with the same keys and different values would present
            // identically. The digests beside them are provenance and need no
            // route; the values do, and have none.
            Self::MessageFilterSnapshot { .. } => Some("message filter assignment values"),
            Self::SavedGameRules { .. } => Some("saved game rule assignment values"),
            // Wholly collection-shaped: the ordered id list IS the group.
            Self::HotjoinState { .. }
            | Self::MatchmakingChat { .. }
            | Self::AiTraceRuntime { .. } => None,
            // Not replay-payload authorities: `admit_frontend_composition`
            // refuses them as nonshipping before this is consulted.
            Self::AccountSummary { .. }
            | Self::SessionDirectory { .. }
            | Self::FriendsRoster { .. }
            | Self::LoadProgress { .. }
            | Self::AchievementState { .. }
            | Self::BackendAccount { .. }
            | Self::BrowserDocument { .. }
            | Self::PopsLogin { .. }
            | Self::SubscriptionState { .. }
            | Self::MatchmakingState { .. }
            | Self::NudgeRuntime { .. }
            | Self::MusicPlayerState { .. } => None,
        }
    }

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
            value.len() == 64
                && value
                    .bytes()
                    .all(|byte| byte.is_ascii_hexdigit() && !byte.is_ascii_uppercase())
        }
        fn unique(values: &[String]) -> bool {
            values.iter().collect::<BTreeSet<_>>().len() == values.len()
        }
        fn nonempty_unique(values: &[String]) -> bool {
            unique(values) && values.iter().all(|value| !value.trim().is_empty())
        }
        fn assignments_valid(values: &BTreeMap<String, String>) -> bool {
            values.keys().all(|key| !key.trim().is_empty())
        }
        match self {
            Self::AccountSummary {
                account_id,
                display_name,
                ..
            }
            | Self::BackendAccount {
                account_id,
                display_name,
                ..
            } => !account_id.trim().is_empty() && !display_name.trim().is_empty(),
            Self::ScenarioSetup {
                scenario_id,
                bookmark_id,
                country_tag,
                game_rules_sha256,
                bookmark_ids_in_order,
                country_tags_in_order,
                game_rule_ids_in_order,
            } => {
                !scenario_id.trim().is_empty()
                    && !bookmark_id.trim().is_empty()
                    && !country_tag.trim().is_empty()
                    && sha256(game_rules_sha256)
                    && nonempty_unique(bookmark_ids_in_order)
                    && nonempty_unique(country_tags_in_order)
                    && nonempty_unique(game_rule_ids_in_order)
                    && bookmark_ids_in_order.contains(bookmark_id)
                    && country_tags_in_order.contains(country_tag)
            }
            Self::SessionDirectory { session_ids } => nonempty_unique(session_ids),
            Self::FriendsRoster {
                account_ids_in_display_order,
            } => nonempty_unique(account_ids_in_display_order),
            Self::LoadProgress {
                stage_id,
                completed_units,
                total_units,
            } => !stage_id.trim().is_empty() && *total_units > 0 && completed_units <= total_units,
            Self::TutorialState {
                tutorial_id,
                step_id,
                completed_step_ids,
            } => {
                !tutorial_id.trim().is_empty()
                    && !step_id.trim().is_empty()
                    && nonempty_unique(completed_step_ids)
            }
            Self::AchievementState {
                achievement_ids_in_display_order,
                unlocked_ids,
                progress_milli,
                ..
            } => {
                nonempty_unique(achievement_ids_in_display_order)
                    && unlocked_ids.iter().all(|id| !id.trim().is_empty())
                    && unlocked_ids
                        .iter()
                        .all(|id| achievement_ids_in_display_order.contains(id))
                    && progress_milli.iter().all(|(id, value)| {
                        achievement_ids_in_display_order.contains(id) && *value <= 1000
                    })
            }
            Self::BrowserDocument {
                document_id,
                content_sha256,
                ..
            } => !document_id.trim().is_empty() && sha256(content_sha256),
            Self::PopsLogin {
                account_id,
                phase_id,
                error_code,
            } => {
                !account_id.trim().is_empty()
                    && !phase_id.trim().is_empty()
                    && error_code
                        .as_ref()
                        .map_or(true, |code| !code.trim().is_empty())
            }
            Self::SubscriptionState {
                offer_id,
                entitlement_ids,
                ..
            } => {
                !offer_id.trim().is_empty()
                    && entitlement_ids
                        .iter()
                        .all(|entitlement| !entitlement.trim().is_empty())
            }
            Self::SettingsSnapshot {
                settings_file_sha256,
                assignments,
            }
            | Self::MessageSettingsSnapshot {
                settings_file_sha256,
                assignments,
            }
            | Self::MessageFilterSnapshot {
                settings_file_sha256,
                assignments,
            } => sha256(settings_file_sha256) && assignments_valid(assignments),
            Self::SavedGameRules {
                rules_sha256,
                assignments,
            } => sha256(rules_sha256) && assignments_valid(assignments),
            Self::PlayerAiPreferences { assignments } => assignments_valid(assignments),
            Self::LiveLobby {
                player_keys_in_slot_order,
                country_tags_in_slot_order,
            } => {
                player_keys_in_slot_order.len() == country_tags_in_slot_order.len()
                    && nonempty_unique(player_keys_in_slot_order)
                    && country_tags_in_slot_order
                        .iter()
                        .all(|tag| !tag.trim().is_empty())
            }
            Self::HotjoinState { event_record_ids }
            | Self::MatchmakingChat { event_record_ids }
            | Self::AiTraceRuntime { event_record_ids } => nonempty_unique(event_record_ids),
            Self::MatchmakingState {
                phase_id,
                selected_session_id,
            } => {
                !phase_id.trim().is_empty()
                    && selected_session_id
                        .as_ref()
                        .map_or(true, |id| !id.trim().is_empty())
            }
            Self::NudgeRuntime {
                mode_id,
                selected_domain_keys,
                ..
            } => !mode_id.trim().is_empty() && nonempty_unique(selected_domain_keys),
            Self::MusicPlayerState {
                track_id,
                elapsed_ms,
                duration_ms,
                playing,
                volume_f32_bits,
                station_ids_in_order,
                selected_station_id,
                ..
            } => {
                let volume = f32::from_bits(*volume_f32_bits);
                nonempty_unique(station_ids_in_order)
                    && !station_ids_in_order.is_empty()
                    && volume.is_finite()
                    && (0.0..=1.0).contains(&volume)
                    && (*duration_ms == 0 || elapsed_ms <= duration_ms)
                    && track_id
                        .as_ref()
                        .map_or(!*playing && *elapsed_ms == 0, |id| {
                            !id.trim().is_empty() && *duration_ms > 0
                        })
                    && selected_station_id
                        .as_ref()
                        .map_or(true, |id| station_ids_in_order.contains(id))
            }
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AdmittedFrontendBinding {
    pub slot_id: String,
    pub source_record_id: String,
    pub payload: FrontendBindingPayload,
    pub frame_hour: Option<i64>,
    pub collections: Vec<replay_core::Xui26ReplayCollection>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct UnavailableFrontendBinding {
    pub slot_id: String,
    pub reason_code: String,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FrontendComposition {
    family_id: String,
    source_file: String,
    frozen_chrome_admitted: bool,
    admitted_bindings: Vec<AdmittedFrontendBinding>,
    unavailable_bindings: Vec<UnavailableFrontendBinding>,
}

impl FrontendComposition {
    pub fn family_id(&self) -> &str {
        &self.family_id
    }

    pub fn source_file(&self) -> &str {
        &self.source_file
    }

    pub fn frozen_chrome_admitted(&self) -> bool {
        self.frozen_chrome_admitted
    }

    pub fn admitted_bindings(&self) -> &[AdmittedFrontendBinding] {
        &self.admitted_bindings
    }

    pub fn unavailable_bindings(&self) -> &[UnavailableFrontendBinding] {
        &self.unavailable_bindings
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct FrontendControllerDispatch {
    pub controller_id: &'static str,
    pub behavior_rvas: &'static [&'static str],
}

pub const FRONTEND_CONTROLLER_DISPATCH: &[FrontendControllerDispatch] = &[
    FrontendControllerDispatch {
        controller_id: "achievement",
        behavior_rvas: &["0x1416755e0"],
    },
    FrontendControllerDispatch {
        controller_id: "music",
        behavior_rvas: &["0x141de3160", "0x141f79420", "0x141f79740"],
    },
    FrontendControllerDispatch {
        controller_id: "settings",
        behavior_rvas: &["0x1402178c0", "0x1419a7a20"],
    },
    FrontendControllerDispatch {
        controller_id: "lobby",
        behavior_rvas: &["0x141861420"],
    },
    FrontendControllerDispatch {
        controller_id: "social",
        behavior_rvas: &["0x141cce1c0"],
    },
    FrontendControllerDispatch {
        controller_id: "observer",
        behavior_rvas: &["0x141cbdbe0"],
    },
    FrontendControllerDispatch {
        controller_id: "nudge",
        behavior_rvas: &["0x141b27d60"],
    },
    FrontendControllerDispatch {
        controller_id: "debug",
        behavior_rvas: &["0x140122410"],
    },
];

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FrontendFamilyScope {
    StaticShipping,
    ReplayPayloadShipping,
    HostRuntimeNonshipping,
    ExternalServiceNonshipping,
}

pub fn frontend_family_scope(
    family_id: &str,
) -> Result<FrontendFamilyScope, FrontendAdmissionError> {
    let family = contract()?
        .families
        .into_iter()
        .find(|row| row.family_id == family_id)
        .ok_or(FrontendAdmissionError::UnknownFamily)?;
    match family.dynamic_bindings.as_slice() {
        [] => Ok(FrontendFamilyScope::StaticShipping),
        [binding] => match binding.authority {
            FrontendAuthority::ReplayPayload => Ok(FrontendFamilyScope::ReplayPayloadShipping),
            FrontendAuthority::HostRuntime => Ok(FrontendFamilyScope::HostRuntimeNonshipping),
            FrontendAuthority::ExternalService => {
                Ok(FrontendFamilyScope::ExternalServiceNonshipping)
            }
            _ => Err(FrontendAdmissionError::ContractInvalid),
        },
        _ => Err(FrontendAdmissionError::ContractInvalid),
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct FrozenFrontendRoot {
    family_id: &'static str,
    root_id: &'static str,
}

const FROZEN_FRONTEND_ROOTS: &[FrozenFrontendRoot] = &[
    FrozenFrontendRoot {
        family_id: "frontend_main",
        root_id: "frontend_background",
    },
    FrozenFrontendRoot {
        family_id: "frontend_game_setup",
        root_id: "frontendgamesetupview",
    },
    FrozenFrontendRoot {
        family_id: "frontend_multiplayer",
        root_id: "frontendmultiplayerview",
    },
    FrozenFrontendRoot {
        family_id: "frontend_friends",
        root_id: "frontend_friends_view",
    },
    FrozenFrontendRoot {
        family_id: "frontend_credits",
        root_id: "frontendcreditsview",
    },
    FrozenFrontendRoot {
        family_id: "credits",
        root_id: "CreditsWindow",
    },
    FrozenFrontendRoot {
        family_id: "load_screen",
        root_id: "load_screen",
    },
    FrozenFrontendRoot {
        family_id: "tutorial_screen",
        root_id: "tutorial_screen_1",
    },
    FrozenFrontendRoot {
        family_id: "achievements",
        root_id: "achievements_window",
    },
    FrozenFrontendRoot {
        family_id: "backend_account",
        root_id: "back_end",
    },
    FrozenFrontendRoot {
        family_id: "browser",
        root_id: "browser_window",
    },
    FrozenFrontendRoot {
        family_id: "pops_login",
        root_id: "pops_login_blocker",
    },
    FrozenFrontendRoot {
        family_id: "subscription_message",
        root_id: "sub_message_overlay_window",
    },
    FrozenFrontendRoot {
        family_id: "settings",
        root_id: "menu_settings",
    },
    FrozenFrontendRoot {
        family_id: "message_settings",
        root_id: "MessageSettingWindow",
    },
    FrozenFrontendRoot {
        family_id: "message_filters",
        root_id: "message_filters",
    },
    FrozenFrontendRoot {
        family_id: "saved_game_rules",
        root_id: "savedgamerules_load",
    },
    FrozenFrontendRoot {
        family_id: "player_ai_preferences",
        root_id: "playeraiprefs",
    },
    FrozenFrontendRoot {
        family_id: "ingame_lobby",
        root_id: "ingame_lobby_window",
    },
    FrozenFrontendRoot {
        family_id: "player_lobby",
        root_id: "player_lobby_window",
    },
    FrozenFrontendRoot {
        family_id: "hotjoin",
        root_id: "hotjoin_window",
    },
    FrozenFrontendRoot {
        family_id: "matchmaking",
        root_id: "matchmaking_window",
    },
    FrozenFrontendRoot {
        family_id: "matchmaking_chat",
        root_id: "matchmaking_chat_window",
    },
    FrozenFrontendRoot {
        family_id: "nudge",
        root_id: "nudge_selection_window",
    },
    FrozenFrontendRoot {
        family_id: "ai_trace",
        root_id: "ai_trace_window",
    },
    FrozenFrontendRoot {
        family_id: "mtg_music",
        root_id: "man_the_guns_faceplate",
    },
    FrozenFrontendRoot {
        family_id: "sabaton_music",
        root_id: "sabaton_faceplate",
    },
    FrozenFrontendRoot {
        family_id: "sabaton_music_vol2",
        root_id: "sabaton_faceplate",
    },
];

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DiscoveredFrontendChrome {
    pub window_id: String,
    pub surface_id: String,
    pub source_file: String,
    pub source_sha256: String,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DiscoveredFrontendComposition {
    chrome: DiscoveredFrontendChrome,
    family: Option<FrontendComposition>,
    manifest_digest_sha256: String,
    product_modset_sha256: String,
    index_identity_sha256: String,
    source_content: UsedContentKey,
}

impl DiscoveredFrontendComposition {
    pub fn chrome(&self) -> &DiscoveredFrontendChrome {
        &self.chrome
    }

    pub fn family(&self) -> Option<&FrontendComposition> {
        self.family.as_ref()
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct StaticFrontendPanelPresentation {
    pub semantics: BTreeMap<String, ReplayAuthenticatedElementSemantic>,
    pub pointer_bindings: BTreeMap<String, ReplayElementPointerAuthority>,
}

pub type FrontendPanelPresentation = StaticFrontendPanelPresentation;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum StaticFrontendPanelRejection {
    NotClassified,
    DynamicBindingsRequired,
    InvalidStaticSemantics,
}

/// THE ELEMENT SEMANTICS, DERIVED.
///
/// The panel's whole authored role set resolves out of the effective `.gui` and
/// localisation the recording admitted - exact coverage by construction, since
/// the map is built BY walking the authored roles. Nothing a producer asserted
/// enters here: resolved per-element presentation is POV-local and outside
/// capture scope, and asking a recorder for it is what made this layer
/// unbuildable. What the recorded payload contributes is the CONTENT of the
/// panel's collections - which rows exist, in what order, and the one runtime
/// string per row - and that is applied by the collections law, not here.
///
/// A family with an unavailable dynamic binding still resolves its authored
/// chrome; the missing slot is named by the disposition census rather than
/// being turned into a whole-panel refusal, and no element is given a
/// substitute value for it.
pub fn admit_frontend_panel_presentation(
    surface: &crate::authored_role_index::AuthenticatedPanelSurface,
) -> Result<FrontendPanelPresentation, StaticFrontendPanelRejection> {
    let family = surface
        .frontend_composition()
        .family()
        .ok_or(StaticFrontendPanelRejection::NotClassified)?;
    let semantics =
        crate::authenticated_semantic_provider::authenticated_static_panel_semantics(surface)
            .map_err(|_| StaticFrontendPanelRejection::InvalidStaticSemantics)?;
    if family.admitted_bindings().len() > 1 {
        return Err(StaticFrontendPanelRejection::DynamicBindingsRequired);
    }
    let pointer_bindings = surface
        .element_roles()
        .iter()
        .map(|role| {
            let authority = match semantics.get(&role.element_id) {
                Some(ReplayAuthenticatedElementSemantic::ReadOnlyButton { .. }) => {
                    ReplayElementPointerAuthority::BlockOnly {
                        visual_enabled: Some(true),
                    }
                }
                Some(
                    ReplayAuthenticatedElementSemantic::PassiveSprite { .. }
                    | ReplayAuthenticatedElementSemantic::Text { .. }
                    | ReplayAuthenticatedElementSemantic::Progress { .. }
                    | ReplayAuthenticatedElementSemantic::BlendFrames { .. }
                    | ReplayAuthenticatedElementSemantic::StandardButton,
                ) => ReplayElementPointerAuthority::BlockOnly {
                    visual_enabled: None,
                },
                _ => ReplayElementPointerAuthority::PassThrough,
            };
            (role.element_id.clone(), authority)
        })
        .collect();
    Ok(FrontendPanelPresentation {
        semantics,
        pointer_bindings,
    })
}

pub fn admit_static_frontend_panel_presentation(
    surface: &crate::authored_role_index::AuthenticatedPanelSurface,
) -> Result<StaticFrontendPanelPresentation, StaticFrontendPanelRejection> {
    let family = surface
        .frontend_composition()
        .family()
        .ok_or(StaticFrontendPanelRejection::NotClassified)?;
    if !family.admitted_bindings().is_empty() || !family.unavailable_bindings().is_empty() {
        return Err(StaticFrontendPanelRejection::DynamicBindingsRequired);
    }
    admit_frontend_panel_presentation(surface)
}

/// lifetime authority. The replay application must additionally prove that
pub fn revalidate_discovered_frontend_chrome(
    composition: &DiscoveredFrontendComposition,
    index: &AuthenticatedGuiRoleIndex,
    admission: &ValidatedContentAdmission,
    panel: &StrictAuthoredPanelAdmission,
) -> bool {
    let snapshots = composition
        .family
        .as_ref()
        .map(|family| {
            family
                .admitted_bindings
                .iter()
                .filter_map(|binding| {
                    let authority = contract()
                        .ok()?
                        .families
                        .into_iter()
                        .find(|row| row.family_id == family.family_id)?
                        .dynamic_bindings
                        .into_iter()
                        .find(|row| row.slot_id == binding.slot_id)?
                        .authority;
                    Some(FrontendBindingSnapshot {
                        slot_id: binding.slot_id.clone(),
                        authority,
                        source_record_id: binding.source_record_id.clone(),
                        complete: true,
                        frame_hour: binding.frame_hour,
                        payload: binding.payload.clone(),
                        collections: binding.collections.clone(),
                    })
                })
                .collect::<Vec<_>>()
        })
        .unwrap_or_default();
    composition.manifest_digest_sha256 == admission.manifest_digest_sha256()
        && composition.product_modset_sha256 == admission.product_modset_sha256()
        && composition.index_identity_sha256 == index.index_identity_sha256
        && composition.source_content == *panel.source_content()
        && admit_discovered_frontend_chrome_with_bindings(index, admission, panel, &snapshots)
            .as_ref()
            == Ok(composition)
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DiscoveredFrontendAdmissionError {
    PanelTokenMismatch,
    IncompleteClosure,
    FrontendContractInvalid,
    FamilyAdmission(FrontendAdmissionError),
    DuplicateClassifiedFamily,
}

pub fn admit_discovered_frontend_chrome(
    index: &AuthenticatedGuiRoleIndex,
    admission: &ValidatedContentAdmission,
    panel: &StrictAuthoredPanelAdmission,
) -> Result<DiscoveredFrontendComposition, DiscoveredFrontendAdmissionError> {
    let surface = index
        .validate_panel_token(admission, panel)
        .map_err(|_| DiscoveredFrontendAdmissionError::PanelTokenMismatch)?;
    if !surface.complete_dependency_closure {
        return Err(DiscoveredFrontendAdmissionError::IncompleteClosure);
    }
    contract().map_err(|_| DiscoveredFrontendAdmissionError::FrontendContractInvalid)?;
    admit_discovered_frontend_chrome_with_bindings(index, admission, panel, &[])
}

pub fn admit_discovered_frontend_chrome_with_bindings(
    index: &AuthenticatedGuiRoleIndex,
    admission: &ValidatedContentAdmission,
    panel: &StrictAuthoredPanelAdmission,
    snapshots: &[FrontendBindingSnapshot],
) -> Result<DiscoveredFrontendComposition, DiscoveredFrontendAdmissionError> {
    let surface = index
        .validate_panel_token(admission, panel)
        .map_err(|_| DiscoveredFrontendAdmissionError::PanelTokenMismatch)?;
    if !surface.complete_dependency_closure {
        return Err(DiscoveredFrontendAdmissionError::IncompleteClosure);
    }
    contract().map_err(|_| DiscoveredFrontendAdmissionError::FrontendContractInvalid)?;
    let family =
        classified_frontend_family_for_surface(&surface.source_content, &surface.surface_id)
            .map_err(DiscoveredFrontendAdmissionError::FamilyAdmission)?
            .map(|family_id| {
                admit_frontend_composition(
                    &family_id,
                    &surface.source_content.payload_sha256,
                    snapshots,
                )
                .map_err(DiscoveredFrontendAdmissionError::FamilyAdmission)
            })
            .transpose()?;
    Ok(DiscoveredFrontendComposition {
        chrome: DiscoveredFrontendChrome {
            window_id: panel.window_id().to_string(),
            surface_id: surface.surface_id.clone(),
            source_file: surface.source_file.clone(),
            source_sha256: surface.source_content.payload_sha256.clone(),
        },
        family,
        manifest_digest_sha256: admission.manifest_digest_sha256().to_string(),
        product_modset_sha256: admission.product_modset_sha256().to_string(),
        index_identity_sha256: index.index_identity_sha256.clone(),
        source_content: surface.source_content.clone(),
    })
}

pub fn admitted_frontend_family_ids(
    index: &AuthenticatedGuiRoleIndex,
    admission: &ValidatedContentAdmission,
) -> Result<Vec<String>, DiscoveredFrontendAdmissionError> {
    index
        .validate(admission)
        .map_err(|_| DiscoveredFrontendAdmissionError::PanelTokenMismatch)?;
    contract().map_err(|_| DiscoveredFrontendAdmissionError::FrontendContractInvalid)?;
    let mut families = Vec::new();
    let mut seen = BTreeSet::new();
    for surface in &index.surfaces {
        if !surface.complete_dependency_closure {
            continue;
        }
        let Some(family_id) =
            classified_frontend_family_for_surface(&surface.source_content, &surface.surface_id)
                .map_err(DiscoveredFrontendAdmissionError::FamilyAdmission)?
        else {
            continue;
        };
        if !seen.insert(family_id.clone()) {
            return Err(DiscoveredFrontendAdmissionError::DuplicateClassifiedFamily);
        }
        families.push(family_id);
    }
    Ok(families)
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FrontendAdmissionError {
    ContractInvalid,
    InvalidSourceIdentity,
    UnknownFamily,
    SourceDigestMismatch,
    UnknownBindingSlot,
    DuplicateBindingSlot,
    BindingAuthorityMismatch,
    MissingBindingProvenance,
    BindingProvenanceMismatch,
    IncompleteBindingSnapshot,
    BindingPayloadMismatch,
    InvalidBindingPayload,
    NonshippingFamilyBinding,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Contract {
    schema: String,
    authority: ContractAuthority,
    production_law: ContractProductionLaw,
    summary: ContractSummary,
    families: Vec<ContractFamily>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct ContractAuthority {
    program: String,
    image_base: String,
    exe_sha256: String,
    game_root_evidence: String,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct ContractProductionLaw {
    audio_policy: String,
    chrome: String,
    dynamic_binding: String,
    external_service_policy: String,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct ContractSummary {
    families: usize,
    base_frontend_meta_dev_families: usize,
    dlc_music_families: usize,
    window_definitions: usize,
    static_only_families: usize,
    fail_closed_binding_slots: usize,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct ContractFamily {
    bytes: u64,
    category: String,
    chrome_authority: String,
    construct_counts: BTreeMap<String, usize>,
    family_id: String,
    source_file: String,
    sha256: String,
    sprite_tokens: Vec<String>,
    window_count: usize,
    dynamic_bindings: Vec<ContractBinding>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct ContractBinding {
    slot_id: String,
    authority: FrontendAuthority,
    source_route: String,
    typed_payload: String,
    missing_disposition: String,
    unavailable_reason_code: String,
}

fn admit_frontend_composition(
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
    if source_sha256.len() != 64
        || !source_sha256
            .bytes()
            .all(|byte| byte.is_ascii_hexdigit() && !byte.is_ascii_uppercase())
    {
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
        if !matches!(
            frontend_family_scope(family_id)?,
            FrontendFamilyScope::ReplayPayloadShipping
        ) {
            return Err(FrontendAdmissionError::NonshippingFamilyBinding);
        }
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
        if snapshot.authority == FrontendAuthority::ReplayPayload && snapshot.frame_hour.is_none() {
            return Err(FrontendAdmissionError::BindingProvenanceMismatch);
        }
    }

    let mut admitted_bindings = Vec::new();
    let mut unavailable_bindings = Vec::new();
    for rule in &family.dynamic_bindings {
        // A payload whose fields do not all reach the panel is NOT admitted.
        // Admitting it would report the slot as available while the recorded
        // values it carries went nowhere and the panel drew authored
        // placeholders in their place.
        let unroutable = supplied
            .get(rule.slot_id.as_str())
            .and_then(|snapshot| snapshot.payload.unroutable_field());
        match (supplied.get(rule.slot_id.as_str()), unroutable) {
            (Some(snapshot), None) => admitted_bindings.push(AdmittedFrontendBinding {
                slot_id: rule.slot_id.clone(),
                source_record_id: snapshot.source_record_id.clone(),
                payload: snapshot.payload.clone(),
                frame_hour: snapshot.frame_hour,
                collections: snapshot.collections.clone(),
            }),
            (Some(_), Some(field)) => unavailable_bindings.push(UnavailableFrontendBinding {
                slot_id: rule.slot_id.clone(),
                reason_code: format!(
                    "{}:{PAYLOAD_FIELD_HAS_NO_PRESENTATION_ROUTE}:{field}",
                    rule.unavailable_reason_code
                ),
            }),
            (None, _) => unavailable_bindings.push(UnavailableFrontendBinding {
                slot_id: rule.slot_id.clone(),
                reason_code: rule.unavailable_reason_code.clone(),
            }),
        }
    }
    Ok(FrontendComposition {
        family_id: family.family_id.clone(),
        source_file: product_source_file(family).to_string(),
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

pub fn frozen_frontend_source_files() -> Result<BTreeSet<String>, FrontendAdmissionError> {
    let contract = contract()?;
    Ok(contract
        .families
        .into_iter()
        .map(|family| family.source_file)
        .collect())
}

pub fn classified_frontend_family_for_surface(
    source: &UsedContentKey,
    surface_id: &str,
) -> Result<Option<String>, FrontendAdmissionError> {
    source
        .validate()
        .map_err(|_| FrontendAdmissionError::InvalidSourceIdentity)?;
    let contract = contract()?;
    let families = contract
        .families
        .iter()
        .filter(|family| {
            normalized_path_eq(
                product_source_file(family),
                &source.normalized_game_relative_path,
            ) && product_layer_matches(family, source)
                && FROZEN_FRONTEND_ROOTS.iter().any(|root| {
                    root.family_id == family.family_id
                        && root.root_id.eq_ignore_ascii_case(surface_id.trim())
                })
        })
        .map(|family| family.family_id.clone())
        .collect::<Vec<_>>();
    if source.layer_kind == ContentLayerKind::Mod
        && families.len() == 2
        && families.iter().any(|family| family == "sabaton_music")
        && families.iter().any(|family| family == "sabaton_music_vol2")
    {
        return Ok(Some("sabaton_music".to_string()));
    }
    Ok((families.len() == 1).then(|| families[0].clone()))
}

pub fn classified_static_frontend_family_for_surface(
    source: &UsedContentKey,
    surface_id: &str,
) -> Result<Option<String>, FrontendAdmissionError> {
    let Some(family_id) = classified_frontend_family_for_surface(source, surface_id)? else {
        return Ok(None);
    };
    let contract = contract()?;
    let family = contract
        .families
        .iter()
        .find(|family| family.family_id == family_id)
        .ok_or(FrontendAdmissionError::UnknownFamily)?;
    Ok(family.dynamic_bindings.is_empty().then_some(family_id))
}

fn product_source_file(family: &ContractFamily) -> &str {
    match family.family_id.as_str() {
        "mtg_music" => "interface/mtg_music.gui",
        "sabaton_music" | "sabaton_music_vol2" => "interface/sabaton_music.gui",
        _ => &family.source_file,
    }
}

fn product_layer_matches(family: &ContractFamily, source: &UsedContentKey) -> bool {
    if source.layer_kind == ContentLayerKind::Mod {
        return source.winning_layer_ordinal > 0 && !source.layer_id.trim().is_empty();
    }
    match family.family_id.as_str() {
        "mtg_music" => {
            source.layer_kind == ContentLayerKind::IntegratedDlc
                && normalized_path_eq(
                    &source.layer_id,
                    "integrated_dlc/dlc023_man_the_guns/dlc023.dlc",
                )
        }
        "sabaton_music" => {
            source.layer_kind == ContentLayerKind::Dlc
                && normalized_path_eq(&source.layer_id, "dlc/dlc013_sabaton/dlc013.dlc")
        }
        "sabaton_music_vol2" => {
            source.layer_kind == ContentLayerKind::Dlc
                && normalized_path_eq(&source.layer_id, "dlc/dlc019_sabaton_vol2/dlc019.dlc")
        }
        _ => {
            source.layer_kind == ContentLayerKind::Base
                && source.winning_layer_ordinal == 0
                && source.layer_id.eq_ignore_ascii_case("base")
        }
    }
}

fn contract() -> Result<Contract, FrontendAdmissionError> {
    if Sha256::hex(CONTRACT_JSON.as_bytes()) != CONTRACT_SHA256 {
        return Err(FrontendAdmissionError::ContractInvalid);
    }
    let contract: Contract =
        serde_json::from_str(CONTRACT_JSON).map_err(|_| FrontendAdmissionError::ContractInvalid)?;
    let ids = contract
        .families
        .iter()
        .map(|row| row.family_id.as_str())
        .collect::<BTreeSet<_>>();
    let sources = contract
        .families
        .iter()
        .map(|row| normalized_path(&row.source_file))
        .collect::<BTreeSet<_>>();
    let binding_slots = contract
        .families
        .iter()
        .map(|family| family.dynamic_bindings.len())
        .sum::<usize>();
    let static_only_families = contract
        .families
        .iter()
        .filter(|family| family.dynamic_bindings.is_empty())
        .count();
    let authority_count = |authority| {
        contract.families.iter().filter(|family| {
        matches!(family.dynamic_bindings.as_slice(), [binding] if binding.authority == authority)
    }).count()
    };
    let rooted_families = FROZEN_FRONTEND_ROOTS
        .iter()
        .map(|root| root.family_id)
        .collect::<BTreeSet<_>>();
    let allowed_categories = [
        "frontend",
        "multiplayer",
        "meta",
        "loading",
        "tutorial",
        "achievements",
        "backend",
        "browser",
        "settings",
        "setup",
        "developer",
        "music",
    ]
    .into_iter()
    .collect::<BTreeSet<_>>();
    let allowed_constructs = [
        "browsertype",
        "buttontype",
        "checkboxtype",
        "containerwindowtype",
        "dropdownboxtype",
        "editboxtype",
        "extendedscrollbartype",
        "gridboxtype",
        "icontype",
        "instanttextboxtype",
        "listboxtype",
        "overlappingelementsboxtype",
        "scrollbartype",
        "windowtype",
    ]
    .into_iter()
    .collect::<BTreeSet<_>>();
    let slots_valid = contract.families.iter().all(|family| {
        let slots = family
            .dynamic_bindings
            .iter()
            .map(|row| row.slot_id.as_str())
            .collect::<BTreeSet<_>>();
        !family.family_id.trim().is_empty()
            && family.bytes > 0
            && allowed_categories.contains(family.category.as_str())
            && family.chrome_authority == "effective_vfs_game_files"
            && !family.construct_counts.is_empty()
            && family
                .construct_counts
                .iter()
                .all(|(name, count)| allowed_constructs.contains(name.as_str()) && *count > 0)
            && !family.sprite_tokens.is_empty()
            && family
                .sprite_tokens
                .iter()
                .all(|token| !token.trim().is_empty())
            && family.sprite_tokens.iter().collect::<BTreeSet<_>>().len()
                == family.sprite_tokens.len()
            && slots.len() == family.dynamic_bindings.len()
            && family.dynamic_bindings.iter().all(|binding| {
                !binding.slot_id.trim().is_empty()
                    && binding.typed_payload == binding.slot_id
                    && valid_source_route(binding.authority, &binding.source_route)
                    && binding.missing_disposition == "explicit_unavailable"
                    && !binding.unavailable_reason_code.trim().is_empty()
            })
            && family.window_count > 0
            && family.sha256.len() == 64
            && family
                .sha256
                .bytes()
                .all(|byte| byte.is_ascii_hexdigit() && !byte.is_ascii_uppercase())
            && !family.source_file.trim().is_empty()
    });
    if contract.schema != "xui26-frontend-meta-contract-v1"
        || contract.authority.program != "hoi4.exe 1.19.2.0.a729"
        || contract.authority.image_base != "0x140000000"
        || contract.authority.exe_sha256
            != "870ebf1f615262facca48a3b947775c41f0a5338f3068c40958209f5c41c65ad"
        || contract.authority.game_root_evidence
            != r"X:\SteamLibrary\steamapps\common\Hearts of Iron IV"
        || contract.production_law.audio_policy
            != "music_chrome_only_audio_playback_is_not_a_rendering-law_input"
        || contract.production_law.chrome != "exact_effective_vfs_definition_and_asset_composition"
        || contract.production_law.dynamic_binding
            != "slot_exact_typed_payload_from_frozen_source_route"
        || contract.production_law.external_service_policy
            != "not_required_for_replay_promotion_fail_closed"
        || contract.summary.families != 28
        || contract.summary.base_frontend_meta_dev_families != 25
        || contract.summary.dlc_music_families != 3
        || contract.summary.window_definitions != 232
        || contract.summary.static_only_families != 2
        || contract.summary.fail_closed_binding_slots != 26
        || contract.families.len() != 28
        || ids.len() != 28
        || sources.len() != 28
        || FROZEN_FRONTEND_ROOTS.len() != 28
        || rooted_families != ids
        || FROZEN_FRONTEND_ROOTS
            .iter()
            .any(|root| root.family_id.trim().is_empty() || root.root_id.trim().is_empty())
        || static_only_families != 2
        || authority_count(FrontendAuthority::ReplayPayload) != 12
        || authority_count(FrontendAuthority::HostRuntime) != 5
        || authority_count(FrontendAuthority::ExternalService) != 9
        || binding_slots != 26
        || !slots_valid
    {
        return Err(FrontendAdmissionError::ContractInvalid);
    }
    Ok(contract)
}

fn normalized_path(value: &str) -> String {
    value
        .replace('\\', "/")
        .trim_start_matches('/')
        .to_ascii_lowercase()
}

fn normalized_path_eq(left: &str, right: &str) -> bool {
    normalized_path(left) == normalized_path(right)
}

fn valid_source_route(authority: FrontendAuthority, route: &str) -> bool {
    let route = route.trim();
    !route.is_empty()
        && route.bytes().all(|byte| {
            byte.is_ascii_lowercase() || byte.is_ascii_digit() || byte == b'.' || byte == b'_'
        })
        && match authority {
            FrontendAuthority::ReplayPayload => {
                route.starts_with("replay.") || route.starts_with("recorder_manifest.")
            }
            FrontendAuthority::ExternalService => route.starts_with("external."),
            FrontendAuthority::LiveSession => route.starts_with("live."),
            FrontendAuthority::HostConfig => route.starts_with("host."),
            FrontendAuthority::HostRuntime => route.starts_with("replay_ui."),
            FrontendAuthority::DeveloperRuntime => route.starts_with("developer."),
        }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::definition_admission::{
        load_effective_definition_catalog, ResolvedGuiDefinitionFile,
    };

    #[test]
    fn native_controller_registry_contains_no_authored_family_or_file_identity() {
        let ids = FRONTEND_CONTROLLER_DISPATCH
            .iter()
            .map(|row| row.controller_id)
            .collect::<BTreeSet<_>>();
        assert_eq!(ids.len(), FRONTEND_CONTROLLER_DISPATCH.len());
        assert!(FRONTEND_CONTROLLER_DISPATCH.iter().all(|row| {
            !row.controller_id.contains('/')
                && !row.controller_id.contains('\\')
                && !row.controller_id.ends_with(".gui")
                && !row.behavior_rvas.is_empty()
                && row.behavior_rvas.iter().all(|rva| rva.starts_with("0x14"))
        }));
    }
    use snapshot::sha256::Sha256;
    use std::fs;

    fn sha(family: &str) -> String {
        contract()
            .unwrap()
            .families
            .into_iter()
            .find(|row| row.family_id == family)
            .unwrap()
            .sha256
    }


    fn used(family_id: &str) -> UsedContentKey {
        let contract = contract().unwrap();
        let family = contract
            .families
            .iter()
            .find(|family| family.family_id == family_id)
            .unwrap();
        let (winning_layer_ordinal, layer_kind, layer_id) = match family_id {
            "mtg_music" => (
                4,
                ContentLayerKind::IntegratedDlc,
                "integrated_dlc/dlc023_man_the_guns/dlc023.dlc",
            ),
            "sabaton_music" => (5, ContentLayerKind::Dlc, "dlc/dlc013_sabaton/dlc013.dlc"),
            "sabaton_music_vol2" => (
                6,
                ContentLayerKind::Dlc,
                "dlc/dlc019_sabaton_vol2/dlc019.dlc",
            ),
            _ => (0, ContentLayerKind::Base, "base"),
        };
        UsedContentKey {
            effective_vfs_manifest_sha256: "1".repeat(64),
            stored_game_relative_path: product_source_file(family).to_string(),
            normalized_game_relative_path: product_source_file(family).to_string(),
            winning_layer_ordinal,
            layer_kind,
            layer_id: layer_id.into(),
            source_sha256: "2".repeat(64),
            payload_sha256: family.sha256.clone(),
            payload_bytes: family.bytes,
        }
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
    fn xui26_family_scope_is_exact_not_a_whole_packet_promotion() {
        let counts = frozen_frontend_family_ids().unwrap().into_iter().fold(
            BTreeMap::<String, usize>::new(),
            |mut counts, family| {
                *counts
                    .entry(format!("{:?}", frontend_family_scope(&family).unwrap()))
                    .or_default() += 1;
                counts
            },
        );
        assert_eq!(counts.get("StaticShipping"), Some(&2));
        assert_eq!(counts.get("ReplayPayloadShipping"), Some(&12));
        assert_eq!(counts.get("HostRuntimeNonshipping"), Some(&5));
        assert_eq!(counts.get("ExternalServiceNonshipping"), Some(&9));
        assert_eq!(counts.values().sum::<usize>(), 28);
    }

    #[test]
    fn family_classifier_requires_effective_path_layer_and_root_not_frozen_payload() {
        let mut source = used("frontend_main");
        assert_eq!(
            classified_frontend_family_for_surface(&source, "FRONTEND_BACKGROUND").unwrap(),
            Some("frontend_main".to_string())
        );

        source.normalized_game_relative_path = r"\INTERFACE\FRONTENDMAINVIEW.GUI".into();
        assert_eq!(
            classified_frontend_family_for_surface(&source, "frontend_background"),
            Err(FrontendAdmissionError::InvalidSourceIdentity)
        );
        source = used("frontend_main");
        source.payload_sha256 = "0".repeat(64);
        assert_eq!(
            classified_frontend_family_for_surface(&source, "frontend_background").unwrap(),
            Some("frontend_main".into())
        );
        source = used("frontend_main");
        assert_eq!(
            classified_frontend_family_for_surface(&source, "owned_dlc_item").unwrap(),
            None
        );
        source.layer_kind = ContentLayerKind::Mod;
        source.layer_id = "mod/byte-identical.mod".into();
        source.winning_layer_ordinal = 3;
        assert_eq!(
            classified_frontend_family_for_surface(&source, "frontend_background").unwrap(),
            Some("frontend_main".into())
        );
    }

    #[test]
    fn all_family_roots_map_and_sabaton_descriptor_disambiguates() {
        for root in FROZEN_FRONTEND_ROOTS {
            assert_eq!(
                classified_frontend_family_for_surface(&used(root.family_id), root.root_id)
                    .unwrap(),
                Some(root.family_id.to_string()),
                "{}",
                root.family_id
            );
        }
        let mut vol2_as_vol1 = used("sabaton_music_vol2");
        vol2_as_vol1.layer_id = "dlc/dlc013_sabaton/dlc013.dlc".into();
        assert_eq!(
            classified_frontend_family_for_surface(&vol2_as_vol1, "sabaton_faceplate").unwrap(),
            Some("sabaton_music".into())
        );
        vol2_as_vol1.layer_kind = ContentLayerKind::Mod;
        vol2_as_vol1.layer_id = "mod/sabaton-copy.mod".into();
        assert_eq!(
            classified_frontend_family_for_surface(&vol2_as_vol1, "sabaton_faceplate").unwrap(),
            Some("sabaton_music".into())
        );
    }

    #[test]
    fn shipping_static_browser_admits_only_the_two_zero_slot_families() {
        let admitted = FROZEN_FRONTEND_ROOTS
            .iter()
            .filter_map(|root| {
                classified_static_frontend_family_for_surface(&used(root.family_id), root.root_id)
                    .unwrap()
            })
            .collect::<BTreeSet<_>>();
        assert_eq!(
            admitted,
            BTreeSet::from(["credits".to_string(), "frontend_credits".to_string()])
        );
    }

    #[test]
    fn empty_dynamic_input_has_a_complete_26_slot_unavailable_census() {
        let dispositions = contract()
            .unwrap()
            .families
            .iter()
            .map(|family| {
                admit_frontend_composition(&family.family_id, &family.sha256, &[]).unwrap()
            })
            .collect::<Vec<_>>();
        assert_eq!(dispositions.len(), 28);
        assert_eq!(
            dispositions
                .iter()
                .map(|family| family.unavailable_bindings().len())
                .sum::<usize>(),
            26
        );
        assert_eq!(
            dispositions
                .iter()
                .filter(|family| family.unavailable_bindings().is_empty())
                .map(|family| family.family_id())
                .collect::<BTreeSet<_>>(),
            BTreeSet::from(["credits", "frontend_credits"])
        );
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
            slot_id: "hotjoin_state".to_string(),
            authority: FrontendAuthority::ReplayPayload,
            source_record_id: "replay:session:0".to_string(),
            complete: true,
            frame_hour: Some(0),
            collections: vec![],
            payload: FrontendBindingPayload::HotjoinState {
                event_record_ids: vec!["session:event:0".into()],
            },
        };
        let expected_payload = snapshot.payload.clone();
        let composition =
            admit_frontend_composition("hotjoin", &sha("hotjoin"), &[snapshot]).unwrap();
        assert_eq!(composition.admitted_bindings.len(), 1);
        assert_eq!(composition.admitted_bindings[0].payload, expected_payload);
        assert!(composition.unavailable_bindings.is_empty());
    }

    /// kills: reporting a slot as AVAILABLE while the values its payload
    /// carries have nowhere on the panel to go.
    ///
    /// A scenario setup carries three selections, a tutorial carries a step and
    /// a settings snapshot carries assignments, and none of the three has an
    /// authored group or a proven store to reach. Admitting them would draw the
    /// panel's authored placeholders and call the slot available, so two
    /// recordings with different values would present identically.
    #[test]
    fn a_payload_field_with_no_presentation_route_is_unavailable_and_names_itself() {
        let cases = [
            (
                "frontend_game_setup",
                "scenario_setup",
                FrontendBindingPayload::ScenarioSetup {
                    scenario_id: "1936".into(),
                    bookmark_id: "gathering_storm".into(),
                    country_tag: "GER".into(),
                    game_rules_sha256: "a".repeat(64),
                    bookmark_ids_in_order: vec!["gathering_storm".into()],
                    country_tags_in_order: vec!["GER".into()],
                    game_rule_ids_in_order: vec![],
                },
                "scenario/bookmark/country selection",
            ),
            (
                "settings",
                "settings_snapshot",
                FrontendBindingPayload::SettingsSnapshot {
                    settings_file_sha256: "a".repeat(64),
                    assignments: BTreeMap::from([("fullscreen".to_string(), "yes".to_string())]),
                },
                "settings assignments",
            ),
            (
                "ingame_lobby",
                "live_lobby",
                FrontendBindingPayload::LiveLobby {
                    player_keys_in_slot_order: vec!["p0".into()],
                    country_tags_in_slot_order: vec!["GER".into()],
                },
                "per-slot country tags",
            ),
            // The group is built from the assignment KEYS alone, so the VALUES
            // reach nothing: two recordings with the same keys and different
            // values would present identically.
            (
                "saved_game_rules",
                "saved_game_rules",
                FrontendBindingPayload::SavedGameRules {
                    rules_sha256: "a".repeat(64),
                    assignments: BTreeMap::from([("difficulty".to_string(), "on".to_string())]),
                },
                "saved game rule assignment values",
            ),
            (
                "tutorial_screen",
                "tutorial_state",
                FrontendBindingPayload::TutorialState {
                    tutorial_id: "basic".into(),
                    step_id: "step_2".into(),
                    completed_step_ids: vec!["step_1".into()],
                },
                "tutorial and step",
            ),
            (
                "message_settings",
                "message_settings_snapshot",
                FrontendBindingPayload::MessageSettingsSnapshot {
                    settings_file_sha256: "a".repeat(64),
                    assignments: BTreeMap::from([("combat".to_string(), "popup".to_string())]),
                },
                "message settings assignments",
            ),
            (
                "player_ai_preferences",
                "player_ai_preferences",
                FrontendBindingPayload::PlayerAiPreferences {
                    assignments: BTreeMap::from([("GER".to_string(), "aggressive".to_string())]),
                },
                "player AI preference assignments",
            ),
            (
                "message_filters",
                "message_filter_snapshot",
                FrontendBindingPayload::MessageFilterSnapshot {
                    settings_file_sha256: "a".repeat(64),
                    assignments: BTreeMap::from([("combat".to_string(), "popup".to_string())]),
                },
                "message filter assignment values",
            ),
        ];
        for (family_id, slot_id, payload, field) in cases {
            let composition = admit_frontend_composition(
                family_id,
                &sha(family_id),
                &[FrontendBindingSnapshot {
                    slot_id: slot_id.to_string(),
                    authority: FrontendAuthority::ReplayPayload,
                    source_record_id: "replay:0".to_string(),
                    complete: true,
                    frame_hour: Some(0),
                    collections: vec![],
                    payload,
                }],
            )
            .unwrap();
            assert!(
                composition.admitted_bindings.is_empty(),
                "{family_id} admitted a payload it cannot present"
            );
            let [unavailable] = composition.unavailable_bindings.as_slice() else {
                panic!("{family_id} did not name one unavailable slot");
            };
            assert_eq!(unavailable.slot_id, slot_id);
            assert!(
                unavailable
                    .reason_code
                    .contains(PAYLOAD_FIELD_HAS_NO_PRESENTATION_ROUTE)
                    && unavailable.reason_code.ends_with(field),
                "{family_id} did not name the field: {}",
                unavailable.reason_code
            );
        }
    }

    #[test]
    fn wrong_authority_incomplete_and_unproven_snapshots_reject() {
        let digest = sha("hotjoin");
        let mut snapshot = FrontendBindingSnapshot {
            slot_id: "hotjoin_state".to_string(),
            authority: FrontendAuthority::LiveSession,
            source_record_id: "replay:session:4".to_string(),
            complete: true,
            frame_hour: Some(4),
            collections: vec![],
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
            admit_frontend_composition("credits", &"0".repeat(63), &[]),
            Err(FrontendAdmissionError::SourceDigestMismatch)
        );
        let unknown = FrontendBindingSnapshot {
            slot_id: "invented".to_string(),
            authority: FrontendAuthority::ExternalService,
            source_record_id: "bad:1".to_string(),
            complete: true,
            frame_hour: None,
            collections: vec![],
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
            frame_hour: Some(0),
            collections: vec![],
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
            frame_hour: None,
            collections: vec![],
            payload: FrontendBindingPayload::LoadProgress {
                stage_id: "parse_definitions".into(),
                completed_units: 11,
                total_units: 10,
            },
        };
        assert_eq!(
            admit_frontend_composition("load_screen", &sha("load_screen"), &[snapshot]),
            Err(FrontendAdmissionError::NonshippingFamilyBinding)
        );
    }

    #[test]
    fn host_runtime_snapshot_is_nonshipping_even_when_structurally_complete() {
        let snapshot = FrontendBindingSnapshot {
            slot_id: "load_progress".into(),
            authority: FrontendAuthority::HostRuntime,
            source_record_id: "replay-ui:loader:parse-definitions".into(),
            complete: true,
            frame_hour: None,
            collections: vec![],
            payload: FrontendBindingPayload::LoadProgress {
                stage_id: "parse_definitions".into(),
                completed_units: 7,
                total_units: 10,
            },
        };
        assert_eq!(
            admit_frontend_composition("load_screen", &sha("load_screen"), &[snapshot]),
            Err(FrontendAdmissionError::NonshippingFamilyBinding)
        );
    }

    #[test]
    fn noncanonical_or_blank_dynamic_identities_reject() {
        let blank_lobby_player = FrontendBindingSnapshot {
            slot_id: "live_lobby".into(),
            authority: FrontendAuthority::ReplayPayload,
            source_record_id: "d57:hour:4".into(),
            complete: true,
            frame_hour: Some(4),
            collections: vec![],
            payload: FrontendBindingPayload::LiveLobby {
                player_keys_in_slot_order: vec!["".into()],
                country_tags_in_slot_order: vec!["GER".into()],
            },
        };
        assert_eq!(
            admit_frontend_composition("ingame_lobby", &sha("ingame_lobby"), &[blank_lobby_player]),
            Err(FrontendAdmissionError::InvalidBindingPayload)
        );

        let uppercase_digest = FrontendBindingSnapshot {
            slot_id: "settings_snapshot".into(),
            authority: FrontendAuthority::ReplayPayload,
            source_record_id: "manifest:settings".into(),
            complete: true,
            frame_hour: Some(0),
            collections: vec![],
            payload: FrontendBindingPayload::SettingsSnapshot {
                settings_file_sha256: "A".repeat(64),
                assignments: BTreeMap::new(),
            },
        };
        assert_eq!(
            admit_frontend_composition("settings", &sha("settings"), &[uppercase_digest]),
            Err(FrontendAdmissionError::InvalidBindingPayload)
        );
    }

    #[test]
    #[cfg_attr(
        not(gate_hoi4_install),
        ignore = "the game install (set BATTLE_PLAN_GAME_ROOT / BATTLE_PLAN_GAME_DIR / BATTLE_PLAN_VANILLA, or install at the default Steam path)"
    )]
    fn installed_xui26_files_hash_and_effective_vfs_shadowing_are_exact() {
        let root = replay_test_gates::require_vanilla_game_root();
        let contract = contract().unwrap();
        let mut resolved = Vec::new();
        for family in &contract.families {
            let path = root.join(family.source_file.replace('/', "\\"));
            let bytes = fs::read(&path).unwrap();
            assert_eq!(Sha256::hex(&bytes), family.sha256, "{}", family.family_id);
            resolved.push(ResolvedGuiDefinitionFile::new(
                family.source_file.clone(),
                bytes,
            ));
        }
        let catalog = load_effective_definition_catalog(resolved).unwrap();
        assert_eq!(catalog.files.len(), 28);
        let count_mismatches = contract
            .families
            .iter()
            .filter_map(|family| {
                let file = catalog
                    .files
                    .iter()
                    .find(|file| normalized_path_eq(&file.source_file, &family.source_file))?;
                let observed = file.admitted_windows + file.rejected_windows;
                let expected = if family.family_id == "sabaton_music" {
                    0
                } else {
                    family.window_count
                };
                (observed != expected).then(|| (family.family_id.clone(), expected, observed))
            })
            .collect::<Vec<_>>();
        assert!(
            count_mismatches.is_empty(),
            "installed XUI26 per-family window census drift: {count_mismatches:?}"
        );
        assert_eq!(
            contract
                .families
                .iter()
                .map(|row| row.window_count)
                .sum::<usize>(),
            232
        );
        assert_eq!(catalog.total_window_count(), 230);
        assert_eq!(catalog.admitted_windows.len(), 230);
        assert!(catalog.rejected_windows.is_empty());
        for expected in FROZEN_FRONTEND_ROOTS {
            if expected.family_id == "sabaton_music" {
                continue;
            }
            let family = contract
                .families
                .iter()
                .find(|family| family.family_id == expected.family_id)
                .unwrap();
            assert!(
                catalog.effective_roots.iter().any(|root| {
                    normalized_path_eq(&root.source_file, &family.source_file)
                        && root.root.name.eq_ignore_ascii_case(expected.root_id)
                }),
                "installed parsed roots omit {} -> {}",
                expected.family_id,
                expected.root_id
            );
        }
    }


}
