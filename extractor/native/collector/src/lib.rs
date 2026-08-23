#![deny(unsafe_op_in_unsafe_fn)]
#![recursion_limit = "256"]

use core::{ptr, slice};
#[cfg(windows)]
use naval_combat_live_lane as naval4950;
use sha2::{Digest, Sha256};
#[cfg(windows)]
use std::collections::VecDeque;
use std::collections::{BTreeMap, BTreeSet};

pub mod domain_registry;
pub mod duplex_client;
#[cfg(windows)]
pub mod private_heap;
#[cfg(all(windows, not(test), not(feature = "test-support")))]
#[global_allocator]
static COLLECTOR_PRIVATE_HEAP: private_heap::PrivateHeap = private_heap::PrivateHeap;
pub mod custody_shadow;
pub mod domain_repeat_gate;
#[cfg(windows)]
pub mod early_render_capture_control;
pub mod engine_localisation_reader;
pub mod range_census;
pub mod read_only_scheduler;
/// provenance-gated coalescing policy, global interval ownership, custody
pub mod snapshot_recipes;

pub const COLLECTOR_FEATURE_BITS: u32 = {
    use compact_contract::control_transport::payloads::CollectorFeatureBits as F;
    F::DECLARED
        | if cfg!(feature = "test-support") {
            F::TEST_SUPPORT
        } else {
            0
        }
        | if cfg!(feature = "range-tracer") {
            F::RANGE_TRACER
        } else {
            0
        }
};

/// so release-artifact inspection can find it without Rust symbol metadata.
#[used]
#[no_mangle]
pub static BATTLE_PLAN_NO_SEMANTIC_ENGINE_STATE_WRITES_V1: [u8; 40] =
    *b"BATTLE_PLAN_NO_SEMANTIC_ENGINE_STATE_WRITES_V1\0";
#[cfg(windows)]
mod a15_counter_ui_hook;
#[cfg(windows)]
pub mod a31_focus_canvas_join;
#[cfg(windows)]
pub mod a34_a40_evidence;
#[cfg(windows)]
mod x15_trade_activation_hook;
pub mod xui21_registry;

#[cfg(windows)]
pub mod a30_controller_evidence {
    pub use diplomacy_intel_operatives_live::a30_controller_evidence::{
        arm_agency_refresh, arm_cipher_refresh, capture_complete, discovered_controllers, drain,
        evidence_activation_enabled, install_fixture_hooks, install_production_hooks, installed,
        receipt_from_drain, seal_drain_and_uninstall, visibility_products_from_drain,
        AgencyRefreshArm, CipherRefreshArm, ControllerEvidenceDiagnostics,
        ControllerEvidenceFamily, ControllerEvidenceReceipt, Domain53ControllerEvidenceRecord,
        HookDrain, InstalledHookAddresses, EVIDENCE_ACTIVATION_ENV,
    };

    pub fn encode_h4cr_batch(
        drain: &HookDrain,
        first_h4cr_sequence: u64,
    ) -> Result<Vec<Vec<u8>>, String> {
        drain
            .records
            .iter()
            .enumerate()
            .map(|(index, record)| {
                let sequence = first_h4cr_sequence
                    .checked_add(index as u64)
                    .ok_or_else(|| "A30 H4CR sequence range overflow".to_string())?;
                record.encode_h4cr(sequence)
            })
            .collect()
    }
}

/// surface-scoped; absent surface ownership remains fail closed.
#[cfg(windows)]
pub mod a30_operative_companion {
    pub use diplomacy_intel_operatives_live::a30_companion_carrier::{
        IntelligenceControllerCompanionCarrier, MAGIC as ICCP_MAGIC, SCHEMA as ICCP_SCHEMA,
    };
    pub use diplomacy_intel_operatives_live::a30_non_operation_action_identity::{
        MissionTargetKind, OperativeMissionProductIdentity,
    };
    pub use diplomacy_intel_operatives_live::a30_operative_operation_evidence::{
        abort_and_uninstall, arm, build_capture_recruitable_join, capture_complete,
        discovered_controllers, evidence_activation_enabled, install_fixture_hooks,
        install_production_hooks, receipt_sha256, seal_selected_present_and_uninstall, CaptureArm,
        ControllerOrderFamily, ControllerOrderRecord, OperativeRawObservation,
        SelectedPresentCompanionJoins, SelectedPresentCompanionReceipt, SelectedPresentStamp,
        ACTIVATION_ENV,
    };
    pub use diplomacy_intel_operatives_live::a30_operative_portrait_join::{
        OperativePortraitIdentityKind, OperativePortraitJoinReceipt,
        OperativePortraitSpriteObservation,
    };
    pub use diplomacy_intel_operatives_live::a30_recruitable_capture_join::{
        seal_capture_recruitable_join, validate_capture_recruitable_join_receipt,
        CaptureProducerObservation, CaptureRecruitableJoinReceipt,
        OperativeCaptureRecruitableSemantic, RecruitmentProducerObservation, RecruitmentVectorKind,
    };
    pub use diplomacy_intel_operatives_live::a30_visibility::{
        latest_observed_surface, IntelligenceSurface, IntelligenceSurfaceGate,
        NetworkProductWitness, VisibilityDisposition, VisibilityObservationKind,
        VisibilityProductJoins, VisibilityReceipt,
    };

    pub fn encode_iccp_payload(
        receipt: &SelectedPresentCompanionReceipt,
    ) -> Result<Vec<u8>, String> {
        receipt.to_production_carrier()?.encode()
    }
}

pub mod a30_capture_recruitable_join {
    pub use diplomacy_intel_operatives_live::a30_recruitable_capture_join::{
        seal_capture_recruitable_join, validate_capture_recruitable_join_receipt,
        CaptureProducerObservation, CaptureRecruitableJoinReceipt,
        OperativeCaptureRecruitableSemantic, RecruitmentProducerObservation, RecruitmentVectorKind,
        CAPTURE_ASSIGNMENT_DISCRIMINATOR, CAPTURE_PRODUCER_JOB_ID,
        CAPTURE_PRODUCER_MANIFEST_SHA256, COUNTRY_RECRUITMENT_WRAPPER_ACCESSOR_RVA,
        COUNTRY_RECRUITMENT_WRAPPER_OFFSET, EXECUTABLE_SHA256, LEADER_ASSIGNMENT_OFFSET,
        LEADER_CAPTURER_COUNTRY_ID_OFFSET, LEADER_CAPTURE_END_DATE_OFFSET, PREFERRED_IMAGE_BASE,
        RECRUITABLE_VECTOR_SOURCE_JOB_ID, RECRUITABLE_VECTOR_SOURCE_MANIFEST_SHA256,
        RECRUITABLE_VECTOR_SOURCE_RVA, RECRUITMENT_ITEM_CODEC_JOB_ID,
        RECRUITMENT_ITEM_CODEC_MANIFEST_SHA256, RECRUITMENT_MAIN_VECTOR_OFFSET,
        RECRUITMENT_NOT_TO_SPY_MASTER_VECTOR_OFFSET, RECRUITMENT_OBJECT_OFFSET_IN_WRAPPER,
        RECRUITMENT_OBJECT_SIZE, RECRUITMENT_OWNER_COUNTRY_POINTER_OFFSET,
        RECRUITMENT_OWNER_PATH_JOB_ID, RECRUITMENT_OWNER_PATH_MANIFEST_SHA256,
        RECRUITMENT_SERIALIZER_JOB_ID, RECRUITMENT_SERIALIZER_MANIFEST_SHA256,
        RECRUITMENT_VTABLE_ARTIFACT_SHA256, RECRUITMENT_VTABLE_JOB_ID, RECRUITMENT_VTABLE_RVA,
    };
}

pub const ABI_VERSION: u32 = 1;
pub const COLLECTOR_VERSION: &str = env!("CARGO_PKG_VERSION");
pub const COLLECTOR_PROFILE_ID: &str = compact_contract::build_profile::COLLECTOR_PROFILE_ID;

#[cfg(windows)]
static C07_O05_RECEIPT_WRITTEN: std::sync::atomic::AtomicBool =
    std::sync::atomic::AtomicBool::new(false);
#[cfg(windows)]
static H_CONVOY_FIXTURE_OWNER: std::sync::OnceLock<std::sync::Mutex<Option<HConvoyFixtureOwner>>> =
    std::sync::OnceLock::new();

pub struct InProcessRenderAdmission {
    process_start_nonce: u64,
    manifest: portable_content_manifest::ContentManifest,
    winners: production_render_submit_live_lane::RawEffectiveVfsWinnerIndexBinding,
    payload_ranges: BTreeMap<ObservedVfsPayloadRangeRequest, Vec<u8>>,
}

#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub struct ObservedVfsPayloadRangeRequest {
    pub content: portable_content_manifest::UsedContentKey,
    pub byte_offset: u64,
    pub byte_count: u64,
    pub range_sha256: String,
}

fn canonical_lower_sha256(value: &str) -> bool {
    value.len() == 64
        && value
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
}

// A recorded-input observation is minted PER selected Present (each hour's
// frame carries an observation of its own Present), so the admission keeps no
// session-wide copy; every hourly receipt must independently cross this
// launch-admission check before it may ride the hour's authority.
pub fn validate_hourly_recorded_input_admission(
    receipt: &production_render_catalog_live::AuthenticatedRecordedInputVariant,
    process_start_nonce: u64,
    source_game_executable_sha256: &str,
) -> Result<(), String> {
    let computed_receipt_sha256 = receipt
        .computed_receipt_sha256()
        .map_err(|error| format!("recorded input-family receipt hash: {error}"))?;
    if receipt.process_id == 0
        || receipt.process_start_nonce != process_start_nonce
        || receipt.executable_sha256 != source_game_executable_sha256
        || receipt.hook_epoch_nonce == 0
        || receipt.observation_sequence == 0
        || !canonical_lower_sha256(&receipt.authority_manifest_sha256)
        || !canonical_lower_sha256(&receipt.observation_receipt_sha256)
        || computed_receipt_sha256 != receipt.observation_receipt_sha256
    {
        return Err(
            "hourly recorded input-family receipt crosses its launch admission".into(),
        );
    }
    Ok(())
}

impl InProcessRenderAdmission {
    pub fn new(
        process_start_nonce: u64,
        manifest: portable_content_manifest::ContentManifest,
        winners: production_render_submit_live_lane::RawEffectiveVfsWinnerIndexBinding,
    ) -> Result<Self, String> {
        manifest.validate()?;
        winners.validate()?;
        if process_start_nonce == 0
            || winners.process_start_nonce != process_start_nonce
            || manifest.effective_vfs_manifest_sha256 != winners.authority_manifest_sha256
            || manifest.manifest_digest_sha256.is_none()
            || !manifest.required_content.is_empty()
            || manifest
                .ordered_layers
                .iter()
                .any(|layer| layer.required_for_playback)
            || manifest
                .dlc_selection
                .iter()
                .any(|dlc| dlc.required_for_playback)
        {
            return Err(
                "render admission envelope/winner/process binding is incomplete or pre-populated"
                    .into(),
            );
        }
        Ok(Self {
            process_start_nonce,
            manifest,
            winners,
            payload_ranges: BTreeMap::new(),
        })
    }

    pub fn validate_hourly_recorded_input(
        &self,
        receipt: &production_render_catalog_live::AuthenticatedRecordedInputVariant,
    ) -> Result<(), String> {
        validate_hourly_recorded_input_admission(
            receipt,
            self.process_start_nonce,
            &self.manifest.source_game.executable_sha256,
        )
    }

    pub fn manifest(&self) -> &portable_content_manifest::ContentManifest {
        &self.manifest
    }

    pub fn process_start_nonce(&self) -> u64 {
        self.process_start_nonce
    }

    pub fn catalog_authority_sha256(&self) -> &str {
        &self.winners.ordered_rows_sha256
    }

    fn winner_index(
        &self,
    ) -> &production_render_submit_live_lane::RawEffectiveVfsWinnerIndexBinding {
        &self.winners
    }

    fn admitted_used_content_for_path(
        &self,
        normalized_game_relative_path: &str,
    ) -> Result<portable_content_manifest::UsedContentKey, String> {
        let mut matches = self
            .winners
            .ordered_rows
            .iter()
            .filter(|row| row.content.logical_path == normalized_game_relative_path);
        let row = matches.next().ok_or_else(|| {
            format!("effective-VFS winner index has no {normalized_game_relative_path} row")
        })?;
        if matches.next().is_some() {
            return Err(format!(
                "effective-VFS winner index repeats {normalized_game_relative_path}"
            ));
        }
        self.winners.authorize_content(&row.content)?;
        manifest_key_for_raw(&self.manifest, &row.content)
    }

    fn missing_exact_authority_payloads(
        &self,
        requests: impl IntoIterator<
            Item = production_render_submit_live_lane::ExactAuthorityPayloadRequest,
        >,
    ) -> Result<Vec<ObservedVfsPayloadRangeRequest>, String> {
        let mut missing = BTreeSet::new();
        for request in requests {
            self.winners.authorize_content(&request.content)?;
            if request.byte_count == 0
                || request
                    .byte_offset
                    .checked_add(request.byte_count)
                    .is_none_or(|end| end > request.content.payload_bytes)
                || request.range_sha256.len() != 64
            {
                return Err("exact authority payload request is malformed".into());
            }
            let observed = ObservedVfsPayloadRangeRequest {
                content: manifest_key_for_raw(&self.manifest, &request.content)?,
                byte_offset: request.byte_offset,
                byte_count: request.byte_count,
                range_sha256: request.range_sha256,
            };
            if !self.payload_ranges.contains_key(&observed) {
                missing.insert(observed);
            }
        }
        Ok(missing.into_iter().collect())
    }

    fn missing_weather_catalog_payloads(
        &self,
        requests: impl IntoIterator<
            Item = production_render_submit_live_lane::WeatherCatalogPayloadRequest,
        >,
    ) -> Result<Vec<ObservedVfsPayloadRangeRequest>, String> {
        let mut missing = BTreeSet::new();
        for request in requests {
            request.content.validate()?;
            let raw = raw_vfs_identity(&request.content);
            self.winners.authorize_content(&raw)?;
            if request.content != manifest_key_for_raw(&self.manifest, &raw)?
                || request.byte_offset != 0
                || request.byte_count != request.content.payload_bytes
                || request.range_sha256 != request.content.payload_sha256
            {
                return Err(
                    "weather catalog payload request is not an exact admitted winner".into(),
                );
            }
            let observed = ObservedVfsPayloadRangeRequest {
                content: request.content,
                byte_offset: request.byte_offset,
                byte_count: request.byte_count,
                range_sha256: request.range_sha256,
            };
            if !self.payload_ranges.contains_key(&observed) {
                missing.insert(observed);
            }
        }
        Ok(missing.into_iter().collect())
    }

    pub fn bind_payload_range(
        &mut self,
        content: portable_content_manifest::UsedContentKey,
        byte_offset: u64,
        byte_count: u64,
        range_sha256: String,
        payload: Vec<u8>,
    ) -> Result<(), String> {
        content.validate()?;
        if content.effective_vfs_manifest_sha256 != self.winners.authority_manifest_sha256
            || byte_count == 0
            || byte_offset
                .checked_add(byte_count)
                .is_none_or(|end| end > content.payload_bytes)
            || payload.len() as u64 != byte_count
            || format!("{:x}", Sha256::digest(&payload)) != range_sha256
        {
            return Err("render payload range is not an exact authenticated winner range".into());
        }
        let raw = raw_vfs_identity(&content);
        self.winners.authorize_content(&raw)?;
        let request = ObservedVfsPayloadRangeRequest {
            content,
            byte_offset,
            byte_count,
            range_sha256,
        };
        if self.payload_ranges.insert(request, payload).is_some() {
            return Err("render payload range was delivered more than once".into());
        }
        Ok(())
    }

    pub fn missing_observed_payloads<'a>(
        &'a self,
        catalogs: impl IntoIterator<
            Item = &'a production_render_submit_live_lane::RawRenderCatalogCapture,
        >,
    ) -> Result<Vec<ObservedVfsPayloadRangeRequest>, String> {
        let mut required = BTreeSet::new();
        for catalog in catalogs {
            if catalog.process.process_start_nonce != self.process_start_nonce {
                return Err("render catalog belongs to another process admission".into());
            }
            let payloads = catalog
                .shader_creations
                .iter()
                .map(|shader| &shader.bytecode)
                .chain(
                    catalog
                        .buffer_snapshots
                        .iter()
                        .map(|snapshot| &snapshot.payload),
                )
                .chain(
                    catalog
                        .texture_resource_snapshots
                        .iter()
                        .flat_map(|texture| texture.ordered_subresources.iter())
                        .map(|subresource| &subresource.payload),
                );
            for payload in payloads {
                if let production_render_submit_live_lane::RawPayloadSource::VfsRange {
                    content,
                    source_byte_offset,
                    byte_count,
                    range_sha256,
                } = &payload.source
                {
                    let key = manifest_key_for_raw(&self.manifest, content)?;
                    self.winners.authorize_content(content)?;
                    let request = ObservedVfsPayloadRangeRequest {
                        content: key,
                        byte_offset: *source_byte_offset,
                        byte_count: *byte_count,
                        range_sha256: range_sha256.clone(),
                    };
                    if !self.payload_ranges.contains_key(&request) {
                        required.insert(request);
                    }
                }
            }
        }
        Ok(required.into_iter().collect())
    }

    pub fn manifest_for_observed_catalogs<'a>(
        &self,
        catalogs: impl IntoIterator<
            Item = &'a production_render_submit_live_lane::RawRenderCatalogCapture,
        >,
    ) -> Result<portable_content_manifest::ContentManifest, String> {
        #[derive(Clone)]
        struct Observation {
            kind: portable_content_manifest::RequiredContentKind,
            count: u64,
            consumers: BTreeSet<String>,
        }

        fn add_content(
            admission: &InProcessRenderAdmission,
            observations: &mut BTreeMap<portable_content_manifest::UsedContentKey, Observation>,
            content: &production_render_submit_live_lane::RawVfsContentIdentity,
            kind: portable_content_manifest::RequiredContentKind,
            consumer: &'static str,
        ) -> Result<(), String> {
            admission.winners.authorize_content(content)?;
            let key = manifest_key_for_raw(&admission.manifest, content)?;
            let row = observations.entry(key).or_insert_with(|| Observation {
                kind: kind.clone(),
                count: 0,
                consumers: BTreeSet::new(),
            });
            if row.kind != kind {
                row.kind = portable_content_manifest::RequiredContentKind::Other;
            }
            row.count = row
                .count
                .checked_add(1)
                .ok_or("render content observation count overflow")?;
            row.consumers.insert(consumer.into());
            Ok(())
        }

        fn add(
            admission: &InProcessRenderAdmission,
            observations: &mut BTreeMap<portable_content_manifest::UsedContentKey, Observation>,
            payload: &production_render_submit_live_lane::RawPayloadSnapshot,
            kind: portable_content_manifest::RequiredContentKind,
            consumer: &'static str,
        ) -> Result<(), String> {
            let production_render_submit_live_lane::RawPayloadSource::VfsRange { content, .. } =
                &payload.source
            else {
                return Ok(());
            };
            add_content(admission, observations, content, kind, consumer)
        }

        let mut observations = BTreeMap::new();
        for catalog in catalogs {
            if catalog.process.process_start_nonce != self.process_start_nonce {
                return Err("render catalog belongs to another process admission".into());
            }
            for shader in &catalog.shader_creations {
                add(
                    self,
                    &mut observations,
                    &shader.bytecode,
                    portable_content_manifest::RequiredContentKind::CompiledShader,
                    "production_render.shader_bytecode",
                )?;
            }
            for buffer in &catalog.buffer_snapshots {
                add(
                    self,
                    &mut observations,
                    &buffer.payload,
                    portable_content_manifest::RequiredContentKind::Other,
                    "production_render.buffer_payload",
                )?;
            }
            for texture in &catalog.texture_resource_snapshots {
                if let Some(content) = &texture.content {
                    add_content(
                        self,
                        &mut observations,
                        content,
                        portable_content_manifest::RequiredContentKind::Texture,
                        "production_render.texture_resource",
                    )?;
                }
                for subresource in &texture.ordered_subresources {
                    add(
                        self,
                        &mut observations,
                        &subresource.payload,
                        portable_content_manifest::RequiredContentKind::Texture,
                        "production_render.texture_subresource",
                    )?;
                }
            }
            for program in &catalog.program_identities {
                add_content(
                    self,
                    &mut observations,
                    &program.source,
                    portable_content_manifest::RequiredContentKind::ShaderSource,
                    "production_render.shader_program_source",
                )?;
            }
            for semantics in &catalog.draw_semantics {
                match &semantics.content_identity {
                    production_render_submit_live_lane::RawDrawContentIdentity::Particle {
                        definition_content,
                        ..
                    } => add_content(
                        self,
                        &mut observations,
                        definition_content,
                        portable_content_manifest::RequiredContentKind::Definition,
                        "production_render.particle_definition",
                    )?,
                    production_render_submit_live_lane::RawDrawContentIdentity::Entity {
                        resolved_entity_asset,
                        mesh,
                        ..
                    } => {
                        add_content(
                            self,
                            &mut observations,
                            resolved_entity_asset,
                            portable_content_manifest::RequiredContentKind::Definition,
                            "production_render.entity_asset",
                        )?;
                        add_content(
                            self,
                            &mut observations,
                            mesh,
                            portable_content_manifest::RequiredContentKind::Mesh,
                            "production_render.entity_mesh",
                        )?;
                    }
                }
                for target in semantics
                    .ordered_color_targets
                    .iter()
                    .chain(semantics.depth_target.iter())
                {
                    if let Some(content) = &target.persistent_content {
                        add_content(
                            self,
                            &mut observations,
                            content,
                            portable_content_manifest::RequiredContentKind::Texture,
                            "production_render.persistent_output",
                        )?;
                    }
                }
            }
        }

        let required_ordinals = observations
            .keys()
            .map(|content| content.winning_layer_ordinal)
            .collect::<BTreeSet<_>>();
        let mut required_dlc_ids = BTreeSet::new();
        for content in observations.keys().filter(|content| {
            content.layer_kind == portable_content_manifest::ContentLayerKind::Dlc
        }) {
            required_dlc_ids.insert(stable_dlc_id_from_layer(&content.layer_id)?);
        }
        let mut manifest = self.manifest.clone();
        manifest.required_content = observations
            .into_iter()
            .map(
                |(content, observation)| portable_content_manifest::RequiredContent {
                    content_key: content.normalized_game_relative_path.clone(),
                    kind: observation.kind,
                    content,
                    observation_count: observation.count,
                    consumers: observation.consumers.into_iter().collect(),
                },
            )
            .collect();
        for layer in &mut manifest.ordered_layers {
            layer.required_for_playback = required_ordinals.contains(&layer.ordinal);
        }
        for dlc in &mut manifest.dlc_selection {
            dlc.required_for_playback = dlc.enabled && required_dlc_ids.contains(&dlc.dlc_id);
        }
        manifest.manifest_digest_sha256 = None;
        manifest.source_profile.selection_sha256 = manifest.computed_selection_sha256()?;
        manifest.with_computed_digest()
    }
}

impl production_render_catalog_live::AdmittedVfsPayloads for InProcessRenderAdmission {
    fn payload(&self, content: &portable_content_manifest::UsedContentKey) -> Option<&[u8]> {
        let request = ObservedVfsPayloadRangeRequest {
            content: content.clone(),
            byte_offset: 0,
            byte_count: content.payload_bytes,
            range_sha256: content.payload_sha256.clone(),
        };
        self.payload_ranges.get(&request).map(Vec::as_slice)
    }

    fn payload_range(
        &self,
        content: &portable_content_manifest::UsedContentKey,
        byte_offset: u64,
        byte_count: u64,
        range_sha256: &str,
    ) -> Option<&[u8]> {
        self.payload_ranges
            .get(&ObservedVfsPayloadRangeRequest {
                content: content.clone(),
                byte_offset,
                byte_count,
                range_sha256: range_sha256.into(),
            })
            .map(Vec::as_slice)
    }
}

impl production_render_submit_live_lane::AdmittedWeatherCatalogPayloads
    for InProcessRenderAdmission
{
    fn payload(&self, content: &portable_content_manifest::UsedContentKey) -> Option<&[u8]> {
        let request = ObservedVfsPayloadRangeRequest {
            content: content.clone(),
            byte_offset: 0,
            byte_count: content.payload_bytes,
            range_sha256: content.payload_sha256.clone(),
        };
        self.payload_ranges.get(&request).map(Vec::as_slice)
    }
}

impl production_render_submit_live_lane::AdmittedAuthorityPayloads for InProcessRenderAdmission {
    fn payload_range(
        &self,
        content: &production_render_submit_live_lane::RawVfsContentIdentity,
        byte_offset: u64,
        byte_count: u64,
        range_sha256: &str,
    ) -> Option<&[u8]> {
        self.winners.authorize_content(content).ok()?;
        let content = manifest_key_for_raw(&self.manifest, content).ok()?;
        self.payload_ranges
            .get(&ObservedVfsPayloadRangeRequest {
                content,
                byte_offset,
                byte_count,
                range_sha256: range_sha256.into(),
            })
            .map(Vec::as_slice)
    }
}

pub struct FinalizedA729RuntimeCapture {
    pub promoted: PromotedA729ProductionRenderSubmit,
    pub raw: production_render_submit_live_lane::RawRenderSubmitCapture,
    pub authority: portable_render_stream::RenderAuthority,
}

fn promote_a729_camera_owner_kind(
    raw: production_render_submit_live_lane::RawA729CameraOwnerKind,
) -> portable_render_stream::A729CameraOwnerKind {
    match raw {
        production_render_submit_live_lane::RawA729CameraOwnerKind::FrontEnd => {
            portable_render_stream::A729CameraOwnerKind::FrontEnd
        }
        production_render_submit_live_lane::RawA729CameraOwnerKind::InGame => {
            portable_render_stream::A729CameraOwnerKind::InGame
        }
        production_render_submit_live_lane::RawA729CameraOwnerKind::Third => {
            portable_render_stream::A729CameraOwnerKind::Third
        }
    }
}

fn promote_a729_camera_event_batch(
    raw: &production_render_submit_live_lane::RawA729CameraEventBatch,
) -> portable_render_stream::A729CameraEventBatch {
    portable_render_stream::A729CameraEventBatch {
        producer_rva: raw.producer_rva,
        owner_kind: promote_a729_camera_owner_kind(raw.owner_kind),
        owner_vtable_rva: raw.owner_vtable_rva,
        action_source_owner_offset: raw.action_source_owner_offset,
        cursor_source_owner_offset: raw.cursor_source_owner_offset,
        event_list_owner_offset: raw.event_list_owner_offset,
        frame_scalar_f32_bits: raw.frame_scalar_f32_bits,
        producer_flag_0_u8: raw.producer_flag_0_u8,
        producer_flag_1_u8: raw.producer_flag_1_u8,
        input_events: raw
            .input_events
            .iter()
            .map(|event| portable_render_stream::A729CameraInputEvent {
                node_ordinal: event.node_ordinal,
                node_kind_i32: event.node_kind_i32,
                consumed_before_u8: event.consumed_before_u8,
                consumed_after_u8: event.consumed_after_u8,
                event_type_i32: event.event_type_i32,
                signed_delta_i32: event.signed_delta_i32,
                embedded_event_16_bytes: event.embedded_event_16_bytes,
            })
            .collect(),
        map_picks: raw
            .map_picks
            .iter()
            .map(|pick| portable_render_stream::A729CameraMapPick {
                call_ordinal: pick.call_ordinal,
                mode_u8: pick.mode_u8,
                screen_xy_f32_bits: pick.screen_xy_f32_bits,
                world_xyz_f32_bits: pick.world_xyz_f32_bits,
            })
            .collect(),
        current_eye_before_f32_bits: raw.current_eye_before_f32_bits,
        target_eye_before_f32_bits: raw.target_eye_before_f32_bits,
        target_look_at_before_f32_bits: raw.target_look_at_before_f32_bits,
        current_eye_after_f32_bits: raw.current_eye_after_f32_bits,
        target_eye_after_f32_bits: raw.target_eye_after_f32_bits,
        target_look_at_after_f32_bits: raw.target_look_at_after_f32_bits,
    }
}

fn promote_a729_camera_carrier(
    raw: Option<&production_render_submit_live_lane::RawA729CameraCarrier>,
) -> Result<
    Option<portable_render_stream::AuthenticatedCameraCarrier>,
    production_render_submit_live_lane::PromotionError,
> {
    let Some(raw) = raw else {
        return Ok(None);
    };
    let owner_kind = promote_a729_camera_owner_kind(raw.owner_kind);
    let mut carrier = portable_render_stream::AuthenticatedCameraCarrier {
        schema: portable_render_stream::A729_CAMERA_TEMPORAL_SCHEMA.into(),
        executable_sha256: portable_render_stream::A729_CAMERA_EXECUTABLE_SHA256.into(),
        capture_boundary_rva: portable_render_stream::A729_CAMERA_ENVELOPE_RVA,
        camera_update_rva: portable_render_stream::A729_CAMERA_UPDATE_RVA,
        carrier_ordinal: raw.carrier_ordinal,
        carrier_offset: raw.carrier_offset,
        owner_kind,
        owner_vtable_rva: raw.owner_vtable_rva,
        render_view_436_bytes: raw.render_view_436_bytes.clone(),
        canonical_camera_root_436_bytes: raw.canonical_render_view_436_bytes.clone(),
        previous_current_eye_f32_bits: raw.previous_current_eye_f32_bits,
        current_eye_f32_bits: raw.current_eye_f32_bits,
        current_look_at_f32_bits: raw.current_look_at_f32_bits,
        target_eye_f32_bits: raw.target_eye_f32_bits,
        target_look_at_f32_bits: raw.target_look_at_f32_bits,
        viewport_i32: raw.viewport_i32,
        min_height_i32: raw.min_height_i32,
        max_height_i32: raw.max_height_i32,
        update_produced_u8: raw.update_produced_u8,
        special_free_state_u8: raw.special_free_state_u8,
        ramp_f32_bits: raw.ramp_f32_bits,
        wrap_branch_u8: raw.wrap_branch_u8,
        camera_event_batches: raw
            .camera_event_batches
            .iter()
            .map(promote_a729_camera_event_batch)
            .collect(),
        payload_sha256: String::new(),
    };
    carrier.payload_sha256 = carrier.computed_payload_sha256();
    Ok(Some(carrier))
}

pub fn portable_submit_from_a729_runtime(
    promoted: PromotedA729ProductionRenderSubmit,
    global_order: u32,
) -> Result<portable_render_stream::RenderSubmit, String> {
    let (mut submit, s1_shadow) =
        portable_submit_with_pending_s1_from_a729_runtime(promoted, global_order)?;
    if let Some(carrier) = s1_shadow {
        fold_a729_s1_shadow_carrier(&mut submit, carrier).map_err(s1_fold_error_message)?;
    }
    Ok(submit)
}

fn portable_submit_with_pending_s1_from_a729_runtime(
    promoted: PromotedA729ProductionRenderSubmit,
    global_order: u32,
) -> Result<
    (
        portable_render_stream::RenderSubmit,
        Option<portable_render_stream::S1ShadowCarrier>,
    ),
    String,
> {
    if promoted.submit_index == 0 {
        return Err("portable d77 submit has a null submit identity".into());
    }
    let particle_packet_set_sha256 = canonical_json_sha256(&promoted.particles)?;
    let s1_shadow = promoted.s1_shadow.clone();
    let submit = portable_render_stream::RenderSubmit {
        global_order,
        camera_carrier_ordinal: promoted.camera_carrier_ordinal,
        camera_carrier: promoted.camera_carrier,
        ordered_static_map_programs: Vec::new(),
        authenticated_texture_snapshots: Vec::new(),
        tree_runtime: None,
        s1_shadow: None,
        particles: promoted.particles,
        particle_packet_set_sha256,
        entities: promoted.entities,
        rail_construction_arrows: Vec::new(),
        ordered_draws: promoted.ordered_draws,
        ordered_render_operations: Vec::new(),
        tiled_lights: Vec::new(),
        weather_membership: Vec::new(),
        political_textures: Vec::new(),
    };
    Ok((submit, s1_shadow))
}

