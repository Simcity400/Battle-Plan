//! Portable, path-independent used-content requirements for a replay.
//! Absolute install/profile paths never cross this boundary.

pub mod attach_content_epoch;
pub mod authored_ui;
pub mod authored_ui_closure;
pub mod content_parse;
pub mod effective_vfs;
pub mod foundation;
pub mod gfx;
pub mod gui;
pub mod hud_emitter;
pub mod hud_role;
pub mod paradox_text;
pub mod symbol_manifest;

use crate::hud_role::validate_sha256;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

pub const MAX_CONTENT_PAYLOAD_BYTES: u64 = 2 << 30;
use std::collections::{BTreeMap, BTreeSet};

pub const SCHEMA: &str = "hoi4.portable-content-manifest.v2";
pub const AUTHORED_GUI_NAMESPACE_CONSUMER: &str =
    "replay.authored_gui_effective_namespace.complete.v1";
pub const AUTHORED_GFX_NAMESPACE_CONSUMER: &str =
    "replay.authored_gfx_effective_namespace.complete.v1";
pub const AUTHORED_LOCALISATION_NAMESPACES_CONSUMER: &str =
    "replay.authored_localisation_effective_namespaces.all.complete.v1";
/// Upper bound for rows emitted by one complete authored-content census.
pub const MAX_REQUIRED_CONTENT_ROWS: usize = 1 << 20;

/// Stable case-insensitive key used by the authored-content closure census.
///
/// **This is not a model of any filesystem and must never decide which FILE a
/// path names.** It is full-Unicode `to_uppercase` then `to_lowercase`, so it
/// collapses pairs NTFS keeps apart — U+00B5 MICRO SIGN onto U+03BC GREEK SMALL
/// MU is the measured one — and, being a table compiled into this binary, it
/// could not be right on every playback volume even if it were corrected (see
/// [`windows_collision_key`]: the `$UpCase` table is written when the volume is
/// FORMATTED and has gained and lost mappings across Windows versions).
///
/// Selection is done by asking the volume, in
/// [`effective_vfs::resolve_literal_layer_path`]. Fail-closed collision
/// detection, which is allowed to over-fold, uses [`windows_collision_key`].
/// Grouping over paths that are already the recorded canonical identity uses
/// that identity ([`normalize_game_relative_path`]).
///
/// What is left for this function is one job with no filesystem to ask: the
/// recorder's cross-layer winner selection over the SOURCE INVENTORY
/// (`recorder_host::portable_content_manifest::source_inventory_winners`),
/// which groups recorded path strings from a JSON document. That it inherits
/// the over-fold is real and is recorded as a residual, not resolved by
/// narrowing this doc comment.
pub fn filesystem_case_key(value: &str) -> String {
    value
        .chars()
        .flat_map(char::to_uppercase)
        .flat_map(char::to_lowercase)
        .collect()
}

/// THE PARTITION RULE: is this setting a property of the MACHINE THAT
/// RECORDED, rather than something the machine that plays back must reproduce?
///
/// Two families answer yes, for the same reason:
///
/// * **Presentation** ([`is_playback_local_presentation_setting`]) — the
///   recorder's window size, refresh rate and GUI scale. The viewer draws in
///   its own window and admits its own UI profile from its own `settings.txt`
///   ([`replay_app::install_paths::playback_ui_profile_from_paths`]), so
///   demanding these would mean "you may only watch this replay on a monitor
///   like the recorder's".
/// * **Proof provenance** — `settings_authority` and `settings_file_sha256`
///   describe how well the recording proved its settings. A replayer can never
///   reproduce a statement the recorder made about its own observation; asking
///   refuses playback with "replay requires material setting
///   \"settings_authority\", but this replayer has no proven parser for it".
///
/// Both therefore live in [`ContentManifest::recorder_local_settings`], which
/// nothing compares against the local install and nothing feeds into the
/// selection digest, while [`ContentManifest::required_settings`] is exactly
/// the settings a replay must reproduce. This is a PARTITION enforced by
/// [`ContentManifest::validate`] on both sides of the wire — not a filter each
/// consumer must remember to apply. Before the split the two disagreed: the
/// binder filtered these keys out of what it asked the local install for, and
/// the comparison then reported every one of them missing, so a recording of a
/// clean vanilla install refused to open on the very machine that made it
/// (measured on t82: 12 of 15 refusals, plus the selection-digest mismatch they
/// caused).
pub fn is_recorder_local_settings(key: &str) -> bool {
    is_playback_local_presentation_setting(key)
        || matches!(key, "settings_authority" | "settings_file_sha256")
}

