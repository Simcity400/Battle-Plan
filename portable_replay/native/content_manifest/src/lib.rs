//! Portable, path-independent used-content requirements for a replay.
//! Absolute install/profile paths never cross this boundary.

pub mod content_parse;
pub mod effective_vfs;
pub mod gfx;
pub mod gui;
pub mod hud_emitter;
pub mod hud_role;
pub mod paradox_text;

use crate::hud_role::validate_sha256;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::collections::{BTreeMap, BTreeSet};
use unicode_normalization::UnicodeNormalization;

pub const SCHEMA: &str = "hoi4.portable-content-manifest.v1";

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ContentManifest {
    pub schema: String,
    pub source_game: SourceGameIdentity,
    pub source_profile: SourceProfileIdentity,
    pub effective_vfs_manifest_sha256: String,
    /// Lowest priority to highest. Unused selections remain recorded but are
    /// not required locally.
    pub ordered_layers: Vec<RecordedContentLayer>,
    pub dlc_selection: Vec<DlcSelection>,
    pub localisation_language: String,
    /// Only settings that affect admitted replay content or rendering.
    pub required_settings: BTreeMap<String, String>,
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
    InvalidManifest,
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
    required_by_path: BTreeMap<String, UsedContentKey>,
    required_by_consumer: BTreeMap<String, Vec<UsedContentKey>>,
}

impl ValidatedContentAdmission {
    pub fn manifest_digest_sha256(&self) -> &str {
        &self.manifest_digest_sha256
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
        for (key, value) in &self.required_settings {
            if key.trim().is_empty() || value.trim().is_empty() {
                return Err("required setting has a blank key/value".into());
            }
        }
        let layers: BTreeMap<_, _> = self
            .ordered_layers
            .iter()
            .map(|layer| (layer.ordinal, layer))
            .collect();
        let mut content_keys = BTreeSet::new();
        let mut paths = BTreeSet::new();
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
            if !paths.insert(required.content.normalized_game_relative_path.as_str()) {
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
        // behind a matching one. The first row is kept so the remaining
        // comparisons still run deterministically.
        let mut local_layers = BTreeMap::new();
        for layer in &playback.layers {
            let key = (layer.recorded_ordinal, layer.layer_id.as_str());
            if local_layers.contains_key(&key) {
                issues.push(issue(
                    ContentIssueCode::InvalidManifest,
                    "playback inventory",
                    format!(
                        "duplicate layer row: ordinal {} id {}",
                        layer.recorded_ordinal, layer.layer_id
                    ),
                ));
                continue;
            }
            local_layers.insert(key, layer);
        }
        for expected in self
            .ordered_layers
            .iter()
            .filter(|layer| layer.required_for_playback)
        {
            let subject = format!("layer:{}", expected.layer_id);
            let Some(actual) = local_layers.get(&(expected.ordinal, expected.layer_id.as_str()))
            else {
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
            // Both sides normalize with error propagation: an un-normalizable
            // playback replace_path is a typed refusal, never silently dropped
            // from the comparison. (The manifest side is guaranteed valid by
            // self.validate() above, but is propagated rather than unwrapped.)
            match (
                normalized_path_set(&actual.replace_paths),
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
            Err(message) => issues.push(issue(
                ContentIssueCode::ProfileMismatch,
                "source_profile.selection_sha256",
                message,
            )),
        }

        let mut local_dlcs = BTreeMap::new();
        for dlc in &playback.dlc_selection {
            if local_dlcs.contains_key(dlc.dlc_id.as_str()) {
                issues.push(issue(
                    ContentIssueCode::InvalidManifest,
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
        for content in &playback.resolved_content {
            let key = content.normalized_game_relative_path.as_str();
            if resolved.contains_key(key) {
                issues.push(issue(
                    ContentIssueCode::InvalidManifest,
                    "playback inventory",
                    format!("duplicate resolved path row: {key}"),
                ));
                continue;
            }
            resolved.insert(key, content);
        }
        let mut verified = 0usize;
        for required in &self.required_content {
            let expected = &required.content;
            let subject = expected.normalized_game_relative_path.clone();
            let Some(actual) = resolved.get(subject.as_str()) else {
                issues.push(issue(
                    ContentIssueCode::MissingContent,
                    subject,
                    format!("required by {}", required.consumers.join(", ")),
                ));
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
        layer.replace_paths = layer
            .replace_paths
            .iter()
            .map(|path| normalize_game_relative_path(path))
            .collect::<Result<BTreeSet<_>, _>>()?
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
    let normalized = path
        .trim()
        .replace('\\', "/")
        .nfc()
        .collect::<String>()
        .to_lowercase();
    if normalized.is_empty()
        || normalized.starts_with('/')
        || normalized.contains(':')
        || normalized
            .split('/')
            .any(|part| part.is_empty() || part == "." || part == "..")
    {
        return Err(format!("unsafe/non-canonical game-relative path {path:?}"));
    }
    Ok(normalized)
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
    validate_sha256(&profile.selection_sha256, "source profile selection")
}

fn validate_layer(layer: &RecordedContentLayer) -> Result<(), String> {
    validate_portable_layer_id(&layer.layer_id)?;
    if layer.display_name.trim().is_empty() {
        return Err("content layer id/display name is blank".into());
    }
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
    if normalized.len() != layer.replace_paths.len() {
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
            issue.code == ContentIssueCode::InvalidManifest
                && issue.message.contains("duplicate resolved path row")
        }));

        let mut dup_layer = inventory(&manifest);
        let repeated = dup_layer.layers[0].clone();
        dup_layer.layers.push(repeated);
        let report = manifest.validate_playback(&dup_layer);
        assert!(!report.compatible);
        assert!(report.issues.iter().any(|issue| {
            issue.code == ContentIssueCode::InvalidManifest
                && issue.message.contains("duplicate layer row")
        }));

        let mut dup_dlc = inventory(&manifest);
        let repeated = dup_dlc.dlc_selection[0].clone();
        dup_dlc.dlc_selection.push(repeated);
        let report = manifest.validate_playback(&dup_dlc);
        assert!(!report.compatible);
        assert!(report.issues.iter().any(|issue| {
            issue.code == ContentIssueCode::InvalidManifest
                && issue.message.contains("duplicate DLC row")
        }));
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
            "gfx/caf\u{e9}.asset"
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
    }
}