fn s1_fold_error_message(error: production_render_submit_live_lane::PromotionError) -> String {
    match error {
        production_render_submit_live_lane::PromotionError::Invalid(message) => message,
        production_render_submit_live_lane::PromotionError::UnresolvedCatalog {
            registration_token,
            packet_ordinal,
        } => format!(
            "S1 fold unexpectedly requested unresolved catalog token {registration_token} packet {packet_ordinal}"
        ),
    }
}

#[cfg(windows)]
#[allow(clippy::too_many_arguments)]
fn promote_a729_rail_supply_arrow_frame(
    raw: &production_render_submit_live_lane::a729_hook::RawRailSupplyArrowFrame,
    expected_frame_sequence: u64,
    expected_submit_index: u64,
    update_index: u64,
    first_sequence: u64,
    last_sequence: u64,
    events: &[production_render_submit_live_lane::RawRenderOrderEvent],
    programs: &[portable_render_stream::StaticMapPassProgram],
    used_content: Vec<portable_content_manifest::UsedContentKey>,
) -> Result<Option<portable_render_stream::RailSupplyArrowFrame>, String> {
    production_render_submit_live_lane::a729_hook::validate_rail_supply_arrow_frame(raw)?;
    if raw.rows.is_empty() && raw.draw_joins.is_empty() {
        return Ok(None);
    }
    if raw.rows.is_empty()
        || raw.draw_joins.is_empty()
        || raw.frame_sequence != expected_frame_sequence
        || raw.submit_index != expected_submit_index
    {
        return Err(
            "rail supply-arrow raw frame is empty on one side or crosses frame identity".into(),
        );
    }

    let construction_mode_u8 = raw.rows[0].construction_mode_u8;
    if raw
        .rows
        .iter()
        .any(|row| row.construction_mode_u8 != construction_mode_u8)
    {
        return Err("rail supply-arrow selector rows mix construction modes".into());
    }
    let construction_mode = match construction_mode_u8 {
        0 => portable_render_stream::RailSupplyConstructionMode::EmptyLevel,
        1 => portable_render_stream::RailSupplyConstructionMode::BelowTargetLevel,
        _ => return Err("rail supply-arrow construction mode is outside exact a729 law".into()),
    };

    let mut capture_to_reference = BTreeMap::new();
    let mut next_static_ordinal = 0u32;
    for event in events {
        match event {
            production_render_submit_live_lane::RawRenderOrderEvent::FinalDraw {
                sequence,
                family: production_render_submit_live_lane::FinalDrawFamily::StaticOrPostProcess,
                ..
            } => {
                let reference = portable_render_stream::RailSupplyArrowDrawRef {
                    program_ordinal: next_static_ordinal,
                    operation_ordinal: next_static_ordinal,
                };
                if capture_to_reference.insert(*sequence, reference).is_some() {
                    return Err("rail supply-arrow static capture sequence is ambiguous".into());
                }
                next_static_ordinal = next_static_ordinal
                    .checked_add(1)
                    .ok_or("rail supply-arrow static operation ordinal overflows")?;
            }
            production_render_submit_live_lane::RawRenderOrderEvent::ClearColor { .. }
            | production_render_submit_live_lane::RawRenderOrderEvent::ClearDepthStencil {
                ..
            } => {
                next_static_ordinal = next_static_ordinal
                    .checked_add(1)
                    .ok_or("rail supply-arrow static operation ordinal overflows")?;
            }
            _ => {}
        }
    }
    if usize::try_from(next_static_ordinal).ok() != Some(programs.len()) {
        return Err("rail supply-arrow static event/program counts disagree".into());
    }
    for (ordinal, program) in programs.iter().enumerate() {
        let ordinal = u32::try_from(ordinal)
            .map_err(|_| "rail supply-arrow static program ordinal exceeds u32")?;
        if program.program_ordinal != ordinal
            || program.ordered_operations.len() != 1
            || program.ordered_operations[0].operation_ordinal != ordinal
        {
            return Err(
                "rail supply-arrow static programs are not canonical singleton operations".into(),
            );
        }
    }

    let mut raw_rows = BTreeMap::new();
    for row in &raw.rows {
        if raw_rows.insert(row.selector_sequence, row).is_some() {
            return Err("rail supply-arrow selector sequence is duplicated".into());
        }
    }
    let mut member_refs =
        BTreeMap::<u64, Vec<portable_render_stream::RailSupplyArrowDrawMemberRef>>::new();
    let mut row_event_sequences = BTreeMap::<u64, u64>::new();
    let mut draws = Vec::with_capacity(raw.draw_joins.len());
    for join in &raw.draw_joins {
        let row = raw_rows
            .get(&join.selector_sequence)
            .copied()
            .ok_or("rail supply-arrow draw references an absent selector row")?;
        let reference = capture_to_reference
            .get(&join.capture_sequence)
            .cloned()
            .ok_or("rail supply-arrow capture sequence has no static operation")?;
        let program = programs
            .get(reference.program_ordinal as usize)
            .ok_or("rail supply-arrow static program reference is absent")?;
        let operation = program
            .ordered_operations
            .first()
            .ok_or("rail supply-arrow static program has no operation")?;
        let portable_render_stream::StaticMapCommand::Draw { draw: exact_draw } =
            &operation.command
        else {
            return Err(
                "rail supply-arrow capture sequence resolves to a non-draw operation".into(),
            );
        };
        let (pass, effect_token) = match join.pass {
            production_render_submit_live_lane::a729_hook::RawRailSupplyArrowPass::Normal => (
                portable_render_stream::RailSupplyArrowDrawPass::Normal,
                "map_no_head",
            ),
            production_render_submit_live_lane::a729_hook::RawRailSupplyArrowPass::SeeThrough => (
                portable_render_stream::RailSupplyArrowDrawPass::SeeThrough,
                "map_no_head_see_through",
            ),
        };
        if exact_draw.shader_program.effect_token != effect_token {
            return Err("rail supply-arrow draw disagrees with authenticated effect token".into());
        }
        let portable_render_stream::StaticGeometrySource::Exact { geometry } = &exact_draw.geometry
        else {
            return Err("rail supply-arrow draw uses interpreter geometry".into());
        };
        let portable_render_stream::DrawCommand::Draw {
            vertex_count,
            start_vertex,
        } = &geometry.draw
        else {
            return Err("rail supply-arrow join is not an exact non-indexed draw".into());
        };
        if *start_vertex != join.first_vertex || *vertex_count != join.vertex_count {
            return Err("rail supply-arrow draw range differs from the raw VB join".into());
        }
        let mut slot_zero = geometry
            .ordered_vertex_buffers
            .iter()
            .filter(|binding| binding.input_slot == 0);
        let vertex_buffer = slot_zero
            .next()
            .ok_or("rail supply-arrow exact draw has no slot-zero vertex buffer")?;
        if slot_zero.next().is_some() {
            return Err("rail supply-arrow exact draw repeats its slot-zero vertex buffer".into());
        }
        let byte_width = match &vertex_buffer.source {
            portable_render_stream::BufferSource::Inline { bytes, .. } => {
                u32::try_from(bytes.len())
                    .map_err(|_| "rail supply-arrow inline vertex buffer exceeds u32")?
            }
            portable_render_stream::BufferSource::UsedContentRange { byte_count, .. } => {
                u32::try_from(*byte_count)
                    .map_err(|_| "rail supply-arrow used-content vertex range exceeds u32")?
            }
            portable_render_stream::BufferSource::ParticleInstanceRecords => {
                return Err("rail supply-arrow draw references particle instance records".into())
            }
        };
        let vertex_end = join
            .first_vertex
            .checked_add(join.vertex_count)
            .ok_or("rail supply-arrow vertex range overflows")?;
        let byte_end = vertex_end
            .checked_mul(vertex_buffer.stride_bytes)
            .and_then(|value| value.checked_add(vertex_buffer.byte_offset))
            .ok_or("rail supply-arrow vertex byte range overflows")?;
        if vertex_buffer.stride_bytes == 0
            || byte_width != join.vertex_buffer_byte_width
            || byte_end > byte_width
        {
            return Err(
                "rail supply-arrow vertex range is outside the authenticated slot-zero buffer"
                    .into(),
            );
        }

        let member_reference = portable_render_stream::RailSupplyArrowDrawMemberRef {
            program_ordinal: reference.program_ordinal,
            operation_ordinal: reference.operation_ordinal,
            member_ordinal: 0,
        };
        member_refs
            .entry(join.selector_sequence)
            .or_default()
            .push(member_reference);
        row_event_sequences
            .entry(join.selector_sequence)
            .and_modify(|sequence| *sequence = (*sequence).min(join.capture_sequence))
            .or_insert(join.capture_sequence);
        draws.push(portable_render_stream::RailSupplyArrowDraw {
            reference,
            pass,
            draw_payload_sha256: exact_draw.payload_sha256.clone(),
            members: vec![portable_render_stream::RailSupplyArrowDrawMember {
                stable_key: portable_render_stream::RailSupplyArrowKey {
                    aggregate_row_ordinal: row.row_ordinal,
                    ordered_province_ids: row.ordered_province_ids.clone(),
                },
                member_ordinal: 0,
                first_vertex: join.first_vertex,
                vertex_count: join.vertex_count,
            }],
        });
    }
    draws.sort_by_key(|draw| draw.reference.operation_ordinal);
    let ordered_draws = draws
        .iter()
        .map(|draw| draw.reference.clone())
        .collect::<Vec<_>>();
    let rows = raw
        .rows
        .iter()
        .map(|row| {
            let mut members = member_refs
                .remove(&row.selector_sequence)
                .ok_or("rail supply-arrow selector row has no exact draw members")?;
            members.sort();
            portable_render_stream::RailSupplyArrowRow::from_exact_a729_observation(
                row_event_sequences
                    .remove(&row.selector_sequence)
                    .ok_or("rail supply-arrow selector row has no exact draw sequence")?,
                row.row_ordinal,
                row.ordered_province_ids.clone(),
                construction_mode,
                row.effective_level_i32,
                row.target_level_i32,
                row.cooldown_selector_i32,
                row.flags_u32,
                row.max_damage_fixed5,
                row.damaged_u8,
                row.selected_rgba_f32_bits,
                row.texture_slot_u32,
                portable_render_stream::RailSupplyVisibility::Submitted,
                members,
            )
        })
        .collect::<Result<Vec<_>, String>>()?;
    if !member_refs.is_empty() || !row_event_sequences.is_empty() {
        return Err("rail supply-arrow draw members remain after semantic-row join".into());
    }
    portable_render_stream::RailSupplyArrowFrame::sealed_exact_a729(
        update_index,
        first_sequence,
        last_sequence,
        construction_mode,
        used_content,
        rows,
        draws,
        ordered_draws,
        programs,
    )
    .map(Some)
}

pub fn assemble_production_render_frame(
    authority: portable_render_stream::RenderAuthority,
    selected_present: portable_render_stream::SelectedPresentReceipt,
    snow_mud_texture: Option<portable_render_stream::SnowMudTexturePacket>,
    update_index: u64,
    first_sequence: u64,
    last_sequence: u64,
    presentation_qpc_frequency_hz: u64,
    submits: Vec<portable_render_stream::RenderSubmit>,
    rail_supply_arrows: Option<portable_render_stream::RailSupplyArrowFrame>,
    authenticated_ui_pass: Option<portable_render_stream::AuthenticatedUiPass>,
) -> Result<portable_render_stream::ProductionRenderFrame, String> {
    if submits.is_empty() {
        return Err("d77 frame has no fully promoted submit".into());
    }
    let display_submit_index = Some(selected_present.selected_candidate_id.to_string());
    let empty_effects: Vec<portable_render_stream::EffectLifecycleEvent> = Vec::new();
    let empty_lights: Vec<portable_render_stream::LightLifecycleEvent> = Vec::new();
    let particles = submits
        .iter()
        .flat_map(|submit| submit.particles.iter())
        .collect::<Vec<_>>();
    let entities = submits
        .iter()
        .flat_map(|submit| submit.entities.iter())
        .collect::<Vec<_>>();
    let mut weather_by_region = BTreeMap::new();
    for membership in submits
        .iter()
        .flat_map(|submit| submit.weather_membership.iter())
    {
        if weather_by_region
            .get(&membership.strategic_region_id)
            .is_some_and(|prior: &portable_render_stream::WeatherMembership| {
                membership.sequence <= prior.sequence
            })
        {
            return Err("weather checkpoint fold is repeated or non-monotonic by region".into());
        }
        weather_by_region.insert(membership.strategic_region_id, membership.clone());
    }
    let weather_checkpoint = weather_by_region.into_values().collect::<Vec<_>>();
    let checkpoint =
        portable_render_stream::RenderCheckpoint {
            last_sequence,
            last_update_index: update_index,
            last_submit_index: display_submit_index.clone(),
            next_stable_ids: BTreeMap::new(),
            active_effect_lifecycle: empty_effects.clone(),
            active_light_lifecycle: empty_lights.clone(),
            latest_tiled_light_payload: None,
            weather_state_and_ordered_membership: weather_checkpoint.clone(),
            active_effect_set_sha256: canonical_json_sha256(&empty_effects)?,
            ordered_particle_packets_sha256: canonical_json_sha256(&particles)?,
            ordered_entity_submissions_sha256: canonical_json_sha256(&entities)?,
            ordered_light_submissions_sha256: canonical_json_sha256(&empty_lights)?,
            weather_state_sha256: portable_render_stream::weather_state_checkpoint_sha256(
                &weather_checkpoint,
            )?,
            weather_membership_sha256: canonical_json_sha256(&weather_checkpoint)?,
            weather_lifecycle_sha256: portable_render_stream::weather_lifecycle_checkpoint_sha256(
                &weather_checkpoint,
                &empty_effects,
            )?,
            weather_visibility_sha256:
                portable_render_stream::weather_visibility_checkpoint_sha256(&weather_checkpoint)?,
            ordered_weather_packets_sha256:
                portable_render_stream::ordered_weather_packets_checkpoint_sha256(&submits)?,
            weather_composition_sha256:
                portable_render_stream::weather_composition_checkpoint_sha256(&submits)?,
        };
    let frame = portable_render_stream::ProductionRenderFrame {
        schema: portable_render_stream::SCHEMA.into(),
        authority,
        update_index,
        first_sequence,
        last_sequence,
        presentation_qpc_frequency_hz,
        display_submit_index,
        selected_present,
        snow_mud_texture,
        submits,
        m02_combat: None,
        m03_arrows: None,
        rail_supply_arrows,
        o04_cities: None,
        o05_buildings: None,
        authenticated_ui_pass,
        checkpoint,
    };
    frame.validate()?;
    Ok(frame)
}

pub fn attach_o05_building_frame(
    frame: &mut portable_render_stream::ProductionRenderFrame,
    raw: &production_render_submit_live_lane::RawO05BuildingFrame,
) -> Result<(), production_render_submit_live_lane::PromotionError> {
    if frame.o05_buildings.is_some()
        || raw.update_index != frame.update_index
        || raw.first_sequence != frame.first_sequence
        || raw.last_sequence != frame.last_sequence
        || raw.map_buildings_source.effective_vfs_manifest_sha256
            != frame.authority.effective_vfs_manifest_sha256
    {
        return Err(production_render_submit_live_lane::PromotionError::Invalid(
            "O05 observation is duplicated or differs from the selected production frame".into(),
        ));
    }
    let submit_index = frame
        .display_submit_index
        .as_deref()
        .ok_or_else(|| {
            production_render_submit_live_lane::PromotionError::Invalid(
                "O05 observation has no authenticated display submit".into(),
            )
        })?
        .parse::<u64>()
        .map_err(|_| {
            production_render_submit_live_lane::PromotionError::Invalid(
                "O05 display submit index is not canonical u64".into(),
            )
        })?;
    let display_submit = frame.submit(submit_index).ok_or_else(|| {
        production_render_submit_live_lane::PromotionError::Invalid(
            "O05 authenticated display submit is absent".into(),
        )
    })?;
    let o05 = production_render_submit_live_lane::promote_o05_building_frame(raw, display_submit)?;
    frame.o05_buildings = Some(o05);
    frame
        .validate()
        .map_err(production_render_submit_live_lane::PromotionError::Invalid)
}

#[cfg(windows)]
fn write_c07_o05_acquisition_receipt_if_requested(
    frame: &portable_render_stream::ProductionRenderFrame,
    scope: &production_render_submit_live_lane::a729_hook::RawO05RuntimeScopeObservation,
) -> Result<(), String> {
    let Some(output_root) = std::env::var_os("BATTLE_PLAN_C07_O05_EVIDENCE_OUT_DIR").or_else(|| std::env::var_os("HOI4_C07_O05_EVIDENCE_OUT_DIR")) else {
        return Ok(());
    };
    let output_root = std::path::PathBuf::from(output_root);
    if !output_root.is_absolute() || !output_root.is_dir() {
        return Err(
            "BATTLE_PLAN_C07_O05_EVIDENCE_OUT_DIR must name an existing absolute directory".into(),
        );
    }
    let o05 = frame
        .o05_buildings
        .as_ref()
        .ok_or("C07 O05 evidence writer received a frame without O05")?;
    if o05.ordered_outputs.is_empty() {
        return Ok(());
    }
    if C07_O05_RECEIPT_WRITTEN
        .compare_exchange(
            false,
            true,
            std::sync::atomic::Ordering::AcqRel,
            std::sync::atomic::Ordering::Acquire,
        )
        .is_err()
    {
        return Ok(());
    }
    let write_result = (|| {
        let frame_bytes = frame
            .encode_json()
            .map_err(|error| format!("C07 O05 production frame encoding failed: {error}"))?;
        let mut raw_hasher = Sha256::new();
        raw_hasher.update(b"hoi4-c07-o05-raw-selected-scope-v1\0");
        raw_hasher.update(scope.game_hour_raw.to_le_bytes());
        raw_hasher.update(scope.first_qpc_ticks.to_le_bytes());
        raw_hasher.update(scope.completed_qpc_ticks.to_le_bytes());
        raw_hasher.update(scope.qpc_frequency_hz.to_le_bytes());
        raw_hasher.update(
            u64::try_from(scope.records.len())
                .map_err(|_| "C07 O05 raw record count exceeds u64")?
                .to_le_bytes(),
        );
        for row in &scope.records {
            raw_hasher.update(row.runtime_object_id.to_le_bytes());
            raw_hasher.update(row.record_bytes);
            raw_hasher.update(row.selected_entity_instance_id.unwrap_or(0).to_le_bytes());
            match row.world_transform_4x4_f32_bits {
                Some(matrix) => {
                    raw_hasher.update([1]);
                    for bits in matrix {
                        raw_hasher.update(bits.to_le_bytes());
                    }
                }
                None => raw_hasher.update([0]),
            }
        }
        let disposition_counts = o05.records.iter().fold([0u64; 3], |mut counts, row| {
            match row.render_disposition {
                portable_render_stream::O05BuildingRenderDisposition::NoEntitySelected => {
                    counts[0] += 1
                }
                portable_render_stream::O05BuildingRenderDisposition::SelectedNotSubmitted => {
                    counts[1] += 1
                }
                portable_render_stream::O05BuildingRenderDisposition::Submitted => counts[2] += 1,
            }
            counts
        });
        let mut body = serde_json::json!({
            "schema": "hoi4.c07.o05.selected_present_observation.v1",
            "status": "candidate",
            "executable_sha256": portable_render_stream::O05_A729_EXECUTABLE_SHA256,
            "producer_manifest_sha256": portable_render_stream::O05_RUNTIME_PRODUCER_MANIFEST_SHA256,
            "consumer_manifest_sha256": portable_render_stream::O05_RUNTIME_CONSUMER_MANIFEST_SHA256,
            "producer_job_id": "20260720T125102Z-w5-c07-runtime-record-layout-selector-fi-4498228b",
            "consumer_job_id": "20260720T125147Z-w5-c07-entity-instance-transform-render--1040cb63",
            "selected_present_sha256": frame.selected_present.computed_sha256()?,
            "production_frame_sha256": format!("{:x}", Sha256::digest(&frame_bytes)),
            "raw_scope_sha256": format!("{:x}", raw_hasher.finalize()),
            "scope": {
                "game_hour_raw": scope.game_hour_raw,
                "first_qpc_ticks": scope.first_qpc_ticks.to_string(),
                "completed_qpc_ticks": scope.completed_qpc_ticks.to_string(),
                "qpc_frequency_hz": scope.qpc_frequency_hz.to_string(),
                "record_count": scope.records.len(),
            },
            "portable_counts": {
                "record_count": o05.records.len(),
                "no_entity_selected": disposition_counts[0],
                "selected_not_submitted": disposition_counts[1],
                "submitted": disposition_counts[2],
                "ordered_output_count": o05.ordered_outputs.len(),
            },
            "production_render_frame": frame,
        });
        let canonical = serde_json::to_vec(&body)
            .map_err(|error| format!("C07 O05 receipt serialization failed: {error}"))?;
        body.as_object_mut()
            .ok_or("C07 O05 receipt body is not an object")?
            .insert(
                "receipt_body_sha256".into(),
                serde_json::Value::String(format!("{:x}", Sha256::digest(&canonical))),
            );
        let mut encoded = serde_json::to_vec_pretty(&body)
            .map_err(|error| format!("C07 O05 receipt serialization failed: {error}"))?;
        encoded.push(b'\n');
        write_synced_new_file(
            &output_root.join("c07_o05_generic_building_observation.json"),
            &encoded,
        )
    })();
    if write_result.is_err() {
        C07_O05_RECEIPT_WRITTEN.store(false, std::sync::atomic::Ordering::Release);
    }
    write_result
}

fn canonical_json_sha256<T: serde::Serialize>(value: &T) -> Result<String, String> {
    let bytes = serde_json::to_vec(value).map_err(|error| error.to_string())?;
    Ok(format!("{:x}", Sha256::digest(bytes)))
}

// Named refusal carried onto the wire through `d77_d80_refusal` (and attributed
// by the so_d77_d80_hourly_input_gap status-origin sample) when an hour observed
// GUI paints but its selected Present minted no hourly recorded-input
// observation — such an hour cannot prove interactivity, so it degrades named
// instead of shipping an unbound pass.
pub(crate) const D77_D80_HOURLY_INPUT_REFUSAL_REASON: &str =
    "selected-Present GUI paints observed with no hourly recorded-input observation";

#[cfg(windows)]
pub fn drain_a729_runtime_captures_in_process(
    admission: &InProcessRenderAdmission,
    recorded_input: Option<&production_render_catalog_live::AuthenticatedRecordedInputVariant>,
) -> Result<Vec<FinalizedA729RuntimeCapture>, production_render_submit_live_lane::PromotionError> {
    let mut captures =
        production_render_submit_live_lane::a729_hook::drain_complete_submit_captures()
            .map_err(production_render_submit_live_lane::PromotionError::Invalid)?;
    finalize_a729_runtime_captures_in_process(admission, recorded_input, &mut captures)
}

#[cfg(windows)]
fn finalize_a729_runtime_captures_in_process(
    admission: &InProcessRenderAdmission,
    recorded_input: Option<&production_render_catalog_live::AuthenticatedRecordedInputVariant>,
    captures: &mut Vec<production_render_submit_live_lane::RawRenderSubmitCapture>,
) -> Result<Vec<FinalizedA729RuntimeCapture>, production_render_submit_live_lane::PromotionError> {
    for capture in captures.iter() {
        production_render_submit_live_lane::finalize_live_submit_authority(capture)
            .map_err(production_render_submit_live_lane::PromotionError::Invalid)?;
    }
    let semantics =
        production_render_submit_live_lane::drain_complete_live_render_catalog_semantics()
            .map_err(production_render_submit_live_lane::PromotionError::Invalid)?;
    if semantics.len() != captures.len() {
        return Err(production_render_submit_live_lane::PromotionError::Invalid(
            "d77 raw/semantic completed queues differ in cardinality".into(),
        ));
    }
    let mut semantics_by_submit = semantics
        .into_iter()
        .map(|row| (row.submit_index(), row))
        .collect::<BTreeMap<_, _>>();
    let mut output = Vec::with_capacity(captures.len());
    for capture in captures.iter_mut() {
        let submit_index = capture.submit.submit_index;
        let semantics = semantics_by_submit.remove(&submit_index).ok_or_else(|| {
            production_render_submit_live_lane::PromotionError::Invalid(
                "d77 raw capture has no exact finalized semantic adapter".into(),
            )
        })?;
        production_render_submit_live_lane::materialize_live_render_catalog_semantics(
            capture, &semantics,
        )?;
        let strict_manifest = admission
            .manifest_for_observed_catalogs([&capture.catalog])
            .map_err(production_render_submit_live_lane::PromotionError::Invalid)?;
        let catalog = production_render_catalog_live::build_authenticated_render_catalog(
            &strict_manifest,
            &capture.submit,
            &capture.catalog,
            admission,
        )
        .map_err(|error| {
            production_render_submit_live_lane::PromotionError::Invalid(format!(
                "live d77 catalog: {error}"
            ))
        })?;
        let particles =
            production_render_submit_live_lane::promote_submit(&capture.submit, &catalog)?;
        let entities = capture
            .submit
            .entity_submissions
            .iter()
            .map(|raw| production_render_submit_live_lane::promote_entity_submission(raw, &catalog))
            .collect::<Result<Vec<_>, _>>()?;
        let mut ordered_draws =
            particles
                .iter()
                .map(|packet| portable_render_stream::RenderOutputRef::Particle {
                    effect_instance_id: packet.effect_instance_id,
                    bucket: packet.bucket,
                    packet_ordinal: packet.packet_ordinal,
                    draw_order: packet.draw_order,
                })
                .chain(entities.iter().map(|draw| {
                    portable_render_stream::RenderOutputRef::Entity {
                        pass_ordinal: draw.pass_ordinal,
                        entity_instance_id: draw.entity_instance_id,
                        draw_order: draw.draw_order,
                    }
                }))
                .collect::<Vec<_>>();
        ordered_draws.sort_by_key(|draw| match draw {
            portable_render_stream::RenderOutputRef::Particle { draw_order, .. }
            | portable_render_stream::RenderOutputRef::Entity { draw_order, .. } => *draw_order,
        });
        if ordered_draws.iter().enumerate().any(|(expected, draw)| {
            let actual = match draw {
                portable_render_stream::RenderOutputRef::Particle { draw_order, .. }
                | portable_render_stream::RenderOutputRef::Entity { draw_order, .. } => draw_order,
            };
            usize::try_from(*actual).ok() != Some(expected)
        }) {
            return Err(production_render_submit_live_lane::PromotionError::Invalid(
                "a729 final-draw order is not contiguous".into(),
            ));
        }
        let authority = production_render_catalog_live::portable_render_authority_with_input(
            &capture.catalog,
            admission.manifest().effective_vfs_manifest_sha256.clone(),
            admission.catalog_authority_sha256().to_owned(),
            admission.catalog_authority_sha256().to_owned(),
            recorded_input,
        )
        .map_err(|error| {
            production_render_submit_live_lane::PromotionError::Invalid(format!(
                "live d77 authority: {error}"
            ))
        })?;
        output.push(FinalizedA729RuntimeCapture {
            promoted: PromotedA729ProductionRenderSubmit {
                submit_index,
                camera_carrier_ordinal: capture.submit.camera_carrier_ordinal,
                camera_carrier: promote_a729_camera_carrier(
                    capture.submit.camera_carrier.as_ref(),
                )?,
                particles,
                entities,
                ordered_draws,
                s1_shadow: None,
            },
            raw: capture.clone(),
            authority,
        });
    }
    if !semantics_by_submit.is_empty() {
        return Err(production_render_submit_live_lane::PromotionError::Invalid(
            "d77 semantic queue contains unmatched submit adapters".into(),
        ));
    }
    Ok(output)
}

#[cfg(windows)]
pub fn drain_finalize_a729_production_frames(
    _admission: &mut InProcessRenderAdmission,
    _update_index: u64,
) -> Result<Vec<portable_render_stream::ProductionRenderFrame>, String> {
    Err("production render finalization requires an exact selected-Present receipt".into())
}

#[cfg(windows)]
#[derive(Debug)]
enum A729ProductionRenderDrain {
    EmptyNoCapture,
    PendingIncomplete,
    Ready(Vec<portable_render_stream::ProductionRenderFrame>),
}

#[cfg(windows)]
pub fn prepare_a729_production_weather_catalog_with_payload_loader<F>(
    admission: &mut InProcessRenderAdmission,
    payload_loader: &mut F,
) -> Result<production_render_submit_live_lane::AuthenticatedWeatherCatalog, String>
where
    F: FnMut(
        &mut InProcessRenderAdmission,
        Vec<ObservedVfsPayloadRangeRequest>,
    ) -> Result<(), String>,
{
    let discovery = production_render_submit_live_lane::AuthenticatedWeatherCatalog::
        discovery_payload_requests(admission.winner_index())?;
    let missing = admission.missing_weather_catalog_payloads(discovery)?;
    payload_loader(admission, missing)?;

    let exact =
        production_render_submit_live_lane::AuthenticatedWeatherCatalog::exact_payload_requests(
            admission.winner_index(),
            admission,
        )?;
    let missing = admission.missing_weather_catalog_payloads(exact)?;
    payload_loader(admission, missing)?;

    production_render_submit_live_lane::AuthenticatedWeatherCatalog::from_admitted_payloads(
        admission.winner_index(),
        admission,
    )
}