pub fn is_playback_local_presentation_setting(key: &str) -> bool {
    matches!(
        key,
        "physical_width"
            | "physical_height"
            | "min_gui_width"
            | "min_gui_height"
            | "gui_scale_f32_bits"
            | "refresh_rate_hz"
            | "max_refresh_rate_hz"
            | "fullscreen"
            | "borderless"
            | "display_mode"
            | "input_variant"
    )
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ContentManifest {
    pub schema: String,
    /// Pointer-free identity of the exact executable/mod/DLC/load-order and
    /// effective-VFS product admitted by the extractor's static-bundle v3
    /// gate. This is provenance, while `required_content` remains the minimal
    /// independently distributable playback capability set.
    pub product_modset_sha256: String,
    pub source_game: SourceGameIdentity,
    pub source_profile: SourceProfileIdentity,
    pub effective_vfs_manifest_sha256: String,
    /// Lowest priority to highest. Unused selections remain recorded but are
    /// not required locally.
    pub ordered_layers: Vec<RecordedContentLayer>,
    pub dlc_selection: Vec<DlcSelection>,
    pub localisation_language: String,
    /// Only settings that affect admitted replay content or rendering — i.e.
    /// exactly the settings a replay must reproduce. Every key here is asked of
    /// the local install and feeds the portable selection digest.
    pub required_settings: BTreeMap<String, String>,
    /// What the RECORDER's own machine was set to, and how well the recorder
    /// proved it (see [`is_recorder_local_settings`]). A reader is entitled to
    /// know both; neither is something the machine playing back can or should
    /// reproduce, so nothing here is demanded of the local install and nothing
    /// here is hashed into the selection digest.
    pub recorder_local_settings: BTreeMap<String, String>,
    /// Exactly the files/catalog payloads consumed by the recording.
    pub required_content: Vec<RequiredContent>,
    /// SHA-256 of this manifest encoded with this field set to null.
    pub manifest_digest_sha256: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SourceGameIdentity {
    pub version: String,
    pub build: String,
    pub executable_sha256: String,
    pub build_profile: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SourceProfileIdentity {
    pub profile_id: String,
    pub playset_id: String,
    /// Digest of the path-redacted launcher selection/settings inputs.
    pub selection_sha256: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SelectionLayerIdentity {
    pub ordinal: u32,
    pub layer_kind: ContentLayerKind,
    pub layer_id: String,
    pub descriptor_sha256: Option<String>,
    pub source_container_sha256: Option<String>,
    pub storage_kind: LayerStorageKind,
    pub replace_paths: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SelectionDlcIdentity {
    pub dlc_id: String,
    pub enabled: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RecordedContentLayer {
    pub ordinal: u32,
    pub layer_kind: ContentLayerKind,
    pub layer_id: String,
    pub display_name: String,
    pub descriptor_sha256: Option<String>,
    /// Archive bytes when archive-backed.
    pub source_container_sha256: Option<String>,
    pub storage_kind: LayerStorageKind,
    pub replace_paths: Vec<String>,
    pub required_for_playback: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ContentLayerKind {
    Base,
    IntegratedDlc,
    Dlc,
    Mod,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum LayerStorageKind {
    Directory,
    ZipArchive,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct DlcSelection {
    pub dlc_id: String,
    pub enabled: bool,
    pub required_for_playback: bool,
}

/// Identity of one file after launcher-equivalent VFS resolution.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct UsedContentKey {
    pub effective_vfs_manifest_sha256: String,
    pub normalized_game_relative_path: String,
    pub winning_layer_ordinal: u32,
    pub layer_kind: ContentLayerKind,
    pub layer_id: String,
    /// Archive bytes for an archived entry; file bytes otherwise.
    pub source_sha256: String,
    /// Resolved/decompressed payload consumed by replay.
    pub payload_sha256: String,
    pub payload_bytes: u64,
}

impl UsedContentKey {
    pub fn validate(&self) -> Result<(), String> {
        validate_used_content_key(self)
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RequiredContentKind {
    Definition,
    ShaderSource,
    CompiledShader,
    Texture,
    Mesh,
    Geometry,
    Localisation,
    GuiDefinition,
    Font,
    Catalog,
    Other,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RequiredContent {
    pub content_key: String,
    pub kind: RequiredContentKind,
    pub content: UsedContentKey,
    pub observation_count: u64,
    pub consumers: Vec<String>,
}

/// Player-produced, root-independent result of resolving the recording's
/// required selection through the chosen install and profile roots.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PlaybackContentInventory {
    pub source_game: SourceGameIdentity,
    pub source_profile: SourceProfileIdentity,
    pub layers: Vec<PlaybackContentLayer>,
    pub dlc_selection: Vec<DlcSelection>,
    pub localisation_language: String,
    pub settings: BTreeMap<String, String>,
    pub resolved_content: Vec<ResolvedPlaybackContent>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PlaybackContentLayer {
    pub recorded_ordinal: u32,
    pub layer_kind: ContentLayerKind,
    pub layer_id: String,
    pub descriptor_sha256: Option<String>,
    pub source_container_sha256: Option<String>,
    pub storage_kind: LayerStorageKind,
    pub replace_paths: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ResolvedPlaybackContent {
    pub normalized_game_relative_path: String,
    pub winning_layer_ordinal: u32,
    pub layer_kind: ContentLayerKind,
    pub layer_id: String,
    pub source_sha256: String,
    pub payload_sha256: String,
    pub payload_bytes: u64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ContentIssueCode {
    /// The REPLAY's own portable content manifest is malformed or absent. The
    /// recording is at fault; no change to the local installation can help.
    InvalidManifest,
    /// The CALLER-supplied inventory of the local installation/profile is
    /// malformed — a bad hash, an unusable layer id, a duplicate or aliasing
    /// row, or selection inputs the portable selection digest cannot be
    /// computed from. The replay's manifest is fine; the local enumeration is
    /// not. Kept distinct from [`ContentIssueCode::InvalidManifest`] because
    /// the two name opposite subjects and therefore opposite remediations.
    InvalidPlaybackInventory,
    GameVersionMismatch,
    ExecutableMismatch,
    BuildProfileMismatch,
    ProfileMismatch,
    MissingLayer,
    LayerIdentityMismatch,
    LayerOrderMismatch,
    ReplacePathMismatch,
    RequiredDlcMismatch,
    LanguageMismatch,
    SettingMismatch,
    MissingContent,
    WinningLayerMismatch,
    SourceHashMismatch,
    PayloadHashMismatch,
    PayloadLengthMismatch,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ContentIssue {
    pub code: ContentIssueCode,
    pub subject: String,
    pub message: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ContentValidationReport {
    pub compatible: bool,
    pub verified_required_content: usize,
    pub issues: Vec<ContentIssue>,
}

/// Capability issued only after the complete standalone compatibility gate
/// passes. Renderers use it to prevent any unrecorded/default resolver lookup.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ValidatedContentAdmission {
    manifest_digest_sha256: String,
    product_modset_sha256: String,
    ordered_layers: Vec<RecordedContentLayer>,
    localisation_language: String,
    required_by_path: BTreeMap<String, UsedContentKey>,
    required_by_consumer: BTreeMap<String, Vec<UsedContentKey>>,
}

impl ValidatedContentAdmission {
    pub fn manifest_digest_sha256(&self) -> &str {
        &self.manifest_digest_sha256
    }

    pub fn product_modset_sha256(&self) -> &str {
        &self.product_modset_sha256
    }

    pub fn ordered_layers(&self) -> &[RecordedContentLayer] {
        &self.ordered_layers
    }

    pub fn localisation_language(&self) -> &str {
        &self.localisation_language
    }

    pub fn used_content(&self, game_relative_path: &str) -> Option<&UsedContentKey> {
        let normalized = normalize_game_relative_path(game_relative_path).ok()?;
        self.required_by_path.get(&normalized)
    }

    pub fn require_used_content(
        &self,
        game_relative_path: &str,
    ) -> Result<&UsedContentKey, String> {
        self.used_content(game_relative_path).ok_or_else(|| {
            format!(
                "replay content admission rejects unrecorded/default lookup {game_relative_path:?}"
            )
        })
    }

    pub fn required_paths(&self) -> impl Iterator<Item = &str> {
        self.required_by_path.keys().map(String::as_str)
    }

    /// Exact VFS-winning inputs recorded for one semantic consumer. This is
    /// the portable replay binding used by authored HUD-role consumers; it
    /// never falls back to a vanilla catalog or a guessed path.
    pub fn used_content_for_consumer(&self, consumer: &str) -> Option<&[UsedContentKey]> {
        self.required_by_consumer.get(consumer).map(Vec::as_slice)
    }

    pub fn require_used_content_for_consumer(
        &self,
        consumer: &str,
    ) -> Result<&[UsedContentKey], String> {
        self.used_content_for_consumer(consumer).ok_or_else(|| {
            format!("replay content admission rejects unrecorded semantic consumer {consumer:?}")
        })
    }

    pub fn required_consumers(&self) -> impl Iterator<Item = &str> {
        self.required_by_consumer.keys().map(String::as_str)
    }
}

impl ContentManifest {
    pub fn encode_json(&self) -> Result<Vec<u8>, String> {
        self.validate()?;
        serde_json::to_vec(self).map_err(|err| err.to_string())
    }

    pub fn decode_json(bytes: &[u8]) -> Result<Self, String> {
        let value: Self = serde_json::from_slice(bytes).map_err(|err| err.to_string())?;
        value.validate()?;
        Ok(value)
    }

    pub fn sha256(&self) -> Result<String, String> {
        self.computed_digest()
    }

    pub fn computed_digest(&self) -> Result<String, String> {
        self.validate_without_digest()?;
        self.digest_of_validated()
    }

    /// Digest body shared by [`Self::computed_digest`] and [`Self::validate`].
    /// Callers must have run [`Self::validate_without_digest`] already; the
    /// bytes must remain exactly the derived `Serialize` output with the
    /// digest field nulled, or every existing signed recording would fail its
    /// digest gate.
    fn digest_of_validated(&self) -> Result<String, String> {
        let mut unsigned = self.clone();
        unsigned.manifest_digest_sha256 = None;
        let bytes = serde_json::to_vec(&unsigned).map_err(|err| err.to_string())?;
        Ok(format!("{:x}", Sha256::digest(bytes)))
    }

    pub fn with_computed_digest(mut self) -> Result<Self, String> {
        self.manifest_digest_sha256 = Some(self.computed_digest()?);
        self.validate()?;
        Ok(self)
    }

    pub fn validate(&self) -> Result<(), String> {
        self.validate_without_digest()?;
        if let Some(declared) = &self.manifest_digest_sha256 {
            validate_sha256(declared, "content manifest digest")?;
            let computed = self.digest_of_validated()?;
            if declared != &computed {
                return Err(format!(
                    "content manifest digest mismatch: declared {declared}, computed {computed}"
                ));
            }
        }
        Ok(())
    }

    fn validate_without_digest(&self) -> Result<(), String> {
        if self.schema != SCHEMA {
            return Err(format!(
                "unsupported portable content schema {}",
                self.schema
            ));
        }
        validate_sha256(&self.product_modset_sha256, "product modset")?;
        validate_source_game(&self.source_game)?;
        validate_source_profile(&self.source_profile)?;
        validate_sha256(
            &self.effective_vfs_manifest_sha256,
            "effective VFS manifest",
        )?;
        if self.localisation_language.trim().is_empty() {
            return Err("localisation language is blank".into());
        }
        let mut layer_ids = BTreeSet::new();
        let mut layer_ordinals = BTreeSet::new();
        let mut prior = None;
        for layer in &self.ordered_layers {
            validate_layer(layer)?;
            if !layer_ids.insert(layer.layer_id.as_str()) {
                return Err(format!("duplicate content layer id {}", layer.layer_id));
            }
            if !layer_ordinals.insert(layer.ordinal) {
                return Err(format!("duplicate content layer ordinal {}", layer.ordinal));
            }
            if prior.is_some_and(|value| layer.ordinal <= value) {
                return Err("content layers are not strictly ordered".into());
            }
            prior = Some(layer.ordinal);
        }
        let mut dlcs = BTreeSet::new();
        for dlc in &self.dlc_selection {
            if dlc.dlc_id.trim().is_empty() || !dlcs.insert(dlc.dlc_id.as_str()) {
                return Err("DLC selection has a blank or duplicate id".into());
            }
            if dlc.required_for_playback && !dlc.enabled {
                return Err(format!("required DLC {} was not enabled", dlc.dlc_id));
            }
        }
        // THE PARTITION, enforced rather than remembered (see
        // [`is_recorder_local_settings`]). `required_settings` is what a replay
        // must REPRODUCE, so a recorder-local row in it is demanded of an
        // install that cannot supply it; `recorder_local_settings` is never
        // compared and never hashed, so a material setting hidden in it escapes
        // both the local comparison and the selection digest. Refusing both
        // directions here means no consumer has to filter, and a hand-edited
        // manifest that moves a row across the line is refused rather than
        // silently honoured.
        for (key, value) in &self.required_settings {
            if key.trim().is_empty() || value.trim().is_empty() {
                return Err("required setting has a blank key/value".into());
            }
            if is_recorder_local_settings(key) {
                return Err(format!(
                    "required setting {key:?} is a property of the machine that recorded, not \
                     something the machine playing back reproduces; it belongs in \
                     recorder_local_settings"
                ));
            }
        }
        for (key, value) in &self.recorder_local_settings {
            if key.trim().is_empty() || value.trim().is_empty() {
                return Err("recorder-local setting has a blank key/value".into());
            }
            if !is_recorder_local_settings(key) {
                return Err(format!(
                    "recorder-local settings carry {key:?}, which is a material setting a replay \
                     must reproduce; it belongs in required_settings"
                ));
            }
        }
        let layers: BTreeMap<_, _> = self
            .ordered_layers
            .iter()
            .map(|layer| (layer.ordinal, layer))
            .collect();
        let mut content_keys = BTreeSet::new();
        let mut paths: BTreeSet<String> = BTreeSet::new();
        for required in &self.required_content {
            if required.content_key.trim().is_empty()
                || !content_keys.insert(required.content_key.as_str())
            {
                return Err("required content has a blank or duplicate content_key".into());
            }
            validate_used_content_key(&required.content)?;
            if required.content.effective_vfs_manifest_sha256 != self.effective_vfs_manifest_sha256
            {
                return Err(format!(
                    "required content {} names a different effective VFS",
                    required.content_key
                ));
            }
            // Uniqueness is decided on the Windows collision key, not on the
            // canonical form: the canonical form is a recorded identity and
            // under-approximates NTFS, so two required rows could otherwise
            // name one file under two spellings and disagree about its bytes.
            let collision =
                windows_collision_key(&required.content.normalized_game_relative_path)?;
            if !paths.insert(collision) {
                return Err(format!(
                    "required content repeats effective path {}",
                    required.content.normalized_game_relative_path
                ));
            }
            let layer = layers
                .get(&required.content.winning_layer_ordinal)
                .ok_or_else(|| {
                    format!(
                        "required content {} names a missing layer",
                        required.content_key
                    )
                })?;
            if layer.layer_id != required.content.layer_id
                || layer.layer_kind != required.content.layer_kind
            {
                return Err(format!(
                    "required content {} disagrees with its winning layer",
                    required.content_key
                ));
            }
            if !layer.required_for_playback {
                return Err(format!(
                    "winning layer {} is not marked required for playback",
                    layer.layer_id
                ));
            }
            if required.consumers.is_empty()
                || required
                    .consumers
                    .iter()
                    .any(|value| value.trim().is_empty())
            {
                return Err(format!(
                    "required content {} has no valid consumer",
                    required.content_key
                ));
            }
            if required.observation_count == 0 {
                return Err(format!(
                    "required content {} has zero observations",
                    required.content_key
                ));
            }
            let ordered_consumers: BTreeSet<_> = required.consumers.iter().collect();
            if ordered_consumers.len() != required.consumers.len()
                || ordered_consumers
                    .iter()
                    .copied()
                    .ne(required.consumers.iter())
            {
                return Err(format!(
                    "required content {} consumers are not sorted and unique",
                    required.content_key
                ));
            }
        }
        let computed_selection = self.computed_selection_sha256()?;
        if self.source_profile.selection_sha256 != computed_selection {
            return Err(format!(
                "source profile selection digest mismatch: declared {}, computed {}",
                self.source_profile.selection_sha256, computed_selection
            ));
        }
        Ok(())
    }

    pub fn computed_selection_sha256(&self) -> Result<String, String> {
        let layers = self
            .ordered_layers
            .iter()
            .filter(|layer| layer.required_for_playback)
            .map(|layer| SelectionLayerIdentity {
                ordinal: layer.ordinal,
                layer_kind: layer.layer_kind,
                layer_id: layer.layer_id.clone(),
                descriptor_sha256: layer.descriptor_sha256.clone(),
                source_container_sha256: layer.source_container_sha256.clone(),
                storage_kind: layer.storage_kind,
                replace_paths: layer.replace_paths.clone(),
            })
            .collect::<Vec<_>>();
        let dlcs = self
            .dlc_selection
            .iter()
            .filter(|dlc| dlc.required_for_playback)
            .map(|dlc| SelectionDlcIdentity {
                dlc_id: dlc.dlc_id.clone(),
                enabled: dlc.enabled,
            })
            .collect::<Vec<_>>();
        compute_selection_sha256(
            &self.effective_vfs_manifest_sha256,
            &layers,
            &dlcs,
            &self.localisation_language,
            &self.required_settings,
        )
    }

    pub fn validate_playback(
        &self,
        playback: &PlaybackContentInventory,
    ) -> ContentValidationReport {
        if let Err(message) = self.validate() {
            return ContentValidationReport {
                compatible: false,
                verified_required_content: 0,
                issues: vec![issue(
                    ContentIssueCode::InvalidManifest,
                    "manifest",
                    message,
                )],
            };
        }
        let mut issues = Vec::new();
        compare_scalar(
            &mut issues,
            ContentIssueCode::GameVersionMismatch,
            "source_game.version",
            &self.source_game.version,
            &playback.source_game.version,
        );
        compare_scalar(
            &mut issues,
            ContentIssueCode::ExecutableMismatch,
            "source_game.executable_sha256",
            &self.source_game.executable_sha256,
            &playback.source_game.executable_sha256,
        );
        compare_scalar(
            &mut issues,
            ContentIssueCode::BuildProfileMismatch,
            "source_game.build_profile",
            &self.source_game.build_profile,
            &playback.source_game.build_profile,
        );
        compare_scalar(
            &mut issues,
            ContentIssueCode::BuildProfileMismatch,
            "source_game.build",
            &self.source_game.build,
            &playback.source_game.build,
        );
        // profile_id/playset_id are path-redacted provenance labels only. They
        // may be run-local and must never make an independently distributed
        // replay unplayable. selection_sha256 is the stable, portable gate.
        // Never trust or compare a run-local profile label/digest supplied by
        // the UX. Recompute the portable selection identity below from exact
        // required layers, DLC, language, and settings.
        compare_scalar(
            &mut issues,
            ContentIssueCode::LanguageMismatch,
            "localisation_language",
            &self.localisation_language,
            &playback.localisation_language,
        );

        // Admission is a capability boundary, so structurally-invalid caller
        // inventory must be refused even when the bad row is unrelated to one
        // of the manifest's required rows. Otherwise a matching required
        // subset can hide malformed ambient state that a later caller might
        // accidentally consume under the same admission.
        if playback.localisation_language.trim().is_empty() {
            issues.push(issue(
                ContentIssueCode::InvalidPlaybackInventory,
                "playback inventory",
                "localisation language is blank".into(),
            ));
        }
        for (key, value) in &playback.settings {
            if key.trim().is_empty() || value.trim().is_empty() {
                issues.push(issue(
                    ContentIssueCode::InvalidPlaybackInventory,
                    "playback inventory",
                    format!("setting has a blank key/value: {key:?}"),
                ));
            }
        }

        for (key, expected) in &self.required_settings {
            match playback.settings.get(key) {
                Some(actual) if actual == expected => {}
                Some(actual) => issues.push(issue(
                    ContentIssueCode::SettingMismatch,
                    key,
                    format!("required setting is {expected:?}, playback has {actual:?}"),
                )),
                None => issues.push(issue(
                    ContentIssueCode::SettingMismatch,
                    key,
                    "required setting is missing".into(),
                )),
            }
        }

        // Duplicate inventory rows are typed refusals, never silently
        // canonicalized: a last-wins collapse would let a mismatching row hide
        // behind a matching one. Ordinal-uniqueness and id-uniqueness are
        // refused INDEPENDENTLY, mirroring manifest validation — a composite
        // (ordinal, id) tuple check alone would admit (1,"mod-a")+(1,"decoy")
        // or one id at two ordinals. The first row is kept so the remaining
        // comparisons still run deterministically.
        let mut local_layers = BTreeMap::new();
        let mut local_layer_ordinals = BTreeSet::new();
        let mut local_layer_ids = BTreeSet::new();
        // Rows refused as malformed above are excluded from local_layers, so the
        // required-layer loop below would otherwise report them as absent
        // (LayerOrderMismatch/MissingLayer). Both codes would be false: the row
        // IS present at its recorded ordinal, it is simply not admissible. Keep
        // the key here so the required-layer loop reports the defect it actually
        // has, once, instead of inventing a load-order remediation.
        let mut refused_layers = BTreeSet::new();
        for layer in &playback.layers {
            let mut invalid = false;
            if let Err(message) = validate_portable_layer_id(&layer.layer_id) {
                issues.push(issue(
                    ContentIssueCode::InvalidPlaybackInventory,
                    "playback inventory",
                    message,
                ));
                invalid = true;
            }
            if let Some(hash) = &layer.descriptor_sha256 {
                if let Err(message) = validate_sha256(hash, "playback layer descriptor") {
                    issues.push(issue(
                        ContentIssueCode::InvalidPlaybackInventory,
                        "playback inventory",
                        message,
                    ));
                    invalid = true;
                }
            }
            if let Some(hash) = &layer.source_container_sha256 {
                if let Err(message) = validate_sha256(hash, "playback layer source container") {
                    issues.push(issue(
                        ContentIssueCode::InvalidPlaybackInventory,
                        "playback inventory",
                        message,
                    ));
                    invalid = true;
                }
            }
            if layer.storage_kind == LayerStorageKind::ZipArchive
                && layer.source_container_sha256.is_none()
            {
                issues.push(issue(
                    ContentIssueCode::InvalidPlaybackInventory,
                    "playback inventory",
                    format!("archive layer {} has no archive hash", layer.layer_id),
                ));
                invalid = true;
            }
            // A replace_path declaration that cannot be normalized, or that
            // duplicates another after normalization, is a replace_path defect
            // and is typed as one — not folded into the generic malformed-row
            // code. The distinction is load-bearing downstream: the issue code
            // selects the remediation the operator is shown.
            if let Err(message) = unique_normalized_path_set(&layer.replace_paths) {
                issues.push(issue(
                    ContentIssueCode::ReplacePathMismatch,
                    format!("layer:{}", layer.layer_id),
                    message,
                ));
                invalid = true;
            }
            let mut duplicate = false;
            if !local_layer_ordinals.insert(layer.recorded_ordinal) {
                issues.push(issue(
                    ContentIssueCode::InvalidPlaybackInventory,
                    "playback inventory",
                    format!("duplicate layer ordinal {}", layer.recorded_ordinal),
                ));
                duplicate = true;
            }
            if !local_layer_ids.insert(layer.layer_id.as_str()) {
                issues.push(issue(
                    ContentIssueCode::InvalidPlaybackInventory,
                    "playback inventory",
                    format!("duplicate layer id {}", layer.layer_id),
                ));
                duplicate = true;
            }
            if invalid || duplicate {
                // `duplicate` counts too. A decoy row that claims the required
                // ordinal (or id) first makes the genuine row the "duplicate",
                // so the genuine row is the one skipped here -- at its correct
                // recorded ordinal. Without recording it, the required-layer
                // loop would report a load-order violation for a row that is
                // exactly where the manifest says it should be.
                refused_layers.insert((layer.recorded_ordinal, layer.layer_id.as_str()));
                continue;
            }
            local_layers.insert((layer.recorded_ordinal, layer.layer_id.as_str()), layer);
        }
        for expected in self
            .ordered_layers
            .iter()
            .filter(|layer| layer.required_for_playback)
        {
            let subject = format!("layer:{}", expected.layer_id);
            let Some(actual) = local_layers.get(&(expected.ordinal, expected.layer_id.as_str()))
            else {
                if refused_layers.contains(&(expected.ordinal, expected.layer_id.as_str())) {
                    // Present at the recorded ordinal, already refused above with
                    // its own typed issue. Report the real defect once.
                    continue;
                }
                if playback
                    .layers
                    .iter()
                    .any(|layer| layer.layer_id == expected.layer_id)
                {
                    issues.push(issue(
                        ContentIssueCode::LayerOrderMismatch,
                        subject,
                        format!(
                            "required recorded ordinal {} is not preserved",
                            expected.ordinal
                        ),
                    ));
                } else {
                    issues.push(issue(
                        ContentIssueCode::MissingLayer,
                        subject,
                        "required content layer is unavailable".into(),
                    ));
                }
                continue;
            };
            // Ordering needs no separate check here: validate() (run at the
            // top of validate_playback) enforces strictly increasing manifest
            // ordinals, and the (ordinal, layer_id) lookup key pins
            // actual.recorded_ordinal to the manifest ordinal. A playback
            // layer that does not preserve its exact recorded ordinal is
            // already refused via the LayerOrderMismatch branch above.
            if actual.layer_kind != expected.layer_kind
                || actual.storage_kind != expected.storage_kind
                || actual.descriptor_sha256 != expected.descriptor_sha256
                || actual.source_container_sha256 != expected.source_container_sha256
            {
                issues.push(issue(
                    ContentIssueCode::LayerIdentityMismatch,
                    subject.clone(),
                    "layer kind/storage/descriptor/archive identity differs".into(),
                ));
            }
            // Both sides normalize with error propagation, so nothing is ever
            // silently dropped from the comparison. Neither Err arm can fire on
            // this path today — a playback layer only reaches local_layers after
            // the ambient loop accepted its replace_paths (including the
            // duplicate-after-normalization refusal for e.g. "gfx/entities" +
            // "GFX\\ENTITIES"), and the manifest side was accepted by
            // self.validate() at the top of validate_playback. They are kept as
            // fail-closed residuals: if either pre-validation is ever narrowed,
            // the comparison still refuses rather than comparing a set that
            // silently collapsed.
            match (
                unique_normalized_path_set(&actual.replace_paths),
                normalized_path_set(&expected.replace_paths),
            ) {
                (Ok(actual_paths), Ok(expected_paths)) if actual_paths == expected_paths => {}
                (Ok(_), Ok(_)) => issues.push(issue(
                    ContentIssueCode::ReplacePathMismatch,
                    subject,
                    "replace_path declarations differ".into(),
                )),
                (Err(message), _) | (_, Err(message)) => issues.push(issue(
                    ContentIssueCode::ReplacePathMismatch,
                    subject,
                    message,
                )),
            }
        }

        let expected_layer_keys: BTreeSet<_> = self
            .ordered_layers
            .iter()
            .filter(|layer| layer.required_for_playback)
            .map(|layer| (layer.ordinal, layer.layer_id.as_str()))
            .collect();
        let selection_layers = playback
            .layers
            .iter()
            .filter(|layer| {
                expected_layer_keys.contains(&(layer.recorded_ordinal, layer.layer_id.as_str()))
            })
            .map(|layer| SelectionLayerIdentity {
                ordinal: layer.recorded_ordinal,
                layer_kind: layer.layer_kind,
                layer_id: layer.layer_id.clone(),
                descriptor_sha256: layer.descriptor_sha256.clone(),
                source_container_sha256: layer.source_container_sha256.clone(),
                storage_kind: layer.storage_kind,
                replace_paths: layer.replace_paths.clone(),
            })
            .collect::<Vec<_>>();
        let expected_dlc_ids: BTreeSet<_> = self
            .dlc_selection
            .iter()
            .filter(|dlc| dlc.required_for_playback)
            .map(|dlc| dlc.dlc_id.as_str())
            .collect();
        let selection_dlcs = playback
            .dlc_selection
            .iter()
            .filter(|dlc| expected_dlc_ids.contains(dlc.dlc_id.as_str()))
            .map(|dlc| SelectionDlcIdentity {
                dlc_id: dlc.dlc_id.clone(),
                enabled: dlc.enabled,
            })
            .collect::<Vec<_>>();
        let selection_settings = self
            .required_settings
            .keys()
            .filter_map(|key| {
                playback
                    .settings
                    .get(key)
                    .map(|value| (key.clone(), value.clone()))
            })
            .collect::<BTreeMap<_, _>>();
        match compute_selection_sha256(
            &self.effective_vfs_manifest_sha256,
            &selection_layers,
            &selection_dlcs,
            &playback.localisation_language,
            &selection_settings,
        ) {
            Ok(actual) if actual == self.source_profile.selection_sha256 => {}
            Ok(actual) => issues.push(issue(
                ContentIssueCode::ProfileMismatch,
                "source_profile.selection_sha256",
                format!(
                    "portable selection digest is {}, playback resolves {}",
                    self.source_profile.selection_sha256, actual
                ),
            )),
            // A digest that CANNOT BE COMPUTED is not a profile mismatch. The
            // inputs are the caller's own layers/DLC/language/settings, so the
            // defect is in the local inventory, and calling it "wrong launcher
            // profile/playset" points the operator at a playset that may be
            // perfectly correct.
            Err(message) => issues.push(issue(
                ContentIssueCode::InvalidPlaybackInventory,
                "source_profile.selection_sha256",
                message,
            )),
        }

        let mut local_dlcs = BTreeMap::new();
        for dlc in &playback.dlc_selection {
            if dlc.dlc_id.trim().is_empty() {
                issues.push(issue(
                    ContentIssueCode::InvalidPlaybackInventory,
                    "playback inventory",
                    "DLC id is blank".into(),
                ));
                continue;
            }
            if local_dlcs.contains_key(dlc.dlc_id.as_str()) {
                issues.push(issue(
                    ContentIssueCode::InvalidPlaybackInventory,
                    "playback inventory",
                    format!("duplicate DLC row: {}", dlc.dlc_id),
                ));
                continue;
            }
            local_dlcs.insert(dlc.dlc_id.as_str(), dlc.enabled);
        }
        for expected in self
            .dlc_selection
            .iter()
            .filter(|dlc| dlc.required_for_playback)
        {
            if local_dlcs.get(expected.dlc_id.as_str()).copied() != Some(expected.enabled) {
                issues.push(issue(
                    ContentIssueCode::RequiredDlcMismatch,
                    format!("dlc:{}", expected.dlc_id),
                    format!("required enabled state is {}", expected.enabled),
                ));
            }
        }

        let mut resolved = BTreeMap::new();
        // Same reason as refused_layers: a resolved row refused as malformed is
        // not "missing", and MissingContent would send the operator after an
        // absent file that is in fact present and already reported as invalid.
        let mut refused_rows: BTreeSet<String> = BTreeSet::new();
        // PROVEN identity (windows_same_file_identity) -> the canonical spelling
        // that claimed it, over every row the caller sent, admitted or refused.
        // This is what decides absence, and it deliberately never over-folds.
        let mut same_file_rows: BTreeMap<String, String> = BTreeMap::new();
        // Collision key -> the spelling that claimed it, over EVERY row the
        // caller sent, admitted or refused. A match here without a match in
        // same_file_rows is the undetermined case: some volume's own case table
        // may fold the two spellings together and some may not.
        let mut near_miss_rows: BTreeMap<String, String> = BTreeMap::new();
        // Rows the collision key could not fold at all. They are in neither map
        // above -- there is no key to put them under -- so the absence decision
        // has to consult them separately or it calls a path missing that one of
        // them may well be naming.
        let mut undecidable_rows: Vec<String> = Vec::new();
        // Windows collision key -> the canonical spelling that claimed it. This
        // one DOES over-fold, so it may only refuse, never prove presence.
        // Separate from `resolved` because `resolved` is looked up by the
        // manifest's RECORDED path, which must stay an exact-identity lookup.
        let mut resolved_collisions: BTreeMap<String, String> = BTreeMap::new();
        for content in &playback.resolved_content {
            let key = match normalize_game_relative_path(&content.normalized_game_relative_path) {
                Ok(key) if key == content.normalized_game_relative_path => key,
                Ok(key) => {
                    issues.push(issue(
                        ContentIssueCode::InvalidPlaybackInventory,
                        "playback inventory",
                        format!(
                            "resolved path is not normalized: {:?} (canonical {:?})",
                            content.normalized_game_relative_path, key
                        ),
                    ));
                    // The row is refused, but it may still NAME a required file.
                    // That question is asked of the RAW spelling, never of `key`:
                    // normalize_game_relative_path trims the whole string, and a
                    // LEADING space is not something Win32 strips (measured: `
                    // e.txt` and `e.txt` are two files). Going through `key`
                    // would let " gfx/x" vouch for "gfx/x" and suppress a real
                    // absence.
                    if let Some(identity) =
                        refused_content_identity(&content.normalized_game_relative_path)
                    {
                        note_row_identity(&mut same_file_rows, &mut near_miss_rows, &mut undecidable_rows, &identity);
                        refused_rows.insert(identity);
                    }
                    continue;
                }
                Err(message) => {
                    issues.push(issue(
                        ContentIssueCode::InvalidPlaybackInventory,
                        "playback inventory",
                        message,
                    ));
                    // A spelling this normalizer refuses can still NAME a
                    // required file: "gfx/./entities/unit.asset" is rejected for
                    // its dot segment, yet Windows resolves it to the canonical
                    // "gfx/entities/unit.asset". Recover that identity for the
                    // absence check only. None means the spelling cannot name
                    // anything inside the game root, so "missing" stays true.
                    if let Some(identity) =
                        refused_content_identity(&content.normalized_game_relative_path)
                    {
                        // Through windows_same_file_identity, so a spelling that
                        // combines a dot component with a trailing dot --
                        // "gfx/./models/unit.mesh." -- is still recognised as
                        // the required "gfx/models/unit.mesh" and does not leave
                        // a present file reported absent.
                        note_row_identity(&mut same_file_rows, &mut near_miss_rows, &mut undecidable_rows, &identity);
                        refused_rows.insert(identity);
                    }
                    continue;
                }
            };
            let mut invalid = false;
            if let Err(message) = validate_portable_layer_id(&content.layer_id) {
                issues.push(issue(
                    ContentIssueCode::InvalidPlaybackInventory,
                    "playback inventory",
                    message,
                ));
                invalid = true;
            }
            for (hash, label) in [
                (&content.source_sha256, "playback content source"),
                (&content.payload_sha256, "playback content payload"),
            ] {
                if let Err(message) = validate_sha256(hash, label) {
                    issues.push(issue(
                        ContentIssueCode::InvalidPlaybackInventory,
                        "playback inventory",
                        message,
                    ));
                    invalid = true;
                }
            }
            if content.payload_bytes > MAX_CONTENT_PAYLOAD_BYTES {
                issues.push(issue(
                    ContentIssueCode::PayloadLengthMismatch,
                    key.clone(),
                    format!(
                        "resolved payload exceeds the {MAX_CONTENT_PAYLOAD_BYTES}-byte ceiling: {}",
                        content.payload_bytes
                    ),
                ));
                invalid = true;
            }
            // Two rows are "the same row" when the playback host would open the
            // same file, which is the collision key, not the canonical form.
            // An undecidable spelling is refused rather than admitted as
            // distinct; either way the row is recorded as refused-not-absent.
            let collision = match windows_collision_key(&key) {
                Ok(collision) => collision,
                Err(message) => {
                    issues.push(issue(
                        ContentIssueCode::InvalidPlaybackInventory,
                        "playback inventory",
                        message,
                    ));
                    note_row_identity(
                        &mut same_file_rows,
                        &mut near_miss_rows,
                        &mut undecidable_rows,
                        &key,
                    );
                    refused_rows.insert(key);
                    continue;
                }
            };
            if let Some(previous) = resolved_collisions.get(&collision) {
                issues.push(issue(
                    ContentIssueCode::InvalidPlaybackInventory,
                    "playback inventory",
                    format!("duplicate resolved path row: {key} (aliases {previous})"),
                ));
                // Present under a refused spelling: the required-content loop
                // must not also announce it absent.
                note_row_identity(&mut same_file_rows, &mut near_miss_rows, &mut undecidable_rows, &key);
                refused_rows.insert(key);
                continue;
            }
            resolved_collisions.insert(collision, key.clone());
            note_row_identity(&mut same_file_rows, &mut near_miss_rows, &mut undecidable_rows, &key);
            if invalid {
                refused_rows.insert(key);
            } else {
                resolved.insert(key, content);
            }
        }
        let mut verified = 0usize;
        for required in &self.required_content {
            let expected = &required.content;
            let subject = expected.normalized_game_relative_path.clone();
            let Some(actual) = resolved.get(subject.as_str()) else {
                if refused_rows.contains(subject.as_str()) {
                    // The caller DID send this exact path and it was refused
                    // above with its own typed issue -- most often because an
                    // aliasing row claimed the slot first. Report the real
                    // defect once, and in particular do not go on to call the
                    // exactly-recorded spelling a "different spelling".
                    continue;
                }
                // ABSENCE is otherwise decided on the PROVEN identity, never on
                // the over-folding collision key. A row may answer "this file is
                // present" only if it is the same file for certain.
                let proven = windows_same_file_identity(subject.as_str())
                    .and_then(|identity| same_file_rows.get(&identity).cloned());
                match proven {
                    // Present under this exact path, or under a spelling
                    // measured to be the same file, and already refused above
                    // with its own typed issue. Report the real defect once.
                    Some(spelling) if refused_rows.contains(&spelling) => {}
                    // Present and admissible in itself, but not spelled as
                    // recorded. It is NOT substituted -- admission goes by the
                    // recorded identity -- yet it is plainly not absent either.
                    Some(spelling) => issues.push(issue(
                        ContentIssueCode::InvalidPlaybackInventory,
                        subject,
                        format!(
                            "resolved row {spelling:?} is this required file under a different Windows-equivalent spelling; the inventory must use the recorded spelling"
                        ),
                    )),
                    None => {
                        // Sameness could not be PROVED. Absence is a claim too,
                        // so it needs proof of its own: that no supported
                        // playback volume would fold these two spellings
                        // together. The widest relation available is the
                        // collision key, whose case fold unions this host's
                        // table with Unicode's simple uppercase (see the OPEN
                        // ITEM on HOST_UPCASE about what that does and does not
                        // establish). So:
                        //
                        //   collision keys DIFFER -> no volume folds them, the
                        //     required file is genuinely absent, say so;
                        //   collision keys MATCH  -> some volume's table may
                        //     fold them and some may not, and this code has read
                        //     none of those tables. Neither MissingContent nor a
                        //     presence claim is TRUE, so it makes neither and
                        //     reports the uncertainty instead. The replay still
                        //     stays refused, on the spelling mismatch alone.
                        //
                        // This deliberately does NOT try to split the match case
                        // into "this host's table folds it" and "only Unicode
                        // folds it". That split needs the $UpCase tables of every
                        // supported Windows version, and a table can gain OR LOSE
                        // a mapping between versions -- final sigma folds to
                        // capital sigma under the XP and Vista tables and is
                        // reset to itself in later ones -- so this host's table
                        // is not even an upper bound on the others. Merging the
                        // two is the conservative reading and costs only
                        // diagnostic precision, never safety.
                        let near_miss = windows_collision_key(subject.as_str())
                            .ok()
                            .and_then(|collision| near_miss_rows.get(&collision).cloned());
                        if let Some(spelling) = near_miss {
                            if !refused_rows.contains(&spelling) {
                                issues.push(issue(
                                    ContentIssueCode::InvalidPlaybackInventory,
                                    subject,
                                    format!(
                                        "resolved row {spelling:?} differs from this required path only by characters some Windows case table folds together; NTFS folds through the $UpCase file written when the playback volume was FORMATTED, and that table has changed across Windows versions, so whether this file is present cannot be decided here -- it is neither admitted nor reported absent"
                                    ),
                                ));
                            }
                            continue;
                        }
                        // Last: a row the collision key could not fold AT ALL
                        // is in neither index, and "no key" is not "no match".
                        // `gfx/conten~1/unit.asset` is refused because the long
                        // name behind `CONTEN~1` is the filesystem's to assign
                        // -- which is exactly why it may be naming this very
                        // path. Calling the path missing would be a claim the
                        // refusal itself says cannot be made.
                        if undecidable_rows
                            .iter()
                            .any(|row| undecidable_row_could_name(row, subject.as_str()))
                        {
                            issues.push(issue(
                                ContentIssueCode::InvalidPlaybackInventory,
                                subject,
                                "a resolved row was refused as undecidable (8.3 short-name shape) and could be naming this required path; whether this file is present cannot be decided here, so it is neither admitted nor reported absent".into(),
                            ));
                            continue;
                        }
                        issues.push(issue(
                            ContentIssueCode::MissingContent,
                            subject,
                            format!("required by {}", required.consumers.join(", ")),
                        ));
                    }
                }
                continue;
            };
            let before = issues.len();
            if actual.winning_layer_ordinal != expected.winning_layer_ordinal
                || actual.layer_kind != expected.layer_kind
                || actual.layer_id != expected.layer_id
            {
                issues.push(issue(
                    ContentIssueCode::WinningLayerMismatch,
                    subject.clone(),
                    "effective VFS selected a different winning layer".into(),
                ));
            }
            compare_scalar(
                &mut issues,
                ContentIssueCode::SourceHashMismatch,
                &subject,
                &expected.source_sha256,
                &actual.source_sha256,
            );
            compare_scalar(
                &mut issues,
                ContentIssueCode::PayloadHashMismatch,
                &subject,
                &expected.payload_sha256,
                &actual.payload_sha256,
            );
            if actual.payload_bytes != expected.payload_bytes {
                issues.push(issue(
                    ContentIssueCode::PayloadLengthMismatch,
                    subject,
                    format!(
                        "required {} bytes, playback resolved {} bytes",
                        expected.payload_bytes, actual.payload_bytes
                    ),
                ));
            }
            if issues.len() == before {
                verified += 1;
            }
        }
        ContentValidationReport {
            compatible: issues.is_empty(),
            verified_required_content: verified,
            issues,
        }
    }

    pub fn admit_playback(
        &self,
        playback: &PlaybackContentInventory,
    ) -> Result<ValidatedContentAdmission, ContentValidationReport> {
        let report = self.validate_playback(playback);
        if !report.compatible {
            return Err(report);
        }
        let manifest_digest_sha256 = match &self.manifest_digest_sha256 {
            Some(digest) => digest.clone(),
            None => self
                .computed_digest()
                .map_err(|message| ContentValidationReport {
                    compatible: false,
                    verified_required_content: 0,
                    issues: vec![issue(
                        ContentIssueCode::InvalidManifest,
                        "manifest",
                        message,
                    )],
                })?,
        };
        let mut required_by_consumer: BTreeMap<String, Vec<UsedContentKey>> = BTreeMap::new();
        for required in &self.required_content {
            for consumer in &required.consumers {
                required_by_consumer
                    .entry(consumer.clone())
                    .or_default()
                    .push(required.content.clone());
            }
        }
        for content in required_by_consumer.values_mut() {
            content.sort();
            content.dedup();
        }
        Ok(ValidatedContentAdmission {
            manifest_digest_sha256,
            product_modset_sha256: self.product_modset_sha256.clone(),
            ordered_layers: self.ordered_layers.clone(),
            localisation_language: self.localisation_language.clone(),
            required_by_path: self
                .required_content
                .iter()
                .map(|required| {
                    (
                        required.content.normalized_game_relative_path.clone(),
                        required.content.clone(),
                    )
                })
                .collect(),
            required_by_consumer,
        })
    }
}

/// Pointer-free identity for one ordered content layer.
pub fn content_layer_identity_sha256(layer: &RecordedContentLayer) -> Result<String, String> {
    let canonical = serde_json::json!({
        "schema": "hoi4.portable-content-layer-identity.v1",
        "layer": layer,
    });
    let bytes = serde_json::to_vec(&canonical).map_err(|err| err.to_string())?;
    Ok(format!("{:x}", Sha256::digest(bytes)))
}

/// THE ONE RULE for turning `launcher-settings.json`'s version text into the
/// `(version, build)` pair that [`SourceGameIdentity`] carries, shared by the
/// recorder that writes the manifest and the viewer that checks it against a
/// local install.
///
/// It lives here because the two sides JOIN on the result. They did not: the
/// attach-time census route stamped the launcher's display string
/// (`"Operation Postern v1.19.2.0.a729 (d245)"`) and a route label
/// (`"hoi4-attach-time-effective-vfs-census-v1"`), while the viewer parsed the
/// same file into `"1.19.2.0"` / `"a729"` / `"hoi4-1.19.2.0-a729"` — so a
/// recording refused to open against the very install that produced it, over
/// two different spellings of one agreed fact (measured on t82).
///
/// `full` is `gameVersion`/`version`; `raw` is the optional `rawVersion`. The
/// version is the first four-part dotted numeral found in either; the build is
/// the first four-character alphanumeric token in `full` that mixes letters and
/// digits (`a729`), lowercased.
///
/// `rawVersion` is OPTIONAL, so `full` alone has to yield the version. In the
/// shipped spelling the version and the build are one dotted run
/// (`v1.19.2.0.a729`), and splitting on "not a digit and not a dot" leaves the
/// separating dot attached: `"1.19.2.0."`. Requiring exactly four pieces
/// refused that token, so a launcher file without `rawVersion` refused
/// altogether even though the version was right there (Codex review round 1).
/// A candidate therefore matches when it has exactly four dot-pieces, or
/// exactly five whose fifth is empty, and the first four are non-empty
/// numerals. That accepts `1.19.2.0` and the `1.19.2.0.` left by a `.a729`
/// suffix, and rejects both a five-part numeral such as `1.19.2.0.5` and a RUN
/// of separating dots such as `1.19.2.0..a729`, which is malformed metadata
/// rather than a version (Codex review round 2).
pub fn parse_launcher_version_and_build(
    full: &str,
    raw: Option<&str>,
) -> Result<(String, String), String> {
    fn four_part_version(part: &str) -> Option<String> {
        let pieces = part.split('.').collect::<Vec<_>>();
        // Exactly four numeric pieces, or exactly five where the fifth is the
        // empty tail left by the single separating dot before the build token
        // (`1.19.2.0` + `.a729` -> `"1.19.2.0."`). A RUN of dots is malformed
        // and stays refused: allowing every trailing piece to be empty turned
        // `1.19.2.0..a729` into a valid version, which the strict rule had
        // rejected (Codex review round 2).
        if pieces.len() != 4 && pieces.len() != 5 {
            return None;
        }
        if pieces.len() == 5 && !pieces[4].is_empty() {
            return None;
        }
        if !pieces[..4]
            .iter()
            .all(|piece| !piece.is_empty() && piece.bytes().all(|byte| byte.is_ascii_digit()))
        {
            return None;
        }
        Some(pieces[..4].join("."))
    }
    let version = [Some(full), raw]
        .into_iter()
        .flatten()
        .find_map(|value| {
            value
                .split(|ch: char| !(ch.is_ascii_digit() || ch == '.'))
                .find_map(four_part_version)
        })
        .ok_or_else(|| format!("launcher metadata has no four-part the game version: {full:?}"))?;
    let build = full
        .split(|ch: char| !ch.is_ascii_alphanumeric())
        .filter(|part| part.len() == 4)
        .find(|part| {
            part.chars().any(|ch| ch.is_ascii_alphabetic())
                && part.chars().any(|ch| ch.is_ascii_digit())
        })
        .ok_or_else(|| format!("launcher metadata has no Paradox build token: {full:?}"))?
        .to_ascii_lowercase();
    Ok((version, build))
}

/// The `build_profile` spelling both sides must produce from the same pair.
pub fn build_profile_label(version: &str, build: &str) -> String {
    format!("hoi4-{version}-{build}")
}

pub fn compute_selection_sha256(
    effective_vfs_manifest_sha256: &str,
    ordered_layers: &[SelectionLayerIdentity],
    dlc_selection: &[SelectionDlcIdentity],
    localisation_language: &str,
    settings: &BTreeMap<String, String>,
) -> Result<String, String> {
    validate_sha256(
        effective_vfs_manifest_sha256,
        "selection effective VFS manifest",
    )?;
    if localisation_language.trim().is_empty() {
        return Err("selection localisation language is blank".into());
    }
    let mut layers = ordered_layers.to_vec();
    layers.sort_by_key(|layer| layer.ordinal);
    let mut layer_keys = BTreeSet::new();
    for layer in &mut layers {
        validate_portable_layer_id(&layer.layer_id)?;
        if !layer_keys.insert((layer.ordinal, layer.layer_id.clone())) {
            return Err("selection layers have a blank/duplicate identity".into());
        }
        if let Some(hash) = &layer.descriptor_sha256 {
            validate_sha256(hash, "selection layer descriptor")?;
        }
        if let Some(hash) = &layer.source_container_sha256 {
            validate_sha256(hash, "selection layer archive")?;
        }
        // Duplicates-after-normalization are refused, not silently deduped:
        // otherwise two declarations collapsing into one set member would
        // still hash to the recorded selection identity.
        layer.replace_paths = unique_normalized_path_set(&layer.replace_paths)?
            .into_iter()
            .collect();
    }
    let mut dlcs = dlc_selection.to_vec();
    dlcs.sort_by(|left, right| left.dlc_id.cmp(&right.dlc_id));
    if dlcs.iter().any(|dlc| dlc.dlc_id.trim().is_empty())
        || dlcs.windows(2).any(|pair| pair[0].dlc_id == pair[1].dlc_id)
    {
        return Err("selection DLC rows have a blank/duplicate identity".into());
    }
    if settings
        .iter()
        .any(|(key, value)| key.trim().is_empty() || value.trim().is_empty())
    {
        return Err("selection settings have a blank key/value".into());
    }
    let canonical = serde_json::json!({
        "schema": "hoi4.portable-content-selection.v1",
        "effective_vfs_manifest_sha256": effective_vfs_manifest_sha256,
        "ordered_required_layers": layers,
        "required_dlc_selection": dlcs,
        "localisation_language": localisation_language,
        "required_settings": settings,
    });
    let bytes = serde_json::to_vec(&canonical).map_err(|err| err.to_string())?;
    Ok(format!("{:x}", Sha256::digest(bytes)))
}

pub fn normalize_game_relative_path(path: &str) -> Result<String, String> {
    let normalized = path.trim().replace('\\', "/").to_ascii_lowercase();
    if normalized.is_empty()
        || normalized.starts_with('/')
        || normalized.contains(':')
        || normalized.split('/').any(|part| part == "." || part == "..")
    {
        return Err(format!("unsafe/non-canonical game-relative path {path:?}"));
    }
    // Empty components are COLLAPSED, not refused, because that is what
    // `sanitizePlatformIndependentPath` does with a `//` run -- see
    // [`sanitize_engine_path`]. Refusing them here while the replay side dropped
    // them was a live disagreement between the two implementations over a path
    // the engine resolves perfectly well.
    let collapsed = normalized
        .split('/')
        .filter(|part| !part.is_empty())
        .collect::<Vec<_>>()
        .join("/");
    if collapsed.is_empty() {
        return Err(format!("unsafe/non-canonical game-relative path {path:?}"));
    }
    Ok(collapsed)
}

/// The engine's own path grammar, applied to a game-relative path before it is
/// used to name a file.
///
/// From `sanitizePlatformIndependentPath` (`0x1424e3ff0`) in `hoi4.exe
/// 1.19.2.0.a729` / `870ebf1f615262facca48a3b947775c41f0a5338f3068c40958209f5c41c65ad`
/// (full working:
/// `extractor/native/runs/lf_vfs_consolidate/engine_path_namespace_from_a729.md`):
///
/// * leading `/` runs are stripped;
/// * `:` or `\` ANYWHERE fails with `PHYSFS_ERR_BAD_FILENAME` (error `0x17`) --
///   the engine does not translate a backslash into a separator;
/// * a component that is exactly `.` or `..` fails with the same error;
/// * consecutive `/` collapse -- empty components disappear, they do not fail;
/// * a trailing `/` ends the path successfully;
/// * every other byte is copied VERBATIM. The engine folds nothing here.
///
/// This is the grammar the LOOKUP must apply, and it is applied inside
/// [`effective_vfs::resolve_literal_layer_path`] so that both layered-VFS
/// implementations get it whatever their own entry validation did first.
///
/// ONE DELIBERATE DEPARTURE, identical on both sides so it cannot make them
/// disagree: the two callers translate `\` to `/` in their own entry validation
/// before this ever runs, because a game-relative path reaching this repository
/// can come from a host API that spells separators the Windows way, which is not
/// the situation the engine's sanitizer is in. Recorded as a departure rather
/// than silently called parity.
pub fn sanitize_engine_path(path: &str) -> Result<String, String> {
    if path.contains(':') || path.contains('\\') {
        return Err(format!(
            "engine path grammar rejects {path:?}: ':' and '\\' are PHYSFS_ERR_BAD_FILENAME"
        ));
    }
    let mut parts = Vec::new();
    for part in path.split('/') {
        if part.is_empty() {
            continue;
        }
        if part == "." || part == ".." {
            return Err(format!(
                "engine path grammar rejects {path:?}: {part:?} is a component the engine refuses"
            ));
        }
        parts.push(part);
    }
    if parts.is_empty() {
        return Err(format!("engine path grammar rejects {path:?}: it names nothing"));
    }
    Ok(parts.join("/"))
}

/// Every DOS device name Win32 still resolves ahead of a same-named file.
/// Measured on Windows 11 build 26200: `gfx/nul` opened the NUL device (the
/// write succeeded, the read-back was empty, and no directory entry appeared)
/// while `gfx/con`, `gfx/aux`, `gfx/prn`, `gfx/com1`, `gfx/lpt1` were ordinary
/// files on that build. Which of these bind to a device is therefore a property
/// of the PLAYBACK HOST, not of the manifest, so the whole documented set is
/// treated alike here.
const DOS_DEVICE_NAMES: [&str; 32] = [
    "CON", "PRN", "AUX", "NUL", "COM0", "COM1", "COM2", "COM3", "COM4", "COM5", "COM6", "COM7",
    "COM8", "COM9", "LPT0", "LPT1", "LPT2", "LPT3", "LPT4", "LPT5", "LPT6", "LPT7", "LPT8", "LPT9",
    "CONIN$", "CONOUT$",
    // MSDN reserves the superscript-digit spellings alongside the ASCII ones.
    // They survive `simple_upper_bmp` unchanged, so they need their own entries.
    "COM\u{b9}", "COM\u{b2}", "COM\u{b3}", "LPT\u{b9}", "LPT\u{b2}", "LPT\u{b3}",
];

/// Validation-only key under which two game-relative paths are the SAME file on
/// a Windows/NTFS playback host.
///
/// This exists because [`normalize_game_relative_path`]'s canonical form is a
/// RECORDED IDENTITY — it feeds [`compute_selection_sha256`] and every stored
/// `normalized_game_relative_path` — so it can never be widened without
/// invalidating digests that already verify. But that canonical form
/// under-approximates what NTFS treats as one file, so two canonical spellings
/// can still name the same byte stream. Collision/duplicate rules use THIS key;
/// nothing derived from it ever reaches a digest, a stored path, or an
/// admission.
///
/// FAIL-CLOSED CONTRACT. A missed collision plays back the wrong file; a false
/// collision only refuses a replay. So every rule here is allowed to fold MORE
/// than NTFS does, and a component this function cannot decide is an `Err`,
/// which every caller turns into a refusal. Never add a rule that folds LESS.
///
/// That applies to the uncertainty about the volume's own case table too. NTFS
/// folds through the `$UpCase` file written when the volume was FORMATTED, and
/// that table has changed across Windows versions, so for some pairs no static
/// answer is correct everywhere. HERE that is resolved toward collision: the
/// case fold below is [`HOST_UPCASE`] unioned with Unicode's simple uppercase,
/// which covers both directions for every mapping either source knows, so a pair
/// that folds under either is treated as one file and refused. Whether that
/// covers EVERY supported volume table is the OPEN ITEM recorded at
/// [`HOST_UPCASE`], not something the evidence here establishes. The opposite
/// resolution -- reporting a required path ABSENT because a case table might not
/// fold it -- is made by [`windows_same_file_identity`] and by the absence arm of
/// `validate_playback`, which are separate from this key for exactly that
/// reason.
///
/// What is folded, and the evidence (all measured on Windows 11 build 26200 in
/// `extractor/native/runs/win_alias_key/fs_aliasing_probe.txt`):
///
/// * **Case, beyond ASCII.** Writing `A.TXT` overwrote `a.txt`; the same held
///   for `Ą`/`ą`, `É`/`é`, Cyrillic `Г`/`г`, Greek `Σ`/`σ`, and fullwidth
///   `Ａ`/`ａ`. `to_ascii_lowercase` folds none of the non-ASCII pairs. NTFS
///   folds through the volume's 16-bit `$UpCase` table, so this uses the SIMPLE
///   (1:1) Unicode uppercase mapping restricted to the BMP: a multi-character
///   expansion (`ß`→`SS`, `ﬁ`→`FI`) keeps the original character, matching the
///   measurement that `straße.txt`/`strasse.txt` and `ﬁx.txt`/`fix.txt` are
///   DISTINCT files, and astral characters are left alone because `$UpCase`
///   cannot address them (measured: two Deseret letters stayed distinct).
/// * **Trailing dots and spaces, per component.** Win32 strips them
///   (MSDN "Naming Files, Paths, and Namespaces": do not end a name with a
///   space or a period). Measured: `b.txt.`, `c.txt...` and `d.txt ` all
///   overwrote `b/c/d.txt`, and the interior form `gfx./u.asset` opened
///   `gfx/u.asset`. The one measured non-alias is an interior trailing SPACE
///   (`gfx /u.asset` failed with ERROR_PATH_NOT_FOUND, because a directory
///   named `gfx ` cannot be created in the first place) — folding it anyway
///   costs nothing, since that spelling can never name an existing file.
/// * **DOS device names**, see [`DOS_DEVICE_NAMES`]: every extension of a
///   reserved stem is folded onto one token, because on a host where the stem
///   binds to a device, `gfx/con`, `gfx/con.txt` and `gfx/con.tar.gz` all reach
///   the same device rather than three files.
///
/// What is deliberately NOT folded, because NTFS does not fold it either
/// (measured, same file): Unicode normalization forms (`café` NFC vs NFD stayed
/// two files), Turkish dotless `ı` vs `I`, KELVIN SIGN vs `k`, `ς` vs `σ`,
/// MICRO SIGN vs `μ`, ANGSTROM SIGN vs `å`. Folding these would only add false
/// collisions; the simple-uppercase rule above already over-folds a few of them
/// (`ı`, `ς`, MICRO SIGN), which is the safe direction.
///
/// Undecidable, therefore refused:
///
/// * **8.3 short names** (`CONTEN~1`). `dir /x` on this worktree shows live 8.3
///   aliases, so the volume generates them — but which long name a `~N`
///   component denotes is assigned by the filesystem from directory creation
///   order and is not derivable from the manifest. The key cannot decide it, so
///   it refuses rather than guess.
/// * A component that is nothing but dots and spaces, which names no file once
///   Win32 has stripped them.
pub fn windows_collision_key(canonical_game_relative_path: &str) -> Result<String, String> {
    let mut folded: Vec<String> = Vec::new();
    for component in canonical_game_relative_path.split('/') {
        folded.push(fold_windows_component(
            component,
            canonical_game_relative_path,
        )?);
    }
    Ok(folded.join("/"))
}

/// What one path component folds to, or why it cannot be folded. The two
/// failure cases are NOT interchangeable and callers must not treat them alike:
/// an 8.3 shape is a possible alias of ANY long name in its directory, while a
/// dots-and-spaces component names nothing at all.
enum ComponentFold {
    Folded(String),
    /// `CONTEN~1`. The long name it stands for is assigned by the filesystem
    /// from directory creation order, so this component could be any of them.
    ShortNameAlias,
    /// `...`, `   `. Win32 strips the lot and nothing is left.
    NamesNothing,
}

fn classify_windows_component(component: &str) -> ComponentFold {
    let stripped = component.trim_end_matches([' ', '.']);
    if stripped.is_empty() {
        return ComponentFold::NamesNothing;
    }
    let upper = simple_upper_bmp(stripped);
    if is_short_name_shaped(&upper) {
        return ComponentFold::ShortNameAlias;
    }
    let stem = upper
        .split('.')
        .next()
        .unwrap_or(upper.as_str())
        .trim_end_matches(' ');
    if let Some(device) = DOS_DEVICE_NAMES.iter().find(|name| **name == stem) {
        return ComponentFold::Folded(format!("<dos-device:{device}>"));
    }
    ComponentFold::Folded(upper)
}

fn fold_windows_component(component: &str, whole: &str) -> Result<String, String> {
    match classify_windows_component(component) {
        ComponentFold::Folded(folded) => Ok(folded),
        ComponentFold::ShortNameAlias => Err(format!(
            "path component {component:?} of {whole:?} has the 8.3 short-name shape; the long name it aliases is assigned by the filesystem and cannot be decided here"
        )),
        ComponentFold::NamesNothing => Err(format!(
            "path component {component:?} of {whole:?} is only dots/spaces, which Win32 strips: it names no file"
        )),
    }
}

/// Is this component unreachable BECAUSE OF ITS POSITION?
///
/// [`classify_windows_component`] cannot answer that: it strips trailing spaces
/// because Win32 does, which is right for a collision key. But Win32 strips them
/// from the FINAL component only. Measured: `gfx /u.asset` fails with
/// ERROR_PATH_NOT_FOUND while `gfx/u.asset ` opens the file, and
/// `CreateDirectoryW("gfx ")` creates a directory actually named `gfx` -- so a
/// directory named `gfx ` cannot exist and nothing can live under it.
///
/// The position-independent case (a component that is only dots and spaces) is
/// [`ComponentFold::NamesNothing`] and is handled by the match in
/// [`undecidable_row_could_name`] instead.
fn component_is_unreachable_interior(component: &str, is_last: bool) -> bool {
    !is_last && component.trim_end_matches('.').ends_with(' ')
}

/// Could the 8.3 component `short` be the generated short name of `long`?
///
/// `~N` is not a wildcard, but it is far more permissive than it looks, and the
/// difference matters in both directions: ruling out a name the generator COULD
/// have produced announces a present file absent, while accepting one it could
/// not suppresses a real absence.
///
/// Measured with `dir /x` over names created for the purpose (section 6 of
/// `extractor/native/runs/win_alias_key/fs_aliasing_probe.txt`):
///
/// ```text
///   Cargo.toml             -> CARGO~1.TOM     extension = first 3 of "toml"
///   Cargo.lock             -> CARGO~1.LOC
///   definitely_absent.lock -> DEFINI~1.LOC
///   unit.tar.gz            -> UNITTA~1.GZ     the LAST extension, kept whole
///   longbasename.a         -> LONGBA~1.A
///   noext_longname         -> NOEXT_~1        no extension either side
///   short.a, UPPER.TXT     -> no short name at all
/// ```
///
/// THE BASE CANNOT BE CONSTRAINED. It looked like a prefix of the long base, and
/// it is not:
///
/// ```text
///   a+b_longname.txt  -> A_B_LO~1.TXT   '+' becomes '_' (also = , ; [ ])
///   a b longname.txt  -> ABLONG~1.TXT   spaces dropped
///   a[b]longname.txt  -> A_A9AC~1.TXT   rewritten around a checksum
///   Longfilename5.txt -> LOC254~1.TXT   after four collisions the generator
///   Longfilename6.txt -> LO0D73~1.TXT   switches to 2 characters + a checksum
///   {u-umlaut}nitmodel.mesh -> NITMOD~1.MES   the leading character is DROPPED
/// ```
///
/// A prefix test rejects every one of those, and each is a real name of a real
/// long file. So the base is not RELATED to the long base here, deliberately:
/// over-accepting only ever withholds a `MissingContent`, while under-accepting
/// invents one.
///
/// That is not the same as accepting any string at all. The candidate must still
/// be a name the generator could have emitted -- no character Win32 reserves,
/// and a plain decimal sequence starting at 1 -- or it names nothing and cannot
/// excuse anything.
///
/// THE EXTENSION CAN BE, but only where the same translation cannot reach it:
/// `longfile.a+b -> LONGFI~1.A_B` shows the extension is rewritten too. So the
/// rule below applies only to an ASCII ALPHANUMERIC long extension -- which is
/// every extension this content actually uses -- and there it is exact rather
/// than a prefix: the generator takes the first `min(3, len)` characters, so
/// `cargo~1.t` is not a possible name for `cargo.toml` either. Presence of an
/// extension must also agree on both sides, so `cargo~1` cannot be `cargo.toml`.
fn short_name_could_stand_for(short: &str, long: &str) -> bool {
    // Win32 strips trailing dots and spaces, and the alias question has to be
    // asked of what the spelling NAMES, not of how it was typed. `cargo~1.tom.`
    // names the real alias `CARGO~1.TOM`; splitting it unstripped would find an
    // empty final suffix, call it extensionless, and rule out a present file.
    // (An interior component with a trailing space has already been rejected by
    // `component_is_unreachable_interior`, so stripping here is safe in both
    // positions.)
    let short = short.trim_end_matches([' ', '.']);
    let long = long.trim_end_matches([' ', '.']);

    // Whatever the base is, the candidate must be a name the generator COULD
    // have emitted. "Unrelated to the long base" is not "any string at all":
    // `*~1.tom` cannot be an 8.3 alias of anything, because `*` cannot occur in
    // a Windows filename at all (MSDN "Naming Files, Paths, and Namespaces"
    // reserves < > : " / \ | ? * and 0x00-0x1f). A row spelled that way names
    // nothing and must not excuse an absence.
    if short
        .chars()
        .any(|ch| matches!(ch, '<' | '>' | ':' | '"' | '|' | '?' | '*') || ch <= '\u{1f}')
    {
        return false;
    }

    let (short_base, short_extension) = split_last_extension(short);
    let (long_base, long_extension) = split_last_extension(long);
    let _ = long_base;
    // A long name with an extension always gets a short name with one, and one
    // without never does.
    if short_extension.is_some() != long_extension.is_some() {
        return false;
    }
    if let (Some(short_extension), Some(long_extension)) = (short_extension, long_extension) {
        if long_extension.chars().all(|ch| ch.is_ascii_alphanumeric()) {
            let expected: String = simple_upper_bmp(long_extension).chars().take(3).collect();
            if simple_upper_bmp(short_extension) != expected {
                return false;
            }
        }
    }
    // The sequence is written in plain decimal starting at 1, so neither `~0`
    // nor a zero-padded `~01` is anything the generator produces.
    let Some((_, tail)) = short_base.rsplit_once('~') else {
        return false;
    };
    !tail.is_empty()
        && !tail.starts_with('0')
        && tail.bytes().all(|byte| byte.is_ascii_digit())
}

/// A trailing dot leaves no extension -- Win32 strips it -- so an EMPTY
/// extension is no extension, not an extension of length zero.
fn split_last_extension(component: &str) -> (&str, Option<&str>) {
    match component.rsplit_once('.') {
        Some((base, extension)) if !base.is_empty() && !extension.is_empty() => {
            (base, Some(extension))
        }
        _ => (component, None),
    }
}

/// Could `row` -- a spelling [`windows_collision_key`] refused as undecidable --
/// be a name for `subject`?
///
/// `true` is the conservative answer: it only ever withholds a `MissingContent`
/// from a path the caller may in fact have supplied, and it never admits
/// anything -- the row has already been refused with its own typed issue. But it
/// must not be a blanket yes, or one bad row would excuse every absence in the
/// manifest. Three things bound it:
///
/// * a short name replaces exactly ONE component, so the component counts must
///   match;
/// * a component that names nothing, in either path, means that path names
///   nothing, so it can match nothing (this is why the check is POSITION-aware
///   and does not simply reuse the collision key, which strips an interior
///   trailing space that the filesystem does not);
/// * a `~N` component is constrained by the little the generator provably
///   preserves -- see [`short_name_could_stand_for`], and note that the BASE is
///   not one of those things.
///
/// Every other component is compared with the collision key: a case near-miss is
/// "possibly the same", which is the right standard here, since the question is
/// what the row COULD name rather than what it certainly does.
fn undecidable_row_could_name(row: &str, subject: &str) -> bool {
    let row_parts: Vec<&str> = row.split('/').collect();
    let subject_parts: Vec<&str> = subject.split('/').collect();
    if row_parts.len() != subject_parts.len() {
        return false;
    }
    let last = row_parts.len() - 1;
    for (index, (row_part, subject_part)) in row_parts.iter().zip(subject_parts.iter()).enumerate()
    {
        let is_last = index == last;
        if component_is_unreachable_interior(row_part, is_last)
            || component_is_unreachable_interior(subject_part, is_last)
        {
            return false;
        }
        match classify_windows_component(row_part) {
            ComponentFold::ShortNameAlias => {
                if !short_name_could_stand_for(row_part, subject_part) {
                    return false;
                }
            }
            ComponentFold::Folded(row_folded) => {
                match classify_windows_component(subject_part) {
                    ComponentFold::Folded(subject_folded) if row_folded == subject_folded => {}
                    _ => return false,
                }
            }
            ComponentFold::NamesNothing => return false,
        }
    }
    true
}

/// THIS machine's case-folding table, measured exhaustively.
///
/// Produced by calling `ntdll!RtlUpcaseUnicodeChar` for all 65024 non-surrogate
/// BMP code points and keeping the 973 that change; generator and raw table in
/// `extractor/native/runs/win_alias_key/upcase_dump.py` and
/// `upcase_table_diff.txt`.
///
/// IT IS NOT UNIVERSAL. NTFS folds through the `$UpCase` file written into the
/// volume when it was FORMATTED, and that table has changed across Windows
/// versions (`ȩ`/`Ȩ` is a documented example of a pair that folds together under
/// one version's table and not another's). A static table therefore cannot
/// answer "these two spellings are certainly the same file" for a volume it has
/// not read. So this table is used for exactly two things, neither of which
/// needs to be universal:
///
/// * [`windows_collision_key`], which is ALLOWED to fold more than the
///   filesystem, because a false collision only refuses; and
/// * feeding the collision key, which is also what tells a case NEAR-MISS apart
///   from a plain absence, so the report can say "undetermined" instead of
///   guessing. The union with Unicode's simple uppercase matters for that second
///   job: a table can LOSE a mapping between Windows versions -- final sigma
///   folds to capital sigma under the XP and Vista tables and is reset in later
///   ones -- so this host's table alone is not even an upper bound on the
///   others, whereas the union covers both directions for every mapping either
///   source knows.
///
///   OPEN ITEM. That the union covers EVERY supported `$UpCase` version is not
///   established by the evidence in
///   `extractor/native/runs/win_alias_key/`, which compares this host's table
///   with Unicode's and nothing else. A mapping present only in some older table
///   and in neither of those two would be a genuine under-fold: it would make
///   the collision key miss a real alias, and make the absence arm call a
///   present file missing. Closing it means archiving an exhaustive comparison
///   against each supported version's table.
///
/// It is deliberately NOT the same as Unicode's simple uppercase. The diff runs
/// both ways, and both directions were then checked against the real filesystem:
///
/// * 27 code points (the Greek iota-subscript letters) are folded HERE and not
///   by Unicode. All 27 were confirmed on disk to be one file. Missing them
///   would make the collision key fold LESS than NTFS, the one direction the
///   fail-closed contract forbids.
/// * 225 code points are folded by Unicode and NOT here (dotless i, MICRO SIGN,
///   LATIN SMALL LETTER LONG S, the title-case digraphs...). 30 were spot-checked
///   on disk and all 30 were two files. Those belong in the collision key, which
///   may over-fold, and must stay out of [`windows_same_file_identity`], which
///   may not.
const HOST_UPCASE: [(u16, u16); 973] = [
    (0x0061,0x0041), (0x0062,0x0042), (0x0063,0x0043), (0x0064,0x0044), (0x0065,0x0045), (0x0066,0x0046),
    (0x0067,0x0047), (0x0068,0x0048), (0x0069,0x0049), (0x006A,0x004A), (0x006B,0x004B), (0x006C,0x004C),
    (0x006D,0x004D), (0x006E,0x004E), (0x006F,0x004F), (0x0070,0x0050), (0x0071,0x0051), (0x0072,0x0052),
    (0x0073,0x0053), (0x0074,0x0054), (0x0075,0x0055), (0x0076,0x0056), (0x0077,0x0057), (0x0078,0x0058),
    (0x0079,0x0059), (0x007A,0x005A), (0x00E0,0x00C0), (0x00E1,0x00C1), (0x00E2,0x00C2), (0x00E3,0x00C3),
    (0x00E4,0x00C4), (0x00E5,0x00C5), (0x00E6,0x00C6), (0x00E7,0x00C7), (0x00E8,0x00C8), (0x00E9,0x00C9),
    (0x00EA,0x00CA), (0x00EB,0x00CB), (0x00EC,0x00CC), (0x00ED,0x00CD), (0x00EE,0x00CE), (0x00EF,0x00CF),
    (0x00F0,0x00D0), (0x00F1,0x00D1), (0x00F2,0x00D2), (0x00F3,0x00D3), (0x00F4,0x00D4), (0x00F5,0x00D5),
    (0x00F6,0x00D6), (0x00F8,0x00D8), (0x00F9,0x00D9), (0x00FA,0x00DA), (0x00FB,0x00DB), (0x00FC,0x00DC),
    (0x00FD,0x00DD), (0x00FE,0x00DE), (0x00FF,0x0178), (0x0101,0x0100), (0x0103,0x0102), (0x0105,0x0104),
    (0x0107,0x0106), (0x0109,0x0108), (0x010B,0x010A), (0x010D,0x010C), (0x010F,0x010E), (0x0111,0x0110),
    (0x0113,0x0112), (0x0115,0x0114), (0x0117,0x0116), (0x0119,0x0118), (0x011B,0x011A), (0x011D,0x011C),
    (0x011F,0x011E), (0x0121,0x0120), (0x0123,0x0122), (0x0125,0x0124), (0x0127,0x0126), (0x0129,0x0128),
    (0x012B,0x012A), (0x012D,0x012C), (0x012F,0x012E), (0x0133,0x0132), (0x0135,0x0134), (0x0137,0x0136),
    (0x013A,0x0139), (0x013C,0x013B), (0x013E,0x013D), (0x0140,0x013F), (0x0142,0x0141), (0x0144,0x0143),
    (0x0146,0x0145), (0x0148,0x0147), (0x014B,0x014A), (0x014D,0x014C), (0x014F,0x014E), (0x0151,0x0150),
    (0x0153,0x0152), (0x0155,0x0154), (0x0157,0x0156), (0x0159,0x0158), (0x015B,0x015A), (0x015D,0x015C),
    (0x015F,0x015E), (0x0161,0x0160), (0x0163,0x0162), (0x0165,0x0164), (0x0167,0x0166), (0x0169,0x0168),
    (0x016B,0x016A), (0x016D,0x016C), (0x016F,0x016E), (0x0171,0x0170), (0x0173,0x0172), (0x0175,0x0174),
    (0x0177,0x0176), (0x017A,0x0179), (0x017C,0x017B), (0x017E,0x017D), (0x0180,0x0243), (0x0183,0x0182),
    (0x0185,0x0184), (0x0188,0x0187), (0x018C,0x018B), (0x0192,0x0191), (0x0195,0x01F6), (0x0199,0x0198),
    (0x019A,0x023D), (0x019E,0x0220), (0x01A1,0x01A0), (0x01A3,0x01A2), (0x01A5,0x01A4), (0x01A8,0x01A7),
    (0x01AD,0x01AC), (0x01B0,0x01AF), (0x01B4,0x01B3), (0x01B6,0x01B5), (0x01B9,0x01B8), (0x01BD,0x01BC),
    (0x01BF,0x01F7), (0x01C6,0x01C4), (0x01C9,0x01C7), (0x01CC,0x01CA), (0x01CE,0x01CD), (0x01D0,0x01CF),
    (0x01D2,0x01D1), (0x01D4,0x01D3), (0x01D6,0x01D5), (0x01D8,0x01D7), (0x01DA,0x01D9), (0x01DC,0x01DB),
    (0x01DD,0x018E), (0x01DF,0x01DE), (0x01E1,0x01E0), (0x01E3,0x01E2), (0x01E5,0x01E4), (0x01E7,0x01E6),
    (0x01E9,0x01E8), (0x01EB,0x01EA), (0x01ED,0x01EC), (0x01EF,0x01EE), (0x01F3,0x01F1), (0x01F5,0x01F4),
    (0x01F9,0x01F8), (0x01FB,0x01FA), (0x01FD,0x01FC), (0x01FF,0x01FE), (0x0201,0x0200), (0x0203,0x0202),
    (0x0205,0x0204), (0x0207,0x0206), (0x0209,0x0208), (0x020B,0x020A), (0x020D,0x020C), (0x020F,0x020E),
    (0x0211,0x0210), (0x0213,0x0212), (0x0215,0x0214), (0x0217,0x0216), (0x0219,0x0218), (0x021B,0x021A),
    (0x021D,0x021C), (0x021F,0x021E), (0x0223,0x0222), (0x0225,0x0224), (0x0227,0x0226), (0x0229,0x0228),
    (0x022B,0x022A), (0x022D,0x022C), (0x022F,0x022E), (0x0231,0x0230), (0x0233,0x0232), (0x023C,0x023B),
    (0x0242,0x0241), (0x0247,0x0246), (0x0249,0x0248), (0x024B,0x024A), (0x024D,0x024C), (0x024F,0x024E),
    (0x0250,0x2C6F), (0x0251,0x2C6D), (0x0253,0x0181), (0x0254,0x0186), (0x0256,0x0189), (0x0257,0x018A),
    (0x0259,0x018F), (0x025B,0x0190), (0x0260,0x0193), (0x0263,0x0194), (0x0268,0x0197), (0x0269,0x0196),
    (0x026B,0x2C62), (0x026F,0x019C), (0x0271,0x2C6E), (0x0272,0x019D), (0x0275,0x019F), (0x027D,0x2C64),
    (0x0280,0x01A6), (0x0283,0x01A9), (0x0288,0x01AE), (0x0289,0x0244), (0x028A,0x01B1), (0x028B,0x01B2),
    (0x028C,0x0245), (0x0292,0x01B7), (0x0371,0x0370), (0x0373,0x0372), (0x0377,0x0376), (0x037B,0x03FD),
    (0x037C,0x03FE), (0x037D,0x03FF), (0x03AC,0x0386), (0x03AD,0x0388), (0x03AE,0x0389), (0x03AF,0x038A),
    (0x03B1,0x0391), (0x03B2,0x0392), (0x03B3,0x0393), (0x03B4,0x0394), (0x03B5,0x0395), (0x03B6,0x0396),
    (0x03B7,0x0397), (0x03B8,0x0398), (0x03B9,0x0399), (0x03BA,0x039A), (0x03BB,0x039B), (0x03BC,0x039C),
    (0x03BD,0x039D), (0x03BE,0x039E), (0x03BF,0x039F), (0x03C0,0x03A0), (0x03C1,0x03A1), (0x03C3,0x03A3),
    (0x03C4,0x03A4), (0x03C5,0x03A5), (0x03C6,0x03A6), (0x03C7,0x03A7), (0x03C8,0x03A8), (0x03C9,0x03A9),
    (0x03CA,0x03AA), (0x03CB,0x03AB), (0x03CC,0x038C), (0x03CD,0x038E), (0x03CE,0x038F), (0x03D7,0x03CF),
    (0x03D9,0x03D8), (0x03DB,0x03DA), (0x03DD,0x03DC), (0x03DF,0x03DE), (0x03E1,0x03E0), (0x03E3,0x03E2),
    (0x03E5,0x03E4), (0x03E7,0x03E6), (0x03E9,0x03E8), (0x03EB,0x03EA), (0x03ED,0x03EC), (0x03EF,0x03EE),
    (0x03F2,0x03F9), (0x03F8,0x03F7), (0x03FB,0x03FA), (0x0430,0x0410), (0x0431,0x0411), (0x0432,0x0412),
    (0x0433,0x0413), (0x0434,0x0414), (0x0435,0x0415), (0x0436,0x0416), (0x0437,0x0417), (0x0438,0x0418),
    (0x0439,0x0419), (0x043A,0x041A), (0x043B,0x041B), (0x043C,0x041C), (0x043D,0x041D), (0x043E,0x041E),
    (0x043F,0x041F), (0x0440,0x0420), (0x0441,0x0421), (0x0442,0x0422), (0x0443,0x0423), (0x0444,0x0424),
    (0x0445,0x0425), (0x0446,0x0426), (0x0447,0x0427), (0x0448,0x0428), (0x0449,0x0429), (0x044A,0x042A),
    (0x044B,0x042B), (0x044C,0x042C), (0x044D,0x042D), (0x044E,0x042E), (0x044F,0x042F), (0x0450,0x0400),
    (0x0451,0x0401), (0x0452,0x0402), (0x0453,0x0403), (0x0454,0x0404), (0x0455,0x0405), (0x0456,0x0406),
    (0x0457,0x0407), (0x0458,0x0408), (0x0459,0x0409), (0x045A,0x040A), (0x045B,0x040B), (0x045C,0x040C),
    (0x045D,0x040D), (0x045E,0x040E), (0x045F,0x040F), (0x0461,0x0460), (0x0463,0x0462), (0x0465,0x0464),
    (0x0467,0x0466), (0x0469,0x0468), (0x046B,0x046A), (0x046D,0x046C), (0x046F,0x046E), (0x0471,0x0470),
    (0x0473,0x0472), (0x0475,0x0474), (0x0477,0x0476), (0x0479,0x0478), (0x047B,0x047A), (0x047D,0x047C),
    (0x047F,0x047E), (0x0481,0x0480), (0x048B,0x048A), (0x048D,0x048C), (0x048F,0x048E), (0x0491,0x0490),
    (0x0493,0x0492), (0x0495,0x0494), (0x0497,0x0496), (0x0499,0x0498), (0x049B,0x049A), (0x049D,0x049C),
    (0x049F,0x049E), (0x04A1,0x04A0), (0x04A3,0x04A2), (0x04A5,0x04A4), (0x04A7,0x04A6), (0x04A9,0x04A8),
    (0x04AB,0x04AA), (0x04AD,0x04AC), (0x04AF,0x04AE), (0x04B1,0x04B0), (0x04B3,0x04B2), (0x04B5,0x04B4),
    (0x04B7,0x04B6), (0x04B9,0x04B8), (0x04BB,0x04BA), (0x04BD,0x04BC), (0x04BF,0x04BE), (0x04C2,0x04C1),
    (0x04C4,0x04C3), (0x04C6,0x04C5), (0x04C8,0x04C7), (0x04CA,0x04C9), (0x04CC,0x04CB), (0x04CE,0x04CD),
    (0x04CF,0x04C0), (0x04D1,0x04D0), (0x04D3,0x04D2), (0x04D5,0x04D4), (0x04D7,0x04D6), (0x04D9,0x04D8),
    (0x04DB,0x04DA), (0x04DD,0x04DC), (0x04DF,0x04DE), (0x04E1,0x04E0), (0x04E3,0x04E2), (0x04E5,0x04E4),
    (0x04E7,0x04E6), (0x04E9,0x04E8), (0x04EB,0x04EA), (0x04ED,0x04EC), (0x04EF,0x04EE), (0x04F1,0x04F0),
    (0x04F3,0x04F2), (0x04F5,0x04F4), (0x04F7,0x04F6), (0x04F9,0x04F8), (0x04FB,0x04FA), (0x04FD,0x04FC),
    (0x04FF,0x04FE), (0x0501,0x0500), (0x0503,0x0502), (0x0505,0x0504), (0x0507,0x0506), (0x0509,0x0508),
    (0x050B,0x050A), (0x050D,0x050C), (0x050F,0x050E), (0x0511,0x0510), (0x0513,0x0512), (0x0515,0x0514),
    (0x0517,0x0516), (0x0519,0x0518), (0x051B,0x051A), (0x051D,0x051C), (0x051F,0x051E), (0x0521,0x0520),
    (0x0523,0x0522), (0x0561,0x0531), (0x0562,0x0532), (0x0563,0x0533), (0x0564,0x0534), (0x0565,0x0535),
    (0x0566,0x0536), (0x0567,0x0537), (0x0568,0x0538), (0x0569,0x0539), (0x056A,0x053A), (0x056B,0x053B),
    (0x056C,0x053C), (0x056D,0x053D), (0x056E,0x053E), (0x056F,0x053F), (0x0570,0x0540), (0x0571,0x0541),
    (0x0572,0x0542), (0x0573,0x0543), (0x0574,0x0544), (0x0575,0x0545), (0x0576,0x0546), (0x0577,0x0547),
    (0x0578,0x0548), (0x0579,0x0549), (0x057A,0x054A), (0x057B,0x054B), (0x057C,0x054C), (0x057D,0x054D),
    (0x057E,0x054E), (0x057F,0x054F), (0x0580,0x0550), (0x0581,0x0551), (0x0582,0x0552), (0x0583,0x0553),
    (0x0584,0x0554), (0x0585,0x0555), (0x0586,0x0556), (0x1D79,0xA77D), (0x1D7D,0x2C63), (0x1E01,0x1E00),
    (0x1E03,0x1E02), (0x1E05,0x1E04), (0x1E07,0x1E06), (0x1E09,0x1E08), (0x1E0B,0x1E0A), (0x1E0D,0x1E0C),
    (0x1E0F,0x1E0E), (0x1E11,0x1E10), (0x1E13,0x1E12), (0x1E15,0x1E14), (0x1E17,0x1E16), (0x1E19,0x1E18),
    (0x1E1B,0x1E1A), (0x1E1D,0x1E1C), (0x1E1F,0x1E1E), (0x1E21,0x1E20), (0x1E23,0x1E22), (0x1E25,0x1E24),
    (0x1E27,0x1E26), (0x1E29,0x1E28), (0x1E2B,0x1E2A), (0x1E2D,0x1E2C), (0x1E2F,0x1E2E), (0x1E31,0x1E30),
    (0x1E33,0x1E32), (0x1E35,0x1E34), (0x1E37,0x1E36), (0x1E39,0x1E38), (0x1E3B,0x1E3A), (0x1E3D,0x1E3C),
    (0x1E3F,0x1E3E), (0x1E41,0x1E40), (0x1E43,0x1E42), (0x1E45,0x1E44), (0x1E47,0x1E46), (0x1E49,0x1E48),
    (0x1E4B,0x1E4A), (0x1E4D,0x1E4C), (0x1E4F,0x1E4E), (0x1E51,0x1E50), (0x1E53,0x1E52), (0x1E55,0x1E54),
    (0x1E57,0x1E56), (0x1E59,0x1E58), (0x1E5B,0x1E5A), (0x1E5D,0x1E5C), (0x1E5F,0x1E5E), (0x1E61,0x1E60),
    (0x1E63,0x1E62), (0x1E65,0x1E64), (0x1E67,0x1E66), (0x1E69,0x1E68), (0x1E6B,0x1E6A), (0x1E6D,0x1E6C),
    (0x1E6F,0x1E6E), (0x1E71,0x1E70), (0x1E73,0x1E72), (0x1E75,0x1E74), (0x1E77,0x1E76), (0x1E79,0x1E78),
    (0x1E7B,0x1E7A), (0x1E7D,0x1E7C), (0x1E7F,0x1E7E), (0x1E81,0x1E80), (0x1E83,0x1E82), (0x1E85,0x1E84),
    (0x1E87,0x1E86), (0x1E89,0x1E88), (0x1E8B,0x1E8A), (0x1E8D,0x1E8C), (0x1E8F,0x1E8E), (0x1E91,0x1E90),
    (0x1E93,0x1E92), (0x1E95,0x1E94), (0x1EA1,0x1EA0), (0x1EA3,0x1EA2), (0x1EA5,0x1EA4), (0x1EA7,0x1EA6),
    (0x1EA9,0x1EA8), (0x1EAB,0x1EAA), (0x1EAD,0x1EAC), (0x1EAF,0x1EAE), (0x1EB1,0x1EB0), (0x1EB3,0x1EB2),
    (0x1EB5,0x1EB4), (0x1EB7,0x1EB6), (0x1EB9,0x1EB8), (0x1EBB,0x1EBA), (0x1EBD,0x1EBC), (0x1EBF,0x1EBE),
    (0x1EC1,0x1EC0), (0x1EC3,0x1EC2), (0x1EC5,0x1EC4), (0x1EC7,0x1EC6), (0x1EC9,0x1EC8), (0x1ECB,0x1ECA),
    (0x1ECD,0x1ECC), (0x1ECF,0x1ECE), (0x1ED1,0x1ED0), (0x1ED3,0x1ED2), (0x1ED5,0x1ED4), (0x1ED7,0x1ED6),
    (0x1ED9,0x1ED8), (0x1EDB,0x1EDA), (0x1EDD,0x1EDC), (0x1EDF,0x1EDE), (0x1EE1,0x1EE0), (0x1EE3,0x1EE2),
    (0x1EE5,0x1EE4), (0x1EE7,0x1EE6), (0x1EE9,0x1EE8), (0x1EEB,0x1EEA), (0x1EED,0x1EEC), (0x1EEF,0x1EEE),
    (0x1EF1,0x1EF0), (0x1EF3,0x1EF2), (0x1EF5,0x1EF4), (0x1EF7,0x1EF6), (0x1EF9,0x1EF8), (0x1EFB,0x1EFA),
    (0x1EFD,0x1EFC), (0x1EFF,0x1EFE), (0x1F00,0x1F08), (0x1F01,0x1F09), (0x1F02,0x1F0A), (0x1F03,0x1F0B),
    (0x1F04,0x1F0C), (0x1F05,0x1F0D), (0x1F06,0x1F0E), (0x1F07,0x1F0F), (0x1F10,0x1F18), (0x1F11,0x1F19),
    (0x1F12,0x1F1A), (0x1F13,0x1F1B), (0x1F14,0x1F1C), (0x1F15,0x1F1D), (0x1F20,0x1F28), (0x1F21,0x1F29),
    (0x1F22,0x1F2A), (0x1F23,0x1F2B), (0x1F24,0x1F2C), (0x1F25,0x1F2D), (0x1F26,0x1F2E), (0x1F27,0x1F2F),
    (0x1F30,0x1F38), (0x1F31,0x1F39), (0x1F32,0x1F3A), (0x1F33,0x1F3B), (0x1F34,0x1F3C), (0x1F35,0x1F3D),
    (0x1F36,0x1F3E), (0x1F37,0x1F3F), (0x1F40,0x1F48), (0x1F41,0x1F49), (0x1F42,0x1F4A), (0x1F43,0x1F4B),
    (0x1F44,0x1F4C), (0x1F45,0x1F4D), (0x1F51,0x1F59), (0x1F53,0x1F5B), (0x1F55,0x1F5D), (0x1F57,0x1F5F),
    (0x1F60,0x1F68), (0x1F61,0x1F69), (0x1F62,0x1F6A), (0x1F63,0x1F6B), (0x1F64,0x1F6C), (0x1F65,0x1F6D),
    (0x1F66,0x1F6E), (0x1F67,0x1F6F), (0x1F70,0x1FBA), (0x1F71,0x1FBB), (0x1F72,0x1FC8), (0x1F73,0x1FC9),
    (0x1F74,0x1FCA), (0x1F75,0x1FCB), (0x1F76,0x1FDA), (0x1F77,0x1FDB), (0x1F78,0x1FF8), (0x1F79,0x1FF9),
    (0x1F7A,0x1FEA), (0x1F7B,0x1FEB), (0x1F7C,0x1FFA), (0x1F7D,0x1FFB), (0x1F80,0x1F88), (0x1F81,0x1F89),
    (0x1F82,0x1F8A), (0x1F83,0x1F8B), (0x1F84,0x1F8C), (0x1F85,0x1F8D), (0x1F86,0x1F8E), (0x1F87,0x1F8F),
    (0x1F90,0x1F98), (0x1F91,0x1F99), (0x1F92,0x1F9A), (0x1F93,0x1F9B), (0x1F94,0x1F9C), (0x1F95,0x1F9D),
    (0x1F96,0x1F9E), (0x1F97,0x1F9F), (0x1FA0,0x1FA8), (0x1FA1,0x1FA9), (0x1FA2,0x1FAA), (0x1FA3,0x1FAB),
    (0x1FA4,0x1FAC), (0x1FA5,0x1FAD), (0x1FA6,0x1FAE), (0x1FA7,0x1FAF), (0x1FB0,0x1FB8), (0x1FB1,0x1FB9),
    (0x1FB3,0x1FBC), (0x1FC3,0x1FCC), (0x1FD0,0x1FD8), (0x1FD1,0x1FD9), (0x1FE0,0x1FE8), (0x1FE1,0x1FE9),
    (0x1FE5,0x1FEC), (0x1FF3,0x1FFC), (0x214E,0x2132), (0x2170,0x2160), (0x2171,0x2161), (0x2172,0x2162),
    (0x2173,0x2163), (0x2174,0x2164), (0x2175,0x2165), (0x2176,0x2166), (0x2177,0x2167), (0x2178,0x2168),
    (0x2179,0x2169), (0x217A,0x216A), (0x217B,0x216B), (0x217C,0x216C), (0x217D,0x216D), (0x217E,0x216E),
    (0x217F,0x216F), (0x2184,0x2183), (0x24D0,0x24B6), (0x24D1,0x24B7), (0x24D2,0x24B8), (0x24D3,0x24B9),
    (0x24D4,0x24BA), (0x24D5,0x24BB), (0x24D6,0x24BC), (0x24D7,0x24BD), (0x24D8,0x24BE), (0x24D9,0x24BF),
    (0x24DA,0x24C0), (0x24DB,0x24C1), (0x24DC,0x24C2), (0x24DD,0x24C3), (0x24DE,0x24C4), (0x24DF,0x24C5),
    (0x24E0,0x24C6), (0x24E1,0x24C7), (0x24E2,0x24C8), (0x24E3,0x24C9), (0x24E4,0x24CA), (0x24E5,0x24CB),
    (0x24E6,0x24CC), (0x24E7,0x24CD), (0x24E8,0x24CE), (0x24E9,0x24CF), (0x2C30,0x2C00), (0x2C31,0x2C01),
    (0x2C32,0x2C02), (0x2C33,0x2C03), (0x2C34,0x2C04), (0x2C35,0x2C05), (0x2C36,0x2C06), (0x2C37,0x2C07),
    (0x2C38,0x2C08), (0x2C39,0x2C09), (0x2C3A,0x2C0A), (0x2C3B,0x2C0B), (0x2C3C,0x2C0C), (0x2C3D,0x2C0D),
    (0x2C3E,0x2C0E), (0x2C3F,0x2C0F), (0x2C40,0x2C10), (0x2C41,0x2C11), (0x2C42,0x2C12), (0x2C43,0x2C13),
    (0x2C44,0x2C14), (0x2C45,0x2C15), (0x2C46,0x2C16), (0x2C47,0x2C17), (0x2C48,0x2C18), (0x2C49,0x2C19),
    (0x2C4A,0x2C1A), (0x2C4B,0x2C1B), (0x2C4C,0x2C1C), (0x2C4D,0x2C1D), (0x2C4E,0x2C1E), (0x2C4F,0x2C1F),
    (0x2C50,0x2C20), (0x2C51,0x2C21), (0x2C52,0x2C22), (0x2C53,0x2C23), (0x2C54,0x2C24), (0x2C55,0x2C25),
    (0x2C56,0x2C26), (0x2C57,0x2C27), (0x2C58,0x2C28), (0x2C59,0x2C29), (0x2C5A,0x2C2A), (0x2C5B,0x2C2B),
    (0x2C5C,0x2C2C), (0x2C5D,0x2C2D), (0x2C5E,0x2C2E), (0x2C61,0x2C60), (0x2C65,0x023A), (0x2C66,0x023E),
    (0x2C68,0x2C67), (0x2C6A,0x2C69), (0x2C6C,0x2C6B), (0x2C73,0x2C72), (0x2C76,0x2C75), (0x2C81,0x2C80),
    (0x2C83,0x2C82), (0x2C85,0x2C84), (0x2C87,0x2C86), (0x2C89,0x2C88), (0x2C8B,0x2C8A), (0x2C8D,0x2C8C),
    (0x2C8F,0x2C8E), (0x2C91,0x2C90), (0x2C93,0x2C92), (0x2C95,0x2C94), (0x2C97,0x2C96), (0x2C99,0x2C98),
    (0x2C9B,0x2C9A), (0x2C9D,0x2C9C), (0x2C9F,0x2C9E), (0x2CA1,0x2CA0), (0x2CA3,0x2CA2), (0x2CA5,0x2CA4),
    (0x2CA7,0x2CA6), (0x2CA9,0x2CA8), (0x2CAB,0x2CAA), (0x2CAD,0x2CAC), (0x2CAF,0x2CAE), (0x2CB1,0x2CB0),
    (0x2CB3,0x2CB2), (0x2CB5,0x2CB4), (0x2CB7,0x2CB6), (0x2CB9,0x2CB8), (0x2CBB,0x2CBA), (0x2CBD,0x2CBC),
    (0x2CBF,0x2CBE), (0x2CC1,0x2CC0), (0x2CC3,0x2CC2), (0x2CC5,0x2CC4), (0x2CC7,0x2CC6), (0x2CC9,0x2CC8),
    (0x2CCB,0x2CCA), (0x2CCD,0x2CCC), (0x2CCF,0x2CCE), (0x2CD1,0x2CD0), (0x2CD3,0x2CD2), (0x2CD5,0x2CD4),
    (0x2CD7,0x2CD6), (0x2CD9,0x2CD8), (0x2CDB,0x2CDA), (0x2CDD,0x2CDC), (0x2CDF,0x2CDE), (0x2CE1,0x2CE0),
    (0x2CE3,0x2CE2), (0x2D00,0x10A0), (0x2D01,0x10A1), (0x2D02,0x10A2), (0x2D03,0x10A3), (0x2D04,0x10A4),
    (0x2D05,0x10A5), (0x2D06,0x10A6), (0x2D07,0x10A7), (0x2D08,0x10A8), (0x2D09,0x10A9), (0x2D0A,0x10AA),
    (0x2D0B,0x10AB), (0x2D0C,0x10AC), (0x2D0D,0x10AD), (0x2D0E,0x10AE), (0x2D0F,0x10AF), (0x2D10,0x10B0),
    (0x2D11,0x10B1), (0x2D12,0x10B2), (0x2D13,0x10B3), (0x2D14,0x10B4), (0x2D15,0x10B5), (0x2D16,0x10B6),
    (0x2D17,0x10B7), (0x2D18,0x10B8), (0x2D19,0x10B9), (0x2D1A,0x10BA), (0x2D1B,0x10BB), (0x2D1C,0x10BC),
    (0x2D1D,0x10BD), (0x2D1E,0x10BE), (0x2D1F,0x10BF), (0x2D20,0x10C0), (0x2D21,0x10C1), (0x2D22,0x10C2),
    (0x2D23,0x10C3), (0x2D24,0x10C4), (0x2D25,0x10C5), (0xA641,0xA640), (0xA643,0xA642), (0xA645,0xA644),
    (0xA647,0xA646), (0xA649,0xA648), (0xA64B,0xA64A), (0xA64D,0xA64C), (0xA64F,0xA64E), (0xA651,0xA650),
    (0xA653,0xA652), (0xA655,0xA654), (0xA657,0xA656), (0xA659,0xA658), (0xA65B,0xA65A), (0xA65D,0xA65C),
    (0xA65F,0xA65E), (0xA663,0xA662), (0xA665,0xA664), (0xA667,0xA666), (0xA669,0xA668), (0xA66B,0xA66A),
    (0xA66D,0xA66C), (0xA681,0xA680), (0xA683,0xA682), (0xA685,0xA684), (0xA687,0xA686), (0xA689,0xA688),
    (0xA68B,0xA68A), (0xA68D,0xA68C), (0xA68F,0xA68E), (0xA691,0xA690), (0xA693,0xA692), (0xA695,0xA694),
    (0xA697,0xA696), (0xA723,0xA722), (0xA725,0xA724), (0xA727,0xA726), (0xA729,0xA728), (0xA72B,0xA72A),
    (0xA72D,0xA72C), (0xA72F,0xA72E), (0xA733,0xA732), (0xA735,0xA734), (0xA737,0xA736), (0xA739,0xA738),
    (0xA73B,0xA73A), (0xA73D,0xA73C), (0xA73F,0xA73E), (0xA741,0xA740), (0xA743,0xA742), (0xA745,0xA744),
    (0xA747,0xA746), (0xA749,0xA748), (0xA74B,0xA74A), (0xA74D,0xA74C), (0xA74F,0xA74E), (0xA751,0xA750),
    (0xA753,0xA752), (0xA755,0xA754), (0xA757,0xA756), (0xA759,0xA758), (0xA75B,0xA75A), (0xA75D,0xA75C),
    (0xA75F,0xA75E), (0xA761,0xA760), (0xA763,0xA762), (0xA765,0xA764), (0xA767,0xA766), (0xA769,0xA768),
    (0xA76B,0xA76A), (0xA76D,0xA76C), (0xA76F,0xA76E), (0xA77A,0xA779), (0xA77C,0xA77B), (0xA77F,0xA77E),
    (0xA781,0xA780), (0xA783,0xA782), (0xA785,0xA784), (0xA787,0xA786), (0xA78C,0xA78B), (0xFF41,0xFF21),
    (0xFF42,0xFF22), (0xFF43,0xFF23), (0xFF44,0xFF24), (0xFF45,0xFF25), (0xFF46,0xFF26), (0xFF47,0xFF27),
    (0xFF48,0xFF28), (0xFF49,0xFF29), (0xFF4A,0xFF2A), (0xFF4B,0xFF2B), (0xFF4C,0xFF2C), (0xFF4D,0xFF2D),
    (0xFF4E,0xFF2E), (0xFF4F,0xFF2F), (0xFF50,0xFF30), (0xFF51,0xFF31), (0xFF52,0xFF32), (0xFF53,0xFF33),
    (0xFF54,0xFF34), (0xFF55,0xFF35), (0xFF56,0xFF36), (0xFF57,0xFF37), (0xFF58,0xFF38), (0xFF59,0xFF39),
    (0xFF5A,0xFF3A),
];

fn host_upcase(ch: char) -> char {
    let code = ch as u32;
    if code > 0xFFFF {
        return ch;
    }
    let code = code as u16;
    match HOST_UPCASE.binary_search_by_key(&code, |(from, _)| *from) {
        Ok(index) => char::from_u32(HOST_UPCASE[index].1 as u32).unwrap_or(ch),
        Err(_) => ch,
    }
}

/// The host relation UNIONED with Unicode's simple uppercase over the BMP.
/// Used by [`windows_collision_key`], which may fold more than the filesystem
/// and must not fold less -- subject to the OPEN ITEM at [`HOST_UPCASE`], which
/// is the one thing that could still make it fold less. Astral characters are left alone because a 16-bit table
/// cannot address them, and a multi-character expansion (sharp s to SS, the fi
/// ligature to FI) keeps its original form because the measurement says those
/// are two files.
fn simple_upper_bmp(value: &str) -> String {
    let mut out = String::with_capacity(value.len());
    for ch in value.chars() {
        let host = host_upcase(ch);
        if host != ch {
            out.push(host);
            continue;
        }
        if ch as u32 > 0xFFFF {
            out.push(ch);
            continue;
        }
        let mut upper = ch.to_uppercase();
        match (upper.next(), upper.next()) {
            (Some(single), None) => out.push(single),
            _ => out.push(ch),
        }
    }
    out
}

/// The `NAME~N[.EXT]` shape the filesystem generates for a long name: a base of
/// at most 8 characters ending in `~` plus digits, and an extension of at most 3
/// characters. Lengths are counted in CHARACTERS, not bytes, and the base is NOT
/// required to be ASCII: `RtlGenerate8dot3Name` places extended characters into
/// generated short names when extended characters are enabled, so a
/// `entite~1.a` spelled with an accented e is as undecidable as `entiti~1`.
fn is_short_name_shaped(upper: &str) -> bool {
    let (base, extension) = match upper.split_once('.') {
        Some((base, extension)) => (base, Some(extension)),
        None => (upper, None),
    };
    if let Some(extension) = extension {
        if extension.chars().count() > 3 || extension.contains('.') {
            return false;
        }
    }
    let base_length = base.chars().count();
    if base_length == 0 || base_length > 8 {
        return false;
    }
    let Some((head, tail)) = base.rsplit_once('~') else {
        return false;
    };
    !head.is_empty() && !tail.is_empty() && tail.bytes().all(|byte| byte.is_ascii_digit())
}

/// The strongest statement that can be made about two canonical paths WITHOUT
/// over-folding: equal values here mean the same file on Windows, full stop.
/// `None` means "cannot prove it".
///
/// This exists because [`windows_collision_key`] is deliberately allowed to fold
/// MORE than the filesystem, and that is exactly right for a refusal (a false
/// collision only refuses) and exactly wrong for a claim of PRESENCE. Suppressing
/// a `MissingContent` on an over-folded match would tell the operator that a file
/// they do not have is present: `gfx/i.asset` and `gfx/{dotless i}.asset` share a
/// collision key, but the filesystem measurement says they are two files, so the
/// required `gfx/i.asset` really can be absent.
///
/// Only the folds MEASURED to be exact are applied here:
///
/// * the last component loses trailing dots and spaces (measured: `b.txt.`,
///   `c.txt...` and `d.txt ` each overwrote the undotted/unspaced file);
/// * an interior component loses trailing DOTS only (measured: `gfx./u.asset`
///   opened `gfx/u.asset`), and an interior component with a trailing SPACE
///   yields `None`, because `gfx /u.asset` was measured to resolve to nothing
///   at all rather than to `gfx/u.asset`;
/// * no case folding beyond the ASCII lowercase the canonical form has already
///   applied. This is a CONSERVATIVE SUBSET, not a claim that ASCII is the only
///   mapping every `$UpCase` version shares -- the known tables agree on far
///   more than ASCII. It is simply the largest subset this code can justify
///   without reading those tables, and being too small here costs only
///   precision: a pair that some table folds is reported UNDETERMINED instead of
///   proven present. A pair the collision key folds but this relation does not
///   is never called absent either; see the absence arm of `validate_playback`.
///
/// SAFETY NET. Even if this relation were slightly wrong on some other host, it
/// can only change WHICH refusal the operator reads, never admit a replay: every
/// branch that suppresses a `MissingContent` has either already emitted an issue
/// for the offending row or emits one itself, so the report stays incompatible.
/// `suppressing_an_absence_never_makes_a_report_compatible` pins that.
///
/// A DOS device stem or an 8.3 short-name shape yields `None` for the same
/// reason: their referent depends on the host, so they prove nothing.
pub fn windows_same_file_identity(canonical_game_relative_path: &str) -> Option<String> {
    let mut parts: Vec<String> = Vec::new();
    let components: Vec<&str> = canonical_game_relative_path.split('/').collect();
    let last = components.len().checked_sub(1)?;
    for (index, component) in components.into_iter().enumerate() {
        let stripped = if index == last {
            component.trim_end_matches([' ', '.'])
        } else {
            let without_dots = component.trim_end_matches('.');
            if without_dots.ends_with(' ') {
                return None;
            }
            without_dots
        };
        if stripped.is_empty() {
            return None;
        }
        let upper = simple_upper_bmp(stripped);
        if is_short_name_shaped(&upper) {
            return None;
        }
        let stem = upper
            .split('.')
            .next()
            .unwrap_or(upper.as_str())
            .trim_end_matches(' ');
        if DOS_DEVICE_NAMES.iter().any(|name| *name == stem) {
            return None;
        }
        // NO case folding beyond the ASCII lowercase the canonical form already
        // applied -- a conservative subset, because anything beyond ASCII is a
        // property of the volume the playback content lives on and this code has
        // not read that volume's $UpCase table. Everything left out here is
        // reported UNDETERMINED by the absence arm rather than decided wrongly.
        parts.push(stripped.to_string());
    }
    Some(parts.join("/"))
}

/// Record the proven identity of one inventory row so a required path spelled
/// equivalently is not later announced absent. First spelling wins; a second row
/// sharing an identity has already been refused by the collision rule, which
/// folds a superset of what this function folds.
fn note_row_identity(
    rows: &mut BTreeMap<String, String>,
    near_miss_rows: &mut BTreeMap<String, String>,
    undecidable_rows: &mut Vec<String>,
    canonical: &str,
) {
    if let Some(identity) = windows_same_file_identity(canonical) {
        rows.entry(identity).or_insert_with(|| canonical.to_string());
    }
    // Refused rows are recorded here too. A row refused for its SPELLING can
    // still be a case near-miss of a required path -- `GFX\Models\{A-ogonek}.Mesh`
    // never reaches the admitted set -- and the absence question must still see
    // it, or it is answered "missing" about a file that may be right there.
    //
    // The Err case is the same argument one step further. It has no key to be
    // filed under, so it needs its own list, and EVERY caller must feed it:
    // a row can be both non-canonical and undecidable
    // (`PORTABLE_REPLAY\NATIVE\CONTEN~1\CARGO.TOML`), and those arrive through
    // the recovery path rather than the canonical one.
    match windows_collision_key(canonical) {
        Ok(collision) => {
            near_miss_rows
                .entry(collision)
                .or_insert_with(|| canonical.to_string());
        }
        Err(_) => undecidable_rows.push(canonical.to_string()),
    }
}

/// Collision keys for an already-canonical path set, propagating the first
/// undecidable component instead of dropping it.
fn collision_key_set(canonical: &BTreeSet<String>) -> Result<BTreeSet<String>, String> {
    canonical
        .iter()
        .map(|path| windows_collision_key(path))
        .collect()
}

/// Best-effort identity of a resolved-content row that
/// [`normalize_game_relative_path`] refused, used ONLY to decide whether a
/// required path was present in some rejected spelling so it is not also
/// reported missing.
///
/// This is deliberately NOT a second canonicalizer. Its result never reaches
/// admission, comparison, or any recorded identity: the row that produced it has
/// already been refused with a typed issue and can never be admitted. It exists
/// so that reporting stays truthful about what the caller actually sent.
///
/// Every rule here fails toward `None`, and `None` is the conservative answer:
/// it means "this row is not a spelling of any required path", so the required
/// path is reported missing (in addition to the row's own refusal) exactly as
/// before. Only a spelling that unambiguously names one file inside the game
/// root earns a suppression.
fn refused_content_identity(path: &str) -> Option<String> {
    // Only the WHITESPACE Win32 actually strips may be ignored here: trailing
    // spaces on the final component, and nothing else. A leading space, or a
    // trailing tab, is part of the name (measured: ` e.txt` and `e.txt` are two
    // files), so a spelling carrying one names something else and vouches for
    // nothing. `normalize_game_relative_path` trims both ends -- it is a frozen
    // recorded identity and cannot be narrowed -- which is precisely why this
    // function must not inherit its result.
    if path.trim_start().len() != path.len() {
        return None;
    }
    if path[path.trim_end().len()..].chars().any(|ch| ch != ' ') {
        return None;
    }
    let lowered = path.trim().replace('\\', "/").to_ascii_lowercase();
    if lowered.is_empty() || lowered.starts_with('/') || lowered.contains(':') {
        return None;
    }
    let segments: Vec<&str> = lowered.split('/').collect();
    // A spelling names a FILE only if it ends in a real component. A trailing
    // separator, or a trailing "." / "..", names a DIRECTORY -- "gfx/a/unit.asset/"
    // is not the file "gfx/a/unit.asset" and must not suppress its absence. An
    // empty interior segment is likewise a spelling this function will not vouch
    // for. Neither is a component carrying a character Win32 forbids in a
    // filename, an embedded NUL above all: such a component cannot name a real
    // file, and without this it could exist just long enough for a following
    // ".." to pop it back off and leave a valid-looking identity behind. The
    // bound is exactly Win32's 0x00-0x1f, no wider: DEL (0x7f) and C1 controls
    // such as U+0085 ARE legal in an NTFS filename, and rejecting them would
    // cost a legitimate alias its recognition for no safety gain.
    if segments
        .iter()
        .any(|segment| segment.is_empty() || segment.chars().any(|ch| ch <= '\u{1f}'))
    {
        return None;
    }
    if segments
        .last()
        .is_some_and(|last| *last == "." || *last == "..")
    {
        return None;
    }
    let mut parts: Vec<&str> = Vec::new();
    for segment in segments {
        match segment {
            "." => {}
            ".." => {
                // Walking above the game root names nothing we could require.
                parts.pop()?;
            }
            other => parts.push(other),
        }
    }
    if parts.is_empty() {
        return None;
    }
    Some(parts.join("/"))
}

/// Convert the resolver's descriptor-relative DLC source identity
/// (`dlc/dlc020/dlc020.dlc`) to the launcher's stable selection identity
/// (`dlc020`). Already-normalized short IDs are accepted. Other path grammars
/// fail closed rather than becoming a second DLC identity namespace.
pub fn normalize_dlc_selection_id(source_id: &str) -> Result<String, String> {
    let normalized = source_id.trim().replace('\\', "/").to_ascii_lowercase();
    if normalized.is_empty() || normalized.starts_with('/') || normalized.contains(':') {
        return Err(format!(
            "unsafe/non-canonical DLC source identity {source_id:?}"
        ));
    }
    let parts: Vec<_> = normalized.split('/').collect();
    let id = match parts.as_slice() {
        [short] if !short.contains('.') => *short,
        ["dlc", directory, descriptor] => {
            let descriptor_stem = descriptor.strip_suffix(".dlc").ok_or_else(|| {
                format!("DLC descriptor identity has no .dlc suffix: {source_id:?}")
            })?;
            if !descriptor_stem.starts_with("dlc") {
                return Err(format!("invalid DLC descriptor identity: {source_id:?}"));
            }
            *directory
        }
        _ => {
            return Err(format!(
                "unsupported DLC source identity grammar: {source_id:?}"
            ))
        }
    };
    if !id.starts_with("dlc")
        || id.len() <= 3
        || !id
            .bytes()
            .all(|byte| byte.is_ascii_lowercase() || byte.is_ascii_digit() || byte == b'_')
    {
        return Err(format!("invalid stable DLC selection identity {id:?}"));
    }
    Ok(id.to_string())
}

fn validate_source_game(source: &SourceGameIdentity) -> Result<(), String> {
    if source.version.trim().is_empty()
        || source.build.trim().is_empty()
        || source.build_profile.trim().is_empty()
    {
        return Err("source game version/build/build profile is blank".into());
    }
    validate_sha256(&source.executable_sha256, "source executable")
}

fn validate_source_profile(profile: &SourceProfileIdentity) -> Result<(), String> {
    if profile.profile_id.trim().is_empty() || profile.playset_id.trim().is_empty() {
        return Err("source profile/playset id is blank".into());
    }
    validate_portable_label(&profile.profile_id, "source profile id")?;
    validate_portable_label(&profile.playset_id, "source playset id")?;
    validate_sha256(&profile.selection_sha256, "source profile selection")
}

fn validate_layer(layer: &RecordedContentLayer) -> Result<(), String> {
    validate_portable_layer_id(&layer.layer_id)?;
    if layer.display_name.trim().is_empty() {
        return Err("content layer id/display name is blank".into());
    }
    validate_portable_label(&layer.display_name, "content layer display name")?;
    if let Some(hash) = &layer.descriptor_sha256 {
        validate_sha256(hash, "layer descriptor")?;
    }
    if let Some(hash) = &layer.source_container_sha256 {
        validate_sha256(hash, "layer source container")?;
    }
    if layer.storage_kind == LayerStorageKind::ZipArchive && layer.source_container_sha256.is_none()
    {
        return Err(format!(
            "archive layer {} has no archive hash",
            layer.layer_id
        ));
    }
    // Normalize (with per-path error propagation) BEFORE the set-length
    // check, so an invalid path reports its accurate error and a length
    // shrink genuinely means duplicate-after-normalization.
    let normalized = normalized_path_set(&layer.replace_paths)?;
    // Duplicate-ness is a Windows filesystem question, so it is decided on the
    // collision key. The canonical set is what the digest sees and is left
    // untouched.
    let collisions = collision_key_set(&normalized)?;
    if collisions.len() != layer.replace_paths.len() {
        return Err(format!(
            "layer {} has duplicate replace_path",
            layer.layer_id
        ));
    }
    Ok(())
}

fn validate_used_content_key(content: &UsedContentKey) -> Result<(), String> {
    validate_sha256(
        &content.effective_vfs_manifest_sha256,
        "content effective VFS",
    )?;
    validate_sha256(&content.source_sha256, "content source")?;
    validate_sha256(&content.payload_sha256, "content payload")?;
    if content.payload_bytes > MAX_CONTENT_PAYLOAD_BYTES {
        return Err(format!(
            "content payload exceeds the {MAX_CONTENT_PAYLOAD_BYTES}-byte ceiling: {}",
            content.payload_bytes
        ));
    }
    let normalized = normalize_game_relative_path(&content.normalized_game_relative_path)?;
    if normalized != content.normalized_game_relative_path {
        return Err(format!(
            "content path is not normalized: {}",
            content.normalized_game_relative_path
        ));
    }
    validate_portable_layer_id(&content.layer_id)?;
    Ok(())
}

/// Layer identities are portable semantic labels, never physical mount paths.
/// A colon may be part of a legacy semantic identity (for example
/// `base:fixture-runtime`), so reject actual absolute/traversal path grammars
/// without narrowing that existing namespace.
fn validate_portable_layer_id(layer_id: &str) -> Result<(), String> {
    let value = layer_id.trim();
    let slash = value.replace('\\', "/");
    let drive_prefix = slash.as_bytes().get(1) == Some(&b':')
        && slash
            .as_bytes()
            .first()
            .is_some_and(u8::is_ascii_alphabetic);
    if value.is_empty()
        || value != layer_id
        || value.chars().any(char::is_control)
        || slash.starts_with('/')
        || slash.starts_with("//")
        || drive_prefix
        || slash
            .split('/')
            .any(|part| part.is_empty() || part == "." || part == "..")
    {
        return Err(format!(
            "content layer id is unsafe/non-portable: {layer_id:?}"
        ));
    }
    Ok(())
}

/// Normalize a replace_path list into a set, propagating the first
/// normalization error instead of silently dropping un-normalizable entries.
fn normalized_path_set(paths: &[String]) -> Result<BTreeSet<String>, String> {
    paths
        .iter()
        .map(|path| normalize_game_relative_path(path))
        .collect()
}

/// As [`normalized_path_set`], additionally refusing declarations that
/// duplicate after normalization instead of letting them silently collapse
/// into one set member. Used for caller-supplied (playback-side) lists; the
/// manifest side is already refused by [`validate_layer`].
///
/// "Duplicate" means [`windows_collision_key`]-equal, not canonically equal:
/// `gfx/entities` and `GFX/ENTITIES.` are one directory on the playback host.
/// The RETURNED set is still the canonical one, so
/// [`compute_selection_sha256`] hashes exactly the bytes it always did.
fn unique_normalized_path_set(paths: &[String]) -> Result<BTreeSet<String>, String> {
    let normalized = normalized_path_set(paths)?;
    let collisions = collision_key_set(&normalized)?;
    if collisions.len() != paths.len() {
        return Err("replace_path declarations duplicate after normalization".into());
    }
    Ok(normalized)
}

fn compare_scalar(
    issues: &mut Vec<ContentIssue>,
    code: ContentIssueCode,
    subject: &str,
    expected: &str,
    actual: &str,
) {
    if expected != actual {
        issues.push(issue(
            code,
            subject,
            format!("required {expected:?}, playback has {actual:?}"),
        ));
    }
}

fn issue(code: ContentIssueCode, subject: impl Into<String>, message: String) -> ContentIssue {
    ContentIssue {
        code,
        subject: subject.into(),
        message,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn h(ch: char) -> String {
        ch.to_string().repeat(64)
    }

    fn layer(
        ordinal: u32,
        kind: ContentLayerKind,
        id: &str,
        replace_paths: &[&str],
        required: bool,
    ) -> RecordedContentLayer {
        RecordedContentLayer {
            ordinal,
            layer_kind: kind,
            layer_id: id.into(),
            display_name: id.into(),
            descriptor_sha256: (kind == ContentLayerKind::Mod).then(|| h('d')),
            source_container_sha256: None,
            storage_kind: LayerStorageKind::Directory,
            replace_paths: replace_paths
                .iter()
                .map(|value| value.to_string())
                .collect(),
            required_for_playback: required,
        }
    }

    fn required(
        key: &str,
        path: &str,
        ordinal: u32,
        kind: ContentLayerKind,
        layer_id: &str,
        payload: char,
    ) -> RequiredContent {
        RequiredContent {
            content_key: key.into(),
            kind: RequiredContentKind::Definition,
            content: UsedContentKey {
                effective_vfs_manifest_sha256: h('f'),
                normalized_game_relative_path: path.into(),
                winning_layer_ordinal: ordinal,
                layer_kind: kind,
                layer_id: layer_id.into(),
                source_sha256: h(payload),
                payload_sha256: h(payload),
                payload_bytes: 17,
            },
            observation_count: 1,
            consumers: vec!["test-renderer".into()],
        }
    }

    fn manifest() -> ContentManifest {
        let mut manifest = ContentManifest {
            schema: SCHEMA.into(),
            product_modset_sha256: h('9'),
            source_game: SourceGameIdentity {
                version: "1.19.2".into(),
                build: "a729".into(),
                executable_sha256: h('a'),
                build_profile: "a729-870ebf".into(),
            },
            source_profile: SourceProfileIdentity {
                profile_id: "profile-stable".into(),
                playset_id: "playset-stable".into(),
                selection_sha256: h('9'),
            },
            effective_vfs_manifest_sha256: h('f'),
            ordered_layers: vec![
                layer(0, ContentLayerKind::Base, "base", &[], true),
                layer(1, ContentLayerKind::Dlc, "dlc-used", &[], true),
                layer(2, ContentLayerKind::Mod, "mod-unused", &[], false),
                layer(
                    3,
                    ContentLayerKind::Mod,
                    "mod-override",
                    &["gfx/entities"],
                    true,
                ),
            ],
            dlc_selection: vec![
                DlcSelection {
                    dlc_id: "dlc-used".into(),
                    enabled: true,
                    required_for_playback: true,
                },
                DlcSelection {
                    dlc_id: "dlc-unused".into(),
                    enabled: true,
                    required_for_playback: false,
                },
            ],
            localisation_language: "l_english".into(),
            required_settings: BTreeMap::from([("graphics.texture_quality".into(), "high".into())]),
            recorder_local_settings: BTreeMap::new(),
            required_content: vec![
                required(
                    "entity-definition",
                    "gfx/entities/unit.asset",
                    3,
                    ContentLayerKind::Mod,
                    "mod-override",
                    'b',
                ),
                required(
                    "mesh",
                    "gfx/models/unit.mesh",
                    1,
                    ContentLayerKind::Dlc,
                    "dlc-used",
                    'c',
                ),
            ],
            manifest_digest_sha256: None,
        };
        manifest.source_profile.selection_sha256 = manifest.computed_selection_sha256().unwrap();
        manifest.with_computed_digest().unwrap()
    }

    fn inventory(manifest: &ContentManifest) -> PlaybackContentInventory {
        PlaybackContentInventory {
            source_game: manifest.source_game.clone(),
            source_profile: manifest.source_profile.clone(),
            layers: manifest
                .ordered_layers
                .iter()
                .filter(|layer| layer.required_for_playback)
                .map(|layer| PlaybackContentLayer {
                    recorded_ordinal: layer.ordinal,
                    layer_kind: layer.layer_kind,
                    layer_id: layer.layer_id.clone(),
                    descriptor_sha256: layer.descriptor_sha256.clone(),
                    source_container_sha256: layer.source_container_sha256.clone(),
                    storage_kind: layer.storage_kind,
                    replace_paths: layer.replace_paths.clone(),
                })
                .collect(),
            dlc_selection: manifest.dlc_selection.clone(),
            localisation_language: manifest.localisation_language.clone(),
            settings: manifest.required_settings.clone(),
            resolved_content: manifest
                .required_content
                .iter()
                .map(|required| ResolvedPlaybackContent {
                    normalized_game_relative_path: required
                        .content
                        .normalized_game_relative_path
                        .clone(),
                    winning_layer_ordinal: required.content.winning_layer_ordinal,
                    layer_kind: required.content.layer_kind,
                    layer_id: required.content.layer_id.clone(),
                    source_sha256: required.content.source_sha256.clone(),
                    payload_sha256: required.content.payload_sha256.clone(),
                    payload_bytes: required.content.payload_bytes,
                })
                .collect(),
        }
    }

    #[test]
    fn mod_override_replace_path_and_relocated_roots_validate() {
        let manifest = manifest();
        let mut inventory = inventory(&manifest);
        inventory.source_profile.profile_id = "different-local-label".into();
        inventory.source_profile.playset_id = "different-local-playset-label".into();
        inventory.source_profile.selection_sha256 = h('0');
        let report = manifest.validate_playback(&inventory);
        assert!(report.compatible, "{:?}", report.issues);
        assert_eq!(report.verified_required_content, 2);
        let admission = manifest.admit_playback(&inventory).unwrap();
        assert!(admission
            .require_used_content("GFX\\Entities\\Unit.Asset")
            .is_ok());
        assert!(admission
            .require_used_content("gfx/entities/unrecorded.asset")
            .is_err());
        let consumer_content = admission
            .require_used_content_for_consumer("test-renderer")
            .unwrap();
        assert_eq!(consumer_content.len(), 2);
        assert_eq!(consumer_content[0].layer_id, "mod-override");
        assert_eq!(consumer_content[1].layer_id, "dlc-used");
        assert!(admission
            .require_used_content_for_consumer("hud_role/v1/topbar/gui/unrecorded")
            .is_err());
        let json = String::from_utf8(manifest.encode_json().unwrap()).unwrap();
        assert!(!json.contains("C:\\"));
        assert!(!json.contains("F:\\"));
    }

    #[test]
    fn changed_asset_definition_and_missing_content_fail_closed() {
        let manifest = manifest();
        let mut inventory = inventory(&manifest);
        inventory.resolved_content[0].payload_sha256 = h('e');
        inventory.resolved_content.pop();
        let report = manifest.validate_playback(&inventory);
        assert!(!report.compatible);
        assert!(report
            .issues
            .iter()
            .any(|issue| issue.code == ContentIssueCode::PayloadHashMismatch));
        assert!(report
            .issues
            .iter()
            .any(|issue| issue.code == ContentIssueCode::MissingContent));
    }

    #[test]
    fn replace_path_and_winning_layer_changes_fail_closed() {
        let manifest = manifest();
        let mut inventory = inventory(&manifest);
        inventory
            .layers
            .iter_mut()
            .find(|layer| layer.layer_id == "mod-override")
            .unwrap()
            .replace_paths
            .clear();
        inventory.resolved_content[0].winning_layer_ordinal = 0;
        inventory.resolved_content[0].layer_id = "base".into();
        inventory.resolved_content[0].layer_kind = ContentLayerKind::Base;
        let report = manifest.validate_playback(&inventory);
        assert!(report
            .issues
            .iter()
            .any(|issue| issue.code == ContentIssueCode::ReplacePathMismatch));
        assert!(report
            .issues
            .iter()
            .any(|issue| issue.code == ContentIssueCode::WinningLayerMismatch));
    }

    #[test]
    fn duplicate_playback_inventory_rows_fail_closed() {
        let manifest = manifest();

        let mut dup_resolved = inventory(&manifest);
        let mut shadowing = dup_resolved.resolved_content[0].clone();
        shadowing.payload_sha256 = h('e');
        dup_resolved.resolved_content.insert(0, shadowing);
        let report = manifest.validate_playback(&dup_resolved);
        assert!(!report.compatible);
        assert!(report.issues.iter().any(|issue| {
            issue.code == ContentIssueCode::InvalidPlaybackInventory
                && issue.message.contains("duplicate resolved path row")
        }));

        let mut dup_layer = inventory(&manifest);
        let repeated = dup_layer.layers[0].clone();
        dup_layer.layers.push(repeated);
        let report = manifest.validate_playback(&dup_layer);
        assert!(!report.compatible);
        assert!(report.issues.iter().any(|issue| {
            issue.code == ContentIssueCode::InvalidPlaybackInventory
                && issue.message.contains("duplicate layer ordinal")
        }));
        assert!(report.issues.iter().any(|issue| {
            issue.code == ContentIssueCode::InvalidPlaybackInventory
                && issue.message.contains("duplicate layer id")
        }));

        // Ordinal-uniqueness and id-uniqueness are independent refusals: a
        // decoy id sharing an ordinal must not slip past a composite-tuple
        // check...
        let mut dup_ordinal = inventory(&manifest);
        let mut decoy = dup_ordinal.layers[0].clone();
        decoy.layer_id = "decoy".into();
        dup_ordinal.layers.push(decoy);
        let report = manifest.validate_playback(&dup_ordinal);
        assert!(!report.compatible);
        assert!(report.issues.iter().any(|issue| {
            issue.code == ContentIssueCode::InvalidPlaybackInventory
                && issue.message.contains("duplicate layer ordinal")
        }));

        // ...and neither must one id declared at two ordinals.
        let mut dup_id = inventory(&manifest);
        let mut moved = dup_id.layers[0].clone();
        moved.recorded_ordinal = 250;
        dup_id.layers.push(moved);
        let report = manifest.validate_playback(&dup_id);
        assert!(!report.compatible);
        assert!(report.issues.iter().any(|issue| {
            issue.code == ContentIssueCode::InvalidPlaybackInventory
                && issue.message.contains("duplicate layer id")
        }));

        let mut dup_dlc = inventory(&manifest);
        let repeated = dup_dlc.dlc_selection[0].clone();
        dup_dlc.dlc_selection.push(repeated);
        let report = manifest.validate_playback(&dup_dlc);
        assert!(!report.compatible);
        assert!(report.issues.iter().any(|issue| {
            issue.code == ContentIssueCode::InvalidPlaybackInventory
                && issue.message.contains("duplicate DLC row")
        }));
    }

    #[test]
    fn playback_replace_paths_duplicating_after_normalization_fail_closed() {
        // manifest expects ["gfx/entities"]; playback declares
        // ["gfx/entities", "GFX\\ENTITIES"] which normalize into ONE set
        // member. Both the set comparison and the selection-hash construction
        // must refuse the collapse instead of reporting compatible=true.
        let manifest = manifest();
        let mut inventory = inventory(&manifest);
        inventory
            .layers
            .iter_mut()
            .find(|layer| layer.layer_id == "mod-override")
            .unwrap()
            .replace_paths = vec!["gfx/entities".into(), "GFX\\ENTITIES".into()];
        let report = manifest.validate_playback(&inventory);
        assert!(!report.compatible);
        assert!(report.issues.iter().any(|issue| {
            issue.code == ContentIssueCode::ReplacePathMismatch
                && issue.message.contains("duplicate after normalization")
        }));
        assert!(report.issues.iter().any(|issue| {
            issue.code == ContentIssueCode::InvalidPlaybackInventory
                && issue.message.contains("duplicate after normalization")
        }));
    }

    #[test]
    fn malformed_ambient_playback_rows_fail_closed() {
        let manifest = manifest();

        // A correctly matching required row must not mask a second spelling
        // that canonicalizes to the same VFS path.
        let mut alias = inventory(&manifest);
        let mut extra = alias.resolved_content[0].clone();
        extra.normalized_game_relative_path = extra
            .normalized_game_relative_path
            .replace('/', "\\")
            .to_ascii_uppercase();
        alias.resolved_content.push(extra);
        let report = manifest.validate_playback(&alias);
        assert!(!report.compatible);
        assert!(report.issues.iter().any(|issue| {
            issue.code == ContentIssueCode::InvalidPlaybackInventory
                && issue.message.contains("resolved path is not normalized")
        }));

        let mut traversal = inventory(&manifest);
        let mut extra = traversal.resolved_content[0].clone();
        extra.normalized_game_relative_path = "../escape.asset".into();
        traversal.resolved_content.push(extra);
        let report = manifest.validate_playback(&traversal);
        assert!(!report.compatible);
        assert!(report
            .issues
            .iter()
            .any(|issue| issue.code == ContentIssueCode::InvalidPlaybackInventory));

        let mut bad_hash = inventory(&manifest);
        let mut extra = bad_hash.resolved_content[0].clone();
        extra.normalized_game_relative_path = "gfx/entities/unused.asset".into();
        extra.payload_sha256 = "not-a-sha256".into();
        bad_hash.resolved_content.push(extra);
        let report = manifest.validate_playback(&bad_hash);
        assert!(!report.compatible);
        assert!(report.issues.iter().any(|issue| {
            issue.code == ContentIssueCode::InvalidPlaybackInventory
                && issue.message.contains("playback content payload")
        }));
    }

    #[test]
    fn malformed_ambient_layer_dlc_and_settings_fail_closed() {
        let manifest = manifest();

        let mut bad_layer = inventory(&manifest);
        let mut extra = bad_layer.layers[0].clone();
        extra.recorded_ordinal = 999;
        extra.layer_id = "../unsafe-layer".into();
        bad_layer.layers.push(extra);
        assert!(!manifest.validate_playback(&bad_layer).compatible);

        let mut blank_dlc = inventory(&manifest);
        blank_dlc.dlc_selection.push(DlcSelection {
            dlc_id: " ".into(),
            enabled: true,
            required_for_playback: false,
        });
        assert!(!manifest.validate_playback(&blank_dlc).compatible);

        let mut blank_setting = inventory(&manifest);
        blank_setting.settings.insert("ambient".into(), " ".into());
        assert!(!manifest.validate_playback(&blank_setting).compatible);
    }

    #[test]
    fn unnormalizable_playback_replace_path_fails_closed() {
        let manifest = manifest();
        let mut inventory = inventory(&manifest);
        inventory
            .layers
            .iter_mut()
            .find(|layer| layer.layer_id == "mod-override")
            .unwrap()
            .replace_paths = vec!["../escape".into(), "gfx/entities".into()];
        let report = manifest.validate_playback(&inventory);
        assert!(!report.compatible);
        assert!(report.issues.iter().any(|issue| {
            issue.code == ContentIssueCode::ReplacePathMismatch
                && issue.message.contains("unsafe/non-canonical")
        }));
    }

    /// A row refused as malformed is present, not absent. Reporting it as a
    /// load-order or missing-layer failure would name a remediation (reorder
    /// the playset / install the mod) that cannot fix the actual defect and
    /// that misstates the operator's install.
    #[test]
    fn refused_playback_layer_is_not_reported_as_absent() {
        let manifest = manifest();
        for bad_paths in [
            vec!["gfx/entities".to_string(), "GFX\\ENTITIES".to_string()],
            vec!["../escape".to_string(), "gfx/entities".to_string()],
        ] {
            let mut inventory = inventory(&manifest);
            inventory
                .layers
                .iter_mut()
                .find(|layer| layer.layer_id == "mod-override")
                .unwrap()
                .replace_paths = bad_paths.clone();
            let report = manifest.validate_playback(&inventory);
            assert!(!report.compatible, "{bad_paths:?}");
            assert!(
                report.issues.iter().any(|issue| {
                    issue.code == ContentIssueCode::ReplacePathMismatch
                        && issue.subject == "layer:mod-override"
                }),
                "{bad_paths:?}: {:#?}",
                report.issues
            );
            assert!(
                !report.issues.iter().any(|issue| {
                    issue.code == ContentIssueCode::LayerOrderMismatch
                        || issue.code == ContentIssueCode::MissingLayer
                }),
                "{bad_paths:?}: refused layer reported as absent: {:#?}",
                report.issues
            );
        }
    }

    /// As above for resolved content rows: a row that is present but malformed
    /// must not also be announced as missing.
    #[test]
    fn refused_resolved_content_row_is_not_reported_as_missing() {
        let manifest = manifest();
        let mut inventory = inventory(&manifest);
        let row = inventory
            .resolved_content
            .iter_mut()
            .find(|row| row.normalized_game_relative_path == "gfx/entities/unit.asset")
            .unwrap();
        row.payload_sha256 = "not-a-sha256".into();
        let report = manifest.validate_playback(&inventory);
        assert!(!report.compatible);
        assert!(report.issues.iter().any(|issue| {
            issue.code == ContentIssueCode::InvalidPlaybackInventory
                && issue.message.contains("playback content payload")
        }));
        assert!(
            !report
                .issues
                .iter()
                .any(|issue| issue.code == ContentIssueCode::MissingContent),
            "present-but-malformed row reported as missing: {:#?}",
            report.issues
        );

        // The suppression is keyed on the refused path, so a genuinely absent
        // required row still reports MissingContent.
        let mut absent = inventory.clone();
        absent
            .resolved_content
            .retain(|row| row.normalized_game_relative_path != "gfx/entities/unit.asset");
        assert!(absent
            .resolved_content
            .iter()
            .all(|row| row.normalized_game_relative_path != "gfx/entities/unit.asset"));
        assert!(manifest
            .validate_playback(&absent)
            .issues
            .iter()
            .any(|issue| issue.code == ContentIssueCode::MissingContent));
    }

    /// A decoy row that claims the required ordinal first makes the GENUINE row
    /// the duplicate, so the genuine row -- at its correct recorded ordinal --
    /// is the one skipped. Reporting that as a load-order violation would tell
    /// the operator to reorder a playset that is already in the right order.
    #[test]
    fn decoy_layer_row_does_not_frame_the_genuine_row_as_misordered() {
        let manifest = manifest();
        let mut inventory = inventory(&manifest);
        let position = inventory
            .layers
            .iter()
            .position(|layer| layer.layer_id == "mod-override")
            .unwrap();
        let genuine = inventory.layers[position].clone();
        inventory.layers.insert(
            position,
            PlaybackContentLayer {
                layer_id: "decoy".into(),
                descriptor_sha256: None,
                replace_paths: Vec::new(),
                ..genuine.clone()
            },
        );
        assert_eq!(inventory.layers[position].recorded_ordinal, 3);
        assert_eq!(inventory.layers[position + 1].layer_id, "mod-override");
        let report = manifest.validate_playback(&inventory);
        assert!(!report.compatible);
        assert!(report.issues.iter().any(|issue| {
            issue.code == ContentIssueCode::InvalidPlaybackInventory
                && issue.message.contains("duplicate layer ordinal 3")
        }));
        assert!(
            !report
                .issues
                .iter()
                .any(|issue| issue.code == ContentIssueCode::LayerOrderMismatch),
            "genuine row at its recorded ordinal reported as misordered: {:#?}",
            report.issues
        );
    }

    /// A required content row spelled in a way this normalizer refuses is still
    /// a spelling OF that row, so it must not also be called missing. That holds
    /// both for spellings the normalizer can canonicalize (case/separator) and
    /// for ones it rejects outright but that a real filesystem still resolves to
    /// the same file (dot segments, interior `..`).
    #[test]
    fn refused_spellings_of_a_required_path_are_not_reported_as_missing() {
        let manifest = manifest();
        for spelling in [
            "GFX\\Entities\\Unit.Asset",
            "gfx/./entities/unit.asset",
            "gfx/models/../entities/unit.asset",
            // DEL is legal in an NTFS filename, so this is a real component that
            // the following ".." then pops: still an alias of the required path.
            // Pins the control-character bound at Win32's 0x00-0x1f, no wider.
            "gfx/\u{7f}/../entities/unit.asset",
        ] {
            let mut inventory = inventory(&manifest);
            inventory
                .resolved_content
                .iter_mut()
                .find(|row| row.normalized_game_relative_path == "gfx/entities/unit.asset")
                .unwrap()
                .normalized_game_relative_path = spelling.into();
            let report = manifest.validate_playback(&inventory);
            assert!(!report.compatible, "{spelling:?} was admitted");
            assert!(
                report
                    .issues
                    .iter()
                    .any(|issue| issue.code == ContentIssueCode::InvalidPlaybackInventory),
                "{spelling:?} was not refused: {:#?}",
                report.issues
            );
            assert!(
                !report
                    .issues
                    .iter()
                    .any(|issue| issue.code == ContentIssueCode::MissingContent),
                "{spelling:?} reported as missing: {:#?}",
                report.issues
            );
        }
    }

    /// The suppression above must stay narrow. A refused row that does not
    /// unambiguously name the required FILE inside the game root -- because it
    /// escapes the root, is drive-qualified, or names a directory via a trailing
    /// separator or dot -- is not a spelling of that path, so the path really is
    /// missing and must say so.
    #[test]
    fn a_refused_row_that_escapes_the_game_root_still_leaves_the_path_missing() {
        let manifest = manifest();
        for spelling in [
            "../gfx/entities/unit.asset",
            "c:/gfx/entities/unit.asset",
            "gfx/entities/unit.asset/",
            "gfx/entities/unit.asset/.",
            "gfx/entities/unit.asset/..",
            "gfx//entities/unit.asset",
            // A component that cannot be a Windows filename must not survive
            // just long enough for the next ".." to pop it off again.
            "\u{0}/../gfx/entities/unit.asset",
            "\u{1f}/../gfx/entities/unit.asset",
        ] {
            let mut inventory = inventory(&manifest);
            inventory
                .resolved_content
                .iter_mut()
                .find(|row| row.normalized_game_relative_path == "gfx/entities/unit.asset")
                .unwrap()
                .normalized_game_relative_path = spelling.into();
            let report = manifest.validate_playback(&inventory);
            assert!(!report.compatible);
            assert!(
                report.issues.iter().any(|issue| {
                    issue.code == ContentIssueCode::MissingContent
                        && issue.subject == "gfx/entities/unit.asset"
                }),
                "{spelling:?}: required path is genuinely absent but was not reported: {:#?}",
                report.issues
            );
        }
    }

    /// Every comparison class carries its own issue code, because the code (not
    /// the free-text message) is what selects the operator-facing remediation
    /// downstream. Without this, a class could silently collapse into another
    /// code — which is exactly how the replace_path classes drifted.
    #[test]
    fn each_playback_mismatch_class_reports_its_own_code() {
        let manifest = manifest();
        type Perturb = fn(&mut PlaybackContentInventory);
        let cases: [(&str, Perturb, ContentIssueCode); 20] = [
            (
                "game version",
                |inventory| inventory.source_game.version = "1.19.3".into(),
                ContentIssueCode::GameVersionMismatch,
            ),
            (
                "executable hash",
                |inventory| inventory.source_game.executable_sha256 = h('e'),
                ContentIssueCode::ExecutableMismatch,
            ),
            (
                "build profile",
                |inventory| inventory.source_game.build_profile = "a729-other".into(),
                ContentIssueCode::BuildProfileMismatch,
            ),
            (
                "build",
                |inventory| inventory.source_game.build = "a730".into(),
                ContentIssueCode::BuildProfileMismatch,
            ),
            (
                "localisation language",
                |inventory| inventory.localisation_language = "l_french".into(),
                ContentIssueCode::LanguageMismatch,
            ),
            (
                "required setting",
                |inventory| {
                    inventory
                        .settings
                        .insert("graphics.texture_quality".into(), "low".into());
                },
                ContentIssueCode::SettingMismatch,
            ),
            (
                "required layer absent",
                |inventory| {
                    inventory
                        .layers
                        .retain(|layer| layer.layer_id != "mod-override")
                },
                ContentIssueCode::MissingLayer,
            ),
            (
                "required layer at a different ordinal",
                |inventory| {
                    inventory
                        .layers
                        .iter_mut()
                        .find(|layer| layer.layer_id == "mod-override")
                        .unwrap()
                        .recorded_ordinal = 7;
                },
                ContentIssueCode::LayerOrderMismatch,
            ),
            (
                "required layer descriptor identity",
                |inventory| {
                    inventory
                        .layers
                        .iter_mut()
                        .find(|layer| layer.layer_id == "mod-override")
                        .unwrap()
                        .descriptor_sha256 = Some(h('e'));
                },
                ContentIssueCode::LayerIdentityMismatch,
            ),
            (
                "resolved source hash",
                |inventory| inventory.resolved_content[0].source_sha256 = h('e'),
                ContentIssueCode::SourceHashMismatch,
            ),
            (
                "resolved payload length",
                |inventory| inventory.resolved_content[0].payload_bytes = 99,
                ContentIssueCode::PayloadLengthMismatch,
            ),
            (
                "resolved payload hash",
                |inventory| inventory.resolved_content[0].payload_sha256 = h('e'),
                ContentIssueCode::PayloadHashMismatch,
            ),
            (
                "required content absent",
                |inventory| {
                    inventory.resolved_content.retain(|row| {
                        row.normalized_game_relative_path != "gfx/models/unit.mesh"
                    })
                },
                ContentIssueCode::MissingContent,
            ),
            (
                "resolved winning layer",
                |inventory| inventory.resolved_content[0].winning_layer_ordinal = 1,
                ContentIssueCode::WinningLayerMismatch,
            ),
            (
                "required DLC disabled",
                |inventory| {
                    inventory
                        .dlc_selection
                        .iter_mut()
                        .find(|dlc| dlc.dlc_id == "dlc-used")
                        .unwrap()
                        .enabled = false;
                },
                ContentIssueCode::RequiredDlcMismatch,
            ),
            (
                "replace_path declarations differ",
                |inventory| {
                    inventory
                        .layers
                        .iter_mut()
                        .find(|layer| layer.layer_id == "mod-override")
                        .unwrap()
                        .replace_paths = vec!["gfx/models".into()];
                },
                ContentIssueCode::ReplacePathMismatch,
            ),
            (
                "portable selection identity differs",
                |inventory| {
                    inventory
                        .layers
                        .iter_mut()
                        .find(|layer| layer.layer_id == "mod-override")
                        .unwrap()
                        .storage_kind = LayerStorageKind::ZipArchive;
                    inventory
                        .layers
                        .iter_mut()
                        .find(|layer| layer.layer_id == "mod-override")
                        .unwrap()
                        .source_container_sha256 = Some(h('7'));
                },
                ContentIssueCode::ProfileMismatch,
            ),
            // The caller-inventory class, which used to be indistinguishable
            // from a malformed REPLAY manifest.
            (
                "caller inventory has a blank setting key",
                |inventory| {
                    inventory.settings.insert(" ".into(), "value".into());
                },
                ContentIssueCode::InvalidPlaybackInventory,
            ),
            (
                "caller inventory repeats a resolved row",
                |inventory| {
                    let duplicate = inventory.resolved_content[0].clone();
                    inventory.resolved_content.push(duplicate);
                },
                ContentIssueCode::InvalidPlaybackInventory,
            ),
            (
                "caller inventory spells a required path as a Windows alias",
                |inventory| {
                    inventory.resolved_content[1].normalized_game_relative_path =
                        "gfx/models/unit.mesh.".into();
                },
                ContentIssueCode::InvalidPlaybackInventory,
            ),
        ];
        for (name, perturb, expected) in cases {
            let mut inventory = inventory(&manifest);
            perturb(&mut inventory);
            let report = manifest.validate_playback(&inventory);
            assert!(!report.compatible, "{name} was admitted");
            assert!(
                report.issues.iter().any(|issue| issue.code == expected),
                "{name}: expected {expected:?}, got {:#?}",
                report.issues
            );
        }
    }

    #[test]
    fn only_used_dlc_and_mod_layers_are_required() {
        let manifest = manifest();
        let mut inventory = inventory(&manifest);
        inventory
            .dlc_selection
            .retain(|dlc| dlc.dlc_id == "dlc-used");
        assert!(manifest.validate_playback(&inventory).compatible);
        let mut missing_required = inventory.clone();
        missing_required.dlc_selection.clear();
        assert!(missing_required.dlc_selection.is_empty());
        assert!(manifest
            .validate_playback(&missing_required)
            .issues
            .iter()
            .any(|issue| issue.code == ContentIssueCode::RequiredDlcMismatch));
        inventory
            .dlc_selection
            .iter_mut()
            .find(|dlc| dlc.dlc_id == "dlc-used")
            .unwrap()
            .enabled = false;
        let report = manifest.validate_playback(&inventory);
        assert!(report
            .issues
            .iter()
            .any(|issue| issue.code == ContentIssueCode::RequiredDlcMismatch));
    }

    #[test]
    fn launcher_descriptor_dlc_identity_normalizes_to_short_id() {
        assert_eq!(
            normalize_dlc_selection_id(r"DLC\DLC003_ROCKET_LAUNCHER_UNIT_PACK\DLC003.DLC").unwrap(),
            "dlc003_rocket_launcher_unit_pack"
        );
        assert_eq!(normalize_dlc_selection_id("dlc020").unwrap(), "dlc020");
        assert_eq!(
            normalize_dlc_selection_id("dlc/dlc020/dlc021.dlc").unwrap(),
            "dlc020"
        );
        assert!(normalize_dlc_selection_id("dlc/dlc020/not-a-dlc.mod").is_err());
        assert!(normalize_dlc_selection_id("mod/ugc_123.mod").is_err());
    }

    #[test]
    fn digest_and_normalized_paths_are_fail_closed() {
        let mut bad_digest = manifest();
        bad_digest.manifest_digest_sha256 = Some(h('0'));
        assert!(bad_digest
            .validate()
            .unwrap_err()
            .contains("digest mismatch"));
        assert_eq!(
            normalize_game_relative_path(r"GFX\FX\Particle.SHADER").unwrap(),
            "gfx/fx/particle.shader"
        );
        assert_eq!(
            normalize_game_relative_path("gfx/Cafe\u{301}.asset").unwrap(),
            "gfx/cafe\u{301}.asset"
        );
        assert!(normalize_game_relative_path("../escape").is_err());
        assert!(normalize_game_relative_path("C:/absolute").is_err());

        let mut leaked_layer = manifest();
        leaked_layer.manifest_digest_sha256 = None;
        leaked_layer.ordered_layers[0].layer_id = r"C:\\Games\\HOI4".into();
        assert!(leaked_layer
            .validate()
            .unwrap_err()
            .contains("unsafe/non-portable"));

        let mut leaked_content_layer = manifest();
        leaked_content_layer.manifest_digest_sha256 = None;
        leaked_content_layer.required_content[0].content.layer_id =
            r"\\server\\profile\\mod".into();
        assert!(leaked_content_layer
            .validate()
            .unwrap_err()
            .contains("unsafe/non-portable"));

        for leaked in [
            r"C:\Users\Example\OneDrive\profile",
            r"\\server\share\playset",
            "loaded from /home/player/profile",
        ] {
            let mut leaked_profile = manifest();
            leaked_profile.manifest_digest_sha256 = None;
            leaked_profile.source_profile.profile_id = leaked.into();
            assert!(leaked_profile
                .validate()
                .unwrap_err()
                .contains("unsafe/non-portable"));

            let mut leaked_display = manifest();
            leaked_display.manifest_digest_sha256 = None;
            leaked_display.ordered_layers[0].display_name = leaked.into();
            assert!(leaked_display
                .validate()
                .unwrap_err()
                .contains("unsafe/non-portable"));
        }
    }

    #[test]
    fn v1_or_missing_product_modset_identity_is_not_admissible() {
        let current = manifest();
        let mut legacy = serde_json::to_value(&current).unwrap();
        legacy["schema"] = serde_json::json!("hoi4.portable-content-manifest.v1");
        legacy
            .as_object_mut()
            .unwrap()
            .remove("product_modset_sha256");
        assert!(ContentManifest::decode_json(&serde_json::to_vec(&legacy).unwrap()).is_err());

        let mut malformed = current;
        malformed.manifest_digest_sha256 = None;
        malformed.product_modset_sha256 = "not-a-sha".into();
        assert!(malformed.validate().unwrap_err().contains("product modset"));
    }

    /// Add one more required row spelling `path`, leaving everything else in
    /// the manifest exactly as it was, and report what validation says.
    fn manifest_with_extra_required_path(path: &str) -> Result<(), String> {
        let mut manifest = manifest();
        manifest.manifest_digest_sha256 = None;
        let mut extra = manifest.required_content[1].clone();
        extra.content_key = "alias-row".into();
        extra.content.normalized_game_relative_path = path.into();
        manifest.required_content.push(extra);
        manifest.validate()
    }

    /// GAP 1 -- case beyond ASCII. `to_ascii_lowercase` leaves every non-ASCII
    /// letter alone, so both spellings are their own canonical form and the
    /// canonical-form duplicate rule saw two paths. NTFS sees one file:
    /// measured on Windows 11 build 26200, writing `fĄ.txt` overwrote
    /// `fą.txt` (and the same for É/é, Г/г, Σ/σ, Ａ/ａ). Evidence:
    /// extractor/native/runs/win_alias_key/fs_aliasing_probe.txt.
    #[test]
    fn non_ascii_case_spellings_are_one_required_path() {
        for (first, second) in [
            ("gfx/models/ą.mesh", "gfx/models/Ą.mesh"),
            ("gfx/models/café.mesh", "gfx/models/cafÉ.mesh"),
            ("gfx/models/г.mesh", "gfx/models/Г.mesh"),
            ("gfx/models/σ.mesh", "gfx/models/Σ.mesh"),
            ("gfx/models/ａ.mesh", "gfx/models/Ａ.mesh"),
        ] {
            assert_eq!(
                normalize_game_relative_path(first).unwrap(),
                first,
                "{first:?} must already be canonical, or this proves nothing"
            );
            assert_eq!(normalize_game_relative_path(second).unwrap(), second);
            assert_ne!(first, second, "the two canonical forms must differ");
            let mut manifest = manifest();
            manifest.manifest_digest_sha256 = None;
            manifest.required_content[1]
                .content
                .normalized_game_relative_path = first.into();
            let mut extra = manifest.required_content[1].clone();
            extra.content_key = "alias-row".into();
            extra.content.normalized_game_relative_path = second.into();
            manifest.required_content.push(extra);
            let error = manifest
                .validate()
                .expect_err("{first:?} and {second:?} are one file on NTFS");
            assert!(error.contains("repeats effective path"), "{error}");
        }
    }

    /// GAP 2 -- trailing dots. Win32 strips them (MSDN "Naming Files, Paths,
    /// and Namespaces"); measured, `b.txt.` and `c.txt...` overwrote `b.txt`
    /// and `c.txt`, and the interior form `gfx./u.asset` opened `gfx/u.asset`.
    #[test]
    fn trailing_dot_spellings_are_one_required_path() {
        for alias in [
            "gfx/models/unit.mesh.",
            "gfx/models/unit.mesh...",
            "gfx/models./unit.mesh",
        ] {
            assert_eq!(normalize_game_relative_path(alias).unwrap(), alias);
            let error = manifest_with_extra_required_path(alias)
                .expect_err("{alias:?} names gfx/models/unit.mesh on Windows");
            assert!(error.contains("repeats effective path"), "{error}");
        }
    }

    /// GAP 3 -- per-segment whitespace. A trailing space on the LAST component
    /// cannot survive `normalize_game_relative_path`'s whole-string `trim`, so
    /// the reachable case is an interior one. Measured, `gfx /u.asset` fails
    /// with ERROR_PATH_NOT_FOUND because a directory named `gfx ` cannot be
    /// created at all -- so this spelling names no file either way, and folding
    /// it is the fail-closed reading.
    #[test]
    fn interior_trailing_space_spellings_are_one_required_path() {
        for alias in ["gfx/models /unit.mesh", "gfx /models/unit.mesh"] {
            assert_eq!(normalize_game_relative_path(alias).unwrap(), alias);
            let error = manifest_with_extra_required_path(alias).expect_err("aliasing spelling");
            assert!(error.contains("repeats effective path"), "{error}");
        }
        // Directly on the key, since a canonical path can never carry one.
        assert_eq!(
            windows_collision_key("gfx/models/unit.mesh ").unwrap(),
            windows_collision_key("gfx/models/unit.mesh").unwrap()
        );
    }

    /// GAP 4 -- DOS device names. Measured on Windows 11 build 26200,
    /// `gfx/nul` opened the NUL device (write accepted, read-back empty, no
    /// directory entry) while `gfx/con` was an ordinary file on that build.
    /// Which stems bind to a device is therefore a property of the PLAYBACK
    /// HOST, so a manifest naming one has no host-independent meaning and all
    /// its extensions are folded together.
    #[test]
    fn dos_device_name_spellings_are_one_required_path() {
        for (first, second) in [
            ("gfx/con", "gfx/con.txt"),
            ("gfx/nul", "gfx/nul.dat"),
            ("gfx/lpt1.mesh", "gfx/lpt1.asset"),
            ("gfx/aux.a", "gfx/aux.b"),
        ] {
            let mut manifest = manifest();
            manifest.manifest_digest_sha256 = None;
            manifest.required_content[1]
                .content
                .normalized_game_relative_path = first.into();
            let mut extra = manifest.required_content[1].clone();
            extra.content_key = "alias-row".into();
            extra.content.normalized_game_relative_path = second.into();
            manifest.required_content.push(extra);
            let error = manifest.validate().expect_err("device-name spellings");
            assert!(error.contains("repeats effective path"), "{error}");
        }
        // A stem that merely starts with a device name is NOT reserved.
        assert_ne!(
            windows_collision_key("gfx/conx.txt").unwrap(),
            windows_collision_key("gfx/con.txt").unwrap()
        );
        assert_ne!(
            windows_collision_key("gfx/common/x.txt").unwrap(),
            windows_collision_key("gfx/con/x.txt").unwrap()
        );
    }

    /// GAP 5 -- 8.3 short names. `dir /x` on this worktree lists live aliases
    /// (`CONTEN~1` for `content_manifest`), so the volume generates them. But
    /// which long name a `~N` component denotes is assigned by the filesystem
    /// from directory creation order and is NOT derivable from the manifest, so
    /// the key cannot decide it and refuses instead of guessing.
    #[test]
    fn short_name_shaped_component_is_undecidable_and_refused() {
        for shaped in [
            "gfx/entiti~1/unit.asset",
            "gfx/models/unit~1.mes",
            "gfx/a~12/unit.asset",
        ] {
            assert_eq!(normalize_game_relative_path(shaped).unwrap(), shaped);
            let error = windows_collision_key(shaped).expect_err("{shaped:?} is undecidable");
            assert!(error.contains("8.3 short-name shape"), "{error}");
            let error = manifest_with_extra_required_path(shaped).expect_err("undecidable path");
            assert!(error.contains("8.3 short-name shape"), "{error}");
        }
        // The generator can place extended characters in a short name
        // (RtlGenerate8dot3Name with extended characters enabled), so a
        // non-ASCII base is exactly as undecidable as an ASCII one, and the
        // 8-character base limit counts CHARACTERS, not UTF-8 bytes.
        for shaped in ["gfx/models/entité~1.a", "gfx/models/ααααα~1.a"] {
            let error = windows_collision_key(shaped).expect_err("extended 8.3 shape");
            assert!(error.contains("8.3 short-name shape"), "{error}");
        }
        // Shapes that are NOT 8.3 must keep resolving normally, or the refusal
        // would be an indiscriminate ban on the tilde character.
        for ordinary in [
            "gfx/models/unit~1.mesh",  // 4-character extension
            "gfx/models/toolong~1.a",  // 9-character base
            "gfx/models/~1.a",         // no head before the tilde
            "gfx/models/unit~.a",      // no digits after the tilde
        ] {
            windows_collision_key(ordinary)
                .unwrap_or_else(|error| panic!("{ordinary:?} must stay decidable: {error}"));
        }
    }

    /// GAP 6 -- a component that is nothing but dots and spaces names no file
    /// once Win32 has stripped them, so it cannot be folded into anything and
    /// is refused.
    #[test]
    fn dots_and_spaces_only_component_is_refused() {
        for empty in ["gfx/.../unit.asset", "gfx/ /unit.asset", "gfx/. ./unit.asset"] {
            assert_eq!(normalize_game_relative_path(empty).unwrap(), empty);
            let error = windows_collision_key(empty).expect_err("{empty:?} names nothing");
            assert!(error.contains("only dots/spaces"), "{error}");
        }
    }

    /// The key is allowed to fold MORE than NTFS, never less. These pairs are
    /// measured DISTINCT files on Windows 11 build 26200, and the key keeps
    /// them distinct -- so the fold is not a blunt "case-insensitive compare"
    /// that would refuse legitimate replays wholesale.
    #[test]
    fn windows_collision_key_keeps_measured_non_aliases_distinct() {
        for (first, second, why) in [
            ("gfx/café.a", "gfx/cafe\u{301}.a", "NFC vs NFD"),
            ("gfx/straße.a", "gfx/strasse.a", "sharp s vs ss"),
            ("gfx/ﬁx.a", "gfx/fix.a", "fi ligature"),
            ("gfx/\u{212a}.a", "gfx/k.a", "KELVIN SIGN vs k"),
            ("gfx/\u{212b}.a", "gfx/å.a", "ANGSTROM SIGN vs a-ring"),
            ("gfx/\u{10428}.a", "gfx/\u{10400}.a", "astral, outside $UpCase"),
        ] {
            assert_ne!(
                windows_collision_key(first).unwrap(),
                windows_collision_key(second).unwrap(),
                "{why}: NTFS keeps these apart, so the key must too"
            );
        }
    }

    /// GAP 7 -- replace_path declarations. Both sides of the comparison refuse
    /// aliasing declarations: the manifest side in `validate_layer`, the
    /// caller side in `unique_normalized_path_set`.
    #[test]
    fn aliasing_replace_path_declarations_are_refused_on_both_sides() {
        for alias in ["gfx/entities.", "gfx/entitiés", "gfx /entities"] {
            let clean = manifest();
            let mut broken = clean.clone();
            broken.manifest_digest_sha256 = None;
            let paths = if alias == "gfx/entitiés" {
                // A pair that differs only by non-ASCII case.
                vec!["gfx/entitiés".to_string(), "gfx/entitiÉs".to_string()]
            } else {
                vec!["gfx/entities".to_string(), alias.to_string()]
            };
            broken.ordered_layers[3].replace_paths = paths.clone();
            let error = broken.validate().expect_err("aliasing replace_path");
            assert!(error.contains("duplicate replace_path"), "{error}");

            let mut inventory = inventory(&clean);
            inventory
                .layers
                .iter_mut()
                .find(|layer| layer.layer_id == "mod-override")
                .unwrap()
                .replace_paths = paths;
            let report = clean.validate_playback(&inventory);
            assert!(!report.compatible);
            // The REASON matters: without the collision rule the two
            // declarations merely make the set differ from the manifest's,
            // which is a much weaker refusal that leaves the layer admitted.
            assert!(
                report.issues.iter().any(|issue| {
                    issue.code == ContentIssueCode::ReplacePathMismatch
                        && issue.message.contains("duplicate after normalization")
                }),
                "{:#?}",
                report.issues
            );
        }
    }

    /// A second resolved row that ALIASES the first is a duplicate row, and --
    /// the predecessor's law -- the required path it names must not then be
    /// announced absent as well.
    #[test]
    fn aliasing_resolved_rows_are_duplicates_and_not_absences() {
        let manifest = manifest();
        for alias in [
            "gfx/models/unit.mesh.",
            "gfx/models./unit.mesh",
            "gfx/models /unit.mesh",
        ] {
            let mut inventory = inventory(&manifest);
            let mut decoy = inventory.resolved_content[1].clone();
            decoy.normalized_game_relative_path = alias.into();
            decoy.payload_sha256 = h('e');
            // The decoy goes FIRST, so the GENUINE row -- spelled exactly as
            // recorded -- is the one the duplicate rule refuses. That is the
            // arrangement in which a missed refused-not-absent record would
            // announce a present file missing, and the arrangement in which
            // calling it an "aliasing spelling" would also be a lie.
            inventory.resolved_content.insert(1, decoy);
            let report = manifest.validate_playback(&inventory);
            assert!(!report.compatible, "{alias:?} was admitted alongside the row it aliases");
            assert!(
                report.issues.iter().any(|issue| {
                    issue.code == ContentIssueCode::InvalidPlaybackInventory
                        && issue.message.contains("duplicate resolved path row")
                }),
                "{alias:?}: {:#?}",
                report.issues
            );
            assert!(
                !report
                    .issues
                    .iter()
                    .any(|issue| issue.code == ContentIssueCode::MissingContent),
                "{alias:?}: a present row was announced absent: {:#?}",
                report.issues
            );
            assert!(
                !report
                    .issues
                    .iter()
                    .any(|issue| issue.message.contains("different Windows-equivalent spelling")),
                "{alias:?}: the row IS spelled as recorded; it was refused as a duplicate: {:#?}",
                report.issues
            );
        }
    }

    /// A row that names a required file under an ALIASING spelling is neither
    /// admitted in its place (the key is allowed to over-fold, so an alias match
    /// is not proof of identity) nor reported missing (it is right there).
    #[test]
    fn a_resolved_row_that_only_aliases_a_required_path_is_refused_not_absent() {
        let manifest = manifest();
        for alias in ["gfx/models/unit.mesh.", "gfx/MODELS/unit.mesh."] {
            let mut inventory = inventory(&manifest);
            inventory.resolved_content[1].normalized_game_relative_path =
                normalize_game_relative_path(alias).unwrap();
            let report = manifest.validate_playback(&inventory);
            assert!(!report.compatible, "{alias:?} was admitted");
            assert!(
                report.issues.iter().any(|issue| {
                    issue.code == ContentIssueCode::InvalidPlaybackInventory
                        && issue.subject == "gfx/models/unit.mesh"
                        && issue.message.contains("different Windows-equivalent spelling")
                }),
                "{alias:?}: {:#?}",
                report.issues
            );
            assert!(
                !report
                    .issues
                    .iter()
                    .any(|issue| issue.code == ContentIssueCode::MissingContent),
                "{alias:?}: a present row was announced absent: {:#?}",
                report.issues
            );
        }
    }

    /// MSDN reserves the superscript-digit COM/LPT spellings too, and they
    /// survive the case fold unchanged, so they need their own entries.
    #[test]
    fn superscript_dos_device_spellings_fold_like_their_ascii_forms() {
        for (device, superscript) in [
            ("gfx/com1", "gfx/com\u{b9}"),
            ("gfx/com2", "gfx/com\u{b2}"),
            ("gfx/com3", "gfx/com\u{b3}"),
            ("gfx/lpt1", "gfx/lpt\u{b9}"),
        ] {
            // Each superscript stem folds all its extensions together, exactly
            // as the ASCII stem does.
            assert_eq!(
                windows_collision_key(&format!("{superscript}.txt")).unwrap(),
                windows_collision_key(&format!("{superscript}.bin")).unwrap(),
                "{superscript} extensions must fold together"
            );
            // ... and the superscript form is its own device, not the ASCII one.
            assert_ne!(
                windows_collision_key(device).unwrap(),
                windows_collision_key(superscript).unwrap()
            );
        }
    }

    /// The host case table folds 27 BMP code points that Unicode's simple
    /// uppercase does not, and all 27 were confirmed on the real filesystem to
    /// be one file. Folding less than the filesystem is the one direction the
    /// fail-closed contract forbids, so these are pinned.
    #[test]
    fn host_only_case_foldings_are_not_missed() {
        for (lower, upper) in [
            ('\u{1f80}', '\u{1f88}'),
            ('\u{1f97}', '\u{1f9f}'),
            ('\u{1fa0}', '\u{1fa8}'),
            ('\u{1fb3}', '\u{1fbc}'),
            ('\u{1fc3}', '\u{1fcc}'),
            ('\u{1ff3}', '\u{1ffc}'),
        ] {
            assert_eq!(
                windows_collision_key(&format!("gfx/x{lower}.a")).unwrap(),
                windows_collision_key(&format!("gfx/x{upper}.a")).unwrap(),
                "U+{:04X}/U+{:04X} are one file on NTFS",
                lower as u32,
                upper as u32
            );
        }
        // The 27 host-only mappings are still in the table, and the 225
        // Unicode-only ones are still out of the PROVEN relation.
        assert_eq!(HOST_UPCASE.len(), 973);
        assert_eq!(host_upcase('\u{1f80}'), '\u{1f88}');
        assert_eq!(host_upcase('\u{131}'), '\u{131}');
        assert_eq!(host_upcase('\u{b5}'), '\u{b5}');
    }

    /// A spelling that stacks an old normalization refusal on top of a Windows
    /// alias still names the required file, and must not leave it reported
    /// absent. `gfx/./models/unit.mesh.` is refused for its dot component, but
    /// Win32 resolves both the dot component AND the trailing period.
    #[test]
    fn a_refused_spelling_that_also_aliases_is_not_reported_missing() {
        let manifest = manifest();
        for spelling in [
            "gfx/./models/unit.mesh.",
            "gfx/models/./unit.mesh.",
            r"GFX\.\MODELS\unit.mesh...",
        ] {
            let mut inventory = inventory(&manifest);
            inventory.resolved_content[1].normalized_game_relative_path = spelling.into();
            let report = manifest.validate_playback(&inventory);
            assert!(!report.compatible, "{spelling:?} was admitted");
            assert!(
                report
                    .issues
                    .iter()
                    .any(|issue| issue.code == ContentIssueCode::InvalidPlaybackInventory),
                "{spelling:?} was not refused: {:#?}",
                report.issues
            );
            assert!(
                !report.issues.iter().any(|issue| {
                    issue.code == ContentIssueCode::MissingContent
                        && issue.subject == "gfx/models/unit.mesh"
                }),
                "{spelling:?} names the required file but it was announced absent: {:#?}",
                report.issues
            );
        }
    }

    /// A case near-miss is never PRESENCE. Nothing is admitted, nothing is
    /// verified, and the replay stays refused -- the only thing the undetermined
    /// answer withholds is the claim that the file is absent.
    #[test]
    fn an_over_folded_alias_is_not_proof_that_a_required_file_is_present() {
        for (required_path, over_folded) in [
            ("gfx/models/i.mesh", "gfx/models/\u{131}.mesh"),
            ("gfx/models/\u{b5}.mesh", "gfx/models/\u{3bc}.mesh"),
            ("gfx/models/\u{17f}.mesh", "gfx/models/s.mesh"),
        ] {
            assert_eq!(
                windows_collision_key(required_path).unwrap(),
                windows_collision_key(over_folded).unwrap(),
                "the collision key must fold this pair, or the test proves nothing"
            );
            assert_ne!(
                windows_same_file_identity(required_path).unwrap(),
                windows_same_file_identity(over_folded).unwrap(),
                "the PROVEN identity must not fold it"
            );
            let mut manifest = manifest();
            manifest.manifest_digest_sha256 = None;
            manifest.required_content[1]
                .content
                .normalized_game_relative_path = required_path.into();
            let manifest = manifest.with_computed_digest().unwrap();
            let mut inventory = inventory(&manifest);
            inventory.resolved_content[1].normalized_game_relative_path = over_folded.into();
            let report = manifest.validate_playback(&inventory);
            assert!(!report.compatible, "{over_folded:?} was admitted");
            assert!(manifest.admit_playback(&inventory).is_err());
            assert_eq!(
                report.verified_required_content, 1,
                "{over_folded:?} must not count as verifying {required_path:?}"
            );
            assert!(
                report.issues.iter().any(|issue| {
                    issue.code == ContentIssueCode::InvalidPlaybackInventory
                        && issue.subject == required_path
                        && issue.message.contains("cannot be decided here")
                }),
                "{over_folded:?}: the near-miss row was not reported: {:#?}",
                report.issues
            );
        }
    }

    /// An 8.3 row is refused precisely BECAUSE the long name behind `CONTEN~1`
    /// is the filesystem's to assign -- which is exactly why it may be naming
    /// the required path. Calling that path missing would be a claim the refusal
    /// itself says cannot be made. The `dir /x` probe shows this volume really
    /// does generate such aliases.
    #[test]
    fn an_undecidable_short_name_row_blocks_a_definite_absence() {
        let manifest = manifest();
        for spelling in [
            "gfx/models~1/unit.mesh",
            "gfx/models/unit~1.mes",
            "gfx/mod~123/unit.mesh",
        ] {
            let mut inventory = inventory(&manifest);
            inventory.resolved_content[1].normalized_game_relative_path = spelling.into();
            let report = manifest.validate_playback(&inventory);
            assert!(!report.compatible, "{spelling:?} was admitted");
            assert!(
                report.issues.iter().any(|issue| {
                    issue.code == ContentIssueCode::InvalidPlaybackInventory
                        && issue.message.contains("8.3 short-name shape")
                }),
                "{spelling:?} was not refused as undecidable: {:#?}",
                report.issues
            );
            assert!(
                !report.issues.iter().any(|issue| {
                    issue.code == ContentIssueCode::MissingContent
                        && issue.subject == "gfx/models/unit.mesh"
                }),
                "{spelling:?} may be naming the required path, but it was called absent: {:#?}",
                report.issues
            );
            assert!(manifest.admit_playback(&inventory).is_err());
        }
    }

    /// A row can be BOTH non-canonical and undecidable. Those arrive through the
    /// refused-spelling recovery path, not the canonical one, so every caller of
    /// note_row_identity has to feed the undecidable list or the recovery is only
    /// half wired and the path is announced absent again.
    #[test]
    fn a_noncanonical_undecidable_row_also_blocks_a_definite_absence() {
        let manifest = manifest();
        for spelling in [
            r"GFX\MODELS~1\UNIT.MESH",
            "gfx/./models~1/unit.mesh",
            "gfx/models~1/unit.mesh ",
            "gfx/sub/../models~1/unit.mesh",
        ] {
            let mut inventory = inventory(&manifest);
            inventory.resolved_content[1].normalized_game_relative_path = spelling.into();
            let report = manifest.validate_playback(&inventory);
            assert!(!report.compatible, "{spelling:?} was admitted");
            assert!(
                report
                    .issues
                    .iter()
                    .any(|issue| issue.code == ContentIssueCode::InvalidPlaybackInventory),
                "{spelling:?} was not refused: {:#?}",
                report.issues
            );
            assert!(
                !report.issues.iter().any(|issue| {
                    issue.code == ContentIssueCode::MissingContent
                        && issue.subject == "gfx/models/unit.mesh"
                }),
                "{spelling:?} may be naming the required path, but it was called absent: {:#?}",
                report.issues
            );
            assert!(manifest.admit_playback(&inventory).is_err());
        }
    }

    /// The suppression above must stay narrow, or one undecidable row would
    /// excuse every absence in the manifest. A short-name row can only be a
    /// spelling of a path with the SAME component count whose other components
    /// fold equal.
    #[test]
    fn an_undecidable_row_only_excuses_the_paths_it_could_actually_name() {
        for (row, subject, could) in [
            ("gfx/models~1/unit.mesh", "gfx/models/unit.mesh", true),
            ("gfx/models/unit~1.mes", "gfx/models/unit.mesh", true),
            ("gfx/models~1/unit.mesh", "gfx/models/other.mesh", false),
            ("gfx/models~1/unit.mesh", "map/models/unit.mesh", false),
            // A short name replaces exactly ONE component, never adds or drops.
            ("gfx/models~1/unit.mesh", "gfx/models/sub/unit.mesh", false),
            ("gfx/models~1/unit.mesh", "gfx/unit.mesh", false),
            // A dots-only component names nothing, so it stands for nothing.
            ("gfx/.../unit.mesh", "gfx/models/unit.mesh", false),
            // An interior trailing space names nothing either, and the collision
            // key would have folded it away (measured: `gfx /u.asset` fails with
            // ERROR_PATH_NOT_FOUND), so the matcher must be position-aware.
            ("gfx /models~1/unit.mesh", "gfx/models/unit.mesh", false),
            ("gfx/models /unit~1.mes", "gfx/models/unit.mesh", false),
            // `~N` is not a wildcard, but the only thing the generator provably
            // preserves is the EXTENSION. Measured with dir /x:
            // Cargo.toml -> CARGO~1.TOM, definitely_absent.lock -> DEFINI~1.LOC.
            ("gfx/models/cargo~1.tom", "gfx/models/cargo.toml", true),
            ("gfx/models/cargo~1.tom", "gfx/models/cargo.lock", false),
            ("gfx/models/cargo~1.tom", "gfx/models/definitely_absent.lock", false),
            ("gfx/models/unitta~1.gz", "gfx/models/unit.tar.gz", true),
            ("gfx/models/abc~1.ddd", "gfx/models/a.b.c.dddd", true),
            ("gfx/models/unitmo~1.mes", "gfx/models/unit model.mesh", true),
            ("gfx/noext_~1/unit.mesh", "gfx/noext_longname/unit.mesh", true),
            // Exactly min(3, len), not a prefix: cargo~1.t is not a name the
            // generator could have produced for cargo.toml.
            ("gfx/models/cargo~1.t", "gfx/models/cargo.toml", false),
            ("gfx/models/cargo~1.to", "gfx/models/cargo.toml", false),
            // Presence must agree on both sides.
            ("gfx/models/cargo~1", "gfx/models/cargo.toml", false),
            ("gfx/models/cargo~1.tom", "gfx/models/cargo", false),
            // The sequence starts at 1; ~0 is never generated.
            ("gfx/models/cargo~0.tom", "gfx/models/cargo.toml", false),
            // The BASE is NOT constrained, because the generator translates
            // punctuation and falls back to a checksum. All of these are
            // measured real pairs, and a prefix rule would have rejected every
            // one of them -- announcing a present file absent.
            ("gfx/models/a_b_lo~1.txt", "gfx/models/a+b_longname.txt", true),
            ("gfx/models/ablong~1.txt", "gfx/models/a b longname.txt", true),
            ("gfx/models/a_a9ac~1.txt", "gfx/models/a[b]longname.txt", true),
            ("gfx/models/loc254~1.txt", "gfx/models/longfilename5.txt", true),
            ("gfx/models~1/unit.mesh", "gfx/interface/unit.mesh", true),
            // A non-ASCII long name can be mangled beyond recognition (measured:
            // four CJK characters became 167F~1), and a translated EXTENSION is
            // the same problem (longfile.a+b -> LONGFI~1.A_B), so the extension
            // rule applies only to an alphanumeric long extension.
            ("gfx/models/167f~1.mes", "gfx/models/\u{6a21}\u{578b}.mesh", true),
            ("gfx/models/bcdefg~1.txt", "gfx/models/ąbcdefghij.txt", true),
            ("gfx/models/longfi~1.a_b", "gfx/models/longfile.a+b", true),
            // Win32 strips trailing dots, so the question is what the spelling
            // NAMES: `cargo~1.tom.` names the real alias CARGO~1.TOM. Splitting
            // it unstripped would find an empty final suffix, call it
            // extensionless and rule out a file that is present.
            ("gfx/models/cargo~1.tom.", "gfx/models/cargo.toml", true),
            ("gfx/models/cargo~1.tom", "gfx/models/cargo.toml.", true),
            ("gfx/models/cargo~1.tom..", "gfx/models/cargo.toml", true),
            // "Unrelated to the long base" is not "any string at all": these
            // cannot be 8.3 aliases of anything, so they excuse nothing. MSDN
            // reserves < > : " / \ | ? * in a filename, and the generator writes
            // its sequence in plain decimal from 1.
            ("gfx/models/*~1.tom", "gfx/models/cargo.toml", false),
            ("gfx/models/a?b~1.tom", "gfx/models/cargo.toml", false),
            ("gfx/models/a|b~1.tom", "gfx/models/cargo.toml", false),
            ("gfx/models/cargo~01.tom", "gfx/models/cargo.toml", false),
            ("gfx/models/cargo~10.tom", "gfx/models/cargo.toml", true),
        ] {
            assert_eq!(
                undecidable_row_could_name(row, subject),
                could,
                "{row:?} vs {subject:?}"
            );
        }
        // ... and a manifest's OTHER required paths still report absent.
        let manifest = manifest();
        let mut inventory = inventory(&manifest);
        inventory.resolved_content[1].normalized_game_relative_path =
            "gfx/models~1/unit.mesh".into();
        inventory
            .resolved_content
            .retain(|row| row.normalized_game_relative_path != "gfx/entities/unit.asset");
        let report = manifest.validate_playback(&inventory);
        assert!(
            report.issues.iter().any(|issue| {
                issue.code == ContentIssueCode::MissingContent
                    && issue.subject == "gfx/entities/unit.asset"
            }),
            "an unrelated absence was excused by the undecidable row: {:#?}",
            report.issues
        );
    }

    /// A row refused for its SPELLING can still be a case near-miss of a
    /// required path. It never reaches the admitted set, so the absence question
    /// has to look at every row the caller sent, not only the admitted ones --
    /// otherwise it answers "missing" about a file that may be right there.
    #[test]
    fn a_refused_row_that_is_a_case_near_miss_still_blocks_a_definite_absence() {
        let mut manifest = manifest();
        manifest.manifest_digest_sha256 = None;
        manifest.required_content[1]
            .content
            .normalized_game_relative_path = "gfx/models/ą.mesh".into();
        let manifest = manifest.with_computed_digest().unwrap();
        for spelling in [
            r"GFX\Models\Ą.Mesh",     // refused: not canonical
            "gfx/./models/Ą.mesh",    // refused: dot component
            "gfx/models/Ą.mesh.",     // accepted, trailing dot, still a near miss
        ] {
            let mut inventory = inventory(&manifest);
            inventory.resolved_content[1].normalized_game_relative_path = spelling.into();
            let report = manifest.validate_playback(&inventory);
            assert!(!report.compatible, "{spelling:?} was admitted");
            assert!(
                !report.issues.iter().any(|issue| {
                    issue.code == ContentIssueCode::MissingContent
                        && issue.subject == "gfx/models/ą.mesh"
                }),
                "{spelling:?} may name the required file, but it was called absent: {:#?}",
                report.issues
            );
        }
    }

    /// The proven identity folds only what was MEASURED to be exact, and says
    /// `None` -- "cannot prove it" -- for everything else.
    #[test]
    fn the_proven_identity_folds_only_the_measured_exact_rules() {
        // Trailing dots and spaces on the last component, trailing dots on an
        // interior one: measured aliases.
        for equivalent in [
            "gfx/models/unit.mesh.",
            "gfx/models/unit.mesh...",
            "gfx/models./unit.mesh",
        ] {
            assert_eq!(
                windows_same_file_identity(equivalent).unwrap(),
                windows_same_file_identity("gfx/models/unit.mesh").unwrap(),
                "{equivalent:?}"
            );
        }
        // An interior trailing SPACE was measured to resolve to nothing, not to
        // the space-less directory, so it proves nothing.
        assert_eq!(windows_same_file_identity("gfx/models /unit.mesh"), None);
        // Host-dependent or undecidable referents prove nothing.
        assert_eq!(windows_same_file_identity("gfx/con.txt"), None);
        assert_eq!(windows_same_file_identity("gfx/entiti~1/unit.asset"), None);
        assert_eq!(windows_same_file_identity("gfx/.../unit.asset"), None);
        // Case beyond ASCII is decided by the VOLUME's own $UpCase table, which
        // this code has not read and which has changed across Windows versions
        // in BOTH directions. So no non-ASCII case pair is proven sameness --
        // not even one measured to be a single file on the probed volume.
        for (lower, upper) in [
            ("gfx/models/ą.mesh", "gfx/models/Ą.mesh"),
            ("gfx/models/café.mesh", "gfx/models/cafÉ.mesh"),
            ("gfx/models/г.mesh", "gfx/models/Г.mesh"),
            ("gfx/models/\u{1f80}.mesh", "gfx/models/\u{1f88}.mesh"),
            // Final sigma folds to capital sigma under the XP and Vista tables
            // and is reset to itself in later ones, so THIS host measuring them
            // as two files proves nothing about the playback volume.
            ("gfx/models/\u{3c2}.mesh", "gfx/models/\u{3a3}.mesh"),
            // Unicode folds these and this host does not; without every
            // supported table in hand that is not proof of difference either.
            ("gfx/models/i.mesh", "gfx/models/\u{131}.mesh"),
            ("gfx/models/\u{b5}.mesh", "gfx/models/\u{3bc}.mesh"),
            ("gfx/models/\u{17f}.mesh", "gfx/models/s.mesh"),
        ] {
            assert_ne!(
                windows_same_file_identity(lower).unwrap(),
                windows_same_file_identity(upper).unwrap(),
                "{lower} / {upper} need a case table this code has not read"
            );
            // ... and every one of them IS caught by the collision key, which is
            // what makes the absence answer "undetermined" rather than "missing".
            assert_eq!(
                windows_collision_key(lower).unwrap(),
                windows_collision_key(upper).unwrap(),
                "{lower} / {upper} must at least collide"
            );
        }
        // Two paths that no case table could fold ARE proven different, and stay
        // a plain absence.
        assert_ne!(
            windows_collision_key("gfx/models/unit.mesh").unwrap(),
            windows_collision_key("gfx/models/other.mesh").unwrap()
        );
    }

    /// The third answer. A spelling that differs from the recorded path only by a
    /// case mapping outside ASCII is the same file on some NTFS volumes and two
    /// files on others -- the `$UpCase` table is written when the volume is
    /// FORMATTED and has changed across Windows versions, in both directions:
    /// final sigma folds to capital sigma under XP and Vista and is reset in
    /// later tables. So neither MissingContent ("this file is absent") nor a
    /// presence claim is true, and the report must say undetermined rather than
    /// pick one. Announcing it absent would name a file that may be right there;
    /// announcing it present would suppress a real absence on another volume.
    #[test]
    fn measured_case_aliases_of_a_required_path_are_not_reported_absent() {
        for (recorded, alias) in [
            ("gfx/models/ą.mesh", "gfx/models/Ą.mesh"),
            ("gfx/models/café.mesh", "gfx/models/cafÉ.mesh"),
            ("gfx/models/г.mesh", "gfx/models/Г.mesh"),
            ("gfx/models/σ.mesh", "gfx/models/Σ.mesh"),
            ("gfx/models/ａ.mesh", "gfx/models/Ａ.mesh"),
            ("gfx/models/\u{1f80}.mesh", "gfx/models/\u{1f88}.mesh"),
            // Folded by an OLDER table and not by this host's.
            ("gfx/models/\u{3a3}.mesh", "gfx/models/\u{3c2}.mesh"),
            // Folded by Unicode and not by this host's.
            ("gfx/models/i.mesh", "gfx/models/\u{131}.mesh"),
            ("gfx/models/\u{17f}.mesh", "gfx/models/s.mesh"),
        ] {
            let mut manifest = manifest();
            manifest.manifest_digest_sha256 = None;
            manifest.required_content[1]
                .content
                .normalized_game_relative_path = recorded.into();
            let manifest = manifest.with_computed_digest().unwrap();
            let mut inventory = inventory(&manifest);
            inventory.resolved_content[1].normalized_game_relative_path = alias.into();
            let report = manifest.validate_playback(&inventory);
            assert!(!report.compatible, "{alias:?} was admitted in place of {recorded:?}");
            assert!(
                !report.issues.iter().any(|issue| {
                    issue.code == ContentIssueCode::MissingContent && issue.subject == recorded
                }),
                "{alias:?} IS {recorded:?} on this filesystem but it was announced absent: {:#?}",
                report.issues
            );
            assert!(
                report.issues.iter().any(|issue| {
                    issue.code == ContentIssueCode::InvalidPlaybackInventory
                        && issue.subject == recorded
                        && issue.message.contains("cannot be decided here")
                }),
                "{alias:?}: the undetermined answer was not reported: {:#?}",
                report.issues
            );
        }
    }

    /// And the converse of the same rule: the report must not claim presence
    /// either. `admit_playback` refuses, and `verified_required_content` does
    /// not count a row whose spelling only maybe-matches.
    #[test]
    fn a_volume_dependent_case_match_is_never_admitted() {
        let mut manifest = manifest();
        manifest.manifest_digest_sha256 = None;
        manifest.required_content[1]
            .content
            .normalized_game_relative_path = "gfx/models/ą.mesh".into();
        let manifest = manifest.with_computed_digest().unwrap();
        let mut inventory = inventory(&manifest);
        inventory.resolved_content[1].normalized_game_relative_path = "gfx/models/Ą.mesh".into();
        let report = manifest.validate_playback(&inventory);
        assert!(!report.compatible, "{:#?}", report.issues);
        assert_eq!(
            report.verified_required_content, 1,
            "only the row spelled as recorded may count as verified"
        );
        assert!(manifest.admit_playback(&inventory).is_err());
    }

    /// `normalize_game_relative_path` trims the whole string, and it is frozen,
    /// so it cannot stop. But a LEADING space is not something Win32 strips
    /// (measured: ` e.txt` and `e.txt` are two files), so a row spelled with one
    /// names a different file and must not vouch for the required path.
    #[test]
    fn a_leading_space_row_does_not_prove_the_required_path_is_present() {
        let manifest = manifest();
        for spelling in [
            " gfx/models/unit.mesh",
            "\tgfx/models/unit.mesh",
            "gfx/models/unit.mesh\t",
        ] {
            assert_eq!(
                normalize_game_relative_path(spelling).unwrap(),
                "gfx/models/unit.mesh",
                "the frozen canonicalizer really does collapse this spelling"
            );
            let mut inventory = inventory(&manifest);
            inventory.resolved_content[1].normalized_game_relative_path = spelling.into();
            let report = manifest.validate_playback(&inventory);
            assert!(!report.compatible);
            assert!(
                report.issues.iter().any(|issue| {
                    issue.code == ContentIssueCode::MissingContent
                        && issue.subject == "gfx/models/unit.mesh"
                }),
                "{spelling:?} names a different file, so the required one is absent: {:#?}",
                report.issues
            );
        }
        // A trailing SPACE on the last component IS stripped by Win32, so that
        // one does name the required file and must not be reported absent.
        let mut inventory = inventory(&manifest);
        inventory.resolved_content[1].normalized_game_relative_path =
            "gfx/models/unit.mesh ".into();
        let report = manifest.validate_playback(&inventory);
        assert!(
            !report
                .issues
                .iter()
                .any(|issue| issue.code == ContentIssueCode::MissingContent),
            "{:#?}",
            report.issues
        );
    }

    /// The safety net under the whole absence rule: suppressing a
    /// MissingContent can only ever change WHICH refusal is reported, never
    /// produce a compatible report. So even a proven-sameness relation that is
    /// slightly wrong on some other host cannot admit a replay.
    #[test]
    fn suppressing_an_absence_never_makes_a_report_compatible() {
        let manifest = manifest();
        for spelling in [
            "gfx/models/unit.mesh.",          // trailing dot: same file
            "gfx/models/Ą.mesh",              // volume-dependent case fold
            "GFX\\Models\\Unit.Mesh",         // refused spelling of the same file
            "gfx/./models/unit.mesh.",        // refused AND aliasing
            "gfx/models/unit.mesh ",          // trailing space: same file
        ] {
            let mut inventory = inventory(&manifest);
            inventory.resolved_content[1].normalized_game_relative_path = spelling.into();
            let report = manifest.validate_playback(&inventory);
            assert!(
                !report.compatible,
                "{spelling:?} produced a COMPATIBLE report: {:#?}",
                report.issues
            );
            assert!(
                !report.issues.is_empty(),
                "{spelling:?} suppressed everything and reported nothing"
            );
        }
    }

    /// The whole point of the collision key: `compute_selection_sha256` and
    /// every stored path keep the UNCHANGED canonical form. These digests are
    /// recorded identities -- a change here breaks every replay already on
    /// disk, so they are pinned literally. The SELECTION digest was READ OUT OF
    /// THE BASE COMMIT (6a8e378f), not copied from the new output; the
    /// 13190-row corpus in
    /// extractor/native/runs/win_alias_key/digest_invariance.txt makes the
    /// same point across 8847 accepted inputs.
    ///
    /// The MANIFEST digest is a different claim and it DID change here, once,
    /// on purpose: `recorder_local_settings` is a new manifest field, so
    /// every manifest serializes one more key and every manifest digest moves.
    /// (It moved from the base commit's
    /// `0665684938524d16a2e9176da325c1d3445624daf5b83d595872e32457bc5766`. The
    /// literal was re-derived once more here: the value this branch first
    /// pinned had been computed before the field settled, so the pin was stale
    /// and this test failed on the branch that introduced it. The SELECTION
    /// digest above is untouched by both, which is the assertion that matters.)
    /// Recordings made before that field stop opening, which the owner ruled
    /// acceptable pre-production. What must NOT move is the selection digest
    /// above -- it is the portable compatibility gate, it is computed from
    /// layers/DLC/language/`required_settings` only, and moving the provenance
    /// rows OUT of `required_settings` was chosen precisely so that a recording
    /// and the install that plays it compute the same one.
    ///
    /// kills: putting `recorder_local_settings` into
    /// `compute_selection_sha256`'s canonical document, or leaving the
    /// provenance rows in `required_settings`, moves the selection digest and
    /// fails the first assertion.
    #[test]
    fn recorded_selection_digests_are_unchanged() {
        let manifest = manifest();
        assert_eq!(
            manifest.computed_selection_sha256().unwrap(),
            "6ec6d98521e0f44eee2d355b651167828f40c6c90d89a6023f97fdf594694502"
        );
        assert_eq!(
            manifest.manifest_digest_sha256.as_deref().unwrap(),
            "4a51d242fb80c722f31e50c5640a2424ec20dc220d19c58fa9a4aa4c857d4105"
        );
        // The canonical form itself is untouched by everything above.
        assert_eq!(
            normalize_game_relative_path(r"GFX\FX\Particle.SHADER").unwrap(),
            "gfx/fx/particle.shader"
        );
        assert_eq!(
            normalize_game_relative_path("gfx/Ą.asset").unwrap(),
            "gfx/Ą.asset"
        );
        assert_eq!(
            normalize_game_relative_path("gfx/unit.asset.").unwrap(),
            "gfx/unit.asset."
        );
    }

    /// A replayer cannot obtain "how well the recorder proved the settings"
    /// from its own install, so demanding it is a refusal nobody can satisfy.
    /// The two maps are a PARTITION and the manifest enforces it in both
    /// directions, so no consumer has to filter and a hand-edited manifest
    /// cannot move a row across the line to change what is demanded.
    ///
    /// kills: dropping either half of the partition check in `validate` makes
    /// one of these three assertions fail. This test builds its manifest by
    /// hand, so it does NOT cover the producer; the assertions that catch
    /// `PortableLaunchBinding::from_modset` putting a provenance or display row
    /// back into `required_settings` live with that constructor, in
    /// `recorder_host::portable_content_manifest`'s
    /// `local_modset_paths_are_reduced_to_portable_launch_identity`
    /// (Codex review round 1).
    #[test]
    fn settings_provenance_and_material_settings_are_a_partition() {
        let mut manifest = manifest();
        manifest.recorder_local_settings = BTreeMap::from([
            ("settings_authority".into(), "pre_launch_bracketed".into()),
            ("settings_file_sha256".into(), h('c')),
        ]);
        manifest.source_profile.selection_sha256 = manifest.computed_selection_sha256().unwrap();
        manifest = manifest.with_computed_digest().unwrap();
        manifest.validate().expect("a partitioned manifest is valid");

        // Provenance in the map that is demanded of the local install.
        let mut wrong = manifest.clone();
        wrong
            .required_settings
            .insert("settings_authority".into(), "pre_launch_bracketed".into());
        // Re-derive the selection identity and DROP the sealed digest, so the
        // only thing left wrong with this manifest is the partition. Left
        // stale, the digest comparison refuses first and the assertion below
        // would pass for the wrong reason; recomputed, it cannot be, because
        // `computed_digest` validates the partition itself.
        wrong.source_profile.selection_sha256 = wrong.computed_selection_sha256().unwrap();
        wrong.manifest_digest_sha256 = None;
        let error = wrong.validate().expect_err("provenance is not a demand");
        assert!(
            error.contains("recorder_local_settings"),
            "unexpected refusal: {error}"
        );

        // A material setting hidden in the map nothing compares or hashes.
        // This case used to insert `display_mode` and leave the digest stale,
        // so the partition passed and what it read as its refusal was the
        // digest check firing -- it passed for the wrong reason, and then
        // stopped passing at all.
        let mut wrong = manifest.clone();
        wrong
            .recorder_local_settings
            // A key that is neither presentation nor proof provenance, so the
            // partition says it belongs in `required_settings`. `display_mode`
            // would NOT work here: it is a recorder-local presentation row and
            // living in this map is exactly where it belongs.
            .insert("graphics.texture_quality".into(), "low".into());
        wrong.manifest_digest_sha256 = None;
        let error = wrong
            .validate()
            .expect_err("a material setting must not hide in provenance");
        assert!(
            error.contains("required_settings"),
            "unexpected refusal: {error}"
        );
    }

    /// Provenance rows are recorded, and they change nothing about what the
    /// local install must reproduce: the portable selection digest is identical
    /// with and without them, so two installs that agree on the material
    /// settings agree on the gate however well either recording proved them.
    ///
    /// kills: hashing `recorder_local_settings` into
    /// `compute_selection_sha256`'s canonical document.
    #[test]
    fn settings_provenance_never_moves_the_portable_selection_digest() {
        let bare = manifest();
        let mut with_provenance = bare.clone();
        with_provenance.recorder_local_settings = BTreeMap::from([
            ("settings_authority".into(), "pre_launch_bracketed".into()),
            ("settings_file_sha256".into(), h('c')),
        ]);
        assert_eq!(
            bare.computed_selection_sha256().unwrap(),
            with_provenance.computed_selection_sha256().unwrap()
        );
    }

    /// ONE RULE, both sides, and it must not need `rawVersion` to work.
    ///
    /// The shipped `launcher-settings.json` spells the version and the build as
    /// one dotted run -- `"Operation Postern v1.19.2.0.a729 (d245)"` -- and
    /// carries `"rawVersion": "1.19.2.0"` beside it. The parser leaned on that
    /// second field: with `raw = None` the only candidate token is `"1.19.2.0."`
    /// (the dot before `a729` stays attached), which a strict four-piece test
    /// refused, so a launcher file without `rawVersion` refused a recording
    /// whose version was right there in the display string.
    ///
    /// kills: requiring exactly four dot-pieces (the `raw = None` case
    /// refuses); allowing every piece past the fourth to be empty (the
    /// dot-run case `1.19.2.0..a729` is accepted); allowing a non-empty fifth
    /// piece (the five-part numeral is truncated into a version); and dropping
    /// the build requirement altogether (a display string with no build token
    /// would produce an identity).
    #[test]
    fn the_launcher_version_parses_without_raw_version() {
        let shipped = "Operation Postern v1.19.2.0.a729 (d245)";
        assert_eq!(
            parse_launcher_version_and_build(shipped, Some("1.19.2.0")).unwrap(),
            ("1.19.2.0".to_string(), "a729".to_string())
        );
        assert_eq!(
            parse_launcher_version_and_build(shipped, None).unwrap(),
            ("1.19.2.0".to_string(), "a729".to_string())
        );
        assert_eq!(
            parse_launcher_version_and_build("1.19.2.0.a729", None).unwrap(),
            ("1.19.2.0".to_string(), "a729".to_string())
        );
        // A five-part numeral is not a the game version and must not be truncated
        // into one.
        assert!(parse_launcher_version_and_build("1.19.2.0.5 a729", None).is_err());
        // ONE separating dot is the shipped spelling; a RUN of them is
        // malformed metadata and must stay refused rather than be normalized
        // into a version (Codex review round 2).
        assert!(parse_launcher_version_and_build("v1.19.2.0..a729", None).is_err());
        assert!(parse_launcher_version_and_build("v1.19.2.0...a729", None).is_err());
        // A version with no build token anywhere is refused, whichever field
        // the version came from.
        assert!(parse_launcher_version_and_build("no build here", Some("1.19.2.0")).is_err());
        // No four-part numeral anywhere is still a refusal.
        assert!(parse_launcher_version_and_build("v1.19.2 a729", None).is_err());
    }

    /// The replay's manifest and the caller's inventory are opposite subjects
    /// and must never share one code: `InvalidManifest` means the recording
    /// cannot be played by anyone, `InvalidPlaybackInventory` means this local
    /// enumeration is broken while the recording is fine.
    #[test]
    fn manifest_and_caller_inventory_malformation_carry_different_codes() {
        let manifest = manifest();
        let clean_inventory = inventory(&manifest);
        let mut inventory = clean_inventory.clone();
        inventory.settings.insert(" ".into(), "value".into());
        let report = manifest.validate_playback(&inventory);
        assert!(report.issues.iter().any(|issue| {
            issue.code == ContentIssueCode::InvalidPlaybackInventory
                && issue.subject == "playback inventory"
        }));
        assert!(
            !report
                .issues
                .iter()
                .any(|issue| issue.code == ContentIssueCode::InvalidManifest),
            "a caller defect blamed the replay's manifest: {:#?}",
            report.issues
        );

        let mut broken = manifest.clone();
        broken.manifest_digest_sha256 = Some(h('0'));
        let report = broken.validate_playback(&clean_inventory);
        assert!(report.issues.iter().any(|issue| {
            issue.code == ContentIssueCode::InvalidManifest && issue.subject == "manifest"
        }));
        assert!(!report
            .issues
            .iter()
            .any(|issue| issue.code == ContentIssueCode::InvalidPlaybackInventory));
    }

    /// A selection digest that cannot be COMPUTED from the caller's inputs is
    /// not evidence of a wrong playset. It used to be reported as one, which
    /// sent the operator to reconfigure a profile that may be correct.
    #[test]
    fn an_uncomputable_selection_digest_is_not_a_profile_mismatch() {
        let manifest = manifest();
        let mut inventory = inventory(&manifest);
        inventory
            .layers
            .iter_mut()
            .find(|layer| layer.layer_id == "mod-override")
            .unwrap()
            .descriptor_sha256 = Some("not-a-sha256".into());
        let report = manifest.validate_playback(&inventory);
        assert!(report.issues.iter().any(|issue| {
            issue.code == ContentIssueCode::InvalidPlaybackInventory
                && issue.subject == "source_profile.selection_sha256"
        }));
        assert!(
            !report
                .issues
                .iter()
                .any(|issue| issue.code == ContentIssueCode::ProfileMismatch),
            "an uncomputable digest was blamed on the playset: {:#?}",
            report.issues
        );
    }

    #[test]
    fn playback_admission_retains_product_modset_provenance() {
        let manifest = manifest();
        let admission = manifest.admit_playback(&inventory(&manifest)).unwrap();
        assert_eq!(
            admission.product_modset_sha256(),
            manifest.product_modset_sha256
        );
    }
}

/// User/profile labels cross the replay distribution boundary but do not
/// participate in physical path resolution. Preserve ordinary display text
/// while refusing absolute, traversal, and embedded host-path grammars.
fn validate_portable_label(value: &str, label: &str) -> Result<(), String> {
    let trimmed = value.trim();
    let slash = value.replace('\\', "/");
    let bytes = slash.as_bytes();
    let embedded_drive = bytes
        .windows(3)
        .any(|window| window[0].is_ascii_alphabetic() && window[1] == b':' && window[2] == b'/');
    let embedded_unix = bytes.iter().enumerate().any(|(index, byte)| {
        *byte == b'/'
            && index > 0
            && bytes.get(index + 1).is_some_and(|next| *next != b'/')
            && matches!(
                bytes[index - 1],
                b' ' | b'\t' | b'=' | b'(' | b'[' | b'{' | b'\'' | b'"'
            )
    });
    if trimmed.is_empty()
        || trimmed != value
        || value.chars().any(char::is_control)
        || slash.starts_with('/')
        || value.starts_with(r"\\")
        || value.contains(r"\\")
        || embedded_drive
        || embedded_unix
        || slash.split('/').any(|part| part == "." || part == "..")
    {
        return Err(format!("{label} is unsafe/non-portable: {value:?}"));
    }
    Ok(())
}