#[cfg(windows)]
fn drain_finalize_a729_production_frames_with_payload_loader<F>(
    admission: &mut InProcessRenderAdmission,
    update_index: u64,
    selected_capture: production_render_submit_live_lane::a729_hook::RawSelectedRenderFrameCapture,
    gui_draws: production_render_submit_live_lane::a729_hook::SelectedPresentGuiDraws,
    owner_captures: Vec<
        runtime_text_inputs_live_lane::owner_glyph_probe::SelectedOwnerGlyphCapture,
    >,
    hour_recorded_input: Option<production_render_catalog_live::AuthenticatedRecordedInputVariant>,
    payload_loader: &mut F,
) -> Result<A729ProductionRenderDrain, String>
where
    F: FnMut(
        &mut InProcessRenderAdmission,
        Vec<ObservedVfsPayloadRangeRequest>,
    ) -> Result<(), String>,
{
    let ui_selected_capture = selected_capture.clone();
    let weather = &selected_capture.selected.frame;
    production_render_submit_live_lane::a729_hook::validate_selected_render_frame_receipt(
        &selected_capture,
    )?;
    if weather.expected_weather_region_count == 0
        || weather.observed_weather_region_count != weather.expected_weather_region_count
        || weather.observed_weather_region_count
            != u64::try_from(weather.weather_membership_updates.len())
                .map_err(|_| "selected-Present weather membership count exceeds u64")?
        || weather.active_airborne_weather_region_count
            != u64::try_from(weather.weather_visibility_updates.len())
                .map_err(|_| "selected-Present weather visibility count exceeds u64")?
    {
        return Err(
            "selected-Present weather snapshot cardinality differs from its discriminator".into(),
        );
    }
    let weather_catalog = if weather.weather_membership_updates.is_empty() {
        None
    } else {
        Some(prepare_a729_production_weather_catalog_with_payload_loader(
            admission,
            payload_loader,
        )?)
    };
    let selected_ordinal = selected_capture
        .receipt
        .candidate_count
        .checked_sub(1)
        .ok_or("selected Present receipt has zero candidates")?;
    let selected_present = portable_render_stream::SelectedPresentReceipt::sealed(
        selected_capture.receipt.candidate_count,
        selected_capture.receipt.candidate_set_sha256.clone(),
        selected_capture.receipt.selected_submit_index,
        selected_ordinal,
        selected_capture.receipt.target_game_hour_raw,
        selected_capture.receipt.selected_presentation_qpc_ticks,
        selected_capture.receipt.cutoff_qpc_ticks,
    );
    let o05_runtime_scope =
        production_render_submit_live_lane::a729_hook::take_o05_scope_for_selected_present(
            selected_capture.receipt.target_game_hour_raw,
            selected_capture.receipt.selected_presentation_qpc_ticks,
            selected_capture
                .selected
                .frame
                .presentation_qpc_frequency_hz,
        )?;
    let joined = vec![selected_capture.selected];
    let mut gui_exact_draws = BTreeMap::new();
    let mut raw_frames = Vec::with_capacity(joined.len());
    let mut raw_runtime = Vec::with_capacity(joined.len());
    let mut s1_execution_by_submit = BTreeMap::new();
    for row in joined {
        let submit_index = row.frame.submit_index;
        if s1_execution_by_submit
            .insert(submit_index, row.s1_execution)
            .is_some()
        {
            return Err("selected render queue repeats an S1 submit identity".into());
        }
        raw_frames.push(row.frame);
        raw_runtime.push(row.runtime);
    }
    let missing =
        admission.missing_observed_payloads(raw_runtime.iter().map(|capture| &capture.catalog))?;
    payload_loader(admission, missing)?;
    let mut runtime =
        finalize_a729_runtime_captures_in_process(admission, hour_recorded_input.as_ref(), &mut raw_runtime)
        .map_err(|error| format!("d77 runtime strict finalization: {error:?}"))?
        .into_iter()
        .map(|row| (row.promoted.submit_index, row))
        .collect::<BTreeMap<_, _>>();
    if raw_frames.len() != runtime.len() {
        return Err("render frame/runtime submit queue cardinality differs".into());
    }
    for frame in &raw_frames {
        let runtime_row = runtime
            .get(&frame.submit_index)
            .ok_or("render frame names an absent runtime submit")?;
        let frame_update_index =
            compact_contract::calendar::hour_index_from_raw(frame.game_hour_raw)
                .and_then(|value| u64::try_from(value).ok())
                .ok_or("selected render frame has a noncanonical game hour")?;
        if frame_update_index != update_index
            || frame.process_start_nonce != admission.process_start_nonce()
            || runtime_row.raw.catalog.process.process_start_nonce
                != admission.process_start_nonce()
            || frame.hook_epoch_nonce != runtime_row.authority.hook_epoch_nonce
            || frame.presentation_qpc_ticks == 0
            || frame.presentation_qpc_frequency_hz == 0
        {
            return Err("render frame/runtime submit uses another process admission".into());
        }
        let mut draws = frame
            .static_d3d_captures
            .iter()
            .map(|capture| capture.state.clone())
            .collect::<Vec<_>>();
        draws.extend(
            gui_draws
                .captures()
                .iter()
                .flat_map(|draw| draw.static_d3d_captures.iter())
                .map(|capture| capture.state.clone()),
        );
        draws.sort_by_key(|draw| draw.capture_sequence);
        if draws
            .windows(2)
            .any(|rows| rows[0].capture_sequence >= rows[1].capture_sequence)
        {
            return Err("selected map/UI static draw sequence is repeated or reordered".into());
        }
        if !draws.is_empty() {
            production_render_submit_live_lane::finalize_static_authority(
                frame.frame_sequence,
                admission.process_start_nonce(),
                &draws,
            )?;
        }
    }
    let mut static_authorities =
        production_render_submit_live_lane::drain_complete_static_authorities()?
            .into_iter()
            .map(|authority| (authority.frame_sequence(), authority))
            .collect::<BTreeMap<_, _>>();
    let mut pending = Vec::with_capacity(raw_frames.len());
    for frame in raw_frames {
        production_render_submit_live_lane::validate_render_order_events(&frame.events)
            .map_err(|error| format!("render frame order: {error:?}"))?;
        let runtime_row = runtime
            .remove(&frame.submit_index)
            .ok_or("render frame/runtime submit pairing was repeated")?;
        let (mut portable_submit, s1_shadow) =
            portable_submit_with_pending_s1_from_a729_runtime(runtime_row.promoted, frame.global_order)?;
        let s1_execution = s1_execution_by_submit
            .remove(&frame.submit_index)
            .ok_or("render frame has no exact S1 execution disposition")?;
        if s1_shadow.is_some() && s1_execution.is_some() {
            return Err("selected submit carries both legacy and exact S1 producers".into());
        }
        portable_submit.rail_construction_arrows = frame
            .rail_construction_arrows
            .iter()
            .map(|row| {
                let entity_registration_token = row
                    .entity_registration_token
                    .clone()
                    .filter(|token| !token.is_empty())
                    .ok_or_else(|| {
                        "rail construction-arrow entity has no portable registration token"
                            .to_string()
                    })?;
                if !portable_submit.entities.iter().any(|entity| {
                    entity.entity_instance_id == row.entity_instance_id
                        && entity
                            .content_identity
                            .native_selection
                            .entity_registration_token
                            == entity_registration_token
                }) {
                    return Err(
                        "rail construction-arrow has no exact promoted entity/content join"
                            .to_string(),
                    );
                }
                Ok(portable_render_stream::RailConstructionArrowVisual {
                    producer_rva: row.producer_rva,
                    province_lo: row.province_lo,
                    province_hi: row.province_hi,
                    edge_type_u8: row.edge_type_u8,
                    entity_instance_id: row.entity_instance_id,
                    entity_registration_token,
                })
            })
            .collect::<Result<Vec<_>, String>>()?;
        let gui_static_capture_count = gui_draws
            .captures()
            .iter()
            .map(|draw| draw.static_d3d_captures.len())
            .sum::<usize>();
        if frame.static_d3d_captures.is_empty() && gui_static_capture_count == 0 {
            if static_authorities.remove(&frame.frame_sequence).is_some() {
                return Err("static authority exists for a frame with no static draws".into());
            }
            let exact_s1 = s1_execution
                .as_ref()
                .map(|execution| build_a729_s1_shadow_carrier(&frame, execution, &portable_submit))
                .transpose()?;
            if exact_s1
                .as_ref()
                .is_some_and(|carrier| !carrier.is_inactive())
            {
                return Err("active exact S1 submit has no d80 operation stream".into());
            }
            if let Some(carrier) = exact_s1.or(s1_shadow) {
                fold_a729_s1_shadow_carrier(&mut portable_submit, carrier)
                    .map_err(s1_fold_error_message)?;
            }
            if let Some(catalog) = weather_catalog.as_ref() {
                fold_a729_weather_snapshot(&frame, &mut portable_submit, catalog)?;
            }
            pending.push((frame, portable_submit, runtime_row.authority));
            continue;
        }

        let authority = static_authorities
            .remove(&frame.frame_sequence)
            .ok_or("static frame has no finalized semantic authority")?;
        let mut catalog = runtime_row.raw.catalog.clone();
        catalog.submit_index = frame.submit_index;
        catalog.frame_sequence = frame.frame_sequence;
        merge_static_catalog_fragment(&mut catalog, &frame.static_catalog_fragment);
        let mut sink = production_render_submit_live_lane::ProductionStaticProgramCaptureSink::new(
            catalog.process.clone(),
            catalog.launch_epoch.clone(),
            frame.submit_index,
            frame.frame_sequence,
        )?;
        authority.apply_to(sink.authority_mut())?;
        let mut cores = frame
            .static_d3d_captures
            .iter()
            .map(|capture| (capture.state.capture_sequence, capture.state.clone()))
            .collect::<BTreeMap<_, _>>();
        if cores.len() != frame.static_d3d_captures.len() {
            return Err("static frame repeats a D3D capture sequence".into());
        }
        let mut gui_cores = gui_draws
            .captures()
            .iter()
            .flat_map(|draw| draw.static_d3d_captures.iter())
            .map(|capture| (capture.state.capture_sequence, capture.state.clone()))
            .collect::<BTreeMap<_, _>>();
        if gui_cores.len() != gui_static_capture_count
            || gui_cores
                .keys()
                .any(|capture_sequence| cores.contains_key(capture_sequence))
        {
            return Err(
                "selected GUI static D3D capture sequence is repeated or map-ambiguous".into(),
            );
        }
        let mut static_local_sequence = 1u64;
        for event in &frame.events {
            match event {
                production_render_submit_live_lane::RawRenderOrderEvent::FinalDraw {
                    sequence,
                    family: production_render_submit_live_lane::FinalDrawFamily::StaticOrPostProcess,
                    ..
                } => {
                    let core = cores
                        .remove(sequence)
                        .ok_or("static final draw has no exact D3D core")?;
                    production_render_submit_live_lane::RawStaticProgramCaptureSink::push_static_draw(
                        &mut sink,
                        core,
                    )?;
                    sink.push_order_operation(
                        production_render_submit_live_lane::RawStaticOrderOperation::Draw {
                            sequence: static_local_sequence,
                            capture_sequence: *sequence,
                        },
                    )?;
                    static_local_sequence = static_local_sequence.saturating_add(1);
                }
                production_render_submit_live_lane::RawRenderOrderEvent::ClearColor {
                    capture_sequence,
                    render_target_object_ptr,
                    render_target_view_ptr,
                    rgba_f32_bits,
                    ..
                } => {
                    sink.authority_mut().observe_opaque_clear_output_target(
                        *capture_sequence,
                        *render_target_view_ptr,
                        production_render_submit_live_lane::RawOutputViewKind::Color,
                        &catalog,
                    )?;
                    sink.push_clear_color(
                        static_local_sequence,
                        *capture_sequence,
                        *render_target_object_ptr,
                        *render_target_view_ptr,
                        *rgba_f32_bits,
                    )?;
                    static_local_sequence = static_local_sequence.saturating_add(1);
                }
                production_render_submit_live_lane::RawRenderOrderEvent::ClearDepthStencil {
                    capture_sequence,
                    render_target_object_ptr,
                    depth_stencil_view_ptr,
                    clear_depth,
                    clear_stencil,
                    depth_f32_bits,
                    stencil_u8,
                    ..
                } => {
                    sink.authority_mut().observe_opaque_clear_output_target(
                        *capture_sequence,
                        *depth_stencil_view_ptr,
                        production_render_submit_live_lane::RawOutputViewKind::DepthStencil,
                        &catalog,
                    )?;
                    sink.push_clear_depth_stencil(
                        static_local_sequence,
                        *capture_sequence,
                        *render_target_object_ptr,
                        *depth_stencil_view_ptr,
                        *clear_depth,
                        *clear_stencil,
                        *depth_f32_bits,
                        *stencil_u8,
                    )?;
                    static_local_sequence = static_local_sequence.saturating_add(1);
                }
                _ => {}
            }
        }
        if !cores.is_empty() {
            return Err("static frame contains D3D cores absent from global order".into());
        }
        let map_program_count = usize::try_from(static_local_sequence.saturating_sub(1))
            .map_err(|_| "map static program count exceeds usize")?;
        let gui_capture_sequences = gui_draws
            .captures()
            .iter()
            .flat_map(|draw| draw.static_d3d_captures.iter())
            .map(|capture| capture.state.capture_sequence)
            .collect::<Vec<_>>();
        for capture_sequence in &gui_capture_sequences {
            let core = gui_cores
                .remove(capture_sequence)
                .ok_or("selected GUI operation has no exact D3D core")?;
            production_render_submit_live_lane::RawStaticProgramCaptureSink::push_static_draw(
                &mut sink, core,
            )?;
            sink.push_order_operation(
                production_render_submit_live_lane::RawStaticOrderOperation::Draw {
                    sequence: static_local_sequence,
                    capture_sequence: *capture_sequence,
                },
            )?;
            static_local_sequence = static_local_sequence.saturating_add(1);
        }
        if !gui_cores.is_empty() {
            return Err("selected GUI static D3D core is absent from UI operation order".into());
        }
        let strict_static_manifest = admission.manifest_for_observed_catalogs([&catalog])?;
        let missing_static = admission.missing_observed_payloads([&catalog])?;
        payload_loader(admission, missing_static)?;
        production_render_submit_live_lane::commit_static_program_capture(sink, catalog)?;
        let promoted = drain_a729_static_program_captures(
            &strict_static_manifest,
            admission,
            admission.catalog_authority_sha256(),
            admission.catalog_authority_sha256(),
        )
        .map_err(|error| format!("d80 strict finalization: {error:?}"))?;
        if promoted.len() != 1
            || promoted[0].submit_index != frame.submit_index
            || promoted[0].frame_sequence != frame.frame_sequence
        {
            return Err("d80 completed queue does not match the current render frame".into());
        }
        let mut promoted = promoted.into_iter().next().expect("length checked");
        if promoted.ordered_programs.len()
            != map_program_count
                .checked_add(gui_capture_sequences.len())
                .ok_or("selected map/UI program count overflow")?
            || promoted.ordered_static_execution.len() != promoted.ordered_programs.len()
        {
            return Err(
                "promoted map/UI static program partition has the wrong cardinality".into(),
            );
        }
        let gui_programs = promoted.ordered_programs.split_off(map_program_count);
        promoted
            .ordered_static_execution
            .truncate(map_program_count);
        for (capture_sequence, program) in gui_capture_sequences.iter().copied().zip(gui_programs) {
            let [operation] = &program.ordered_operations[..] else {
                return Err("selected GUI static program does not contain one exact draw".into());
            };
            let portable_render_stream::StaticMapCommand::Draw { draw } = &operation.command else {
                return Err("selected GUI static program materialized a non-draw operation".into());
            };
            if draw.ordered_shader_resources.iter().any(|binding| {
                !matches!(
                    &binding.source,
                    portable_render_stream::StaticShaderResourceSource::UsedContent { .. }
                )
            }) {
                return Err(
                    "selected GUI exact draw depends on a non-VFS resource unavailable to UI replay"
                        .into(),
                );
            }
            if gui_exact_draws
                .insert(capture_sequence, draw.clone())
                .is_some()
            {
                return Err("selected GUI promoted draw sequence is duplicated".into());
            }
        }
        let unified = exact_unified_order(
            &frame.events,
            &frame.runtime_render_order_joins,
            &runtime_row.raw,
            &promoted,
        )?;
        attach_selected_occupation_secondary_texture(
            &mut portable_submit,
            &promoted,
            &frame,
            &selected_present,
        )?;
        fold_a729_static_program_capture(&mut portable_submit, promoted, unified)
            .map_err(|error| format!("d80 fold: {error:?}"))?;
        let exact_s1 = s1_execution
            .as_ref()
            .map(|execution| build_a729_s1_shadow_carrier(&frame, execution, &portable_submit))
            .transpose()?;
        if let Some(carrier) = exact_s1.or(s1_shadow) {
            fold_a729_s1_shadow_carrier(&mut portable_submit, carrier)
                .map_err(s1_fold_error_message)?;
        }
        if let Some(catalog) = weather_catalog.as_ref() {
            fold_a729_weather_snapshot(&frame, &mut portable_submit, catalog)?;
        }
        pending.push((frame, portable_submit, runtime_row.authority));
    }
    if !runtime.is_empty() || !static_authorities.is_empty() || !s1_execution_by_submit.is_empty() {
        return Err("render drain left unmatched runtime/static authority rows".into());
    }
    if pending.is_empty() {
        return Ok(A729ProductionRenderDrain::EmptyNoCapture);
    }
    if pending.len() != 1 {
        return Err("production render carrier retained more than selected Present".into());
    }
    let authenticated_ui_pass = match gui_draws.is_observed() {
        false => None,
        true => {
            // The consumer law binds any authenticated UI pass to a recorded-input
            // observation of THIS frame's selected Present. An hour whose Present
            // carries no fresh pointer-movement observation cannot prove interactivity,
            // so materializing a pass there would be unfalsifiable: refuse the hour
            // by name (the disposition degrades it, named on the wire) instead of
            // shipping an unbound pass.
            let hour_input = hour_recorded_input.as_ref().ok_or(
                crate::D77_D80_HOURLY_INPUT_REFUSAL_REASON,
            )?;
            Some(
                production_render_submit_live_lane::a25_runtime_text_present::
                    materialize_direct_selected_present_ui_pass(
                        &ui_selected_capture,
                        admission.winner_index(),
                        gui_draws.captures(),
                        owner_captures,
                        gui_exact_draws,
                        update_index,
                        hour_input.observation_receipt_sha256.clone(),
                        None,
                    )?,
            )
        }
    };
    let rail_supply_arrows = (|| -> Result<_, String> {
        let frame = &pending[0].0;
        let raw = &frame.rail_supply_arrows;
        if raw.rows.is_empty() && raw.draw_joins.is_empty() {
            return Ok(None);
        }
        let used_content = vec![
            admission.admitted_used_content_for_path(
                portable_render_stream::RAIL_SUPPLY_MAPARROWS_PATH,
            )?,
            admission.admitted_used_content_for_path(
                portable_render_stream::RAIL_SUPPLY_GRAPHICS_DEFINES_PATH,
            )?,
        ];
        let render_first = frame
            .events
            .first()
            .map(production_render_submit_live_lane::RawRenderOrderEvent::sequence)
            .ok_or("rail supply-arrow selected frame has no render events")?;
        let first_sequence = frame
            .weather_membership_updates
            .iter()
            .map(|row| row.sequence)
            .chain(
                frame
                    .weather_visibility_updates
                    .iter()
                    .map(|row| row.sequence),
            )
            .fold(render_first, u64::min);
        let last_sequence = frame
            .events
            .last()
            .map(production_render_submit_live_lane::RawRenderOrderEvent::sequence)
            .ok_or("rail supply-arrow selected frame has no render events")?;
        promote_a729_rail_supply_arrow_frame(
            raw,
            frame.frame_sequence,
            frame.submit_index,
            update_index,
            first_sequence,
            last_sequence,
            &frame.events,
            &pending[0].1.ordered_static_map_programs,
            used_content,
        )
    })()
    .unwrap_or(None);
    let selected_present_sha256 = selected_present
        .computed_sha256()
        .map_err(|error| format!("SnowMudData selected-Present receipt: {error}"))?;
    let snow_mud_fade_receipt = pending[0]
        .0
        .snow_mud_texture
        .as_ref()
        .map(|_| {
            portable_render_stream::selected_present_snow_mud_fade_receipt(
                &pending[0].1,
                &selected_present_sha256,
            )
        })
        .transpose()
        .map_err(|error| format!("SnowMudData selected-Present fade: {error}"))?;
    let snow_mud_texture = pending[0]
        .0
        .snow_mud_texture
        .as_deref()
        .map(|raw| {
            production_render_submit_live_lane::promote_snow_mud_texture_observation(
                raw,
                &selected_present,
                pending[0].0.presentation_qpc_frequency_hz,
                pending[0].0.process_start_nonce,
                pending[0].0.hook_epoch_nonce,
                snow_mud_fade_receipt
                    .as_ref()
                    .expect("receipt exists when raw observation exists"),
            )
        })
        .transpose()
        .map_err(|error| format!("SnowMudData strict promotion: {error:?}"))?;
    let authority = pending[0].2.clone();
    let frequency_hz = pending[0].0.presentation_qpc_frequency_hz;
    let mut first_sequence = None;
    let mut last_sequence = None;
    let mut prior_frame_sequence = None;
    let mut prior_global_order = None;
    let mut prior_presentation_qpc = None;
    let mut submits = Vec::with_capacity(pending.len());
    for (frame, submit, frame_authority) in pending {
        let render_first = frame
            .events
            .first()
            .map(production_render_submit_live_lane::RawRenderOrderEvent::sequence)
            .ok_or("render frame event stream is empty")?;
        let frame_first = frame
            .weather_membership_updates
            .iter()
            .map(|row| row.sequence)
            .chain(
                frame
                    .weather_visibility_updates
                    .iter()
                    .map(|row| row.sequence),
            )
            .fold(render_first, u64::min);
        let frame_last = frame
            .events
            .last()
            .map(production_render_submit_live_lane::RawRenderOrderEvent::sequence)
            .ok_or("render frame event stream is empty")?;
        if frame_authority != authority
            || frame.presentation_qpc_frequency_hz != frequency_hz
            || prior_frame_sequence.is_some_and(|prior| frame.frame_sequence <= prior)
            || last_sequence.is_some_and(|prior| frame_first <= prior)
            || prior_global_order
                .is_some_and(|prior: u32| submit.global_order != prior.saturating_add(1))
            || prior_presentation_qpc.is_some_and(|prior| frame.presentation_qpc_ticks <= prior)
        {
            return Err(
                "hourly render aggregation crosses authority/frequency/order epochs".into(),
            );
        }
        first_sequence.get_or_insert(frame_first);
        last_sequence = Some(frame_last);
        prior_frame_sequence = Some(frame.frame_sequence);
        prior_global_order = Some(submit.global_order);
        prior_presentation_qpc = Some(frame.presentation_qpc_ticks);
        submits.push(submit);
    }
    let first_sequence = first_sequence.expect("nonempty pending");
    let last_sequence = last_sequence.expect("nonempty pending");
    let mut frame = assemble_production_render_frame(
        authority,
        selected_present,
        snow_mud_texture,
        update_index,
        first_sequence,
        last_sequence,
        frequency_hz,
        submits,
        rail_supply_arrows,
        authenticated_ui_pass,
    )?;
    if let Some(scope) = o05_runtime_scope {
        if scope.game_hour_raw != selected_capture.receipt.target_game_hour_raw
            || scope.qpc_frequency_hz != frequency_hz
            || scope.completed_qpc_ticks > selected_capture.receipt.selected_presentation_qpc_ticks
        {
            return Err("O05 runtime scope crossed the selected-Present authority".into());
        }
        let records = scope
            .records
            .iter()
            .enumerate()
            .map(|(ordinal, observation)| {
                let update_ordinal = u32::try_from(ordinal).map_err(|_| {
                    production_render_submit_live_lane::PromotionError::Invalid(
                        "O05 update ordinal exceeds u32".into(),
                    )
                })?;
                production_render_submit_live_lane::RawO05BuildingRecord::decode_a729_post_update(
                    last_sequence,
                    observation.runtime_object_id,
                    update_ordinal,
                    &observation.record_bytes,
                    observation.selected_entity_instance_id,
                    observation.world_transform_4x4_f32_bits,
                )
            })
            .collect::<Result<Vec<_>, _>>()
            .map_err(|error| format!("O05 selected runtime scope: {error:?}"))?;
        let raw = production_render_submit_live_lane::RawO05BuildingFrame {
            map_buildings_source: admission.admitted_used_content_for_path("map/buildings.txt")?,
            update_index,
            first_sequence,
            last_sequence,
            complete_selected_present_scope: true,
            records,
        };
        attach_o05_building_frame(&mut frame, &raw)
            .map_err(|error| format!("O05 selected frame attachment: {error:?}"))?;
        write_c07_o05_acquisition_receipt_if_requested(&frame, &scope)?;
    }
    Ok(A729ProductionRenderDrain::Ready(vec![frame]))
}

#[cfg(windows)]
fn attach_selected_occupation_secondary_texture(
    submit: &mut portable_render_stream::RenderSubmit,
    promoted: &PromotedA729StaticProgramCapture,
    frame: &production_render_submit_live_lane::a729_hook::RawRenderFrameCapture,
    selected_present: &portable_render_stream::SelectedPresentReceipt,
) -> Result<(), String> {
    let mut identity: Option<(u32, String)> = None;
    let mut terrain_t6_consumer_count = 0u32;
    let mut water_t3_consumer_count = 0u32;
    for binding in promoted
        .ordered_programs
        .iter()
        .flat_map(|program| program.ordered_operations.iter())
        .filter_map(|operation| match &operation.command {
            portable_render_stream::StaticMapCommand::Draw { draw } => Some(draw),
            portable_render_stream::StaticMapCommand::ClearColor { .. }
            | portable_render_stream::StaticMapCommand::ClearDepthStencil { .. } => None,
        })
        .flat_map(|draw| draw.ordered_shader_resources.iter())
    {
        let portable_render_stream::StaticShaderResourceSource::PoliticalTexture {
            texture_kind,
            target_binding_identity,
            upload_order,
            payload_sha256,
            ..
        } = &binding.source
        else {
            continue;
        };
        if *texture_kind != portable_render_stream::PoliticalTextureKind::Secondary
            || target_binding_identity
                != production_render_submit_live_lane::OCCUPATION_SECONDARY_TARGET_BINDING
            || binding.semantic_token
                != production_render_submit_live_lane::OCCUPATION_SECONDARY_TARGET_BINDING
            || binding.stage != portable_render_stream::ShaderStage::Pixel
        {
            return Err(
                "selected-submit political source is not the exact reflected occupation secondary identity"
                    .into(),
            );
        }
        let current = (*upload_order, payload_sha256.clone());
        if identity
            .as_ref()
            .is_some_and(|expected| expected != &current)
        {
            return Err(
                "selected-submit occupation consumers disagree on upload order/payload".into(),
            );
        }
        identity.get_or_insert(current);
        match binding.slot {
            6 => {
                terrain_t6_consumer_count = terrain_t6_consumer_count
                    .checked_add(1)
                    .ok_or("occupation terrain-t6 consumer count overflow")?;
            }
            3 => {
                water_t3_consumer_count = water_t3_consumer_count
                    .checked_add(1)
                    .ok_or("occupation water-t3 consumer count overflow")?;
            }
            _ => {
                return Err(
                    "occupation secondary source appears outside reflected terrain-t6/water-t3"
                        .into(),
                )
            }
        }
    }
    let Some((upload_order, payload_sha256)) = identity else {
        return Ok(());
    };
    if terrain_t6_consumer_count == 0 || water_t3_consumer_count == 0 {
        return Err(
            "occupation secondary source lacks complete terrain-t6/water-t3 topology".into(),
        );
    }
    let matches = frame
        .occupation_secondary_textures
        .iter()
        .filter(|raw| raw.upload_order == upload_order && raw.payload_sha256 == payload_sha256)
        .collect::<Vec<_>>();
    let [raw] = &matches[..] else {
        return Err(
            "occupation secondary selected-submit source lacks one unique CPU upload observation"
                .into(),
        );
    };
    let packet =
        production_render_submit_live_lane::promote_occupation_secondary_texture_observation(
            (*raw).as_ref(),
            selected_present,
            frame.presentation_qpc_frequency_hz,
            frame.process_start_nonce,
            frame.hook_epoch_nonce,
            terrain_t6_consumer_count,
            water_t3_consumer_count,
        )
        .map_err(|error| format!("occupation secondary strict promotion: {error:?}"))?;
    if !submit.political_textures.is_empty() {
        return Err("selected submit already contains political texture packets".into());
    }
    submit.political_textures.push(packet);
    Ok(())
}

#[cfg(windows)]
fn fold_a729_weather_snapshot(
    frame: &production_render_submit_live_lane::a729_hook::RawRenderFrameCapture,
    submit: &mut portable_render_stream::RenderSubmit,
    catalog: &production_render_submit_live_lane::AuthenticatedWeatherCatalog,
) -> Result<(), String> {
    if !submit.weather_membership.is_empty() {
        return Err("selected submit already contains weather membership".into());
    }
    let mut visibility_by_region = frame
        .weather_visibility_updates
        .iter()
        .map(|row| (row.strategic_region_id, row))
        .collect::<BTreeMap<_, _>>();
    if visibility_by_region.len() != frame.weather_visibility_updates.len() {
        return Err("selected-Present weather visibility repeats a region".into());
    }
    let mut visual_epoch_by_region = frame
        .weather_visual_epochs
        .iter()
        .map(|epoch| (epoch.strategic_region_id, epoch))
        .collect::<BTreeMap<_, _>>();
    if visual_epoch_by_region.len() != frame.weather_visual_epochs.len() {
        return Err("selected-Present weather visual epochs repeat a region".into());
    }
    let mut prior_region = None;
    for raw in &frame.weather_membership_updates {
        if prior_region.is_some_and(|prior| raw.strategic_region_id <= prior) {
            return Err("selected-Present weather membership is not ordered by region".into());
        }
        prior_region = Some(raw.strategic_region_id);
        let mut membership = production_render_submit_live_lane::promote_weather_observation(
            raw,
            raw.sequence,
            raw.update_index,
            catalog,
        )
        .map_err(|error| format!("XN14 weather membership promotion: {error:?}"))?;
        if let Some(epoch) = visual_epoch_by_region.remove(&raw.strategic_region_id) {
            if epoch.airborne_runtime_code != raw.airborne_runtime_code || epoch.ordinal == 0 {
                return Err("weather visual epoch stamp disagrees with its membership row".into());
            }
            membership.visual_epoch = Some(portable_render_stream::WeatherVisualEpochStamp {
                ordinal: epoch.ordinal,
                game_hour_raw: epoch.game_hour_raw,
            });
        }
        if raw.airborne_runtime_code == 0 {
            if visibility_by_region
                .remove(&raw.strategic_region_id)
                .is_some()
            {
                return Err("code-zero weather region carries a visibility update".into());
            }
        } else {
            let visibility = visibility_by_region
                .remove(&raw.strategic_region_id)
                .ok_or("active weather region has no selected-Present visibility row")?;
            let phase = production_render_submit_live_lane::promote_weather_visibility_observation(
                visibility,
                visibility.sequence,
                visibility.update_index,
            )
            .map_err(|error| format!("XN14 weather visibility promotion: {error:?}"))?;
            production_render_submit_live_lane::append_weather_visibility_phase(
                &mut membership,
                phase,
            )
            .map_err(|error| format!("XN14 weather visibility fold: {error:?}"))?;
        }
        production_render_submit_live_lane::join_weather_membership_to_submit_outputs(
            &mut membership,
            submit,
        )
        .map_err(|error| format!("XN14 weather selected-submit join: {error:?}"))?;
        submit.weather_membership.push(membership);
    }
    if !visibility_by_region.is_empty() {
        return Err("selected-Present weather visibility has no membership epoch".into());
    }
    if !visual_epoch_by_region.is_empty() {
        return Err("weather visual epoch stamp has no membership row".into());
    }
    Ok(())
}

fn merge_static_catalog_fragment(
    catalog: &mut production_render_submit_live_lane::RawRenderCatalogCapture,
    fragment: &production_render_submit_live_lane::a729_hook::RawStaticCatalogFragment,
) {
    extend_exact_unique(&mut catalog.shader_creations, &fragment.shader_creations);
    extend_exact_unique(
        &mut catalog.input_layout_creations,
        &fragment.input_layout_creations,
    );
    extend_exact_unique(
        &mut catalog.blend_state_creations,
        &fragment.blend_state_creations,
    );
    extend_exact_unique(
        &mut catalog.depth_stencil_state_creations,
        &fragment.depth_stencil_state_creations,
    );
    extend_exact_unique(
        &mut catalog.rasterizer_state_creations,
        &fragment.rasterizer_state_creations,
    );
    extend_exact_unique(
        &mut catalog.sampler_state_creations,
        &fragment.sampler_state_creations,
    );
    extend_exact_unique(&mut catalog.buffer_snapshots, &fragment.buffer_snapshots);
    extend_exact_unique(
        &mut catalog.texture_resource_snapshots,
        &fragment.texture_resource_snapshots,
    );
    extend_exact_unique(
        &mut catalog.shader_resource_view_snapshots,
        &fragment.shader_resource_view_snapshots,
    );
    extend_exact_unique(
        &mut catalog.render_target_view_snapshots,
        &fragment.render_target_view_snapshots,
    );
    extend_exact_unique(
        &mut catalog.render_target_view_snapshots,
        &fragment.clear_output_view_snapshots,
    );
}

fn extend_exact_unique<T: Clone + PartialEq>(target: &mut Vec<T>, source: &[T]) {
    for row in source {
        if !target.contains(row) {
            target.push(row.clone());
        }
    }
}

fn exact_unified_order(
    events: &[production_render_submit_live_lane::RawRenderOrderEvent],
    runtime_joins: &[production_render_submit_live_lane::a729_hook::RawRuntimeRenderOrderJoin],
    runtime: &production_render_submit_live_lane::RawRenderSubmitCapture,
    static_capture: &PromotedA729StaticProgramCapture,
) -> Result<Vec<portable_render_stream::RenderExecutionRef>, String> {
    let mut runtime_by_draw = BTreeMap::new();
    for draw in &runtime.catalog.draw_states {
        let (family, output) = match &draw.identity {
            production_render_submit_live_lane::RawDrawIdentity::Particle {
                effect_instance_id,
                draw_order,
                packet_ordinal,
                ..
            } => {
                let packet = runtime
                    .submit
                    .packets
                    .iter()
                    .find(|packet| packet.draw_order == *draw_order)
                    .ok_or("runtime particle draw identity has no packet")?;
                (
                    production_render_submit_live_lane::FinalDrawFamily::Particle,
                    portable_render_stream::RenderOutputRef::Particle {
                        effect_instance_id: *effect_instance_id,
                        bucket: ((packet.render_list_class & 0xffff) << 16)
                            | u32::from(packet.render_subbucket),
                        packet_ordinal: *packet_ordinal,
                        draw_order: *draw_order,
                    },
                )
            }
            production_render_submit_live_lane::RawDrawIdentity::Entity {
                pass_ordinal,
                entity_instance_id,
                draw_order,
                ..
            } => (
                production_render_submit_live_lane::FinalDrawFamily::Entity,
                portable_render_stream::RenderOutputRef::Entity {
                    pass_ordinal: *pass_ordinal,
                    entity_instance_id: *entity_instance_id,
                    draw_order: *draw_order,
                },
            ),
        };
        if runtime_by_draw
            .insert((family as u8, draw.draw_order), output)
            .is_some()
        {
            return Err("runtime family/draw-order identity is repeated".into());
        }
    }
    let mut runtime_by_sequence = BTreeMap::new();
    for join in runtime_joins {
        if join.submit_index != runtime.submit.submit_index
            || !matches!(
                join.family,
                production_render_submit_live_lane::FinalDrawFamily::Particle
                    | production_render_submit_live_lane::FinalDrawFamily::Entity
            )
        {
            return Err("runtime render-order join crosses submit/family identity".into());
        }
        let output = runtime_by_draw
            .remove(&(join.family as u8, join.draw_order))
            .ok_or("runtime render-order join has no exact promoted output")?;
        if runtime_by_sequence.insert(join.sequence, output).is_some() {
            return Err("runtime render-order sequence is repeated".into());
        }
    }
    if !runtime_by_draw.is_empty() {
        return Err("promoted runtime draw has no exact render-order join".into());
    }
    let static_count = static_capture.ordered_static_execution.len();
    let mut next_static = 0u32;
    let mut output = Vec::new();
    for event in events {
        match event {
            production_render_submit_live_lane::RawRenderOrderEvent::FinalDraw {
                sequence,
                family: production_render_submit_live_lane::FinalDrawFamily::StaticOrPostProcess,
                ..
            }
            | production_render_submit_live_lane::RawRenderOrderEvent::ClearColor {
                sequence,
                ..
            }
            | production_render_submit_live_lane::RawRenderOrderEvent::ClearDepthStencil {
                sequence,
                ..
            } => {
                let _ = sequence;
                output.push(portable_render_stream::RenderExecutionRef::Static {
                    operation_ordinal: next_static,
                });
                next_static = next_static.saturating_add(1);
            }
            production_render_submit_live_lane::RawRenderOrderEvent::FinalDraw {
                sequence, ..
            } => output.push(portable_render_stream::RenderExecutionRef::Runtime {
                output: runtime_by_sequence
                    .remove(sequence)
                    .ok_or("runtime final draw has no exact promoted output")?,
            }),
            production_render_submit_live_lane::RawRenderOrderEvent::Boundary { .. } => {}
        }
    }
    if next_static as usize != static_count || !runtime_by_sequence.is_empty() {
        return Err("unified order does not consume exact static/runtime subsequences".into());
    }
    Ok(output)
}

fn s1_execution_by_sequence(
    frame: &production_render_submit_live_lane::a729_hook::RawRenderFrameCapture,
    submit: &portable_render_stream::RenderSubmit,
) -> Result<BTreeMap<u64, portable_render_stream::RenderExecutionRef>, String> {
    let sequences = frame
        .events
        .iter()
        .filter(|event| {
            !matches!(
                event,
                production_render_submit_live_lane::RawRenderOrderEvent::Boundary { .. }
            )
        })
        .map(production_render_submit_live_lane::RawRenderOrderEvent::sequence)
        .collect::<Vec<_>>();
    if sequences.len() != submit.ordered_render_operations.len() {
        return Err("S1 event/unified operation cardinality differs".into());
    }
    let map = sequences
        .into_iter()
        .zip(submit.ordered_render_operations.iter().cloned())
        .collect::<BTreeMap<_, _>>();
    if map.len() != submit.ordered_render_operations.len() {
        return Err("S1 unified operation sequence repeats".into());
    }
    Ok(map)
}

fn resolve_s1_execution(
    raw: &production_render_submit_live_lane::a729_hook::RawS1DrawExecutionRef,
    frame: &production_render_submit_live_lane::a729_hook::RawRenderFrameCapture,
    by_sequence: &BTreeMap<u64, portable_render_stream::RenderExecutionRef>,
) -> Result<portable_render_stream::RenderExecutionRef, String> {
    let event = frame
        .events
        .iter()
        .find(|event| event.sequence() == raw.sequence)
        .ok_or("S1 execution names an absent render-order event")?;
    let production_render_submit_live_lane::RawRenderOrderEvent::FinalDraw { family, .. } = event
    else {
        return Err("S1 draw execution does not name a final draw".into());
    };
    if *family != raw.family {
        return Err("S1 draw execution changes final-draw family".into());
    }
    match raw.family {
        production_render_submit_live_lane::FinalDrawFamily::StaticOrPostProcess => {
            if raw.runtime_draw_order.is_some() || raw.static_capture_sequence != Some(raw.sequence)
            {
                return Err("S1 static execution carries a runtime or foreign sequence".into());
            }
        }
        production_render_submit_live_lane::FinalDrawFamily::Particle
        | production_render_submit_live_lane::FinalDrawFamily::Entity => {
            let draw_order = raw
                .runtime_draw_order
                .ok_or("S1 runtime execution has no draw order")?;
            if raw.static_capture_sequence.is_some()
                || !frame.runtime_render_order_joins.iter().any(|join| {
                    join.sequence == raw.sequence
                        && join.submit_index == frame.submit_index
                        && join.family == raw.family
                        && join.draw_order == draw_order
                })
            {
                return Err("S1 runtime execution has no exact d77 join".into());
            }
        }
    }
    by_sequence
        .get(&raw.sequence)
        .cloned()
        .ok_or("S1 execution is absent from unified render order".into())
}

fn resolve_s1_target_binding(
    registrations: &[production_render_submit_live_lane::RawRenderTargetRegistrationObservation],
    view_ptr: usize,
    resource_ptr: usize,
) -> Result<portable_render_stream::S1TargetBinding, String> {
    let matches = registrations
        .iter()
        .filter(|row| row.view_ptr == view_ptr && row.resource_ptr == resource_ptr)
        .collect::<Vec<_>>();
    let [registration] = matches.as_slice() else {
        return Err("S1 target pointer pair does not select exactly one registration".into());
    };
    let receipt = registration.materialize()?;
    let prepared_target = match receipt.target.prepared_target {
        production_render_submit_live_lane::RawPreparedTarget::SwapchainBackBuffer => {
            portable_render_stream::PreparedRenderTarget::SwapchainBackBuffer
        }
        production_render_submit_live_lane::RawPreparedTarget::OpaqueRenderTarget {
            attachment_ordinal,
        } => {
            portable_render_stream::PreparedRenderTarget::OpaqueRenderTarget { attachment_ordinal }
        }
    };
    Ok(portable_render_stream::S1TargetBinding {
        stable_resource_id: receipt.resource_identity_sha256,
        prepared_target,
        width: receipt.resource_desc.width,
        height: receipt.resource_desc.height,
        dxgi_format_u32: receipt.resource_desc.dxgi_format_u32,
        mip_levels: receipt.resource_desc.mip_levels,
        bind_flags_u32: receipt.resource_desc.bind_flags_u32,
    })
}

fn build_a729_s1_shadow_carrier(
    frame: &production_render_submit_live_lane::a729_hook::RawRenderFrameCapture,
    execution: &production_render_submit_live_lane::a729_hook::RawS1SelectedSubmitExecutionJoin,
    submit: &portable_render_stream::RenderSubmit,
) -> Result<portable_render_stream::S1ShadowCarrier, String> {
    let observation = frame
        .s1_shadow_observation
        .as_ref()
        .ok_or("exact S1 execution has no selected-submit producer observation")?;
    let mut carrier = portable_render_stream::S1ShadowCarrier {
        schema: portable_render_stream::S1_SHADOW_SCHEMA.into(),
        executable_sha256: portable_render_stream::S1_A729_EXECUTABLE_SHA256.into(),
        l02_authority_sha256: portable_render_stream::S1_L02_AUTHORITY_SHA256.into(),
        l03_authority_sha256: portable_render_stream::S1_L03_AUTHORITY_SHA256.into(),
        s1_chain_authority_sha256: portable_render_stream::S1_CHAIN_AUTHORITY_SHA256.into(),
        shadow_pass_active: execution.shadow_pass_active,
        blur_camera_factor_f32_bits: observation.blur_camera_factor_f32_bits,
        selected_light: None,
        light_view_projection_f32_bits: None,
        shadow_map_texture_matrix_f32_bits: None,
        camera_y_f32_bits: None,
        enabled_partition_mask: None,
        light_depth_target: None,
        producer_and_final_target: None,
        blur_intermediate_target: None,
        light_depth_clear: None,
        ordered_light_depth_submissions: Vec::new(),
        ordered_terrain_unlit_draws: Vec::new(),
        horizontal_blur_draw: None,
        vertical_blur_draw: None,
        ordered_shadow_consumers: Vec::new(),
        payload_sha256: String::new(),
    };
    if !execution.shadow_pass_active {
        if !execution.light_depth_clear_sequences.is_empty()
            || !execution.light_depth_caster_draws.is_empty()
            || !execution.generic_pdxmesh_casters.is_empty()
            || !execution.tree_casters.is_empty()
            || !execution.producer_and_final_draws.is_empty()
            || execution.blur_horizontal_draw.is_some()
            || execution.blur_vertical_draw.is_some()
            || !execution.final_shadow_consumers.is_empty()
        {
            return Err("inactive exact S1 execution contains active draw work".into());
        }
        carrier.payload_sha256 = carrier.computed_payload_sha256()?;
        return Ok(carrier);
    }

    let selected_light = match (
        observation.selected_direction_f32_bits == observation.primary_sun_f32_bits,
        observation.selected_direction_f32_bits == observation.primary_moon_f32_bits,
    ) {
        (true, false) => portable_render_stream::S1SelectedLight::Sun,
        (false, true) => portable_render_stream::S1SelectedLight::Moon,
        _ => return Err("active S1 selected direction is absent or ambiguous".into()),
    };
    let by_sequence = s1_execution_by_sequence(frame, submit)?;
    let [clear_sequence] = execution.light_depth_clear_sequences.as_slice() else {
        return Err("active S1 light-depth target does not have exactly one clear".into());
    };
    let clear = by_sequence
        .get(clear_sequence)
        .cloned()
        .ok_or("active S1 clear is absent from unified render order")?;
    let registrations = &frame.static_catalog_fragment.render_target_registrations;
    let light_depth_target = resolve_s1_target_binding(
        registrations,
        observation.light_depth_target_render_target_view_ptr,
        observation.light_depth_target_resource_ptr,
    )?;
    let producer_and_final_target = resolve_s1_target_binding(
        registrations,
        observation.producer_and_final_target_render_target_view_ptr,
        observation.producer_and_final_target_resource_ptr,
    )?;
    let blur_intermediate_target = resolve_s1_target_binding(
        registrations,
        observation.blur_intermediate_target_render_target_view_ptr,
        observation.blur_intermediate_target_resource_ptr,
    )?;

    let mut typed = execution
        .generic_pdxmesh_casters
        .iter()
        .map(|row| build_s1_generic_pdxmesh_submission(row, frame, submit, &by_sequence))
        .collect::<Result<Vec<_>, _>>()?;
    if execution.tree_casters.is_empty() {
        if let Some(tree) = submit.tree_runtime.as_ref() {
            typed.extend(tree.s1_primary_shadow_submissions_against_submit(submit)?);
        }
    } else {
        typed.extend(
            execution
                .tree_casters
                .iter()
                .map(|row| build_s1_dynamic_tree_submission(row, submit, &by_sequence))
                .collect::<Result<Vec<_>, _>>()?,
        );
    }
    let raw_casters = execution
        .light_depth_caster_draws
        .iter()
        .map(|row| resolve_s1_execution(row, frame, &by_sequence))
        .collect::<Result<Vec<_>, _>>()?;
    let typed_casters = typed
        .iter()
        .map(|row| row.execution().clone())
        .collect::<Vec<_>>();
    if raw_casters != typed_casters {
        return Err(
            "S1 typed caster projection does not exactly cover raw light-depth draws".into(),
        );
    }
    let enabled_partition_mask = typed.iter().fold(0u8, |mask, row| {
        mask | match row {
            portable_render_stream::S1LightDepthSubmission::Pdxmesh { partition_mask, .. }
            | portable_render_stream::S1LightDepthSubmission::Tree { partition_mask, .. } => {
                *partition_mask
            }
        }
    });
    if enabled_partition_mask == 0
        || enabled_partition_mask & !observation.camera_partition_gate_mask != 0
    {
        return Err("S1 executed partition set exceeds the camera gate".into());
    }
    let terrain = execution
        .producer_and_final_draws
        .iter()
        .map(|row| resolve_s1_execution(row, frame, &by_sequence))
        .collect::<Result<Vec<_>, _>>()?;
    let horizontal = resolve_s1_execution(
        execution
            .blur_horizontal_draw
            .as_ref()
            .ok_or("active S1 has no horizontal blur")?,
        frame,
        &by_sequence,
    )?;
    let vertical = resolve_s1_execution(
        execution
            .blur_vertical_draw
            .as_ref()
            .ok_or("active S1 has no vertical blur")?,
        frame,
        &by_sequence,
    )?;
    let consumers = execution
        .final_shadow_consumers
        .iter()
        .map(|row| resolve_s1_execution(row, frame, &by_sequence))
        .collect::<Result<Vec<_>, _>>()?;

    carrier.selected_light = Some(selected_light);
    carrier.light_view_projection_f32_bits = Some(observation.light_view_projection_f32_bits);
    carrier.shadow_map_texture_matrix_f32_bits =
        Some(observation.shadow_map_texture_matrix_f32_bits);
    carrier.camera_y_f32_bits = Some(observation.camera_y_f32_bits);
    carrier.enabled_partition_mask = Some(enabled_partition_mask);
    carrier.light_depth_target = Some(light_depth_target);
    carrier.producer_and_final_target = Some(producer_and_final_target);
    carrier.blur_intermediate_target = Some(blur_intermediate_target);
    carrier.light_depth_clear = Some(clear);
    carrier.ordered_light_depth_submissions = typed;
    carrier.ordered_terrain_unlit_draws = terrain;
    carrier.horizontal_blur_draw = Some(horizontal);
    carrier.vertical_blur_draw = Some(vertical);
    carrier.ordered_shadow_consumers = consumers;
    carrier.payload_sha256 = carrier.computed_payload_sha256()?;
    Ok(carrier)
}

fn manifest_key_for_raw(
    manifest: &portable_content_manifest::ContentManifest,
    raw: &production_render_submit_live_lane::RawVfsContentIdentity,
) -> Result<portable_content_manifest::UsedContentKey, String> {
    let layer_kind = match raw.layer_kind.as_str() {
        "base" => portable_content_manifest::ContentLayerKind::Base,
        "integrated_dlc" => portable_content_manifest::ContentLayerKind::IntegratedDlc,
        "dlc" => portable_content_manifest::ContentLayerKind::Dlc,
        "mod" => portable_content_manifest::ContentLayerKind::Mod,
        _ => return Err("observed VFS content has an unknown layer kind".into()),
    };
    let layer = manifest
        .ordered_layers
        .iter()
        .find(|layer| layer.ordinal == raw.winning_layer_ordinal)
        .ok_or("observed VFS content names a missing selection layer")?;
    if layer.layer_kind != layer_kind || layer.layer_id != raw.layer_id {
        return Err("observed VFS content disagrees with its selection layer".into());
    }
    let key = portable_content_manifest::UsedContentKey {
        effective_vfs_manifest_sha256: manifest.effective_vfs_manifest_sha256.clone(),
        normalized_game_relative_path: raw.logical_path.clone(),
        winning_layer_ordinal: raw.winning_layer_ordinal,
        layer_kind,
        layer_id: raw.layer_id.clone(),
        source_sha256: raw.source_sha256.clone(),
        payload_sha256: raw.payload_sha256.clone(),
        payload_bytes: raw.payload_bytes,
    };
    key.validate()?;
    Ok(key)
}

fn raw_vfs_identity(
    content: &portable_content_manifest::UsedContentKey,
) -> production_render_submit_live_lane::RawVfsContentIdentity {
    production_render_submit_live_lane::RawVfsContentIdentity {
        logical_path: content.normalized_game_relative_path.clone(),
        winning_layer_ordinal: content.winning_layer_ordinal,
        layer_kind: layer_kind_token(content.layer_kind).into(),
        layer_id: content.layer_id.clone(),
        source_sha256: content.source_sha256.clone(),
        payload_sha256: content.payload_sha256.clone(),
        payload_bytes: content.payload_bytes,
    }
}

fn layer_kind_token(kind: portable_content_manifest::ContentLayerKind) -> &'static str {
    match kind {
        portable_content_manifest::ContentLayerKind::Base => "base",
        portable_content_manifest::ContentLayerKind::IntegratedDlc => "integrated_dlc",
        portable_content_manifest::ContentLayerKind::Dlc => "dlc",
        portable_content_manifest::ContentLayerKind::Mod => "mod",
    }
}

fn stable_dlc_id_from_layer(layer_id: &str) -> Result<String, String> {
    let normalized = layer_id.trim().replace('\\', "/").to_ascii_lowercase();
    let parts = normalized.split('/').collect::<Vec<_>>();
    if parts.len() != 3
        || parts[0] != "dlc"
        || parts[1].is_empty()
        || !parts[2].ends_with(".dlc")
        || parts
            .iter()
            .any(|part| part.is_empty() || *part == "." || *part == "..")
    {
        return Err(format!(
            "required DLC layer is not dlc/<id>/<descriptor>.dlc: {layer_id:?}"
        ));
    }
    Ok(parts[1].into())
}

#[cfg(windows)]
pub fn install_a729_production_render_bootstrap_hooks(
) -> Result<production_render_submit_live_lane::a729_hook::BootstrapOnlyInstallReceipt, String> {
    production_render_submit_live_lane::a729_hook::install_bootstrap_only()
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PromotedA729ProductionRenderSubmit {
    pub submit_index: u64,
    pub camera_carrier_ordinal: u8,
    pub camera_carrier: Option<portable_render_stream::AuthenticatedCameraCarrier>,
    pub particles: Vec<portable_render_stream::ParticlePacket>,
    pub entities: Vec<portable_render_stream::EntitySubmission>,
    pub ordered_draws: Vec<portable_render_stream::RenderOutputRef>,
    pub s1_shadow: Option<portable_render_stream::S1ShadowCarrier>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PromotedA729StaticProgramCapture {
    pub submit_index: u64,
    pub frame_sequence: u64,
    pub authority: portable_render_stream::RenderAuthority,
    pub ordered_programs: Vec<portable_render_stream::StaticMapPassProgram>,
    pub ordered_static_execution: Vec<portable_render_stream::RenderExecutionRef>,
    pub authenticated_texture_snapshots:
        Vec<portable_render_stream::AuthenticatedTextureSnapshotPacket>,
}

pub fn promote_a729_static_program_capture<
    V: production_render_catalog_live::AdmittedVfsPayloads,
>(
    manifest: &portable_content_manifest::ContentManifest,
    admitted_vfs: &V,
    capture: &production_render_submit_live_lane::RawStaticProgramCapture,
    effect_catalog_sha256: String,
    entity_animation_catalog_sha256: String,
) -> Result<PromotedA729StaticProgramCapture, production_render_submit_live_lane::PromotionError> {
    let programs = production_render_catalog_live::build_authenticated_static_programs(
        manifest,
        capture,
        admitted_vfs,
    )
    .map_err(|error| {
        production_render_submit_live_lane::PromotionError::Invalid(format!(
            "live d80 static program: {error}"
        ))
    })?;
    programs.validate_canonical_partition().map_err(|error| {
        production_render_submit_live_lane::PromotionError::Invalid(format!(
            "live d80 canonical partition: {error}"
        ))
    })?;
    let authority = production_render_catalog_live::portable_render_authority(
        &capture.catalog,
        manifest.effective_vfs_manifest_sha256.clone(),
        effect_catalog_sha256,
        entity_animation_catalog_sha256,
    )
    .map_err(|error| {
        production_render_submit_live_lane::PromotionError::Invalid(format!(
            "live d80 authority: {error}"
        ))
    })?;
    Ok(PromotedA729StaticProgramCapture {
        submit_index: capture.submit_index,
        frame_sequence: capture.frame_sequence,
        authority,
        ordered_programs: programs.ordered_programs,
        ordered_static_execution: programs.ordered_execution,
        authenticated_texture_snapshots: programs.authenticated_texture_snapshots,
    })
}

#[cfg(windows)]
pub fn drain_a729_static_program_captures<
    V: production_render_catalog_live::AdmittedVfsPayloads,
>(
    manifest: &portable_content_manifest::ContentManifest,
    admitted_vfs: &V,
    effect_catalog_sha256: &str,
    entity_animation_catalog_sha256: &str,
) -> Result<Vec<PromotedA729StaticProgramCapture>, production_render_submit_live_lane::PromotionError>
{
    let captures = production_render_submit_live_lane::drain_complete_static_program_captures()
        .map_err(production_render_submit_live_lane::PromotionError::Invalid)?;
    captures
        .iter()
        .map(|capture| {
            promote_a729_static_program_capture(
                manifest,
                admitted_vfs,
                capture,
                effect_catalog_sha256.to_owned(),
                entity_animation_catalog_sha256.to_owned(),
            )
        })
        .collect()
}

fn static_programs_bind(
    programs: &[portable_render_stream::StaticMapPassProgram],
    matches: fn(&portable_render_stream::StaticShaderResourceSource) -> bool,
) -> bool {
    programs
        .iter()
        .flat_map(|program| program.ordered_operations.iter())
        .filter_map(|operation| match &operation.command {
            portable_render_stream::StaticMapCommand::Draw { draw } => Some(draw),
            portable_render_stream::StaticMapCommand::ClearColor { .. }
            | portable_render_stream::StaticMapCommand::ClearDepthStencil { .. } => None,
        })
        .flat_map(|draw| draw.ordered_shader_resources.iter())
        .any(|binding| matches(&binding.source))
}

fn source_requires_political_carrier(
    source: &portable_render_stream::StaticShaderResourceSource,
) -> bool {
    matches!(
        source,
        portable_render_stream::StaticShaderResourceSource::PoliticalTexture { .. }
    )
}

fn source_requires_tiled_light_carrier(
    source: &portable_render_stream::StaticShaderResourceSource,
) -> bool {
    matches!(
        source,
        portable_render_stream::StaticShaderResourceSource::TiledLightIndex { .. }
            | portable_render_stream::StaticShaderResourceSource::TiledLightData { .. }
    )
}

#[cfg(test)]
mod fold_carrier_guard_tests {
    use super::*;
    use portable_render_stream::{
        PoliticalTextureKind, ShaderResourceViewDesc, StaticShaderResourceSource,
        TextureResourceDesc, TextureResourceDimension,
    };

    fn view() -> ShaderResourceViewDesc {
        ShaderResourceViewDesc::Texture2d {
            dxgi_format_u32: 28,
            most_detailed_mip: 0,
            mip_levels: 1,
        }
    }

    fn resource() -> TextureResourceDesc {
        TextureResourceDesc {
            dimension: TextureResourceDimension::Texture2d,
            width: 64,
            height: 64,
            depth: 1,
            mip_levels: 1,
            array_size: 1,
            dxgi_format_u32: 28,
            sample_count: 1,
            sample_quality: 0,
            usage_u32: 0,
            bind_flags_u32: 8,
            cpu_access_flags_u32: 0,
            misc_flags_u32: 0,
            ordered_subresources: Vec::new(),
        }
    }

    #[test]
    fn tiled_light_predicate_covers_both_variants_and_nothing_else() {
        let index = StaticShaderResourceSource::TiledLightIndex {
            writer_index: 1,
            payload_sha256: "a".repeat(64),
            resource: resource(),
            view: view(),
        };
        let data = StaticShaderResourceSource::TiledLightData {
            writer_index: 1,
            payload_sha256: "b".repeat(64),
            resource: resource(),
            view: view(),
        };
        assert!(source_requires_tiled_light_carrier(&index));
        assert!(source_requires_tiled_light_carrier(&data));

        let snapshot = StaticShaderResourceSource::AuthenticatedSnapshot {
            snapshot_sha256: "c".repeat(64),
            view: view(),
        };
        assert!(!source_requires_tiled_light_carrier(&snapshot));

        let political = StaticShaderResourceSource::PoliticalTexture {
            texture_kind: PoliticalTextureKind::Primary,
            target_binding_identity: "primary".into(),
            upload_order: 0,
            payload_sha256: "d".repeat(64),
            resource: resource(),
            view: view(),
        };
        assert!(!source_requires_tiled_light_carrier(&political));
    }

    #[test]
    fn political_predicate_is_unchanged_by_the_shared_helper_refactor() {
        let political = StaticShaderResourceSource::PoliticalTexture {
            texture_kind: PoliticalTextureKind::GradientBorderChannel1,
            target_binding_identity: "gradient-1".into(),
            upload_order: 3,
            payload_sha256: "e".repeat(64),
            resource: resource(),
            view: view(),
        };
        assert!(source_requires_political_carrier(&political));

        let index = StaticShaderResourceSource::TiledLightIndex {
            writer_index: 7,
            payload_sha256: "f".repeat(64),
            resource: resource(),
            view: view(),
        };
        assert!(!source_requires_political_carrier(&index));

        let snapshot = StaticShaderResourceSource::AuthenticatedSnapshot {
            snapshot_sha256: "0".repeat(64),
            view: view(),
        };
        assert!(!source_requires_political_carrier(&snapshot));
    }

    #[test]
    fn empty_program_set_binds_nothing() {
        assert!(!static_programs_bind(
            &[],
            source_requires_tiled_light_carrier
        ));
        assert!(!static_programs_bind(
            &[],
            source_requires_political_carrier
        ));
    }

    fn program_with_bound_draw(
        source: StaticShaderResourceSource,
    ) -> portable_render_stream::StaticMapPassProgram {
        use portable_render_stream::*;
        let bytecode = b"guard-test".to_vec();
        let content = portable_content_manifest::UsedContentKey {
            effective_vfs_manifest_sha256: "1".repeat(64),
            normalized_game_relative_path: "gfx/fx/pdxmap.shader".into(),
            winning_layer_ordinal: 0,
            layer_kind: portable_content_manifest::ContentLayerKind::Base,
            layer_id: "base".into(),
            source_sha256: "2".repeat(64),
            payload_sha256: "3".repeat(64),
            payload_bytes: 1,
        };
        let shader = |profile: &str| CompiledShaderPayload {
            compiler_identity: "guard-test-compiler".into(),
            target_profile: profile.into(),
            compiler_flags_u64: 0,
            sha256: "4".repeat(64),
            bytecode: bytecode.clone(),
        };
        let face = StencilFaceDesc {
            fail_operation: StencilOperation::Keep,
            depth_fail_operation: StencilOperation::Keep,
            pass_operation: StencilOperation::Keep,
            comparison: Comparison::Always,
        };
        let target = RenderTargetDesc {
            prepared_target: PreparedRenderTarget::SwapchainBackBuffer,
            width: 64,
            height: 64,
            dxgi_format_u32: 28,
            mip_level: 0,
            sample_count: 1,
            sample_quality: 0,
            persistent_content: None,
            exact_resource: None,
        };
        StaticMapPassProgram {
            program_ordinal: 0,
            pass_token: "guard-test-pass".into(),
            ordered_content_keys: Vec::new(),
            ordered_operations: vec![StaticMapOperation {
                operation_ordinal: 0,
                payload_sha256: String::new(),
                command: StaticMapCommand::Draw {
                    draw: StaticMapDraw {
                        draw_token: "guard-test-draw".into(),
                        shader_program: ShaderProgram {
                            source: content,
                            producer_registration_root_rva: None,
                            effect_token: "guard_test".into(),
                            vertex_entry_token: "V".into(),
                            pixel_entry_token: "P".into(),
                            ordered_defines: Vec::new(),
                            compiled_vertex_shader: shader("vs_5_0"),
                            compiled_pixel_shader: shader("ps_5_0"),
                            geometry_shader: None,
                            hull_shader: None,
                            domain_shader: None,
                            compute_shader: None,
                        },
                        ordered_shader_resources: vec![StaticShaderResourceBinding {
                            stage: ShaderStage::Pixel,
                            slot: 10,
                            semantic_token: "LightIndexMap".into(),
                            source,
                        }],
                        pipeline: PipelineDesc {
                            topology: PrimitiveTopology::TriangleList,
                            blend: BlendDesc {
                                alpha_to_coverage_enabled: false,
                                independent_blend_enabled: false,
                                enabled: false,
                                source_color: BlendFactor::One,
                                destination_color: BlendFactor::Zero,
                                color_operation: BlendOperation::Add,
                                source_alpha: BlendFactor::One,
                                destination_alpha: BlendFactor::Zero,
                                alpha_operation: BlendOperation::Add,
                                write_mask_rgba_u8: 0xf,
                                additional_targets: Vec::new(),
                                blend_factor_rgba_f32_bits: [0; 4],
                                sample_mask_u32: u32::MAX,
                            },
                            depth: DepthDesc {
                                enabled: false,
                                write_enabled: false,
                                comparison: Comparison::Always,
                                stencil_enabled: false,
                                stencil_read_mask_u8: 0xff,
                                stencil_write_mask_u8: 0xff,
                                front_face: face.clone(),
                                back_face: face,
                                stencil_reference_u32: 0,
                            },
                            rasterizer: RasterizerDesc {
                                fill: FillMode::Solid,
                                cull: CullMode::None,
                                front_counter_clockwise: false,
                                depth_clip_enabled: true,
                                scissor_enabled: false,
                                depth_bias_i32: 0,
                                depth_bias_clamp_f32_bits: 0,
                                slope_scaled_depth_bias_f32_bits: 0,
                                multisample_enabled: false,
                                antialiased_line_enabled: false,
                            },
                            samplers: Vec::new(),
                            input_layout: Vec::new(),
                            index_format: IndexFormat::None,
                            static_indices_u16: Vec::new(),
                        },
                        geometry: StaticGeometrySource::Exact {
                            geometry: GeometryBinding {
                                ordered_vertex_buffers: Vec::new(),
                                index_buffer: None,
                                stream_output_targets: Vec::new(),
                                draw: DrawCommand::Draw {
                                    vertex_count: 3,
                                    start_vertex: 0,
                                },
                            },
                        },
                        ordered_constant_buffers: Vec::new(),
                        output_state: RenderOutputState {
                            ordered_color_targets: vec![target],
                            depth_target: None,
                            viewport: ViewportDesc {
                                top_left_x_f32_bits: 0,
                                top_left_y_f32_bits: 0,
                                width_f32_bits: 64f32.to_bits(),
                                height_f32_bits: 64f32.to_bits(),
                                min_depth_f32_bits: 0,
                                max_depth_f32_bits: 1f32.to_bits(),
                            },
                            scissor: None,
                        },
                        payload_sha256: String::new(),
                    },
                },
            }],
            payload_sha256: String::new(),
        }
    }

    fn authority() -> portable_render_stream::RenderAuthority {
        portable_render_stream::RenderAuthority {
            process_id: 1,
            process_start_nonce: 1,
            observer_installed_qpc: 1,
            d3d_device_created_qpc: 1,
            first_creation_sequence: 1,
            hook_epoch_nonce: 1,
            hook_set_receipt_sha256: "5".repeat(64),
            capture_receipt_sha256: "6".repeat(64),
            launch_epoch_sha256: "7".repeat(64),
            effective_vfs_manifest_sha256: "1".repeat(64),
            effect_catalog_sha256: "8".repeat(64),
            entity_animation_catalog_sha256: "9".repeat(64),
            recorded_input_variant: None,
            recorded_input_receipt_sha256: None,
            recorded_input_observation: None,
            particle_runtime_mode:
                portable_render_stream::ParticleRuntimeMode::SubmittedPacketReplay,
        }
    }

    fn empty_submit(global_order: u32) -> portable_render_stream::RenderSubmit {
        portable_render_stream::RenderSubmit {
            global_order,
            camera_carrier_ordinal: 0,
            camera_carrier: None,
            ordered_static_map_programs: Vec::new(),
            authenticated_texture_snapshots: Vec::new(),
            tree_runtime: None,
            s1_shadow: None,
            particles: Vec::new(),
                particle_packet_set_sha256: String::new(),
            entities: Vec::new(),
            rail_construction_arrows: Vec::new(),
            ordered_draws: Vec::new(),
            ordered_render_operations: Vec::new(),
                tiled_lights: Vec::new(),
            weather_membership: Vec::new(),
            political_textures: Vec::new(),
        }
    }

    fn tiled_light_index_source() -> StaticShaderResourceSource {
        StaticShaderResourceSource::TiledLightIndex {
            writer_index: 1,
            payload_sha256: "a".repeat(64),
            resource: resource(),
            view: view(),
        }
    }

    #[test]
    fn fold_refuses_a_tiled_light_draw_when_the_carrier_is_empty() {
        let promoted = PromotedA729StaticProgramCapture {
            submit_index: 1,
            frame_sequence: 1,
            authority: authority(),
            ordered_programs: vec![program_with_bound_draw(tiled_light_index_source())],
            ordered_static_execution: Vec::new(),
            authenticated_texture_snapshots: Vec::new(),
        };
        let mut submit = empty_submit(1);
        assert!(submit.tiled_lights.is_empty());

        let error = fold_a729_static_program_capture(&mut submit, promoted, Vec::new())
            .expect_err("fold must refuse a program-bearing capture without scene-CB authority");
        let production_render_submit_live_lane::PromotionError::Invalid(message) = error else {
            panic!("expected an Invalid refusal, got {error:?}");
        };
        assert!(
            message.contains("scene constant-buffer authority"),
            "refusal must name the missing scene-CB authority first, got: {message}"
        );
    }

    #[test]
    fn program_binding_fold_refusal_is_named_on_the_wire() {
        // kills: carrier.render_submit.ordered_scene_constant_buffers - folding a
        // program-bearing hour refuses with EXACTLY the static reason the
        // d77/d80 scene-CB gap attributes on the wire, and the gap payload that
        // seals for such an hour decodes to a domain-77 Both/SourceUnavailable
        // H4DG for the refused hour (WorldFold names the domain via its
        // DOMAIN_GAP arm and stamps DOMAIN_GAP_PROBLEM with this cause).
        let promoted = PromotedA729StaticProgramCapture {
            submit_index: 1,
            frame_sequence: 1,
            authority: authority(),
            ordered_programs: vec![program_with_bound_draw(tiled_light_index_source())],
            ordered_static_execution: Vec::new(),
            authenticated_texture_snapshots: Vec::new(),
        };
        let error = fold_a729_static_program_capture(&mut empty_submit(1), promoted, Vec::new())
            .expect_err("program-bearing folds are refused without scene-CB authority");
        let production_render_submit_live_lane::PromotionError::Invalid(message) = error else {
            panic!("expected an Invalid refusal, got {error:?}");
        };
        assert_eq!(message, crate::attach_log::D77_D80_SCENE_CB_REFUSAL_REASON);

        let payload = crate::attach_log::d77_d80_scene_cb_gap_payload(60_759_359, 3)
            .expect("bounded d77/d80 scene-CB H4DG payload");
        let decoded =
            compact_contract::domain_gap::DomainGapRecordV1::decode(&payload)
                .expect("the wire gap payload decodes");
        assert_eq!(decoded.part, compact_contract::domain_gap::DomainGapPart::Both);
        assert_eq!(
            decoded.reason,
            compact_contract::domain_gap::DomainGapReason::SourceUnavailable
        );
        assert_eq!(decoded.target_hour_raw, 60_759_359);
        assert_eq!(decoded.outcome_ordinal, 3);
    }

    #[test]
    fn fold_refuses_any_program_capture_without_scene_cb_authority() {
        // kills: carrier.render_submit.ordered_scene_constant_buffers — program-bearing
        // captures are refused at the fold because no sealed scene constant-buffer bytes exist.
        let snapshot = StaticShaderResourceSource::AuthenticatedSnapshot {
            snapshot_sha256: "c".repeat(64),
            view: view(),
        };
        let promoted = PromotedA729StaticProgramCapture {
            submit_index: 1,
            frame_sequence: 1,
            authority: authority(),
            ordered_programs: vec![program_with_bound_draw(snapshot)],
            ordered_static_execution: Vec::new(),
            authenticated_texture_snapshots: Vec::new(),
        };
        let mut submit = empty_submit(1);

        let error = fold_a729_static_program_capture(&mut submit, promoted, Vec::new())
            .expect_err("program-bearing folds are refused without scene-CB authority");
        let production_render_submit_live_lane::PromotionError::Invalid(message) = error else {
            panic!("expected an Invalid refusal, got {error:?}");
        };
        assert!(message.contains("scene constant-buffer authority"), "{message}");
    }
}

pub fn fold_a729_static_program_capture(
    submit: &mut portable_render_stream::RenderSubmit,
    promoted: PromotedA729StaticProgramCapture,
    ordered_execution: Vec<portable_render_stream::RenderExecutionRef>,
) -> Result<(), production_render_submit_live_lane::PromotionError> {
    // kills: carrier.render_submit.ordered_scene_constant_buffers — the retired submit-level
    // scene constant-buffer carrier never had a producer edge, so program-bearing folds have
    // no sealed scene-CB authority and are refused before any doomed frame is serialized.
    if !promoted.ordered_programs.is_empty() {
        return Err(production_render_submit_live_lane::PromotionError::Invalid(
            "d80 static program fold has no selected-submit scene constant-buffer authority"
                .into(),
        ));
    }
    if !submit.ordered_static_map_programs.is_empty()
        || !submit.authenticated_texture_snapshots.is_empty()
        || !submit.ordered_render_operations.is_empty()
        || submit.tree_runtime.is_some()
    {
        return Err(production_render_submit_live_lane::PromotionError::Invalid(
            "d80 fold targets a different or already-populated submit".into(),
        ));
    }
    let static_subsequence = ordered_execution
        .iter()
        .filter(|row| {
            matches!(
                row,
                portable_render_stream::RenderExecutionRef::Static { .. }
            )
        })
        .cloned()
        .collect::<Vec<_>>();
    if static_subsequence != promoted.ordered_static_execution {
        return Err(production_render_submit_live_lane::PromotionError::Invalid(
            "d80 fold interleave changes the canonical static subsequence".into(),
        ));
    }
    let runtime_subsequence = ordered_execution
        .iter()
        .filter_map(|row| match row {
            portable_render_stream::RenderExecutionRef::Runtime { output } => Some(output.clone()),
            _ => None,
        })
        .collect::<Vec<_>>();
    if runtime_subsequence != submit.ordered_draws {
        return Err(production_render_submit_live_lane::PromotionError::Invalid(
            "d80 fold interleave changes the authenticated runtime subsequence".into(),
        ));
    }
    let requires_political_texture_observation = static_programs_bind(
        &promoted.ordered_programs,
        source_requires_political_carrier,
    );
    if requires_political_texture_observation && submit.political_textures.is_empty() {
        return Err(production_render_submit_live_lane::PromotionError::Invalid(
            "d80 selected-submit draw references a political texture, but the live collector has no authenticated selected-Present political upload observation; refusing an unbound packet"
                .into(),
        ));
    }
    let requires_tiled_light_observation = static_programs_bind(
        &promoted.ordered_programs,
        source_requires_tiled_light_carrier,
    );
    if requires_tiled_light_observation && submit.tiled_lights.is_empty() {
        return Err(production_render_submit_live_lane::PromotionError::Invalid(
            "d80 selected-submit draw references a tiled point-light texture, but the live collector has no authenticated tiled-light submission to join its payload digest against; refusing an unbound packet"
                .into(),
        ));
    }
    submit.authenticated_texture_snapshots = promoted.authenticated_texture_snapshots;
    submit.ordered_static_map_programs = promoted.ordered_programs;
    submit.ordered_render_operations = ordered_execution;
    let tree_runtime =
        portable_render_stream::C07TreeRuntimeCarrier::from_authenticated_selected_submit(submit)
            .map_err(|error| {
            production_render_submit_live_lane::PromotionError::Invalid(format!(
                "C07 selected-submit tree runtime: {error}"
            ))
        })?;
    submit.tree_runtime = tree_runtime;
    Ok(())
}

pub fn fold_a729_s1_shadow_carrier(
    submit: &mut portable_render_stream::RenderSubmit,
    carrier: portable_render_stream::S1ShadowCarrier,
) -> Result<(), production_render_submit_live_lane::PromotionError> {
    if submit.s1_shadow.is_some() {
        return Err(production_render_submit_live_lane::PromotionError::Invalid(
            "S1 fold targets an already-bound selected submit".into(),
        ));
    }
    if !carrier.is_inactive()
        && (submit.ordered_static_map_programs.is_empty()
            || submit.ordered_render_operations.is_empty())
    {
        return Err(production_render_submit_live_lane::PromotionError::Invalid(
            "active S1 fold requires a complete d80 operation stream".into(),
        ));
    }
    carrier.validate_against_submit(submit).map_err(|error| {
        production_render_submit_live_lane::PromotionError::Invalid(format!(
            "S1 selected-submit carrier: {error}"
        ))
    })?;
    submit.s1_shadow = Some(carrier);
    Ok(())
}

fn s1_static_draw<'a>(
    submit: &'a portable_render_stream::RenderSubmit,
    execution: &portable_render_stream::RenderExecutionRef,
) -> Result<&'a portable_render_stream::StaticMapDraw, String> {
    let portable_render_stream::RenderExecutionRef::Static { operation_ordinal } = execution else {
        return Err("generic PdxMesh S1 caster is not a static draw".into());
    };
    let matches = submit
        .ordered_static_map_programs
        .iter()
        .flat_map(|program| program.ordered_operations.iter())
        .filter_map(|operation| {
            (operation.operation_ordinal == *operation_ordinal).then_some(&operation.command)
        })
        .collect::<Vec<_>>();
    let [portable_render_stream::StaticMapCommand::Draw { draw }] = matches.as_slice() else {
        return Err("generic PdxMesh S1 execution does not select exactly one static draw".into());
    };
    Ok(draw)
}

fn build_s1_generic_pdxmesh_submission(
    raw: &production_render_submit_live_lane::a729_hook::RawS1GenericPdxmeshCasterObservation,
    frame: &production_render_submit_live_lane::a729_hook::RawRenderFrameCapture,
    submit: &portable_render_stream::RenderSubmit,
    by_sequence: &BTreeMap<u64, portable_render_stream::RenderExecutionRef>,
) -> Result<portable_render_stream::S1LightDepthSubmission, String> {
    let execution = resolve_s1_execution(&raw.execution, frame, by_sequence)?;
    let draw = s1_static_draw(submit, &execution)?;
    if draw.shader_program.effect_token == "PdxMeshBorderShadow" && raw.mesh_draw_skinned {
        return Err("S1 fixed-border PdxMesh caster cannot be skinned".into());
    }
    if !draw.shader_program.effect_token.starts_with("PdxMesh")
        || !draw.shader_program.effect_token.ends_with("Shadow")
    {
        return Err("S1 generic caster does not use an exact PdxMesh shadow effect".into());
    }
    if draw.shader_program.effect_token != "PdxMeshBorderShadow"
        && draw.shader_program.effect_token.contains("Skinned") != raw.mesh_draw_skinned
    {
        return Err("S1 generic caster skinning branch disagrees with shader effect".into());
    }
    let pipeline = if draw.shader_program.effect_token == "PdxMeshBorderShadow" {
        portable_render_stream::S1ShadowPipeline::FixedMapFurnitureBorder
    } else if raw.mesh_draw_skinned {
        portable_render_stream::S1ShadowPipeline::StandardSkinned
    } else {
        portable_render_stream::S1ShadowPipeline::StandardUnskinned
    };
    let portable_render_stream::DrawCommand::Draw {
        vertex_count,
        start_vertex,
    } = (match &draw.geometry {
        portable_render_stream::StaticGeometrySource::Exact { geometry } => &geometry.draw,
        portable_render_stream::StaticGeometrySource::InterpreterOutput { draw, .. } => draw,
    })
    else {
        return Err("S1 generic PdxMesh producer did not issue non-indexed Draw".into());
    };
    if *vertex_count != raw.vertex_count || *start_vertex != raw.start_vertex {
        return Err("S1 generic PdxMesh producer/d80 draw arguments differ".into());
    }
    let selected_mesh = format!("sha256:{}", canonical_json_sha256(&draw.geometry)?);
    let selected_submesh_material = format!(
        "sha256:{}",
        canonical_json_sha256(&(
            &draw.shader_program,
            &draw.ordered_shader_resources,
            &draw.pipeline,
        ))?
    );
    let stable_typed_upstream_id = format!(
        "sha256:{}",
        canonical_json_sha256(&(
            raw.partition_mask,
            raw.object_ordinal,
            raw.group_ordinal,
            raw.world_matrix_f32_bits,
            &selected_mesh,
            &selected_submesh_material,
        ))?
    );
    Ok(portable_render_stream::S1LightDepthSubmission::Pdxmesh {
        partition_mask: raw.partition_mask,
        stable_typed_upstream_id,
        owner: portable_render_stream::S1TypedOwner::GenericPdxmesh,
        world_matrix_f32_bits: raw.world_matrix_f32_bits,
        selected_mesh,
        selected_submesh_material,
        group_ordinal: raw.group_ordinal,
        submesh_ordinal: raw.submesh_ordinal,
        pipeline,
        draw: portable_render_stream::S1DrawArguments::Draw {
            vertex_count: raw.vertex_count,
            start_vertex: raw.start_vertex,
        },
        execution,
    })
}

fn build_s1_dynamic_tree_submission(
    raw: &production_render_submit_live_lane::a729_hook::RawS1TreeCasterObservation,
    submit: &portable_render_stream::RenderSubmit,
    by_sequence: &BTreeMap<u64, portable_render_stream::RenderExecutionRef>,
) -> Result<portable_render_stream::S1LightDepthSubmission, String> {
    let execution = by_sequence
        .get(&raw.final_draw_sequence)
        .cloned()
        .ok_or("S1 dynamic tree execution is absent from unified render order")?;
    let draw = s1_static_draw(submit, &execution)?;
    let portable_render_stream::DrawCommand::DrawIndexedInstanced {
        index_count_per_instance,
        instance_count,
        start_index,
        base_vertex,
        start_instance,
    } = (match &draw.geometry {
        portable_render_stream::StaticGeometrySource::Exact { geometry } => &geometry.draw,
        portable_render_stream::StaticGeometrySource::InterpreterOutput { draw, .. } => draw,
    })
    else {
        return Err("S1 dynamic tree producer did not issue indexed-instanced Draw".into());
    };
    if *instance_count != raw.source_instance_count {
        return Err("S1 dynamic tree source/d80 instance counts differ".into());
    }
    let selected_mesh = format!("sha256:{}", canonical_json_sha256(&draw.geometry)?);
    let selected_material = format!(
        "sha256:{}",
        canonical_json_sha256(&(
            raw.selected_material_effect_key_u32,
            &draw.shader_program,
            &draw.ordered_shader_resources,
            &draw.pipeline,
        ))?
    );
    Ok(portable_render_stream::S1LightDepthSubmission::Tree {
        partition_mask: raw.partition_mask,
        source_row_ordinal: raw.source_row_ordinal,
        selected_mesh_ordinal: 0,
        selected_lod_index: raw.selected_lod_index,
        selected_mesh,
        selected_material,
        pipeline: portable_render_stream::S1ShadowPipeline::TreeShadow,
        draw: portable_render_stream::S1DrawArguments::DrawIndexedInstanced {
            index_count_per_instance: *index_count_per_instance,
            instance_count: *instance_count,
            start_index: *start_index,
            base_vertex: *base_vertex,
            start_instance: *start_instance,
        },
        execution,
    })
}

#[cfg(windows)]
pub fn drain_a729_production_render_submits<
    V: production_render_catalog_live::AdmittedVfsPayloads,
    R: production_render_submit_live_lane::ExactLiveRenderCatalogSemantics,
>(
    manifest: &portable_content_manifest::ContentManifest,
    admitted_vfs: &V,
    semantics: &R,
) -> Result<
    Vec<PromotedA729ProductionRenderSubmit>,
    production_render_submit_live_lane::PromotionError,
> {
    let mut captures =
        production_render_submit_live_lane::a729_hook::drain_complete_submit_captures()
            .map_err(production_render_submit_live_lane::PromotionError::Invalid)?;
    for capture in &mut captures {
        production_render_submit_live_lane::materialize_live_render_catalog_semantics(
            capture, semantics,
        )?;
    }
    captures
        .iter()
        .map(|capture| {
            let submit = &capture.submit;
            let catalog = production_render_catalog_live::build_authenticated_render_catalog(
                manifest,
                submit,
                &capture.catalog,
                admitted_vfs,
            )
            .map_err(|error| {
                production_render_submit_live_lane::PromotionError::Invalid(format!(
                    "live d77 catalog: {error}"
                ))
            })?;
            let particles = production_render_submit_live_lane::promote_submit(submit, &catalog)?;
            let entities = submit
                .entity_submissions
                .iter()
                .map(|raw| {
                    production_render_submit_live_lane::promote_entity_submission(raw, &catalog)
                })
                .collect::<Result<Vec<_>, _>>()?;
            let mut ordered_draws = particles
                .iter()
                .map(|packet| portable_render_stream::RenderOutputRef::Particle {
                    effect_instance_id: packet.effect_instance_id,
                    bucket: packet.bucket,
                    packet_ordinal: packet.packet_ordinal,
                    draw_order: packet.draw_order,
                })
                .chain(entities.iter().map(|draw| {
                    portable_render_stream::RenderOutputRef::Entity {
                        pass_ordinal: draw.pass_ordinal,
                        entity_instance_id: draw.entity_instance_id,
                        draw_order: draw.draw_order,
                    }
                }))
                .collect::<Vec<_>>();
            ordered_draws.sort_by_key(|draw| match draw {
                portable_render_stream::RenderOutputRef::Particle { draw_order, .. }
                | portable_render_stream::RenderOutputRef::Entity { draw_order, .. } => *draw_order,
            });
            if ordered_draws.iter().enumerate().any(|(expected, draw)| {
                let actual = match draw {
                    portable_render_stream::RenderOutputRef::Particle { draw_order, .. }
                    | portable_render_stream::RenderOutputRef::Entity { draw_order, .. } => {
                        *draw_order
                    }
                };
                usize::try_from(actual).ok() != Some(expected)
            }) {
                return Err(production_render_submit_live_lane::PromotionError::Invalid(
                    "a729 final-draw order is not a complete contiguous cross-family sequence"
                        .into(),
                ));
            }
            Ok(PromotedA729ProductionRenderSubmit {
                submit_index: submit.submit_index,
                camera_carrier_ordinal: submit.camera_carrier_ordinal,
                camera_carrier: promote_a729_camera_carrier(submit.camera_carrier.as_ref())?,
                particles,
                entities,
                ordered_draws,
                s1_shadow: None,
            })
        })
        .collect()
}

#[cfg(windows)]
pub fn drain_a729_production_weather_changes(
) -> Result<Vec<production_render_submit_live_lane::RawWeatherObservation>, String> {
    production_render_submit_live_lane::a729_hook::drain_complete_weather_changes()
}

pub fn promote_a729_production_weather_change<
    R: production_render_submit_live_lane::ExactWeatherCatalogResolver,
>(
    raw: &production_render_submit_live_lane::RawWeatherObservation,
    sequence: u64,
    update_index: u64,
    resolver: &R,
) -> Result<
    portable_render_stream::WeatherMembership,
    production_render_submit_live_lane::PromotionError,
> {
    production_render_submit_live_lane::promote_weather_observation(
        raw,
        sequence,
        update_index,
        resolver,
    )
}

pub fn finalize_a729_production_weather_change<
    R: production_render_submit_live_lane::ExactWeatherCatalogResolver,
>(
    raw: &production_render_submit_live_lane::RawWeatherObservation,
    sequence: u64,
    update_index: u64,
    visibility: &[(
        production_render_submit_live_lane::RawWeatherVisibilityObservation,
        u64,
        u64,
    )],
    submit: &portable_render_stream::RenderSubmit,
    resolver: &R,
) -> Result<
    portable_render_stream::WeatherMembership,
    production_render_submit_live_lane::PromotionError,
> {
    let mut membership = production_render_submit_live_lane::promote_weather_observation(
        raw,
        sequence,
        update_index,
        resolver,
    )?;
    for (observation, observation_sequence, observation_update_index) in visibility {
        let phase = production_render_submit_live_lane::promote_weather_visibility_observation(
            observation,
            *observation_sequence,
            *observation_update_index,
        )?;
        production_render_submit_live_lane::append_weather_visibility_phase(
            &mut membership,
            phase,
        )?;
    }
    production_render_submit_live_lane::join_weather_membership_to_submit_outputs(
        &mut membership,
        submit,
    )?;
    Ok(membership)
}

pub fn promote_a729_production_entity_submission<
    R: production_render_submit_live_lane::ExactEntityCatalogResolver,
>(
    raw: &production_render_submit_live_lane::RawEntitySubmission,
    resolver: &R,
) -> Result<
    portable_render_stream::EntitySubmission,
    production_render_submit_live_lane::PromotionError,
> {
    production_render_submit_live_lane::promote_entity_submission(raw, resolver)
}

pub fn promote_a729_production_runtime_execution_order(
    raw: &production_render_submit_live_lane::RawParticleSubmit,
) -> Result<
    Vec<portable_render_stream::RenderExecutionRef>,
    production_render_submit_live_lane::PromotionError,
> {
    production_render_submit_live_lane::promote_runtime_execution_order(raw)
}

#[cfg(windows)]
pub fn drain_a729_production_map_shader_lookups(
) -> Result<Vec<production_render_submit_live_lane::RawMapShaderLookup>, String> {
    production_render_submit_live_lane::a729_hook::drain_map_shader_lookups()
}

#[cfg(windows)]
pub fn drain_a729_production_render_order_capture(
) -> Result<production_render_submit_live_lane::a729_hook::RawRenderOrderCapture, String> {
    production_render_submit_live_lane::a729_hook::drain_render_order_capture()
}

pub fn validate_a729_production_render_order_events(
    raw: &[production_render_submit_live_lane::RawRenderOrderEvent],
) -> Result<(), production_render_submit_live_lane::PromotionError> {
    production_render_submit_live_lane::validate_render_order_events(raw)
}

pub fn promote_a729_production_unified_render_execution_order<
    R: production_render_submit_live_lane::ExactStaticRenderOperationResolver,
>(
    raw: &[production_render_submit_live_lane::RawRenderOrderEvent],
    runtime_order: &[portable_render_stream::RenderOutputRef],
    static_programs: &[portable_render_stream::StaticMapPassProgram],
    resolver: &R,
) -> Result<
    Vec<portable_render_stream::RenderExecutionRef>,
    production_render_submit_live_lane::PromotionError,
> {
    production_render_submit_live_lane::promote_unified_render_execution_order(
        raw,
        runtime_order,
        static_programs,
        resolver,
    )
}

pub fn promote_a729_production_tiled_lights<
    R: production_render_submit_live_lane::ExactLightCatalogResolver,
>(
    raw: &production_render_submit_live_lane::RawTiledLightObservation,
    resolver: &R,
) -> Result<
    portable_render_stream::TiledLightSubmission,
    production_render_submit_live_lane::PromotionError,
> {
    production_render_submit_live_lane::promote_tiled_light_observation(raw, resolver)
}

#[cfg(windows)]
pub fn uninstall_a729_production_render_bootstrap_hooks() -> Result<(), String> {
    production_render_submit_live_lane::a729_hook::uninstall()
}

#[cfg(windows)]
pub fn install_a729_hud_role_hooks(
) -> Result<hud_role_observer_live_lane::a729_hook::InstalledAddresses, String> {
    hud_role_observer_live_lane::a729_hook::install()
}

#[cfg(windows)]
pub fn drain_a729_hud_role_snapshot<R: hud_role_observer_live_lane::ExactHudRoleResolver>(
    resolver: &R,
) -> Result<hud_role_observer_live_lane::HudRoleSnapshot, String> {
    let roots = hud_role_observer_live_lane::a729_hook::drain_complete_roots()?;
    hud_role_observer_live_lane::promote_snapshot(&roots, resolver)
}

#[cfg(windows)]
pub fn drain_a729_hud_role_snapshot_with_designer<
    R: hud_role_observer_live_lane::ExactHudRoleResolver,
>(
    resolver: &R,
) -> Result<hud_role_observer_live_lane::HudRoleSnapshot, String> {
    let mut roots = hud_role_observer_live_lane::a729_hook::drain_complete_roots()?;
    if let Some(designer) = design_catalog_context_live_lane::a729_hook::latest_active_role_root()?
    {
        let surface = match designer.context.family {
            design_catalog_context_live_lane::DesignerFamily::CountryEquipment => {
                hud_role_observer_live_lane::HudSurface::DesignerCountryEquipment
            }
            design_catalog_context_live_lane::DesignerFamily::Tank => {
                hud_role_observer_live_lane::HudSurface::DesignerTank
            }
            design_catalog_context_live_lane::DesignerFamily::Plane => {
                hud_role_observer_live_lane::HudSurface::DesignerPlane
            }
        };
        let sequence = roots
            .last()
            .map(|root| root.sequence.saturating_add(1))
            .ok_or("fixed HUD root set unexpectedly empty")?;
        roots.push(hud_role_observer_live_lane::RawHudControllerRoot {
            sequence,
            setup_rva: hud_role_observer_live_lane::DESIGNER_REFRESH_RVA,
            surface,
            controller_ptr: designer.view_ptr,
            runtime_root_ptr: designer.runtime_root_ptr,
            runtime_root_name: hud_role_observer_live_lane::a729_hook::runtime_root_name(
                designer.runtime_root_ptr,
            )?,
        });
    }
    hud_role_observer_live_lane::promote_snapshot(&roots, resolver)
}

#[cfg(windows)]
pub fn uninstall_a729_hud_role_hooks() -> Result<(), String> {
    hud_role_observer_live_lane::a729_hook::uninstall()
}

#[cfg(windows)]
pub fn install_a729_design_catalog_context_hook(
) -> Result<design_catalog_context_live_lane::a729_hook::InstalledHookAddress, String> {
    design_catalog_context_live_lane::a729_hook::install()
}

#[cfg(windows)]
pub fn encode_a729_design_catalog_context_frame(
    frame_hour: u64,
    elapsed_wall_ms: u64,
    status_flags: u32,
) -> Result<Vec<u8>, design_catalog_context_live_lane::CarrierError> {
    design_catalog_context_live_lane::a729_hook::encode_latest(
        frame_hour,
        elapsed_wall_ms,
        status_flags,
    )
}

#[cfg(windows)]
pub fn uninstall_a729_design_catalog_context_hook() -> Result<(), String> {
    design_catalog_context_live_lane::a729_hook::uninstall()
}

pub fn encode_a28_exact_controller_frame<
    S: dynamic_collection_trace_live_lane::ExactControllerTraceSource,
>(
    source: &S,
    frame_hour: u64,
    elapsed_wall_ms: u64,
    status_flags: u32,
) -> Result<Vec<u8>, dynamic_collection_trace_live_lane::TraceError> {
    dynamic_collection_trace_live_lane::collect_and_encode(
        source,
        frame_hour,
        elapsed_wall_ms,
        status_flags,
    )
}

pub fn encode_a28_air_mio_controller_frame(
    source: &dynamic_collection_trace_live_lane::a729_air_mio_controller::A729AirMioControllerSource,
    frame_hour: u64,
    elapsed_wall_ms: u64,
    status_flags: u32,
) -> Result<Vec<u8>, dynamic_collection_trace_live_lane::TraceError> {
    dynamic_collection_trace_live_lane::collect_and_encode(
        source,
        frame_hour,
        elapsed_wall_ms,
        status_flags,
    )
}

#[cfg(windows)]
pub struct A28PassiveCollectionSession {
    air: dynamic_collection_trace_live_lane::a729_air_selection_hook::A729AirSelectionHookOwner,
    mio: dynamic_collection_trace_live_lane::a729_mio_history_hook::A729MioHistoryHookOwner,
    mio_bonus: dynamic_collection_trace_live_lane::a729_mio_bonus_hook::A729MioBonusHookOwner,
}

#[cfg(windows)]
impl A28PassiveCollectionSession {
    pub fn install(game_clock_address: usize) -> Result<Self, String> {
        let mut air =
            dynamic_collection_trace_live_lane::a729_air_selection_hook::A729AirSelectionHookOwner::install(
                game_clock_address,
            )?;
        let mut mio = match dynamic_collection_trace_live_lane::a729_mio_history_hook::A729MioHistoryHookOwner::install(
            game_clock_address,
        ) {
            Ok(mio) => mio,
            Err(error) => {
                let rollback = air.uninstall();
                return Err(match rollback {
                    Ok(()) => format!("A28 MIO hook install failed: {error}"),
                    Err(rollback) => format!(
                        "A28 MIO hook install failed: {error}; air rollback failed: {rollback}"
                    ),
                });
            }
        };
        match dynamic_collection_trace_live_lane::a729_mio_bonus_hook::A729MioBonusHookOwner::install(
            game_clock_address,
        ) {
            Ok(mio_bonus) => Ok(Self {
                air,
                mio,
                mio_bonus,
            }),
            Err(error) => {
                let mio_rollback = mio.uninstall();
                let air_rollback = air.uninstall();
                let rollback = match (mio_rollback, air_rollback) {
                    (Ok(()), Ok(())) => String::new(),
                    (mio, air) => format!(
                        "; rollback failures: MIO history={mio:?}; air={air:?}"
                    ),
                };
                Err(format!(
                    "A28 MIO aggregate-bonus hook install failed: {error}{rollback}"
                ))
            }
        }
    }

    pub fn drain_payload(
        &self,
        frame_hour: u64,
        game_hour_raw: i32,
        elapsed_wall_ms: u64,
        status_flags: u32,
    ) -> Result<Option<Vec<u8>>, String> {
        let air_diagnostics = self.air.diagnostics()?;
        let mio_diagnostics = self.mio.diagnostics()?;
        let mio_bonus_diagnostics = self.mio_bonus.diagnostics()?;
        if !air_diagnostics.installed
            || !air_diagnostics.callbacks_open
            || !mio_diagnostics.installed
            || !mio_diagnostics.callbacks_open
            || !mio_bonus_diagnostics.installed
            || !mio_bonus_diagnostics.callbacks_open
        {
            return Err("A28 passive hook owner is unexpectedly inactive".into());
        }
        if air_diagnostics.poisoned || mio_diagnostics.poisoned || mio_bonus_diagnostics.poisoned {
            return Err("A28 passive hook owner is poisoned".into());
        }
        let mut sources = Vec::new();
        if air_diagnostics.pending_observations != 0 {
            sources.push(self.air.drain_latest_source(game_hour_raw)?);
        }
        if mio_diagnostics.pending_observation_pairs != 0 {
            sources.push(self.mio.drain_latest_source(game_hour_raw)?);
        }
        if mio_bonus_diagnostics.pending_observations != 0 {
            sources.push(self.mio_bonus.drain_latest_source(game_hour_raw)?);
        }
        let source = if sources.is_empty() {
            let Some(proof) =
                dynamic_collection_trace_live_lane::a729_headless_inactive::prove_all_tracked_controllers_inactive(
                    frame_hour,
                )?
            else {
                return Ok(None);
            };
            dynamic_collection_trace_live_lane::a729_air_mio_controller::A729AirMioControllerSource::admit_hook_owned_inactive(
                frame_hour,
                &proof,
            )?
        } else {
            dynamic_collection_trace_live_lane::a729_air_mio_controller::A729AirMioControllerSource::combine_hook_owned(
                frame_hour,
                sources,
            )?
        };
        crate::encode_a28_air_mio_controller_frame(
            &source,
            frame_hour,
            elapsed_wall_ms,
            status_flags,
        )
        .map(Some)
        .map_err(|error| format!("A28 DCT1 encode failed: {error:?}"))
    }

    pub fn uninstall(&mut self) -> Result<(), String> {
        let mio_bonus = self.mio_bonus.uninstall();
        let mio = self.mio.uninstall();
        let air = self.air.uninstall();
        let mut failures = Vec::new();
        if let Err(error) = mio_bonus {
            failures.push(format!("MIO aggregate-bonus={error}"));
        }
        if let Err(error) = mio {
            failures.push(format!("MIO history={error}"));
        }
        if let Err(error) = air {
            failures.push(format!("air={error}"));
        }
        if failures.is_empty() {
            Ok(())
        } else {
            Err(format!("A28 hook teardown failed: {}", failures.join("; ")))
        }
    }
}

pub fn encode_a29_exact_technology_canvas_transition<
    S: dynamic_collection_trace_live_lane::a729_technology_canvas::ExactTechnologyCanvasObserverSource,
>(
    source: &S,
    raw: dynamic_collection_trace_live_lane::a729_technology_canvas::RawBoundaryObservation,
) -> Result<Vec<u8>, dynamic_collection_trace_live_lane::a729_technology_canvas::ObserverError> {
    dynamic_collection_trace_live_lane::a729_technology_canvas::collect_after_boundary(source, raw)
}

pub fn encode_a36_exact_ability_frame<
    S: active_command_abilities_live_lane::ExactA729AbilityControllerSource,
>(
    source: &S,
    frame_hour: u64,
) -> Result<Vec<u8>, active_command_abilities_live_lane::AbilityError> {
    active_command_abilities_live_lane::collect_and_encode(source, frame_hour)
}

pub fn encode_a36_production_ability_frame<
    S: active_command_abilities_live_lane::ExactA729AbilityControllerSource,
>(
    source: &S,
    frame_hour: u64,
    elapsed_wall_ms: u64,
) -> Result<Vec<u8>, active_command_abilities_live_lane::production_carrier::CarrierError> {
    active_command_abilities_live_lane::production_carrier::collect_and_encode(
        source,
        frame_hour,
        elapsed_wall_ms,
    )
}

pub fn encode_a36_production_ability_unavailable(
    frame_hour: u64,
    elapsed_wall_ms: u64,
    status_flags: u32,
) -> Result<Vec<u8>, active_command_abilities_live_lane::production_carrier::CarrierError> {
    active_command_abilities_live_lane::production_carrier::encode_unavailable(
        frame_hour,
        elapsed_wall_ms,
        status_flags,
    )
}

pub fn encode_a26_exact_tooltip_frame<
    S: resolved_tooltip_instances_live_lane::ExactTooltipInstanceSource,
>(
    source: &S,
    selected_present: &resolved_tooltip_instances_live_lane::SelectedPresentStamp,
    elapsed_wall_ms: u64,
    status_flags: u32,
) -> Result<Vec<u8>, resolved_tooltip_instances_live_lane::TooltipTraceError> {
    resolved_tooltip_instances_live_lane::collect_and_encode(
        source,
        selected_present,
        elapsed_wall_ms,
        status_flags,
    )
}

#[cfg(windows)]
pub fn install_a26_tooltip_capture_owner(
) -> Result<resolved_tooltip_instances_live_lane::a729_hook::A729ResolvedTooltipHookOwner, String> {
    resolved_tooltip_instances_live_lane::a729_hook::A729ResolvedTooltipHookOwner::install()
}

#[cfg(windows)]
pub struct A26TooltipCaptureSession {
    owner: resolved_tooltip_instances_live_lane::a729_hook::A729ResolvedTooltipHookOwner,
}

#[cfg(windows)]
impl A26TooltipCaptureSession {
    pub fn install() -> Result<Self, String> {
        let owner = install_a26_tooltip_capture_owner()?;
        Ok(Self { owner })
    }

    pub fn capture_selected_present_payload(
        &self,
        selected_present: &portable_render_stream::SelectedPresentReceipt,
        elapsed_wall_ms: u64,
        pov_country: &str,
    ) -> Result<Vec<u8>, resolved_tooltip_instances_live_lane::TooltipTraceError> {
        let frame_hour = compact_contract::calendar::hour_index_from_raw(
            selected_present.selected_game_hour_raw,
        )
        .and_then(|hour| u64::try_from(hour).ok())
        .ok_or_else(|| {
            resolved_tooltip_instances_live_lane::TooltipTraceError::Invalid(
                "selected Present game hour is invalid".into(),
            )
        })?;
        let receipt_sha256 = selected_present.computed_sha256().map_err(|error| {
            resolved_tooltip_instances_live_lane::TooltipTraceError::Invalid(format!(
                "selected Present receipt cannot be sealed: {error}"
            ))
        })?;
        let stamp = resolved_tooltip_instances_live_lane::SelectedPresentStamp {
            frame_hour,
            selected_present_candidate_id: selected_present.selected_candidate_id,
            selected_present_ordinal: selected_present.selected_ordinal,
            selected_present_qpc_ticks: selected_present.selected_qpc_ticks,
            selected_present_receipt_sha256: receipt_sha256,
        };
        let source = self
            .owner
            .source(pov_country)
            .map_err(resolved_tooltip_instances_live_lane::TooltipTraceError::Source)?;
        encode_a26_exact_tooltip_frame(&source, &stamp, elapsed_wall_ms, 0)
    }

    pub fn capture_raw_selected_present_payload(
        &self,
        selected: &production_render_submit_live_lane::a729_hook::RawSelectedRenderFrameCapture,
        elapsed_wall_ms: u64,
        pov_country: &str,
    ) -> Result<Vec<u8>, resolved_tooltip_instances_live_lane::TooltipTraceError> {
        production_render_submit_live_lane::a729_hook::validate_selected_render_frame_receipt(
            selected,
        )
        .map_err(resolved_tooltip_instances_live_lane::TooltipTraceError::Source)?;
        let selected_ordinal =
            selected
                .receipt
                .candidate_count
                .checked_sub(1)
                .ok_or_else(|| {
                    resolved_tooltip_instances_live_lane::TooltipTraceError::Invalid(
                        "selected Present receipt has zero candidates".into(),
                    )
                })?;
        let portable = portable_render_stream::SelectedPresentReceipt::sealed(
            selected.receipt.candidate_count,
            selected.receipt.candidate_set_sha256.clone(),
            selected.receipt.selected_submit_index,
            selected_ordinal,
            selected.receipt.target_game_hour_raw,
            selected.receipt.selected_presentation_qpc_ticks,
            selected.receipt.cutoff_qpc_ticks,
        );
        self.capture_selected_present_payload(&portable, elapsed_wall_ms, pov_country)
    }

    pub fn uninstall(mut self) -> Result<(), String> {
        self.owner.uninstall()
    }

    fn reset_campaign(&self) -> Result<(), String> {
        self.owner.reset_campaign_observations()
    }
}

pub fn encode_a39_exact_character_presentation_frame<
    S: character_presentation_live_lane::ExactCharacterPresentationSource,
>(
    source: &S,
    frame_hour: u64,
    elapsed_wall_ms: u64,
    completeness_record_identity: impl Into<String>,
) -> Result<Vec<u8>, character_presentation_live_lane::CarrierError> {
    character_presentation_live_lane::collect_and_encode(
        source,
        frame_hour,
        elapsed_wall_ms,
        completeness_record_identity,
    )
}

pub fn encode_a40_exact_selected_designer_frame<
    S: design_catalog_context_live_lane::selected_designer_identity::ExactSelectedDesignerIdentitySource,
>(
    source: &S,
    frame_hour: u64,
    elapsed_wall_ms: u64,
    family: design_catalog_context_live_lane::selected_designer_identity::SelectedDesignerFamily,
) -> Result<
    Vec<u8>,
    design_catalog_context_live_lane::selected_designer_identity::SelectedDesignerError,
>{
    design_catalog_context_live_lane::selected_designer_identity::collect_and_encode(
        source,
        frame_hour,
        elapsed_wall_ms,
        family,
    )
}

#[cfg(windows)]
pub fn install_a729_selected_designer_fixture_hooks() -> Result<
    design_catalog_context_live_lane::selected_designer_identity::a729_hook::InstalledHookAddresses,
    String,
> {
    design_catalog_context_live_lane::a729_hook::install()
        .map_err(|error| format!("A40 fixture d78 prerequisite failed: {error}"))?;
    design_catalog_context_live_lane::selected_designer_identity::a729_hook::install_for_fixture()
}

#[cfg(windows)]
pub fn encode_a729_selected_designer_fixture_frame(
    frame_hour: u64,
    elapsed_wall_ms: u64,
    family: design_catalog_context_live_lane::selected_designer_identity::SelectedDesignerFamily,
) -> Result<
    Vec<u8>,
    design_catalog_context_live_lane::selected_designer_identity::SelectedDesignerError,
> {
    design_catalog_context_live_lane::selected_designer_identity::a729_hook::encode_fixture_latest(
        frame_hour,
        elapsed_wall_ms,
        family,
    )
}

#[cfg(windows)]
pub fn uninstall_a729_selected_designer_fixture_hooks() -> Result<(), String> {
    design_catalog_context_live_lane::selected_designer_identity::a729_hook::uninstall_fixture_hooks(
    )
}

#[cfg(windows)]
pub fn install_a729_selected_designer_hooks() -> Result<
    design_catalog_context_live_lane::selected_designer_identity::a729_hook::InstalledHookAddresses,
    String,
> {
    design_catalog_context_live_lane::a729_hook::install()
        .map_err(|error| format!("A40 production d78 prerequisite failed: {error}"))?;
    design_catalog_context_live_lane::selected_designer_identity::a729_hook::install()
}

#[cfg(windows)]
pub fn encode_a729_selected_designer_frame(
    frame_hour: u64,
    elapsed_wall_ms: u64,
) -> Result<
    Vec<u8>,
    design_catalog_context_live_lane::selected_designer_identity::SelectedDesignerError,
> {
    design_catalog_context_live_lane::selected_designer_identity::a729_hook::encode_latest(
        frame_hour,
        elapsed_wall_ms,
    )
}

#[cfg(windows)]
pub fn uninstall_a729_selected_designer_hooks() -> Result<(), String> {
    design_catalog_context_live_lane::selected_designer_identity::a729_hook::uninstall()
}

pub fn encode_a17_exact_nuke_stockpile_frame<
    S: nuke_stockpile_live_lane::ExactNukeStockpileSource,
>(
    source: &S,
    frame_hour: u64,
    elapsed_wall_ms: u64,
) -> Result<Vec<u8>, nuke_stockpile_live_lane::CarrierError> {
    nuke_stockpile_live_lane::collect_and_encode(source, frame_hour, elapsed_wall_ms)
}

pub fn encode_e20_exact_country_army_upgrade_frame<
    S: country_army_upgrade_requests_live_lane::ExactArmyUpgradeSource,
>(
    source: &S,
    frame_hour: u64,
    elapsed_wall_ms: u64,
) -> Result<Vec<u8>, country_army_upgrade_requests_live_lane::CarrierError> {
    country_army_upgrade_requests_live_lane::collect_and_encode(source, frame_hour, elapsed_wall_ms)
}

pub fn encode_a33_exact_sunk_ship_frame<S: sunk_ship_history_live_lane::ExactSunkShipSource>(
    source: &S,
    frame_hour: u64,
    elapsed_wall_ms: u64,
) -> Result<Vec<u8>, sunk_ship_history_live_lane::CarrierError> {
    sunk_ship_history_live_lane::collect_and_encode(source, frame_hour, elapsed_wall_ms)
}

pub fn encode_b7_exact_raid_instances_fixture(
    frame_hour: u64,
    elapsed_wall_ms: u64,
    parents: &[compact_contract::raid_instances::RaidParentRow],
    children: &[compact_contract::raid_instances::RaidInstanceRow],
) -> Result<Vec<u8>, String> {
    compact_contract::raid_instances::encode_payload(
        frame_hour,
        elapsed_wall_ms,
        0,
        parents,
        children,
        u32::try_from(parents.len()).map_err(|_| "B7 parent population exceeds u32".to_string())?,
        u32::try_from(children.len()).map_err(|_| "B7 child population exceeds u32".to_string())?,
    )
}

#[cfg(windows)]
pub fn begin_a729_convoy_transport_attack_evidence(executable_sha256: &str) -> Result<(), String> {
    attach_log::begin_convoy_transport_evidence(executable_sha256)
}

#[cfg(windows)]
pub fn take_a729_convoy_transport_attack_evidence_batches() -> Result<Vec<Vec<u8>>, String> {
    attach_log::take_convoy_transport_evidence_batches()
}

#[cfg(windows)]
pub fn seal_a729_convoy_transport_attack_evidence() -> Result<Vec<Vec<u8>>, String> {
    attach_log::seal_convoy_transport_evidence()
}

#[cfg(windows)]
struct HConvoyFixtureOwner {
    output_root: std::path::PathBuf,
    arm: String,
    session_nonce: String,
    client: naval4950::ConvoyTerminalEntityHandle,
    utc_start: String,
    negative_start: Option<naval4950::HConvoyControlRawObservation>,
}

#[cfg(windows)]
impl HConvoyFixtureOwner {
    fn install(
        output_root: &str,
        arm: &str,
        session_nonce: &str,
        client_id: u32,
        client_type_tag: u32,
        utc_start: &str,
    ) -> Result<Self, String> {
        let expected_nonce = std::env::var("BATTLE_PLAN_H_CONVOY_SESSION_NONCE").or_else(|_| std::env::var("HOI4_H_CONVOY_SESSION_NONCE"))
            .map_err(|_| "H-CONVOY launch nonce environment is absent".to_string())?;
        if session_nonce.len() < 32
            || session_nonce.len() > 128
            || !session_nonce
                .bytes()
                .all(|byte| byte.is_ascii_alphanumeric() || byte == b'-')
            || expected_nonce != session_nonce
        {
            return Err("H-CONVOY session nonce is absent, malformed, or not launch-bound".into());
        }
        if arm != "positive" && arm != "negative" {
            return Err("H-CONVOY arm must be positive or negative".into());
        }
        let output_root = std::path::PathBuf::from(output_root);
        if !output_root.is_absolute() {
            return Err("H-CONVOY output root must be absolute".into());
        }
        std::fs::create_dir_all(&output_root)
            .map_err(|error| format!("H-CONVOY output root creation failed: {error}"))?;
        if std::fs::read_dir(&output_root)
            .map_err(|error| format!("H-CONVOY output root read failed: {error}"))?
            .next()
            .is_some()
        {
            return Err("H-CONVOY output root must be empty".into());
        }
        let client = naval4950::ConvoyTerminalEntityHandle {
            id: client_id,
            type_tag: client_type_tag,
        };
        if client.id == 0 || client.type_tag != naval4950::GAME_ENTITY_TYPE_TAG {
            return Err("H-CONVOY client identity is invalid".into());
        }
        begin_a729_convoy_transport_attack_evidence(naval4950::A729_EXE_SHA256)?;
        let negative_start = if arm == "negative" {
            match naval4950::capture_h_convoy_control_raw(client) {
                Ok(value) => Some(value),
                Err(error) => {
                    let _ = naval4950::uninstall_convoy_terminal_hook();
                    return Err(error);
                }
            }
        } else {
            None
        };
        Ok(Self {
            output_root,
            arm: arm.into(),
            session_nonce: session_nonce.into(),
            client,
            utc_start: utc_start.into(),
            negative_start,
        })
    }

    fn write_json_role(
        &self,
        role: &str,
        value: &serde_json::Value,
        utc: &str,
        entries: &mut Vec<serde_json::Value>,
    ) -> Result<std::path::PathBuf, String> {
        let path = self.output_root.join(format!("{role}.json"));
        let mut bytes = serde_json::to_vec_pretty(value)
            .map_err(|error| format!("H-CONVOY {role} serialization failed: {error}"))?;
        bytes.push(b'\n');
        write_synced_new_file(&path, &bytes)?;
        let sha256 = format!("{:x}", sha2::Sha256::digest(&bytes));
        entries.push(serde_json::json!({
            "role": role,
            "path": path.file_name().and_then(|name| name.to_str()).ok_or("H-CONVOY role filename is not UTF-8")?,
            "sha256": sha256,
            "size": bytes.len(),
            "utc": utc,
        }));
        Ok(path)
    }

    fn route_snapshot(route: &naval4950::HConvoyRouteRawObservation) -> serde_json::Value {
        serde_json::json!({
            "schema": "h-convoy-route-snapshot-v1",
            "client": [route.client_id, route.client_type_tag],
            "producer_kind": route.producer_kind,
            "source_kind": route.source_kind,
            "destination_kind": route.destination_kind,
            "source_id": route.source_id,
            "destination_id": route.destination_id,
        })
    }

    fn route_object(route: &naval4950::HConvoyRouteRawObservation) -> serde_json::Value {
        let mut value = serde_json::json!({
            "schema": "h-convoy-route-object-window-v1",
            "address": route.route_object,
            "bytes_hex": h_convoy_hex(&route.route_object_bytes),
        });
        if !route.source_endpoint_bytes.is_empty() {
            value["source_state"] = serde_json::json!({
                "address": route.source_endpoint_object,
                "bytes_hex": h_convoy_hex(&route.source_endpoint_bytes),
            });
            value["destination_state"] = serde_json::json!({
                "address": route.destination_endpoint_object,
                "bytes_hex": h_convoy_hex(&route.destination_endpoint_bytes),
            });
        }
        value
    }

    fn seal_and_write(self, utc_end: &str) -> Result<std::path::PathBuf, String> {
        let negative_end = if self.arm == "negative" {
            Some(naval4950::capture_h_convoy_control_raw(self.client)?)
        } else {
            None
        };
        let batches = seal_a729_convoy_transport_attack_evidence()?;
        if batches.is_empty() {
            return Err("H-CONVOY seal returned no CTA2 custody batch".into());
        }
        let positive = if self.arm == "positive" {
            Some(naval4950::take_h_convoy_positive_raw()?)
        } else {
            None
        };
        let mut raw_files = Vec::new();
        let mut arm_hour = None;
        let mut route_start = None;
        let mut route_end = None;
        let mut d70_before_sha256 = None;
        let mut d70_after_sha256 = None;
        if let Some(raw) = positive.as_ref() {
            arm_hour = Some(raw.game_hour_raw);
            self.write_json_role(
                "context",
                &serde_json::json!({
                    "schema": "h-convoy-context-window-v1", "module_base": raw.module_base,
                    "context_rva": "0x1962364", "bytes_hex": h_convoy_hex(&raw.context_bytes),
                }),
                &self.utc_start,
                &mut raw_files,
            )?;
            self.write_json_role(
                "registers",
                &serde_json::json!({
                    "schema": "h-convoy-registers-v1", "game_hour_raw": raw.game_hour_raw,
                    "rcx": raw.rcx, "edx": raw.edx, "r8": raw.r8, "r9": raw.r9,
                    "r12": raw.r12, "r15": raw.r15, "killer_tag_db_id": raw.killer_tag_db_id,
                }),
                &self.utc_start,
                &mut raw_files,
            )?;
            self.write_json_role(
                "member",
                &serde_json::json!({
                    "schema": "h-convoy-member-window-v1", "address": raw.r12,
                    "bytes_hex": h_convoy_hex(&raw.member_bytes),
                }),
                &self.utc_start,
                &mut raw_files,
            )?;
            self.write_json_role(
                "route_registry",
                &serde_json::json!({
                    "schema": "h-convoy-route-registry-slot-v1",
                    "registry_root": raw.route.reference_registry,
                    "slot_address": raw.route.reference_slot,
                    "bytes_hex": h_convoy_hex(&raw.route.route_slot_bytes),
                }),
                &self.utc_start,
                &mut raw_files,
            )?;
            self.write_json_role(
                "route_object",
                &Self::route_object(&raw.route),
                &self.utc_start,
                &mut raw_files,
            )?;
            self.write_json_role(
                "d49_d50_epoch",
                &serde_json::json!({
                    "schema": "h-convoy-combat-epoch-v1", "member_address": raw.r12,
                    "combat_id": raw.combat_id, "combat_generation": raw.combat_generation,
                    "game_hour_raw": raw.game_hour_raw,
                }),
                &self.utc_start,
                &mut raw_files,
            )?;
            self.write_json_role(
                "d70_before",
                &serde_json::json!({
                    "schema": "h-convoy-d70-vector-v1", "rows": raw.d70_before,
                }),
                &self.utc_start,
                &mut raw_files,
            )?;
            self.write_json_role(
                "d70_after",
                &serde_json::json!({
                    "schema": "h-convoy-d70-vector-v1", "rows": raw.d70_after,
                }),
                utc_end,
                &mut raw_files,
            )?;
        } else {
            let before = self
                .negative_start
                .as_ref()
                .ok_or("H-CONVOY negative start is absent")?;
            let after = negative_end
                .as_ref()
                .ok_or("H-CONVOY negative end is absent")?;
            if before.game_hour_raw != after.game_hour_raw
                || before.route != after.route
                || before.d70 != after.d70
                || before.heartbeat.telemetry.callbacks_seen != 0
                || after.heartbeat.telemetry.callbacks_seen != 0
            {
                return Err(
                    "H-CONVOY negative arm changed hour, route, D70, or callback count".into(),
                );
            }
            arm_hour = Some(before.game_hour_raw);
            route_start = Some(Self::route_snapshot(&before.route));
            route_end = Some(Self::route_snapshot(&after.route));
            let before_d70 =
                serde_json::json!({"schema":"h-convoy-d70-vector-v1","rows":before.d70});
            let after_d70 = serde_json::json!({"schema":"h-convoy-d70-vector-v1","rows":after.d70});
            self.write_json_role(
                "heartbeat_before",
                &serde_json::json!({
                    "schema":"h-convoy-heartbeat-v1", "armed":true,
                    "game_hour_raw":before.game_hour_raw,
                    "callbacks_seen":before.heartbeat.telemetry.callbacks_seen,
                    "sequence":before.heartbeat_sequence,
                }),
                &self.utc_start,
                &mut raw_files,
            )?;
            self.write_json_role(
                "heartbeat_after",
                &serde_json::json!({
                    "schema":"h-convoy-heartbeat-v1", "armed":true,
                    "game_hour_raw":after.game_hour_raw,
                    "callbacks_seen":after.heartbeat.telemetry.callbacks_seen,
                    "sequence":after.heartbeat_sequence,
                }),
                utc_end,
                &mut raw_files,
            )?;
            self.write_json_role(
                "route_start",
                route_start.as_ref().unwrap(),
                &self.utc_start,
                &mut raw_files,
            )?;
            self.write_json_role(
                "route_end",
                route_end.as_ref().unwrap(),
                utc_end,
                &mut raw_files,
            )?;
            self.write_json_role(
                "d49_d50_epoch",
                &serde_json::json!({
                    "schema":"h-convoy-combat-epoch-v1", "member_address":before.member_address,
                    "combat_id":before.combat_id, "combat_generation":before.combat_generation,
                    "game_hour_raw":before.game_hour_raw,
                    "client":[before.route.client_id,before.route.client_type_tag],
                }),
                &self.utc_start,
                &mut raw_files,
            )?;
            let before_path =
                self.write_json_role("d70_before", &before_d70, &self.utc_start, &mut raw_files)?;
            let after_path =
                self.write_json_role("d70_after", &after_d70, utc_end, &mut raw_files)?;
            d70_before_sha256 = Some(h_convoy_sha256_file(&before_path)?);
            d70_after_sha256 = Some(h_convoy_sha256_file(&after_path)?);
        }
        let mut batch_entries = Vec::new();
        for (index, batch) in batches.iter().enumerate() {
            let path = self.output_root.join(format!("cta2_{index:04}.bin"));
            write_synced_new_file(&path, batch)?;
            batch_entries.push(serde_json::json!({
                "path": path.file_name().and_then(|name| name.to_str()).ok_or("H-CONVOY CTA2 filename is not UTF-8")?,
                "sha256": format!("{:x}", sha2::Sha256::digest(batch)),
                "size": batch.len(), "utc": utc_end,
            }));
        }
        let result = serde_json::json!({
            "schema":"h-convoy-arm-acquisition-v1", "id":self.arm.clone(),
            "session_nonce":self.session_nonce.clone(), "utc_start":self.utc_start.clone(), "utc_end":utc_end,
            "game_hour_raw":arm_hour.ok_or("H-CONVOY arm hour is absent")?,
            "raw_files":raw_files, "cta2_batches":batch_entries,
            "route_start":route_start, "route_end":route_end,
            "d70_before_sha256":d70_before_sha256, "d70_after_sha256":d70_after_sha256,
            "production_domain_90_scheduled":false,
        });
        let result_path = self.output_root.join("arm_acquisition.json");
        let mut bytes = serde_json::to_vec_pretty(&result)
            .map_err(|error| format!("H-CONVOY arm result serialization failed: {error}"))?;
        bytes.push(b'\n');
        write_synced_new_file(&result_path, &bytes)?;
        Ok(result_path)
    }
}

#[cfg(windows)]
impl Drop for HConvoyFixtureOwner {
    fn drop(&mut self) {
        let _ = naval4950::uninstall_convoy_terminal_hook();
    }
}

#[cfg(windows)]
fn h_convoy_hex(bytes: &[u8]) -> String {
    use std::fmt::Write;
    let mut out = String::with_capacity(bytes.len() * 2);
    for byte in bytes {
        let _ = write!(&mut out, "{byte:02x}");
    }
    out
}

#[cfg(windows)]
fn h_convoy_sha256_file(path: &std::path::Path) -> Result<String, String> {
    use std::io::Read;
    let mut file =
        std::fs::File::open(path).map_err(|error| format!("H-CONVOY hash open failed: {error}"))?;
    let mut digest = sha2::Sha256::new();
    let mut buffer = [0u8; 1024 * 1024];
    loop {
        let count = file
            .read(&mut buffer)
            .map_err(|error| format!("H-CONVOY hash read failed: {error}"))?;
        if count == 0 {
            break;
        }
        sha2::Digest::update(&mut digest, &buffer[..count]);
    }
    Ok(format!("{:x}", sha2::Digest::finalize(digest)))
}

/// Evidence-only, one-argument remote-thread ABI.  The launch-bound nonce and
#[cfg(windows)]
#[no_mangle]
pub unsafe extern "system" fn battle-plan_h_convoy_fixture_command_json(
    command: *const std::ffi::c_char,
) -> u32 {
    use std::ffi::CStr;
    if command.is_null() {
        return 100;
    }
    let bytes = unsafe { CStr::from_ptr(command) }.to_bytes();
    if bytes.is_empty() || bytes.len() > 16 * 1024 {
        return 101;
    }
    let Ok(value) = serde_json::from_slice::<serde_json::Value>(bytes) else {
        return 102;
    };
    let Some(object) = value.as_object() else {
        return 103;
    };
    let Some(operation) = object.get("operation").and_then(|value| value.as_str()) else {
        return 104;
    };
    match operation {
        "install" => {
            let Some(output_root) = object.get("output_root").and_then(|value| value.as_str())
            else {
                return 110;
            };
            let Some(arm) = object.get("arm").and_then(|value| value.as_str()) else {
                return 111;
            };
            let Some(nonce) = object.get("session_nonce").and_then(|value| value.as_str()) else {
                return 112;
            };
            let Some(client_id) = object
                .get("client_id")
                .and_then(|value| value.as_u64())
                .and_then(|value| u32::try_from(value).ok())
            else {
                return 113;
            };
            let Some(client_type_tag) = object
                .get("client_type_tag")
                .and_then(|value| value.as_u64())
                .and_then(|value| u32::try_from(value).ok())
            else {
                return 114;
            };
            let Some(utc_start) = object.get("utc_start").and_then(|value| value.as_str()) else {
                return 115;
            };
            let Ok(mut slot) = H_CONVOY_FIXTURE_OWNER
                .get_or_init(|| std::sync::Mutex::new(None))
                .lock()
            else {
                return 116;
            };
            if slot.is_some() {
                return 117;
            }
            match HConvoyFixtureOwner::install(
                output_root,
                arm,
                nonce,
                client_id,
                client_type_tag,
                utc_start,
            ) {
                Ok(owner) => {
                    *slot = Some(owner);
                    0
                }
                Err(_) => 118,
            }
        }
        "seal" => {
            let Some(utc_end) = object.get("utc_end").and_then(|value| value.as_str()) else {
                return 120;
            };
            let Some(owner) = H_CONVOY_FIXTURE_OWNER
                .get()
                .and_then(|slot| slot.lock().ok())
                .and_then(|mut slot| slot.take())
            else {
                return 121;
            };
            match owner.seal_and_write(utc_end) {
                Ok(_) => 0,
                Err(_) => 122,
            }
        }
        "abort" => {
            let _ = H_CONVOY_FIXTURE_OWNER
                .get()
                .and_then(|slot| slot.lock().ok())
                .and_then(|mut slot| slot.take());
            let _ = naval4950::uninstall_convoy_terminal_hook();
            0
        }
        _ => 105,
    }
}

pub fn encode_a31_exact_national_focus_presentation_frame<
    S: compact_contract::national_focus_presentation::ExactNationalFocusPresentationSource,
>(
    source: &S,
    frame_hour: u64,
    elapsed_wall_ms: u64,
    country_tag: &str,
    selected_present: compact_contract::national_focus_presentation::SelectedPresentBinding,
) -> Result<Vec<u8>, compact_contract::national_focus_presentation::CarrierError> {
    compact_contract::national_focus_presentation::collect_and_encode(
        source,
        frame_hour,
        elapsed_wall_ms,
        country_tag,
        selected_present,
    )
}

#[cfg(windows)]
struct A31ProductionSession {
    generation: Option<u64>,
    frame_hour: Option<u64>,
    country_tag: Option<String>,
    country_object: Option<usize>,
    pending_gui_join: Option<
        focus_research_decisions_depth_live::a729_focus_item_hook::PendingNationalFocusGuiJoin,
    >,
}

#[cfg(windows)]
impl A31ProductionSession {
    fn install() -> Result<Self, String> {
        use std::sync::Arc;
        focus_research_decisions_depth_live::a729_focus_item_hook::install_production(Arc::new(
            attach_log::A31CollectorDecoder,
        ))?;
        Ok(Self {
            generation: None,
            frame_hour: None,
            country_tag: None,
            country_object: None,
            pending_gui_join: None,
        })
    }

    fn ensure_armed(
        &mut self,
        frame_hour: u64,
        country_tag: &str,
        country_object: usize,
    ) -> Result<(), String> {
        if self.generation.is_some()
            && self.frame_hour == Some(frame_hour)
            && self.country_tag.as_deref() == Some(country_tag)
            && self.country_object == Some(country_object)
        {
            return Ok(());
        }
        focus_research_decisions_depth_live::a729_focus_item_hook::cancel_active_epoch()?;
        let generation =
            focus_research_decisions_depth_live::a729_focus_item_hook::arm_selected_present_epoch(
                frame_hour,
                country_tag,
                country_object,
            )?;
        self.generation = Some(generation);
        self.frame_hour = Some(frame_hour);
        self.country_tag = Some(country_tag.to_owned());
        self.country_object = Some(country_object);
        self.pending_gui_join = None;
        Ok(())
    }

    fn register_selected_present_gui_root(
        &mut self,
        selected: &production_render_submit_live_lane::a729_hook::RawSelectedRenderFrameCapture,
    ) -> Result<(), String> {
        production_render_submit_live_lane::a729_hook::validate_selected_render_frame_receipt(
            selected,
        )?;
        let generation = self.generation.ok_or("A31 production epoch is not armed")?;
        let join =
            focus_research_decisions_depth_live::a729_focus_item_hook::pending_focus_gui_join(
                generation,
            )?;
        let selected_hour =
            compact_contract::calendar::hour_index_from_raw(selected.receipt.target_game_hour_raw)
                .and_then(|value| u64::try_from(value).ok())
                .ok_or("A31 selected-Present game hour is invalid")?;
        if join.root.frame_hour != selected_hour
            || join.root.callback_qpc_ticks > selected.receipt.selected_presentation_qpc_ticks
        {
            return Err("A31 focus GUI root does not precede the selected Present".into());
        }
        let runtime_root_ptr = attach_log::read_u64_stable(
            join.root
                .focus_view_ptr
                .checked_add(0x588)
                .ok_or("A31 focus runtime-root address overflow")?,
        )
        .and_then(|value| usize::try_from(value).ok())
        .filter(|value| *value != 0)
        .ok_or("A31 focus runtime ContainerWindow root is unreadable")?;
        production_render_submit_live_lane::a729_hook::register_selected_present_gui_root(
            selected.receipt.selected_submit_index,
            selected.receipt.selected_frame_sequence,
            join.root.authored_surface_identity,
            join.root.runtime_root_identity,
            join.root.effective_interface_winner,
            join.root.callback_qpc_ticks,
            selected.receipt.target_game_hour_raw,
            join.root.focus_view_ptr,
            runtime_root_ptr,
            join.root.runtime_graphical_root_ptr,
        )?;
        self.pending_gui_join = Some(join);
        Ok(())
    }

    fn seal_selected_present(
        &mut self,
        selected: &production_render_submit_live_lane::a729_hook::RawSelectedRenderFrameCapture,
        gui_draws: &[production_render_submit_live_lane::a729_hook::SelectedPresentGuiDrawCapture],
        ui_pass: &portable_render_stream::AuthenticatedUiPass,
        elapsed_wall_ms: u64,
    ) -> Result<Vec<u8>, String> {
        production_render_submit_live_lane::a729_hook::validate_selected_render_frame_receipt(
            selected,
        )?;
        let frame_hour =
            compact_contract::calendar::hour_index_from_raw(selected.receipt.target_game_hour_raw)
                .and_then(|value| u64::try_from(value).ok())
                .ok_or("A31 selected-Present game hour is invalid")?;
        if self.frame_hour != Some(frame_hour) {
            return Err("A31 armed and selected-Present frame hours differ".into());
        }
        let ordinal = selected
            .receipt
            .candidate_count
            .checked_sub(1)
            .ok_or("A31 selected-Present candidate set is empty")?;
        let portable = portable_render_stream::SelectedPresentReceipt::sealed(
            selected.receipt.candidate_count,
            selected.receipt.candidate_set_sha256.clone(),
            selected.receipt.selected_submit_index,
            ordinal,
            selected.receipt.target_game_hour_raw,
            selected.receipt.selected_presentation_qpc_ticks,
            selected.receipt.cutoff_qpc_ticks,
        );
        let stamp =
            focus_research_decisions_depth_live::a729_focus_item_hook::SelectedPresentStamp {
                frame_hour,
                selected_present_global_order: selected.receipt.selected_global_order,
                selected_present_qpc_ticks: selected.receipt.selected_presentation_qpc_ticks,
                qpc_frequency_hz: selected.receipt.cutoff_qpc_frequency_hz,
                selected_present_receipt_sha256: selected.receipt.selection_receipt_sha256.clone(),
                portable_selected_present_receipt_sha256: portable.computed_sha256()?,
            };
        let generation = self
            .generation
            .take()
            .ok_or("A31 production epoch is not armed")?;
        let join = self
            .pending_gui_join
            .take()
            .ok_or("A31 production epoch has no registered GUI root")?;
        self.frame_hour = None;
        self.country_tag = None;
        self.country_object = None;
        let result =
            (|| {
                let geometry =
                focus_research_decisions_depth_live::a729_focus_item_hook::
                    sample_pending_focus_gui_geometry(generation)?;
                let metadata =
                    crate::a31_focus_canvas_join::national_focus_selected_metadata(selected)?;
                let canvas = crate::a31_focus_canvas_join::bind_national_focus_canvas(
                    &join, gui_draws, ui_pass, &metadata, &geometry,
                )?;
                focus_research_decisions_depth_live::a729_focus_item_hook::
                seal_selected_present_epoch_with_canvas(
                generation,
                stamp,
                canvas,
            )?;
                Ok(focus_research_decisions_depth_live::a729_focus_item_hook::drain_evidence_record(
                elapsed_wall_ms,
            )?.encoded_nfp1)
            })();
        if result.is_err() {
            let _ =
                focus_research_decisions_depth_live::a729_focus_item_hook::cancel_active_epoch();
        }
        result
    }

    fn uninstall(self) -> Result<(), String> {
        let _ = focus_research_decisions_depth_live::a729_focus_item_hook::cancel_active_epoch();
        focus_research_decisions_depth_live::a729_focus_item_hook::uninstall()
    }

    fn reset_campaign(&mut self) -> Result<(), String> {
        focus_research_decisions_depth_live::a729_focus_item_hook::cancel_active_epoch()?;
        self.generation = None;
        self.frame_hour = None;
        self.country_tag = None;
        self.country_object = None;
        self.pending_gui_join = None;
        Ok(())
    }
}

#[cfg(windows)]
fn write_synced_new_file(path: &std::path::Path, bytes: &[u8]) -> Result<(), String> {
    use std::io::Write;
    let temporary = path.with_extension(format!(
        "{}.tmp",
        path.extension()
            .and_then(|value| value.to_str())
            .unwrap_or("evidence")
    ));
    let mut file = std::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&temporary)
        .map_err(|error| format!("A31 temporary evidence open failed: {error}"))?;
    file.write_all(bytes)
        .and_then(|_| file.sync_all())
        .map_err(|error| format!("A31 evidence write/sync failed: {error}"))?;
    drop(file);
    std::fs::rename(&temporary, path)
        .map_err(|error| format!("A31 evidence commit failed: {error}"))
}

#[cfg(windows)]
fn hex_bytes(bytes: &[u8]) -> String {
    let mut output = String::with_capacity(bytes.len() * 2);
    for byte in bytes {
        use std::fmt::Write as _;
        let _ = write!(output, "{byte:02x}");
    }
    output
}

pub fn a729_decision_panel_selected_present_stamp(
    frame_hour: u64,
    selected: &portable_render_stream::SelectedPresentReceipt,
    qpc_frequency_hz: u64,
) -> Result<decision_panel_live_lane::SelectedPresentStamp, String> {
    if compact_contract::calendar::hour_index_from_raw(selected.selected_game_hour_raw)
        .and_then(|value| u64::try_from(value).ok())
        != Some(frame_hour)
        || selected.selected_qpc_ticks == 0
        || selected.selection_cutoff_qpc_ticks < selected.selected_qpc_ticks
        || qpc_frequency_hz == 0
    {
        return Err("A34 selected-Present receipt hour/order/QPC is invalid".into());
    }
    Ok(decision_panel_live_lane::SelectedPresentStamp {
        frame_hour,
        selected_present_ordinal: selected.selected_ordinal,
        selected_present_qpc_ticks: selected.selected_qpc_ticks,
        qpc_frequency_hz,
        selected_present_receipt_sha256: selected.computed_sha256()?,
    })
}

pub fn authenticate_a38_definition_winner(
    definition: &scripted_state_flags_variables_live_lane::ScriptedGuiDefinitionIdentity,
    winners: &duplex_client::AdmittedEarlyRenderVfsIndex,
) -> Result<(), String> {
    let position = winners
        .ordered_rows
        .binary_search_by(|row| {
            row.normalized_game_relative_path
                .as_str()
                .cmp(definition.logical_path.as_str())
        })
        .map_err(|_| "A38 definition is absent from the admitted effective-VFS winners")?;
    let winner = &winners.ordered_rows[position];
    let layer_kind_matches = matches!(
        (winner.layer_kind, definition.layer_kind.as_str()),
        (
            compact_contract::early_render_vfs::EarlyRenderVfsLayerKind::Base,
            "base"
        ) | (
            compact_contract::early_render_vfs::EarlyRenderVfsLayerKind::IntegratedDlc,
            "integrated_dlc"
        ) | (
            compact_contract::early_render_vfs::EarlyRenderVfsLayerKind::Dlc,
            "dlc"
        ) | (
            compact_contract::early_render_vfs::EarlyRenderVfsLayerKind::Mod,
            "mod"
        )
    );
    if winner.winning_layer_ordinal != definition.winning_layer_ordinal
        || !layer_kind_matches
        || winner.layer_id != definition.layer_id
        || winner.source_sha256 != definition.source_sha256
        || winner.payload_sha256 != definition.payload_sha256
        || winner.payload_bytes != definition.payload_bytes
    {
        return Err(
            "A38 definition identity disagrees with the admitted effective-VFS winner".into(),
        );
    }
    Ok(())
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AuthenticatedA38DefinitionContract {
    pub definition: scripted_state_flags_variables_live_lane::ScriptedGuiDefinitionIdentity,
    pub expected_trigger_names: BTreeSet<String>,
    pub expected_property_keys: BTreeSet<(String, String)>,
    pub expected_effect_names: BTreeSet<String>,
}

pub struct AuthenticatedA38FrameSource<'a> {
    frame: scripted_state_flags_variables_live_lane::ScriptedGuiRuntimeFrame,
    winners: &'a duplex_client::AdmittedEarlyRenderVfsIndex,
    authenticated_registry: &'a BTreeMap<String, AuthenticatedA38DefinitionContract>,
    authenticated_manager: &'a AuthenticatedA38CurrentManagerSnapshot,
    authenticated_tombstones: BTreeSet<(String, String)>,
    ui14_engine_vfs_sha256: [u8; 32],
    static_authority_receipt_sha256: [u8; 32],
    producer_manifest_sha256: [u8; 32],
}

const A38_MANAGER_UPDATE_OWNER_RVA: u64 = 0x00b7_9c90;
const A38_INVALID_REBUILD_GENERATION: u32 = u32::MAX;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AuthenticatedA38CurrentManagerSnapshot {
    process_start_nonce: u64,
    hour_index: u64,
    producer_sequence: u64,
    producer_record_identity: String,
    rebuild_generation: u32,
    manager_order: Vec<(String, String)>,
}

#[cfg(test)]
fn authenticate_a38_current_manager_snapshot(
    process_start_nonce: u64,
    hour_index: u64,
    producer_sequence: u64,
    producer_record_identity: String,
    completed_owner_rva: u64,
    rebuild_generation_before: u32,
    rebuild_generation_after: u32,
    manager_order: Vec<(String, String)>,
) -> Result<AuthenticatedA38CurrentManagerSnapshot, String> {
    if process_start_nonce == 0
        || producer_sequence == 0
        || producer_record_identity.trim().is_empty()
        || completed_owner_rva != A38_MANAGER_UPDATE_OWNER_RVA
        || rebuild_generation_before == A38_INVALID_REBUILD_GENERATION
        || rebuild_generation_before != rebuild_generation_after
    {
        return Err(
            "A38 current manager observation did not cross the exact post-update owner".into(),
        );
    }
    let unique = manager_order.iter().cloned().collect::<BTreeSet<_>>();
    if unique.len() != manager_order.len() {
        return Err("A38 current manager snapshot repeats an identity".into());
    }
    Ok(AuthenticatedA38CurrentManagerSnapshot {
        process_start_nonce,
        hour_index,
        producer_sequence,
        producer_record_identity,
        rebuild_generation: rebuild_generation_after,
        manager_order,
    })
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AuthenticatedA38CommittedManagerSnapshot {
    process_start_nonce: u64,
    hour_index: u64,
    producer_sequence: u64,
    producer_record_identity: String,
    executable_sha256: [u8; 32],
    static_authority_receipt_sha256: [u8; 32],
    manager_order: Vec<(String, String)>,
}

impl<'a> AuthenticatedA38FrameSource<'a> {
    pub fn new(
        frame: scripted_state_flags_variables_live_lane::ScriptedGuiRuntimeFrame,
        winners: &'a duplex_client::AdmittedEarlyRenderVfsIndex,
        authenticated_registry: &'a BTreeMap<String, AuthenticatedA38DefinitionContract>,
        authenticated_manager: &'a AuthenticatedA38CurrentManagerSnapshot,
        authenticated_previous: Option<&AuthenticatedA38CommittedManagerSnapshot>,
        ui14_engine_vfs_sha256: [u8; 32],
        static_authority_receipt_sha256: [u8; 32],
        producer_manifest_sha256: [u8; 32],
    ) -> Result<Self, String> {
        if winners.process_start_nonce == 0
            || winners.ordered_rows.is_empty()
            || authenticated_registry.is_empty()
            || ui14_engine_vfs_sha256 == [0; 32]
            || static_authority_receipt_sha256
                != scripted_state_flags_variables_live_lane::EXACT20_A38_STATIC_AUTHORITY_RECEIPT_SHA256
            || producer_manifest_sha256 == [0; 32]
            || frame.executable_sha256
                != scripted_state_flags_variables_live_lane::scripted_gui_runtime_carrier::EXACT_A729_EXECUTABLE_SHA256
            || frame.ui14_engine_vfs_sha256 != ui14_engine_vfs_sha256
            || frame.static_authority_receipt_sha256 != static_authority_receipt_sha256
            || frame.producer_manifest_sha256 != producer_manifest_sha256
            || authenticated_manager.process_start_nonce != winners.process_start_nonce
            || authenticated_manager.hour_index != frame.hour_index
            || authenticated_manager.producer_record_identity != frame.producer_record_identity
            || authenticated_manager.rebuild_generation == A38_INVALID_REBUILD_GENERATION
        {
            return Err("A38 authenticated frame source identity is incomplete".into());
        }
        let current_keys = authenticated_manager
            .manager_order
            .iter()
            .cloned()
            .collect::<BTreeSet<_>>();
        if current_keys.len() != authenticated_manager.manager_order.len() {
            return Err("A38 current manager snapshot repeats an identity".into());
        }
        let previous_keys = if let Some(previous) = authenticated_previous {
            if previous.process_start_nonce != winners.process_start_nonce
                || previous.executable_sha256 != frame.executable_sha256
                || previous.static_authority_receipt_sha256 != frame.static_authority_receipt_sha256
                || previous.hour_index > frame.hour_index
                || previous.producer_sequence >= authenticated_manager.producer_sequence
                || previous.producer_record_identity == frame.producer_record_identity
            {
                return Err(
                    "A38 preceding committed manager receipt is not sequence-compatible".into(),
                );
            }
            let keys = previous
                .manager_order
                .iter()
                .cloned()
                .collect::<BTreeSet<_>>();
            if keys.len() != previous.manager_order.len() {
                return Err("A38 preceding committed manager receipt repeats an identity".into());
            }
            keys
        } else {
            BTreeSet::new()
        };
        let source = Self {
            frame,
            winners,
            authenticated_registry,
            authenticated_manager,
            authenticated_tombstones: previous_keys.difference(&current_keys).cloned().collect(),
            ui14_engine_vfs_sha256,
            static_authority_receipt_sha256,
            producer_manifest_sha256,
        };
        source.authenticate_complete_frame(&source.frame)?;
        Ok(source)
    }

    pub fn committed_manager_snapshot(&self) -> AuthenticatedA38CommittedManagerSnapshot {
        AuthenticatedA38CommittedManagerSnapshot {
            process_start_nonce: self.winners.process_start_nonce,
            hour_index: self.frame.hour_index,
            producer_sequence: self.authenticated_manager.producer_sequence,
            producer_record_identity: self.frame.producer_record_identity.clone(),
            executable_sha256: self.frame.executable_sha256,
            static_authority_receipt_sha256: self.frame.static_authority_receipt_sha256,
            manager_order: self.authenticated_manager.manager_order.clone(),
        }
    }

    fn authenticate_complete_frame(
        &self,
        frame: &scripted_state_flags_variables_live_lane::ScriptedGuiRuntimeFrame,
    ) -> Result<(), String> {
        if frame.rows.len() != self.authenticated_manager.manager_order.len() {
            return Err("A38 frame does not cover the authenticated manager order".into());
        }
        let mut expected_keys = BTreeSet::new();
        for (position, (row, expected_key)) in frame
            .rows
            .iter()
            .zip(self.authenticated_manager.manager_order.iter())
            .enumerate()
        {
            if row.manager_order_index as usize != position
                || (&row.country_tag, &row.gui_id) != (&expected_key.0, &expected_key.1)
                || !expected_keys.insert(expected_key.clone())
            {
                return Err("A38 row order/key differs from authenticated manager snapshot".into());
            }
            let contract = self
                .authenticated_registry
                .get(&row.gui_id)
                .ok_or_else(|| {
                    "A38 gui id is absent from the authenticated effective registry".to_string()
                })?;
            if row.definition != contract.definition
                || row.expected_trigger_names != contract.expected_trigger_names
                || row.expected_property_keys != contract.expected_property_keys
                || row.expected_effect_names != contract.expected_effect_names
            {
                return Err("A38 row disagrees with authenticated definition contract".into());
            }
            authenticate_a38_definition_winner(&row.definition, self.winners)?;
        }
        if frame.expected_gui_keys != expected_keys {
            return Err("A38 frame coverage differs from authenticated manager snapshot".into());
        }
        if &frame.tombstones != &self.authenticated_tombstones {
            return Err(
                "A38 tombstones differ from the authenticated prior manager snapshot".into(),
            );
        }
        Ok(())
    }
}

impl scripted_state_flags_variables_live_lane::ExactScriptedGuiRuntimeSource
    for AuthenticatedA38FrameSource<'_>
{
    fn is_exact_a729_scripted_gui_source(&self) -> bool {
        self.frame.executable_sha256
            == scripted_state_flags_variables_live_lane::scripted_gui_runtime_carrier::EXACT_A729_EXECUTABLE_SHA256
    }

    fn executable_sha256(&self) -> [u8; 32] {
        scripted_state_flags_variables_live_lane::scripted_gui_runtime_carrier::EXACT_A729_EXECUTABLE_SHA256
    }

    fn ui14_engine_vfs_sha256(&self) -> [u8; 32] {
        self.ui14_engine_vfs_sha256
    }

    fn static_authority_receipt_sha256(&self) -> [u8; 32] {
        self.static_authority_receipt_sha256
    }

    fn producer_manifest_sha256(&self) -> [u8; 32] {
        self.producer_manifest_sha256
    }

    fn collect_runtime_frame(
        &self,
        hour_index: u64,
    ) -> Result<scripted_state_flags_variables_live_lane::ScriptedGuiRuntimeFrame, String> {
        if self.frame.hour_index != hour_index {
            return Err("A38 frame hour differs from requested collector hour".into());
        }
        Ok(self.frame.clone())
    }

    fn authenticate_runtime_frame(
        &self,
        frame: &scripted_state_flags_variables_live_lane::ScriptedGuiRuntimeFrame,
    ) -> Result<(), String> {
        if frame != &self.frame {
            return Err("A38 source returned a frame other than its authenticated snapshot".into());
        }
        self.authenticate_complete_frame(frame)
    }
}

pub fn collect_a38_exact_scripted_gui_runtime<
    S: scripted_state_flags_variables_live_lane::ExactScriptedGuiRuntimeSource,
>(
    source: &S,
    frame_hour: u64,
) -> Result<
    scripted_state_flags_variables_live_lane::ScriptedGuiRuntimeFrame,
    scripted_state_flags_variables_live_lane::scripted_gui_runtime_carrier::CarrierError,
> {
    scripted_state_flags_variables_live_lane::collect_and_validate_scripted_gui_runtime(
        source, frame_hour,
    )
}

pub fn encode_a38_exact_scripted_gui_runtime_frame<
    S: scripted_state_flags_variables_live_lane::ExactScriptedGuiRuntimeSource,
>(
    source: &S,
    frame_hour: u64,
    elapsed_wall_ms: u64,
) -> Result<Vec<u8>, scripted_state_flags_variables_live_lane::ScriptedGuiCompactError> {
    scripted_state_flags_variables_live_lane::collect_and_encode_scripted_gui_runtime(
        source,
        frame_hour,
        elapsed_wall_ms,
    )
}

#[cfg(windows)]
pub fn encode_a38_hook_owned_scripted_gui_transaction(
    receipt: scripted_state_flags_variables_live_lane::a729_scripted_gui_runtime::AuthenticatedA38OwnerReceipt,
    winners: &duplex_client::AdmittedEarlyRenderVfsIndex,
    authenticated_registry: &BTreeMap<String, AuthenticatedA38DefinitionContract>,
    authenticated_previous: Option<&AuthenticatedA38CommittedManagerSnapshot>,
    ui14_engine_vfs_sha256: [u8; 32],
    static_authority_receipt_sha256: [u8; 32],
    producer_manifest_sha256: [u8; 32],
    elapsed_wall_ms: u64,
) -> Result<(Vec<u8>, AuthenticatedA38CommittedManagerSnapshot), String> {
    if receipt.process_start_nonce != winners.process_start_nonce
        || receipt.completed_owner_rva
            != scripted_state_flags_variables_live_lane::a729_scripted_gui_runtime::OWNER_RVA
        || receipt.rebuild_generation
            == scripted_state_flags_variables_live_lane::a729_scripted_gui_runtime::INVALID_REBUILD_GENERATION
        || receipt.producer_sequence == 0
        || compact_contract::calendar::hour_index_from_raw(receipt.game_hour_raw)
            .and_then(|value| u64::try_from(value).ok())
            != Some(receipt.hour_index)
        || receipt.hour_index != receipt.frame.hour_index
        || receipt.producer_record_identity != receipt.frame.producer_record_identity
        || receipt.manager_order.len() != receipt.frame.rows.len()
    {
        return Err("A38 hook-owned receipt identity is incomplete".into());
    }
    let authenticated_manager = AuthenticatedA38CurrentManagerSnapshot {
        process_start_nonce: receipt.process_start_nonce,
        hour_index: receipt.hour_index,
        producer_sequence: receipt.producer_sequence,
        producer_record_identity: receipt.producer_record_identity,
        rebuild_generation: receipt.rebuild_generation,
        manager_order: receipt.manager_order,
    };
    let source = AuthenticatedA38FrameSource::new(
        receipt.frame,
        winners,
        authenticated_registry,
        &authenticated_manager,
        authenticated_previous,
        ui14_engine_vfs_sha256,
        static_authority_receipt_sha256,
        producer_manifest_sha256,
    )?;
    let payload = encode_a38_exact_scripted_gui_runtime_frame(
        &source,
        authenticated_manager.hour_index,
        elapsed_wall_ms,
    )
    .map_err(|error| format!("A38 d84 compact encoding failed: {error:?}"))?;
    let committed = source.committed_manager_snapshot();
    Ok((payload, committed))
}

#[cfg(windows)]
#[derive(Debug, PartialEq, Eq)]
pub struct A38HookOwnedScriptedGuiTransactionReservation {
    pub reservation_id: u64,
    pub payload: Vec<u8>,
    pub next_committed_manager_snapshot: AuthenticatedA38CommittedManagerSnapshot,
}

#[cfg(windows)]
pub fn reserve_a38_hook_owned_scripted_gui_transaction(
    frame_hour: u64,
    winners: &duplex_client::AdmittedEarlyRenderVfsIndex,
    authenticated_registry: &BTreeMap<String, AuthenticatedA38DefinitionContract>,
    authenticated_previous: Option<&AuthenticatedA38CommittedManagerSnapshot>,
    ui14_engine_vfs_sha256: [u8; 32],
    static_authority_receipt_sha256: [u8; 32],
    producer_manifest_sha256: [u8; 32],
    elapsed_wall_ms: u64,
) -> Result<Option<A38HookOwnedScriptedGuiTransactionReservation>, String> {
    let Some(reservation) =
        scripted_state_flags_variables_live_lane::a729_scripted_gui_runtime::reserve_receipt(
            frame_hour,
        )?
    else {
        return Ok(None);
    };
    let reservation_id = reservation.reservation_id;
    match encode_a38_hook_owned_scripted_gui_transaction(
        reservation.receipt,
        winners,
        authenticated_registry,
        authenticated_previous,
        ui14_engine_vfs_sha256,
        static_authority_receipt_sha256,
        producer_manifest_sha256,
        elapsed_wall_ms,
    ) {
        Ok((payload, next_committed_manager_snapshot)) => {
            Ok(Some(A38HookOwnedScriptedGuiTransactionReservation {
                reservation_id,
                payload,
                next_committed_manager_snapshot,
            }))
        }
        Err(encode_error) => {
            match scripted_state_flags_variables_live_lane::a729_scripted_gui_runtime::rollback_receipt(
                reservation_id,
            ) {
                Ok(()) => Err(encode_error),
                Err(rollback_error) => Err(format!(
                    "{encode_error}; A38 receipt rollback after encoding failure also failed: {rollback_error}"
                )),
            }
        }
    }
}

#[cfg(windows)]
pub fn commit_a38_hook_owned_scripted_gui_transaction(reservation_id: u64) -> Result<(), String> {
    scripted_state_flags_variables_live_lane::a729_scripted_gui_runtime::commit_receipt(
        reservation_id,
    )
}

#[cfg(windows)]
pub fn rollback_a38_hook_owned_scripted_gui_transaction(reservation_id: u64) -> Result<(), String> {
    scripted_state_flags_variables_live_lane::a729_scripted_gui_runtime::rollback_receipt(
        reservation_id,
    )
}

#[cfg(windows)]
pub fn drain_a38_hook_owned_scripted_gui_transaction(
    frame_hour: u64,
    winners: &duplex_client::AdmittedEarlyRenderVfsIndex,
    authenticated_registry: &BTreeMap<String, AuthenticatedA38DefinitionContract>,
    authenticated_previous: Option<&AuthenticatedA38CommittedManagerSnapshot>,
    ui14_engine_vfs_sha256: [u8; 32],
    static_authority_receipt_sha256: [u8; 32],
    producer_manifest_sha256: [u8; 32],
    elapsed_wall_ms: u64,
) -> Result<Option<(Vec<u8>, AuthenticatedA38CommittedManagerSnapshot)>, String> {
    let Some(reservation) = reserve_a38_hook_owned_scripted_gui_transaction(
        frame_hour,
        winners,
        authenticated_registry,
        authenticated_previous,
        ui14_engine_vfs_sha256,
        static_authority_receipt_sha256,
        producer_manifest_sha256,
        elapsed_wall_ms,
    )?
    else {
        return Ok(None);
    };
    commit_a38_hook_owned_scripted_gui_transaction(reservation.reservation_id)?;
    Ok(Some((
        reservation.payload,
        reservation.next_committed_manager_snapshot,
    )))
}

#[cfg(windows)]
pub fn install_a38_scripted_gui_fixture_observer(
    compiler: std::sync::Arc<
        dyn scripted_state_flags_variables_live_lane::a729_scripted_gui_runtime::ExactA729A38TransactionCompiler,
    >,
    game_hour_raw: i32,
) -> Result<
    scripted_state_flags_variables_live_lane::a729_scripted_gui_runtime::InstalledAddresses,
    String,
> {
    scripted_state_flags_variables_live_lane::a729_scripted_gui_runtime::register_transaction_compiler(
        compiler,
    )?;
    scripted_state_flags_variables_live_lane::a729_scripted_gui_runtime::publish_expected_game_hour_raw(
        game_hour_raw,
    );
    scripted_state_flags_variables_live_lane::a729_scripted_gui_runtime::install_for_fixture()
}

#[cfg(test)]
mod a38_production_admission_tests {
    use super::*;
    use compact_contract::early_render_vfs::{EarlyRenderVfsLayerKind, EarlyRenderVfsWinnerRowV1};
    use scripted_state_flags_variables_live_lane::{
        EngineResult, ScriptedGuiDefinitionIdentity, ScriptedGuiRuntimeFrame,
        ScriptedGuiRuntimeRow, EXACT20_A38_STATIC_AUTHORITY_RECEIPT_SHA256,
    };

    const GUI_ID: &str = "fixture_gui";
    const LOGICAL_PATH: &str = "common/scripted_guis/fixture.txt";

    fn definition() -> ScriptedGuiDefinitionIdentity {
        ScriptedGuiDefinitionIdentity {
            logical_path: LOGICAL_PATH.into(),
            winning_layer_ordinal: 0,
            layer_kind: "base".into(),
            layer_id: "base".into(),
            source_sha256: [3; 32],
            payload_sha256: [4; 32],
            payload_bytes: 128,
            context_type: "country".into(),
            window_name: "fixture_window".into(),
        }
    }

    fn admitted_winners() -> duplex_client::AdmittedEarlyRenderVfsIndex {
        duplex_client::AdmittedEarlyRenderVfsIndex {
            process_start_nonce: 91,
            effective_vfs_manifest_sha256: [7; 32],
            ordered_rows_sha256: [8; 32],
            ordered_rows: vec![EarlyRenderVfsWinnerRowV1 {
                normalized_game_relative_path: LOGICAL_PATH.into(),
                winning_layer_ordinal: 0,
                layer_kind: EarlyRenderVfsLayerKind::Base,
                layer_id: "base".into(),
                source_sha256: [3; 32],
                payload_sha256: [4; 32],
                payload_bytes: 128,
                source_container_sha256: None,
                source_byte_offset: 0,
                source_byte_count: 128,
                source_range_sha256: [4; 32],
            }],
        }
    }

    fn registry() -> BTreeMap<String, AuthenticatedA38DefinitionContract> {
        BTreeMap::from([(
            GUI_ID.into(),
            AuthenticatedA38DefinitionContract {
                definition: definition(),
                expected_trigger_names: BTreeSet::from(["fixture_visible".into()]),
                expected_property_keys: BTreeSet::from([("fixture_icon".into(), "image".into())]),
                expected_effect_names: BTreeSet::from(["fixture_effect".into()]),
            },
        )])
    }

    fn frame() -> ScriptedGuiRuntimeFrame {
        ScriptedGuiRuntimeFrame {
            hour_index: 44,
            producer_record_identity: "a38:44:fixture".into(),
            executable_sha256: scripted_state_flags_variables_live_lane::scripted_gui_runtime_carrier::EXACT_A729_EXECUTABLE_SHA256,
            ui14_engine_vfs_sha256: [7; 32],
            static_authority_receipt_sha256: EXACT20_A38_STATIC_AUTHORITY_RECEIPT_SHA256,
            producer_manifest_sha256: [9; 32],
            expected_gui_keys: BTreeSet::from([("USA".into(), GUI_ID.into())]),
            rows: vec![ScriptedGuiRuntimeRow {
                manager_order_index: 0,
                country_tag: "USA".into(),
                gui_id: GUI_ID.into(),
                definition: definition(),
                visible: EngineResult::Value(true),
                enabled: EngineResult::Value(true),
                expected_trigger_names: BTreeSet::from(["fixture_visible".into()]),
                triggers: BTreeMap::from([("fixture_visible".into(), EngineResult::Value(true))]),
                expected_property_keys: BTreeSet::from([("fixture_icon".into(), "image".into())]),
                properties: BTreeMap::from([(("fixture_icon".into(), "image".into()), EngineResult::Value("GFX_fixture".into()))]),
                expected_effect_names: BTreeSet::from(["fixture_effect".into()]),
                effect_enabled: BTreeMap::from([("fixture_effect".into(), EngineResult::Value(false))]),
            }],
            tombstones: BTreeSet::new(),
        }
    }

    fn current_manager(
        frame: &ScriptedGuiRuntimeFrame,
        producer_sequence: u64,
        manager_order: Vec<(String, String)>,
    ) -> AuthenticatedA38CurrentManagerSnapshot {
        authenticate_a38_current_manager_snapshot(
            91,
            frame.hour_index,
            producer_sequence,
            frame.producer_record_identity.clone(),
            A38_MANAGER_UPDATE_OWNER_RVA,
            7,
            7,
            manager_order,
        )
        .unwrap()
    }

    #[test]
    fn h4ct_winner_registry_and_manager_order_admit_complete_frame() {
        let winners = admitted_winners();
        let registry = registry();
        let frame = frame();
        let manager = current_manager(&frame, 1, vec![("USA".into(), GUI_ID.into())]);
        let source = AuthenticatedA38FrameSource::new(
            frame,
            &winners,
            &registry,
            &manager,
            None,
            [7; 32],
            EXACT20_A38_STATIC_AUTHORITY_RECEIPT_SHA256,
            [9; 32],
        )
        .unwrap();
        let admitted = collect_a38_exact_scripted_gui_runtime(&source, 44).unwrap();
        assert_eq!(admitted.rows[0].properties.len(), 1);
    }

    #[test]
    fn self_declared_winner_hash_cannot_cross_production_adapter() {
        let winners = admitted_winners();
        let registry = registry();
        let mut untrusted = frame();
        untrusted.rows[0].definition.payload_sha256 = [5; 32];
        let manager = current_manager(&untrusted, 1, vec![("USA".into(), GUI_ID.into())]);
        assert!(AuthenticatedA38FrameSource::new(
            untrusted,
            &winners,
            &registry,
            &manager,
            None,
            [7; 32],
            EXACT20_A38_STATIC_AUTHORITY_RECEIPT_SHA256,
            [9; 32],
        )
        .is_err());
    }

    #[test]
    fn incomplete_manager_order_or_registry_result_inventory_fails_closed() {
        let winners = admitted_winners();
        let complete_registry = registry();
        let complete_frame = frame();
        let empty_manager = current_manager(&complete_frame, 1, Vec::new());
        assert!(AuthenticatedA38FrameSource::new(
            complete_frame,
            &winners,
            &complete_registry,
            &empty_manager,
            None,
            [7; 32],
            EXACT20_A38_STATIC_AUTHORITY_RECEIPT_SHA256,
            [9; 32],
        )
        .is_err());

        let manager_order = vec![("USA".into(), GUI_ID.into())];
        let mut incomplete_registry = registry();
        incomplete_registry
            .get_mut(GUI_ID)
            .unwrap()
            .expected_effect_names
            .clear();
        let incomplete_frame = frame();
        let manager = current_manager(&incomplete_frame, 1, manager_order.clone());
        assert!(AuthenticatedA38FrameSource::new(
            incomplete_frame,
            &winners,
            &incomplete_registry,
            &manager,
            None,
            [7; 32],
            EXACT20_A38_STATIC_AUTHORITY_RECEIPT_SHA256,
            [9; 32],
        )
        .is_err());

        let mut forged_tombstone = frame();
        forged_tombstone
            .tombstones
            .insert(("CAN".into(), "removed_gui".into()));
        let manager = current_manager(&forged_tombstone, 1, manager_order.clone());
        assert!(AuthenticatedA38FrameSource::new(
            forged_tombstone,
            &winners,
            &complete_registry,
            &manager,
            None,
            [7; 32],
            EXACT20_A38_STATIC_AUTHORITY_RECEIPT_SHA256,
            [9; 32],
        )
        .is_err());

        let mut prior_frame = frame();
        prior_frame.hour_index = 43;
        prior_frame.producer_record_identity = "a38:43:fixture".into();
        let prior_manager = current_manager(&prior_frame, 1, manager_order.clone());
        let prior_source = AuthenticatedA38FrameSource::new(
            prior_frame,
            &winners,
            &complete_registry,
            &prior_manager,
            None,
            [7; 32],
            EXACT20_A38_STATIC_AUTHORITY_RECEIPT_SHA256,
            [9; 32],
        )
        .unwrap();
        let prior = prior_source.committed_manager_snapshot();
        let mut exact_tombstone = frame();
        exact_tombstone.rows.clear();
        exact_tombstone.expected_gui_keys.clear();
        exact_tombstone
            .tombstones
            .insert(("USA".into(), GUI_ID.into()));
        let removed_manager = current_manager(&exact_tombstone, 2, Vec::new());
        assert!(AuthenticatedA38FrameSource::new(
            exact_tombstone,
            &winners,
            &complete_registry,
            &removed_manager,
            Some(&prior),
            [7; 32],
            EXACT20_A38_STATIC_AUTHORITY_RECEIPT_SHA256,
            [9; 32],
        )
        .is_ok());

        let mut cross_process_frame = frame();
        cross_process_frame.rows.clear();
        cross_process_frame.expected_gui_keys.clear();
        cross_process_frame
            .tombstones
            .insert(("USA".into(), GUI_ID.into()));
        let mut cross_process_winners = admitted_winners();
        cross_process_winners.process_start_nonce += 1;
        let cross_process_manager = authenticate_a38_current_manager_snapshot(
            cross_process_winners.process_start_nonce,
            cross_process_frame.hour_index,
            2,
            cross_process_frame.producer_record_identity.clone(),
            A38_MANAGER_UPDATE_OWNER_RVA,
            7,
            7,
            Vec::new(),
        )
        .unwrap();
        assert!(AuthenticatedA38FrameSource::new(
            cross_process_frame,
            &cross_process_winners,
            &complete_registry,
            &cross_process_manager,
            Some(&prior),
            [7; 32],
            EXACT20_A38_STATIC_AUTHORITY_RECEIPT_SHA256,
            [9; 32],
        )
        .is_err());
    }

    #[test]
    fn cold_or_replayed_manager_receipts_cannot_drive_tombstones() {
        let observation_frame = frame();
        assert!(authenticate_a38_current_manager_snapshot(
            91,
            observation_frame.hour_index,
            1,
            observation_frame.producer_record_identity.clone(),
            0x00b7_9dd0,
            7,
            7,
            vec![("USA".into(), GUI_ID.into())],
        )
        .is_err());
        assert!(authenticate_a38_current_manager_snapshot(
            91,
            observation_frame.hour_index,
            1,
            observation_frame.producer_record_identity,
            A38_MANAGER_UPDATE_OWNER_RVA,
            A38_INVALID_REBUILD_GENERATION,
            A38_INVALID_REBUILD_GENERATION,
            vec![("USA".into(), GUI_ID.into())],
        )
        .is_err());

        let winners = admitted_winners();
        let registry = registry();
        let prior_frame = frame();
        let prior_manager = current_manager(&prior_frame, 2, vec![("USA".into(), GUI_ID.into())]);
        let prior_source = AuthenticatedA38FrameSource::new(
            prior_frame,
            &winners,
            &registry,
            &prior_manager,
            None,
            [7; 32],
            EXACT20_A38_STATIC_AUTHORITY_RECEIPT_SHA256,
            [9; 32],
        )
        .unwrap();
        let prior = prior_source.committed_manager_snapshot();
        let mut replayed = frame();
        replayed.producer_record_identity = "a38:44:replayed".into();
        replayed.rows.clear();
        replayed.expected_gui_keys.clear();
        replayed.tombstones.insert(("USA".into(), GUI_ID.into()));
        let replayed_manager = current_manager(&replayed, 2, Vec::new());
        assert!(AuthenticatedA38FrameSource::new(
            replayed,
            &winners,
            &registry,
            &replayed_manager,
            Some(&prior),
            [7; 32],
            EXACT20_A38_STATIC_AUTHORITY_RECEIPT_SHA256,
            [9; 32],
        )
        .is_err());
    }
}

pub fn encode_xui21_exact_fired_event_frame<
    S: fired_event_popups_live_lane::ExactFiredEventSource,
>(
    source: &S,
    frame_hour: u64,
    elapsed_wall_ms: u64,
) -> Result<Vec<u8>, fired_event_popups_live_lane::CarrierError> {
    fired_event_popups_live_lane::collect_and_encode(source, frame_hour, elapsed_wall_ms)
}

#[cfg(windows)]
pub fn install_xui21_a729_fired_event_runtime(
    image_base: usize,
    bridge: std::sync::Arc<dyn fired_event_popups_live_lane::a729_runtime::A729EventBridge>,
    capacity: usize,
) -> Result<(), String> {
    fired_event_popups_live_lane::a729_runtime::install_a729_event_runtime(
        image_base, bridge, capacity,
    )
}

#[cfg(windows)]
pub fn encode_xui21_installed_fired_event_frame(
    frame_hour: u64,
    elapsed_wall_ms: u64,
) -> Result<Vec<u8>, fired_event_popups_live_lane::CarrierError> {
    let source =
        fired_event_popups_live_lane::a729_runtime::installed_source().ok_or_else(|| {
            fired_event_popups_live_lane::CarrierError::Source(
                "XUI21 exact a729 hook runtime is not installed".into(),
            )
        })?;
    encode_xui21_exact_fired_event_frame(source, frame_hour, elapsed_wall_ms)
}

#[cfg(windows)]
pub fn uninstall_xui21_a729_fired_event_runtime() -> Result<(), String> {
    fired_event_popups_live_lane::a729_runtime::uninstall_a729_event_runtime()
}

#[cfg(windows)]
#[derive(Clone, Debug, PartialEq, Eq)]
enum A38PropertySource {
    Literal(String),
    EngineMaterialized,
}

#[cfg(windows)]
#[derive(Clone, Debug, PartialEq, Eq)]
struct A38ProductionDefinition {
    contract: AuthenticatedA38DefinitionContract,
    enabled_constant: Option<bool>,
    trigger_elements: BTreeMap<String, String>,
    effect_elements: BTreeMap<String, String>,
    property_sources: BTreeMap<(String, String), A38PropertySource>,
}

#[cfg(windows)]
#[derive(Clone, Debug, PartialEq, Eq)]
struct A38AdmittedSource {
    winner: compact_contract::early_render_vfs::EarlyRenderVfsWinnerRowV1,
    bytes: Vec<u8>,
}

#[cfg(windows)]
struct A38H4ctTransactionCompiler {
    process_start_nonce: u64,
    catalog_epoch: u64,
    ui14_engine_vfs_sha256: [u8; 32],
    producer_manifest_sha256: [u8; 32],
    definitions: BTreeMap<String, A38ProductionDefinition>,
}

#[cfg(windows)]
struct A38ProductionBundle {
    compiler: std::sync::Arc<A38H4ctTransactionCompiler>,
    registry: BTreeMap<String, AuthenticatedA38DefinitionContract>,
    ui14_engine_vfs_sha256: [u8; 32],
    producer_manifest_sha256: [u8; 32],
}

#[cfg(windows)]
impl scripted_state_flags_variables_live_lane::a729_scripted_gui_runtime::ExactA729A38TransactionCompiler
    for A38H4ctTransactionCompiler
{
    fn identity_catalog_epoch(&self) -> u64 {
        self.catalog_epoch
    }

    fn compile_transaction(
        &self,
        transaction: &scripted_state_flags_variables_live_lane::a729_scripted_gui_runtime::ProcessLocalA38OwnerTransaction,
    ) -> Result<
        scripted_state_flags_variables_live_lane::a729_scripted_gui_runtime::AuthenticatedA38OwnerReceipt,
        String,
    > {
        use scripted_state_flags_variables_live_lane::a729_scripted_gui_runtime::{
            A38MaterializedValueKind, AuthenticatedA38OwnerReceipt,
        };
        use scripted_state_flags_variables_live_lane::{
            EngineResult, ScriptedGuiRuntimeFrame, ScriptedGuiRuntimeRow,
            EXACT20_A38_STATIC_AUTHORITY_RECEIPT_SHA256,
        };

        if transaction.owner_rva
            != scripted_state_flags_variables_live_lane::a729_scripted_gui_runtime::OWNER_RVA
            || transaction.game_hour_raw < 0
            || transaction.producer_sequence == 0
        {
            return Err("A38 H4CT compiler received an invalid owner transaction".into());
        }
        let hour_index = compact_contract::calendar::hour_index_from_raw(
            transaction.game_hour_raw,
        )
        .and_then(|value| u64::try_from(value).ok())
        .ok_or("A38 owner hour is outside the compact calendar")?;
        let producer_record_identity = format!(
            "a38-owner-{:016x}-{hour_index}-{}",
            self.process_start_nonce, transaction.producer_sequence
        );
        let mut rows = Vec::with_capacity(transaction.updater_order.len());
        let mut manager_order = Vec::with_capacity(transaction.updater_order.len());
        for (position, observed) in transaction.updater_order.iter().enumerate() {
            if observed.rejected || observed.live_window_after == 0 {
                return Err("A38 completed updater observation is unavailable".into());
            }
            let gui_id = observed
                .definition_id
                .as_ref()
                .filter(|value| !value.is_empty())
                .ok_or("A38 completed updater has no exact definition id")?;
            let country_tag = observed
                .country_tag
                .as_ref()
                .filter(|value| value.len() == 3)
                .ok_or("A38 completed updater has no canonical country tag")?;
            let definition = self
                .definitions
                .get(gui_id)
                .ok_or("A38 live definition is absent from the H4CT registry")?;
            let mut visible_by_name = BTreeMap::new();
            for widget in &observed.visible_widgets {
                if visible_by_name
                    .insert(widget.name.clone(), widget.visible)
                    .is_some()
                {
                    return Err("A38 visible widget name is ambiguous".into());
                }
            }
            let mut effect_by_name = BTreeMap::new();
            for widget in &observed.effect_widgets {
                if effect_by_name
                    .insert(widget.name.clone(), widget.enabled)
                    .is_some()
                {
                    return Err("A38 effect widget name is ambiguous".into());
                }
            }
            let triggers = definition
                .contract
                .expected_trigger_names
                .iter()
                .map(|name| {
                    let value = definition
                        .trigger_elements
                        .get(name)
                        .and_then(|element| {
                            if name.ends_with("_visible") {
                                visible_by_name.get(element)
                            } else {
                                effect_by_name.get(element)
                            }
                        })
                        .copied()
                        .map(EngineResult::Value)
                        .unwrap_or_else(|| {
                            EngineResult::Unavailable(
                                "engine_trigger_consumer_not_observed".into(),
                            )
                        });
                    (name.clone(), value)
                })
                .collect();
            let effect_enabled = definition
                .contract
                .expected_effect_names
                .iter()
                .map(|name| {
                    let value = definition
                        .effect_elements
                        .get(name)
                        .and_then(|element| effect_by_name.get(element))
                        .copied()
                        .map(EngineResult::Value)
                        .unwrap_or_else(|| {
                            EngineResult::Unavailable(
                                "engine_effect_consumer_not_observed".into(),
                            )
                        });
                    (name.clone(), value)
                })
                .collect();
            let mut properties = BTreeMap::new();
            for (key, source) in &definition.property_sources {
                let result = match source {
                    A38PropertySource::Literal(value) => EngineResult::Value(value.clone()),
                    A38PropertySource::EngineMaterialized => {
                        let matching_targets = observed
                            .property_bindings
                            .iter()
                            .filter(|binding| {
                                binding.authored_element == key.0
                                    && binding.authored_property == key.1
                            })
                            .map(|binding| binding.target_ptr)
                            .collect::<BTreeSet<_>>();
                        let matching_values = observed
                            .values
                            .iter()
                            .filter(|value| {
                                matching_targets.contains(&value.widget_ptr)
                                    && matches!(
                                        value.kind,
                                        A38MaterializedValueKind::PrimaryText
                                            | A38MaterializedValueKind::Sprite
                                    )
                            })
                            .map(|value| value.value.clone())
                            .collect::<BTreeSet<_>>();
                        if matching_targets.len() == 1 && matching_values.len() == 1 {
                            EngineResult::Value(
                                matching_values.into_iter().next().expect("one value"),
                            )
                        } else {
                            EngineResult::Unavailable(
                                "engine_property_consumer_not_observed".into(),
                            )
                        }
                    }
                };
                properties.insert(key.clone(), result);
            }
            let key = (country_tag.clone(), gui_id.clone());
            manager_order.push(key.clone());
            rows.push(ScriptedGuiRuntimeRow {
                manager_order_index: u32::try_from(position)
                    .map_err(|_| "A38 manager order exceeds u32")?,
                country_tag: country_tag.clone(),
                gui_id: gui_id.clone(),
                definition: definition.contract.definition.clone(),
                visible: EngineResult::Value(
                    observed.root_visible.ok_or("A38 root visibility is absent")?,
                ),
                enabled: definition
                    .enabled_constant
                    .map(EngineResult::Value)
                    .unwrap_or_else(|| {
                        EngineResult::Unavailable("engine_enabled_gate_not_materialized".into())
                    }),
                expected_trigger_names: definition.contract.expected_trigger_names.clone(),
                triggers,
                expected_property_keys: definition.contract.expected_property_keys.clone(),
                properties,
                expected_effect_names: definition.contract.expected_effect_names.clone(),
                effect_enabled,
            });
        }
        let expected_gui_keys = manager_order.iter().cloned().collect::<BTreeSet<_>>();
        if expected_gui_keys.len() != manager_order.len() {
            return Err("A38 manager repeats a country/GUI identity".into());
        }
        let frame = ScriptedGuiRuntimeFrame {
            hour_index,
            producer_record_identity: producer_record_identity.clone(),
            executable_sha256: scripted_state_flags_variables_live_lane::scripted_gui_runtime_carrier::EXACT_A729_EXECUTABLE_SHA256,
            ui14_engine_vfs_sha256: self.ui14_engine_vfs_sha256,
            static_authority_receipt_sha256: EXACT20_A38_STATIC_AUTHORITY_RECEIPT_SHA256,
            producer_manifest_sha256: self.producer_manifest_sha256,
            expected_gui_keys,
            rows,
            tombstones: BTreeSet::new(),
        };
        Ok(AuthenticatedA38OwnerReceipt {
            process_start_nonce: self.process_start_nonce,
            game_hour_raw: transaction.game_hour_raw,
            hour_index,
            producer_sequence: transaction.producer_sequence,
            producer_record_identity,
            completed_owner_rva: transaction.owner_rva,
            rebuild_generation: transaction.rebuild_generation,
            manager_order,
            frame,
        })
    }
}

#[cfg(windows)]
fn build_a38_h4ct_production_bundle(
    admitted: &duplex_client::AdmittedEarlyRenderVfsIndex,
    sources: Vec<A38AdmittedSource>,
) -> Result<A38ProductionBundle, String> {
    use portable_content_manifest::paradox_text::{parse_bytes, Value};

    if admitted.process_start_nonce == 0
        || admitted.effective_vfs_manifest_sha256 == [0; 32]
        || sources.is_empty()
    {
        return Err("A38 H4CT registry inputs are incomplete".into());
    }
    let mut definitions = BTreeMap::new();
    let mut manifest = Sha256::new();
    manifest.update(b"A38-H4CT-PRODUCER-MANIFEST-V1\0");
    manifest.update(admitted.process_start_nonce.to_le_bytes());
    manifest.update(admitted.effective_vfs_manifest_sha256);
    manifest.update(admitted.ordered_rows_sha256);
    manifest.update(
        scripted_state_flags_variables_live_lane::EXACT20_A38_STATIC_AUTHORITY_RECEIPT_SHA256,
    );
    for source in sources {
        let winner_position = admitted
            .ordered_rows
            .binary_search_by(|row| {
                row.normalized_game_relative_path
                    .cmp(&source.winner.normalized_game_relative_path)
            })
            .map_err(|_| "A38 source is absent from the admitted winner index")?;
        if admitted.ordered_rows.get(winner_position) != Some(&source.winner)
            || source.bytes.len() as u64 != source.winner.payload_bytes
            || <[u8; 32]>::from(Sha256::digest(&source.bytes)) != source.winner.payload_sha256
            || !canonical_scripted_gui_source_path(&source.winner.normalized_game_relative_path)
        {
            return Err("A38 source bytes disagree with the H4CT winner".into());
        }
        update_a38_manifest_text(&mut manifest, &source.winner.normalized_game_relative_path);
        manifest.update(source.winner.payload_sha256);
        let document = parse_bytes(&source.bytes);
        if !document.recoveries.is_empty() {
            return Err("A38 scripted-GUI source required parser recovery".into());
        }
        for entry in document.root.fields() {
            let Some(key) = entry.key.as_deref() else {
                return Err("A38 scripted-GUI source has an anonymous top-level entry".into());
            };
            if key.starts_with('@') {
                continue;
            }
            if !key.eq_ignore_ascii_case("scripted_gui") {
                return Err("A38 scripted-GUI source has an unsupported top-level entry".into());
            }
            let rows = entry
                .value
                .as_object()
                .ok_or("A38 scripted_gui root is not an object")?;
            for row in rows {
                let gui_id = row
                    .key
                    .as_deref()
                    .filter(|value| !value.is_empty())
                    .ok_or("A38 scripted-GUI definition id is absent")?;
                let body = row
                    .value
                    .as_object()
                    .ok_or("A38 scripted-GUI definition is not an object")?;
                let value = Value::Object(body.to_vec());
                let context_type = value
                    .first_scalar_ci("context_type")
                    .filter(|value| !value.is_empty())
                    .unwrap_or("country")
                    .to_string();
                let window_name = value
                    .first_scalar_ci("window_name")
                    .filter(|value| !value.is_empty())
                    .unwrap_or(gui_id)
                    .to_string();
                let enabled_constant = value
                    .first_value_ci("enabled")
                    .map(a38_constant_predicate)
                    .unwrap_or(Some(true));
                let mut trigger_elements = BTreeMap::new();
                let mut expected_trigger_names = BTreeSet::new();
                if let Some(trigger_value) = value.first_value_ci("triggers") {
                    let trigger_rows = trigger_value
                        .as_object()
                        .ok_or("A38 scripted-GUI triggers block is malformed")?;
                    for trigger in trigger_rows {
                        let name = trigger
                            .key
                            .as_deref()
                            .filter(|value| !value.is_empty())
                            .ok_or("A38 scripted-GUI trigger name is absent")?;
                        expected_trigger_names.insert(name.to_string());
                        if let Some(element) = name
                            .strip_suffix("_visible")
                            .or_else(|| name.strip_suffix("_click_enabled"))
                            .filter(|value| !value.is_empty())
                        {
                            trigger_elements.insert(name.to_string(), element.to_string());
                        }
                    }
                }
                let mut effect_elements = BTreeMap::new();
                let mut expected_effect_names = BTreeSet::new();
                if let Some(effect_value) = value.first_value_ci("effects") {
                    let effect_rows = effect_value
                        .as_object()
                        .ok_or("A38 scripted-GUI effects block is malformed")?;
                    for effect in effect_rows {
                        let name = effect
                            .key
                            .as_deref()
                            .filter(|value| !value.is_empty())
                            .ok_or("A38 scripted-GUI effect name is absent")?;
                        expected_effect_names.insert(name.to_string());
                        if let Some(element) = effect
                            .value
                            .first_scalar_ci("on_click_of")
                            .filter(|value| !value.is_empty())
                        {
                            effect_elements.insert(name.to_string(), element.to_string());
                        }
                    }
                }
                let mut property_sources = BTreeMap::new();
                if let Some(property_value) = value.first_value_ci("properties") {
                    let elements = property_value
                        .as_object()
                        .ok_or("A38 scripted-GUI properties block is malformed")?;
                    for element in elements {
                        let element_name = element
                            .key
                            .as_deref()
                            .filter(|value| !value.is_empty())
                            .ok_or("A38 scripted-GUI property element is absent")?;
                        let property_rows = element
                            .value
                            .as_object()
                            .ok_or("A38 scripted-GUI property element is malformed")?;
                        for property in property_rows {
                            let property_name = property
                                .key
                                .as_deref()
                                .filter(|value| !value.is_empty())
                                .ok_or("A38 scripted-GUI property name is absent")?;
                            let raw = property
                                .value
                                .as_scalar()
                                .ok_or("A38 scripted-GUI property value is not scalar")?;
                            let source = if raw.contains('[') {
                                A38PropertySource::EngineMaterialized
                            } else {
                                A38PropertySource::Literal(raw.to_string())
                            };
                            if property_sources
                                .insert(
                                    (element_name.to_string(), property_name.to_string()),
                                    source,
                                )
                                .is_some()
                            {
                                return Err("A38 scripted-GUI property key repeats".into());
                            }
                        }
                    }
                }
                let layer_kind = source.winner.layer_kind.as_str().to_string();
                let contract = AuthenticatedA38DefinitionContract {
                    definition:
                        scripted_state_flags_variables_live_lane::ScriptedGuiDefinitionIdentity {
                            logical_path: source.winner.normalized_game_relative_path.clone(),
                            winning_layer_ordinal: source.winner.winning_layer_ordinal,
                            layer_kind,
                            layer_id: source.winner.layer_id.clone(),
                            source_sha256: source.winner.source_sha256,
                            payload_sha256: source.winner.payload_sha256,
                            payload_bytes: source.winner.payload_bytes,
                            context_type,
                            window_name,
                        },
                    expected_trigger_names,
                    expected_property_keys: property_sources.keys().cloned().collect(),
                    expected_effect_names,
                };
                update_a38_manifest_text(&mut manifest, gui_id);
                update_a38_manifest_text(&mut manifest, &contract.definition.context_type);
                update_a38_manifest_text(&mut manifest, &contract.definition.window_name);
                for name in &contract.expected_trigger_names {
                    update_a38_manifest_text(&mut manifest, name);
                }
                for key in &contract.expected_property_keys {
                    update_a38_manifest_text(&mut manifest, &key.0);
                    update_a38_manifest_text(&mut manifest, &key.1);
                }
                for name in &contract.expected_effect_names {
                    update_a38_manifest_text(&mut manifest, name);
                }
                if definitions
                    .insert(
                        gui_id.to_string(),
                        A38ProductionDefinition {
                            contract,
                            enabled_constant,
                            trigger_elements,
                            effect_elements,
                            property_sources,
                        },
                    )
                    .is_some()
                {
                    return Err("A38 effective registry repeats a GUI id".into());
                }
            }
        }
    }
    if definitions.is_empty() {
        return Err("A38 H4CT registry contains no definitions".into());
    }
    let producer_manifest_sha256: [u8; 32] = manifest.finalize().into();
    let mut epoch_bytes = [0u8; 8];
    epoch_bytes.copy_from_slice(&producer_manifest_sha256[..8]);
    let catalog_epoch = u64::from_le_bytes(epoch_bytes).max(1);
    let registry = definitions
        .iter()
        .map(|(id, definition)| (id.clone(), definition.contract.clone()))
        .collect();
    let compiler = std::sync::Arc::new(A38H4ctTransactionCompiler {
        process_start_nonce: admitted.process_start_nonce,
        catalog_epoch,
        ui14_engine_vfs_sha256: admitted.effective_vfs_manifest_sha256,
        producer_manifest_sha256,
        definitions,
    });
    Ok(A38ProductionBundle {
        compiler,
        registry,
        ui14_engine_vfs_sha256: admitted.effective_vfs_manifest_sha256,
        producer_manifest_sha256,
    })
}

#[cfg(windows)]
fn a38_constant_predicate(value: &portable_content_manifest::paradox_text::Value) -> Option<bool> {
    match value.as_scalar()?.to_ascii_lowercase().as_str() {
        "yes" | "true" => Some(true),
        "no" | "false" => Some(false),
        _ => None,
    }
}

#[cfg(windows)]
fn canonical_scripted_gui_source_path(value: &str) -> bool {
    value
        .strip_prefix("common/scripted_guis/")
        .and_then(|relative| relative.strip_suffix(".txt"))
        .is_some_and(|stem| {
            !stem.is_empty()
                && !stem.ends_with('/')
                && !stem.contains('\\')
                && !stem
                    .split('/')
                    .any(|part| part.is_empty() || part == "." || part == "..")
        })
}

#[cfg(windows)]
fn update_a38_manifest_text(hasher: &mut Sha256, value: &str) {
    hasher.update((value.len() as u64).to_le_bytes());
    hasher.update(value.as_bytes());
}

const HEARTBEAT_JSON: &[u8] = br#"{"type":"heartbeat","backend":"in_process_collector","collector_version":"0.1.0","abi_version":1,"domains":[],"quality_status":"ok"}"#;

#[no_mangle]
pub extern "C" fn battle-plan_collector_abi_version() -> u32 {
    ABI_VERSION
}

#[no_mangle]
pub extern "C" fn battle-plan_collector_heartbeat_len() -> usize {
    HEARTBEAT_JSON.len()
}

#[no_mangle]
pub unsafe extern "C" fn battle-plan_collector_heartbeat_json(out: *mut u8, cap: usize) -> usize {
    if !out.is_null() && cap != 0 {
        let n = HEARTBEAT_JSON.len().min(cap);
        unsafe {
            ptr::copy_nonoverlapping(HEARTBEAT_JSON.as_ptr(), out, n);
        }
    }
    HEARTBEAT_JSON.len()
}

pub fn heartbeat_json_for_tests() -> &'static [u8] {
    HEARTBEAT_JSON
}

/// Test helper mirroring the C ABI with a Rust slice.
pub fn copy_heartbeat_json(out: &mut [u8]) -> usize {
    let n = HEARTBEAT_JSON.len().min(out.len());
    out[..n].copy_from_slice(&HEARTBEAT_JSON[..n]);
    HEARTBEAT_JSON.len()
}

unsafe fn _caller_buffer<'a>(ptr: *mut u8, len: usize) -> Option<&'a mut [u8]> {
    if ptr.is_null() {
        return None;
    }
    Some(unsafe { slice::from_raw_parts_mut(ptr, len) })
}

#[cfg(windows)]
pub(crate) mod attach_log;

#[cfg(windows)]
#[no_mangle]
pub unsafe extern "system" fn DllMain(
    module: *mut core::ffi::c_void,
    reason: u32,
    _reserved: *mut core::ffi::c_void,
) -> i32 {
    unsafe { attach_log::dll_main(module, reason) }
}

#[cfg(windows)]
mod hook_proc {
    use core::ffi::c_void;
    use core::ptr;

    #[link(name = "user32")]
    extern "system" {
        fn CallNextHookEx(hook: *mut c_void, code: i32, wparam: usize, lparam: isize) -> isize;
    }

    pub unsafe fn get_msg_proc(code: i32, wparam: usize, lparam: isize) -> isize {
        unsafe { CallNextHookEx(ptr::null_mut(), code, wparam, lparam) }
    }
}

#[cfg(windows)]
#[no_mangle]
pub unsafe extern "system" fn Hoi4ReplayGetMsgProc(
    code: i32,
    wparam: usize,
    lparam: isize,
) -> isize {
    attach_log::trigger_attach_worker_from_hook();
    unsafe { hook_proc::get_msg_proc(code, wparam, lparam) }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn collector_features_cover_every_declared_cargo_feature() {
        use compact_contract::control_transport::payloads::CollectorFeatureBits as F;
        let manifest = std::fs::read_to_string(concat!(env!("CARGO_MANIFEST_DIR"), "/Cargo.toml"))
            .expect("collector Cargo.toml is readable from its own manifest dir")
            .replace("\r\n", "\n");
        let features_section = manifest
            .split("\n[features]\n")
            .nth(1)
            .expect("collector Cargo.toml declares a [features] section");
        let mut declared: Vec<&str> = Vec::new();
        for line in features_section.lines() {
            let line = line.trim();
            if line.starts_with('[') {
                break;
            }
            if line.starts_with('#') || line.is_empty() {
                continue;
            }
            let Some((name, _)) = line.split_once('=') else {
                continue;
            };
            declared.push(name.trim());
        }
        assert!(
            !declared.is_empty(),
            "parsed no cargo features; the [features] scan is broken, not the coverage"
        );
        let named: Vec<&str> = F::FEATURE_NAMES.iter().map(|(_, name)| *name).collect();
        for feature in &declared {
            assert!(
                named.contains(feature),
                "cargo feature {feature:?} has no CollectorFeatureBits bit; a build carrying it \
                 would declare itself clean"
            );
        }
        for name in &named {
            assert!(
                declared.contains(name),
                "CollectorFeatureBits names {name:?}, which collector/Cargo.toml no longer declares"
            );
        }
        let bits = F::from_bits(COLLECTOR_FEATURE_BITS).expect("compiled feature word is valid");
        assert!(bits.declared());
        assert_eq!(
            bits.feature_names()
                .expect("a declaring build names its features")
                .contains(&"range-tracer"),
            cfg!(feature = "range-tracer")
        );
        assert_eq!(
            bits.diagnostic_build(),
            Some(cfg!(feature = "range-tracer") || cfg!(feature = "test-support"))
        );
    }

    #[test]
    fn heartbeat_exports_expected_metadata() {
        let json = core::str::from_utf8(heartbeat_json_for_tests()).unwrap();
        assert!(json.contains("\"type\":\"heartbeat\""));
        assert!(json.contains("\"backend\":\"in_process_collector\""));
        assert!(json.contains("\"collector_version\":\"0.1.0\""));
        assert_eq!(battle-plan_collector_abi_version(), ABI_VERSION);
        assert_eq!(
            battle-plan_collector_heartbeat_len(),
            heartbeat_json_for_tests().len()
        );
    }

    #[test]
    fn heartbeat_copy_reports_required_length() {
        let mut buf = [0u8; 16];
        let required = copy_heartbeat_json(&mut buf);
        assert_eq!(required, heartbeat_json_for_tests().len());
        assert_eq!(&buf, &heartbeat_json_for_tests()[..16]);
    }

    #[test]
    fn heartbeat_abi_accepts_null_for_length_probe() {
        let required = unsafe { battle-plan_collector_heartbeat_json(ptr::null_mut(), 0) };
        assert_eq!(required, heartbeat_json_for_tests().len());
    }
}
