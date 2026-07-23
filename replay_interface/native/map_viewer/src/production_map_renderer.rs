//! Capture-independent production-map admission and execution planning.
//!
//! Production content is selected by the portable replay's content manifest,
//! resolved through the launcher-equivalent VFS, and checked again at use.
//! Paths are data carried by `UsedContentKey`; this module has no vanilla asset
//! catalog, fallback path, or capture-bundle constructor.

use crate::{bind_exact_rail_d80_draws, ExactRailD80Binding, GameFileResolver, Result};
use replay_core::{
    AuthenticatedCameraCarrier, AuthenticatedUiPass, BufferSource, ConstantBufferBinding, ContentManifest, EntitySubmission,
    C07TreeRuntimeCarrier, HudCommandExecution, M02CombatFrame,
    M03ArrowFrame, O04CityFrame, O05BuildingFrame, ParticlePacket, PoliticalTexturePacket, ProductionRenderFrame,
    RailSupplyArrowFrame,
    RenderExecutionRef, RenderOutputRef, RenderSubmit, RequiredContentKind, StaticGeometrySource, StaticMapCommand,
    StaticMapPassProgram, StaticShaderResourceSource, TiledLightSubmission, UiPassOperation,
    UsedContentKey, ValidatedContentAdmission, S1ShadowCarrier,
};
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet};

/// One byte-exact effective-VFS payload, loaded only through the complete
/// replay-admitted `UsedContentKey` identity.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ProductionMapResource {
    pub content_key: String,
    pub kind: RequiredContentKind,
    pub content: UsedContentKey,
    pub bytes: Vec<u8>,
    pub consumers: Vec<String>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "family", rename_all = "snake_case")]
pub enum PreparedRuntimeDraw {
    Particle {
        ordered_draw: RenderOutputRef,
        packet: ParticlePacket,
    },
    Entity {
        ordered_draw: RenderOutputRef,
        submission: EntitySubmission,
    },
    O04City {
        ordered_draw: RenderOutputRef,
        submission: EntitySubmission,
        generator_ordinal: u32,
    },
    O05Building {
        ordered_draw: RenderOutputRef,
        submission: EntitySubmission,
        runtime_object_id: u64,
        update_ordinal: u32,
    },
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ProductionMapDataNeed {
    pub packet: String,
    pub feature: String,
    pub reason: String,
}

pub const C03_CAMERA_REPLAY_BINDING_SCHEMA: &str = "hoi4.c03.camera-replay-binding.v1";

/// Replay consumes the authenticated envelope camera state exactly once. Raw
/// wheel/pan/map-pick rows authenticate that state transition but are never
/// reapplied as input, which would double-advance an already captured camera.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum C03CameraReplayMode {
    RecordedEnvelopeStateOnly,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct C03CameraReplayBinding {
    pub schema: String,
    pub submit_index: u64,
    pub submit_global_order: u32,
    pub carrier_payload_sha256: String,
    pub event_batch_count: u32,
    pub raw_input_event_count: u32,
    pub map_pick_count: u32,
    pub current_eye_f32_bits: [u32; 3],
    pub target_eye_f32_bits: [u32; 3],
    pub target_look_at_f32_bits: [u32; 3],
    pub mode: C03CameraReplayMode,
    /// Must remain zero: the recorded envelope state already includes every
    /// accepted raw event in `camera_event_batches`.
    pub raw_event_reapplication_count: u8,
    /// Exactly one selected envelope camera state is bound to this submit.
    pub envelope_state_selection_count: u8,
}

fn c03_binding_from_submit(submit: &RenderSubmit) -> Result<Option<C03CameraReplayBinding>> {
    let Some(carrier) = &submit.camera_carrier else {
        return Ok(None);
    };
    if carrier.payload_sha256 != carrier.computed_payload_sha256() {
        return Err("C03 camera replay binding blocked: carrier payload digest mismatch".into());
    }
    let mut raw_input_event_count = 0usize;
    let mut map_pick_count = 0usize;
    for batch in &carrier.camera_event_batches {
        raw_input_event_count = raw_input_event_count
            .checked_add(batch.input_events.len())
            .ok_or("C03 camera raw-event count overflow")?;
        map_pick_count = map_pick_count
            .checked_add(batch.map_picks.len())
            .ok_or("C03 camera map-pick count overflow")?;
    }
    Ok(Some(C03CameraReplayBinding {
        schema: C03_CAMERA_REPLAY_BINDING_SCHEMA.into(),
        submit_index: submit.submit_index,
        submit_global_order: submit.global_order,
        carrier_payload_sha256: carrier.payload_sha256.clone(),
        event_batch_count: u32::try_from(carrier.camera_event_batches.len())
            .map_err(|_| "C03 camera event-batch count exceeds u32")?,
        raw_input_event_count: u32::try_from(raw_input_event_count)
            .map_err(|_| "C03 camera raw-event count exceeds u32")?,
        map_pick_count: u32::try_from(map_pick_count)
            .map_err(|_| "C03 camera map-pick count exceeds u32")?,
        current_eye_f32_bits: carrier.current_eye_f32_bits,
        target_eye_f32_bits: carrier.target_eye_f32_bits,
        target_look_at_f32_bits: carrier.target_look_at_f32_bits,
        mode: C03CameraReplayMode::RecordedEnvelopeStateOnly,
        raw_event_reapplication_count: 0,
        envelope_state_selection_count: 1,
    }))
}

pub fn selected_c03_camera_replay_binding(
    frame: &ProductionRenderFrame,
) -> Result<Option<C03CameraReplayBinding>> {
    frame
        .validate()
        .map_err(|error| format!("invalid portable production-render frame: {error}"))?;
    match selected_submit(frame)? {
        Some(submit) => c03_binding_from_submit(submit),
        None => Ok(None),
    }
}

pub fn validate_c03_camera_consumer(
    carrier: &AuthenticatedCameraCarrier,
    binding: &C03CameraReplayBinding,
) -> Result<()> {
    validate_c03_camera_replay_binding(binding)?;
    let raw_input_event_count = carrier
        .camera_event_batches
        .iter()
        .try_fold(0usize, |count, batch| {
            count.checked_add(batch.input_events.len())
        })
        .ok_or("C03 camera raw-event count overflow")?;
    let map_pick_count = carrier
        .camera_event_batches
        .iter()
        .try_fold(0usize, |count, batch| {
            count.checked_add(batch.map_picks.len())
        })
        .ok_or("C03 camera map-pick count overflow")?;
    if carrier.payload_sha256 != carrier.computed_payload_sha256()
        || binding.carrier_payload_sha256 != carrier.payload_sha256
        || binding.current_eye_f32_bits != carrier.current_eye_f32_bits
        || binding.target_eye_f32_bits != carrier.target_eye_f32_bits
        || binding.target_look_at_f32_bits != carrier.target_look_at_f32_bits
        || binding.event_batch_count as usize != carrier.camera_event_batches.len()
        || binding.raw_input_event_count as usize != raw_input_event_count
        || binding.map_pick_count as usize != map_pick_count
        || binding.mode != C03CameraReplayMode::RecordedEnvelopeStateOnly
        || binding.raw_event_reapplication_count != 0
        || binding.envelope_state_selection_count != 1
    {
        return Err("C03 camera consumer binding/state policy mismatch".into());
    }
    Ok(())
}

pub fn validate_c03_camera_replay_binding(binding: &C03CameraReplayBinding) -> Result<()> {
    if binding.schema != C03_CAMERA_REPLAY_BINDING_SCHEMA
        || binding.carrier_payload_sha256.len() != 64
        || !binding
            .carrier_payload_sha256
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
        || binding.mode != C03CameraReplayMode::RecordedEnvelopeStateOnly
        || binding.raw_event_reapplication_count != 0
        || binding.envelope_state_selection_count != 1
    {
        return Err("C03 camera replay binding identity/policy mismatch".into());
    }
    Ok(())
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct PreparedProductionMapFrame {
    pub content_manifest_sha256: String,
    pub product_modset_sha256: String,
    pub effective_vfs_manifest_sha256: String,
    pub selected_submit_index: Option<u64>,
    pub selected_submit_global_order: Option<u32>,
    /// Receipt-ready C03 identity for the selected submit. Raw inputs are
    /// evidence only; consumers select the recorded envelope state once.
    pub c03_camera_replay: Option<C03CameraReplayBinding>,
    /// Exact scene bindings from the selected portable submit. Static map
    /// interpreters consume these directly; they must never recover camera,
    /// clock, or other scene state from a retained capture bundle.
    pub ordered_scene_constant_buffers: Vec<ConstantBufferBinding>,
    /// Complete static map programs carried by the selected portable submit.
    /// An empty vector is authoritative unavailability, never permission to
    /// synthesize pipeline state from vanilla paths or retained captures.
    pub ordered_static_map_programs: Vec<StaticMapPassProgram>,
    pub authenticated_texture_snapshots:
        Vec<replay_core::AuthenticatedTextureSnapshotPacket>,
    /// D81 semantics joined to engine-built D80 railroad draws through the
    /// authenticated a729 registration producer. `None` never authorizes
    /// reconstructed centroid ribbons or shader-name inference.
    pub rail_ribbon: Option<ExactRailD80Binding>,
    /// Exact selected-Present runtime SnowMudData packet consumed by static
    /// bindings carrying `StaticShaderResourceSource::SnowMudTexture`.
    pub snow_mud_texture: Option<replay_core::SnowMudTexturePacket>,
    /// C07 operation join admitted against the selected submit. The D3D
    /// preparation/execution path must retain every referenced static draw.
    pub tree_runtime: Option<C07TreeRuntimeCarrier>,
    /// Selected-submit S1 carrier already validated against the unified
    /// static/runtime operation stream. Absence remains a typed omission.
    pub s1_shadow: Option<S1ShadowCarrier>,
    /// Sole authenticated cross-family execution order. Backends may not
    /// concatenate static and runtime families heuristically.
    pub ordered_render_operations: Vec<RenderExecutionRef>,
    /// Exact dynamic texture uploads available to static shader-resource
    /// bindings. They remain submit payloads, never VFS/default assets.
    pub political_textures: Vec<PoliticalTexturePacket>,
    pub tiled_lights: Vec<TiledLightSubmission>,
    pub ordered_runtime_draws: Vec<PreparedRuntimeDraw>,
    /// Validated exact M02 carrier. Pixel execution remains omitted until the
    /// complete UI pass can interleave its authenticated global ordinals.
    pub m02_combat: Option<M02CombatFrame>,
    /// Complete authenticated selected-Present operation stream, retained
    /// independently of map-dimension admission for execution before Present.
    pub authenticated_ui_pass: Option<AuthenticatedUiPass>,
    pub m03_arrows: Option<M03ArrowFrame>,
    /// Separate exact supply-map railway carrier. Its draw payloads remain in
    /// `ordered_static_map_programs` and execute through the static path.
    pub rail_supply_arrows: Option<RailSupplyArrowFrame>,
    pub o04_cities: Option<O04CityFrame>,
    pub o05_buildings: Option<O05BuildingFrame>,
    pub omitted_data_needs: Vec<ProductionMapDataNeed>,
}

/// Capture-independent renderer front-end. A D3D backend can execute the
/// static law interpreters and the exact prepared runtime draws directly.
pub struct ProductionMapRenderer {
    resolver: GameFileResolver,
    manifest_sha256: String,
    product_modset_sha256: String,
    effective_vfs_manifest_sha256: String,
    resource_catalog: BTreeMap<String, ProductionMapResource>,
    resources: BTreeMap<String, ProductionMapResource>,
}

impl ProductionMapRenderer {
    pub fn new(
        resolver: &GameFileResolver,
        admission: &ValidatedContentAdmission,
        manifest: &ContentManifest,
    ) -> Result<Self> {
        if !resolver.has_content_admission() {
            return Err("production map renderer requires an admission-bound effective VFS".into());
        }
        manifest
            .validate()
            .map_err(|err| format!("invalid content manifest: {err}"))?;
        let manifest_sha256 = manifest
            .manifest_digest_sha256
            .as_deref()
            .ok_or("production map renderer requires a signed portable content manifest")?;
        if manifest_sha256 != admission.manifest_digest_sha256() {
            return Err(
                "production map renderer admission belongs to a different content manifest".into(),
            );
        }
        if manifest.product_modset_sha256 != admission.product_modset_sha256() {
            return Err(
                "production map renderer admission belongs to a different product modset".into(),
            );
        }

        let mut resource_catalog = BTreeMap::new();
        for row in &manifest.required_content {
            let admitted = admission
                .require_used_content(&row.content.normalized_game_relative_path)
                .map_err(|err| format!("manifest content is not admitted: {err}"))?;
            if admitted != &row.content {
                return Err(format!(
                    "manifest/admission UsedContentKey mismatch for {}",
                    row.content.normalized_game_relative_path
                )
                .into());
            }
            let resource = ProductionMapResource {
                content_key: row.content_key.clone(),
                kind: row.kind.clone(),
                content: row.content.clone(),
                // Bytes are loaded only after an authenticated selected frame
                // declares this opaque key. Unrelated UI/hour content cannot
                // gate or inflate the current non-UI route.
                bytes: Vec::new(),
                consumers: row.consumers.clone(),
            };
            if resource_catalog
                .insert(row.content_key.clone(), resource)
                .is_some()
            {
                return Err(
                    format!("duplicate production-map content key {}", row.content_key).into(),
                );
            }
        }

        Ok(Self {
            resolver: resolver.clone(),
            manifest_sha256: manifest_sha256.to_string(),
            product_modset_sha256: manifest.product_modset_sha256.clone(),
            effective_vfs_manifest_sha256: manifest.effective_vfs_manifest_sha256.clone(),
            resource_catalog,
            resources: BTreeMap::new(),
        })
    }

    pub fn resources(&self) -> &BTreeMap<String, ProductionMapResource> {
        &self.resources
    }

    pub fn prepare_frame(
        &mut self,
        production_render: Option<&ProductionRenderFrame>,
    ) -> Result<PreparedProductionMapFrame> {
        self.prepare_frame_with_rail(production_render, None, None)
    }

    pub fn prepare_frame_with_rail(
        &mut self,
        production_render: Option<&ProductionRenderFrame>,
        rail_replay: Option<&replay_core::RailReplayView>,
        production_by_country: Option<&BTreeMap<String, replay_core::ProductionView>>,
    ) -> Result<PreparedProductionMapFrame> {
        self.resources.clear();
        let mut omitted_data_needs = Vec::new();
        let (
            selected_submit_index,
            selected_submit_global_order,
            c03_camera_replay,
            ordered_scene_constant_buffers,
            ordered_static_map_programs,
            authenticated_texture_snapshots,
            snow_mud_texture,
            tree_runtime,
            s1_shadow,
            ordered_render_operations,
            political_textures,
            tiled_lights,
            ordered_runtime_draws,
            m02_combat,
            authenticated_ui_pass,
            m03_arrows,
            rail_supply_arrows,
            o04_cities,
            o05_buildings,
        ) = match production_render {
            None => {
                omitted_data_needs.push(ProductionMapDataNeed {
                        packet: "XN15".into(),
                        feature: "runtime entity and particle submissions".into(),
                        reason: "portable frame has no authenticated production_render carrier; runtime draws omitted".into(),
                    });
                (
                    None,
                    None,
                    None,
                    Vec::new(),
                    Vec::new(),
                    Vec::new(),
                    None,
                    None,
                    None,
                    Vec::new(),
                    Vec::new(),
                    Vec::new(),
                    Vec::new(),
                    None,
                    None,
                    None,
                    None,
                    None,
                    None,
                )
            }
            Some(frame) => {
                frame
                    .validate()
                    .map_err(|err| format!("invalid portable production-render frame: {err}"))?;
                if frame.authority.effective_vfs_manifest_sha256
                    != self.effective_vfs_manifest_sha256
                {
                    return Err("production-render authority effective-VFS digest does not match playback admission".into());
                }
                if let Some(m02) = &frame.m02_combat {
                    self.validate_m02_content(m02)?;
                }
                if let Some(ui_pass) = &frame.authenticated_ui_pass {
                    self.validate_ui_pass_content(ui_pass)?;
                }
                if let Some(m03) = &frame.m03_arrows {
                    self.validate_m03_content(m03)?;
                }
                if let Some(rail) = &frame.rail_supply_arrows {
                    self.validate_rail_supply_content(rail)?;
                }
                if let Some(o04) = &frame.o04_cities {
                    self.validate_o04_content(frame, o04)?;
                }
                if let Some(o05) = &frame.o05_buildings {
                    self.validate_o05_content(frame, o05)?;
                }
                match selected_submit(frame)? {
                    None => (
                        None,
                        None,
                        None,
                        Vec::new(),
                        Vec::new(),
                        Vec::new(),
                        None,
                        None,
                        None,
                        Vec::new(),
                        Vec::new(),
                        Vec::new(),
                        Vec::new(),
                        frame.m02_combat.clone(),
                        frame.authenticated_ui_pass.clone(),
                        frame.m03_arrows.clone(),
                        frame.rail_supply_arrows.clone(),
                        frame.o04_cities.clone(),
                        frame.o05_buildings.clone(),
                    ),
                    Some(submit) => {
                        if let Some(tree_runtime) = &submit.tree_runtime {
                            tree_runtime
                                .content_selection_against_submit(submit)
                                .map_err(|error| {
                                    format!("C07 tree runtime admission blocked: {error}")
                                })?;
                        }
                        if let Some(s1_shadow) = &submit.s1_shadow {
                            s1_shadow.validate_against_submit(submit).map_err(|error| {
                                format!("S1 shadow runtime admission blocked: {error}")
                            })?;
                        }
                        self.load_selected_static_resources(submit)?;
                        self.validate_static_program_content(submit)?;
                        self.validate_submit_content(submit)?;
                        let draws = ordered_draws(
                            submit,
                            frame.o04_cities.as_ref(),
                            frame.o05_buildings.as_ref(),
                        )?;
                        (
                            Some(submit.submit_index),
                            Some(submit.global_order),
                            c03_binding_from_submit(submit)?,
                            submit.ordered_scene_constant_buffers.clone(),
                            submit.ordered_static_map_programs.clone(),
                            submit.authenticated_texture_snapshots.clone(),
                            frame.snow_mud_texture.clone(),
                            submit.tree_runtime.clone(),
                            submit.s1_shadow.clone(),
                            submit.ordered_render_operations.clone(),
                            submit.political_textures.clone(),
                            submit.tiled_lights.clone(),
                            draws,
                            frame.m02_combat.clone(),
                            frame.authenticated_ui_pass.clone(),
                            frame.m03_arrows.clone(),
                            frame.rail_supply_arrows.clone(),
                            frame.o04_cities.clone(),
                            frame.o05_buildings.clone(),
                        )
                    }
                }
            }
        };

        let selected_rail_submit = production_render
            .map(selected_submit)
            .transpose()?
            .flatten();
        let rail_ribbon = rail_replay
            .map(|view| {
                bind_exact_rail_d80_draws(
                    view,
                    production_by_country,
                    selected_rail_submit
                        .map(|submit| submit.rail_construction_arrows.as_slice())
                        .unwrap_or(&[]),
                    selected_rail_submit
                        .map(|submit| submit.entities.as_slice())
                        .unwrap_or(&[]),
                    &ordered_static_map_programs,
                    &ordered_render_operations,
                )
            })
            .transpose()?
            .flatten();
        if rail_ribbon.is_none() {
            omitted_data_needs.push(ProductionMapDataNeed {
                packet: "D81/D80".into(),
                feature: "exact railroad ribbon draw".into(),
                reason: "no D81 carrier was joined to an authenticated railroad-registration D80 draw; centroid tessellation, shader-name inference, and guessed interleave are forbidden".into(),
            });
        }

        if m02_combat.is_none() {
            omitted_data_needs.push(ProductionMapDataNeed {
                packet: "M02".into(),
                feature: "combat indicators".into(),
                reason:
                    "portable production frame has no authenticated M02 carrier; feature omitted"
                        .into(),
            });
        }
        if tree_runtime.is_none() {
            omitted_data_needs.push(ProductionMapDataNeed {
                packet: "C07/T1".into(),
                feature: "exact tree visibility/model/LOD draws".into(),
                reason: "selected submit has no authenticated C07 tree runtime carrier; tree draws omitted rather than reconstructed from camera or files".into(),
            });
        }
        if s1_shadow.is_none() {
            omitted_data_needs.push(ProductionMapDataNeed {
                packet: "C07/S1".into(),
                feature: "exact selected-light shadow producer/blur/consumer chain".into(),
                reason: "selected submit has no authenticated S1 shadow carrier; the shadow chain is omitted rather than reconstructed from capture identities or guessed caster state".into(),
            });
        }
        if m03_arrows.is_none() {
            omitted_data_needs.push(ProductionMapDataNeed {
                packet: "M03".into(),
                feature: "order and movement arrows".into(),
                reason:
                    "portable production frame has no authenticated M03 carrier; feature omitted"
                        .into(),
            });
        }
        if rail_supply_arrows.is_none() {
            omitted_data_needs.push(ProductionMapDataNeed {
                packet: "D81/rail-supply-map".into(),
                feature: "railway supply-map damage/cooldown arrows".into(),
                reason: "portable production frame has no authenticated rail supply-map carrier joined to exact normal and see-through static draws; M03 and construction-pause state are not substitutes".into(),
            });
        }
        if o04_cities.is_none() {
            omitted_data_needs.push(ProductionMapDataNeed {
                packet: "O04".into(),
                feature: "city entity submissions".into(),
                reason:
                    "portable production frame has no authenticated O04 carrier; feature omitted"
                        .into(),
            });
        }
        if o05_buildings.is_none() {
            omitted_data_needs.push(ProductionMapDataNeed {
                packet: "O05".into(),
                feature: "generic map-building entity submissions".into(),
                reason: "portable production frame has no complete selected-Present O05 runtime-object carrier; static map/buildings.txt placements and retained GPU keys are not promoted as runtime authority".into(),
            });
        }

        Ok(PreparedProductionMapFrame {
            content_manifest_sha256: self.manifest_sha256.clone(),
            product_modset_sha256: self.product_modset_sha256.clone(),
            effective_vfs_manifest_sha256: self.effective_vfs_manifest_sha256.clone(),
            selected_submit_index,
            selected_submit_global_order,
            c03_camera_replay,
            ordered_scene_constant_buffers,
            ordered_static_map_programs,
            authenticated_texture_snapshots,
            rail_ribbon,
            snow_mud_texture,
            tree_runtime,
            s1_shadow,
            ordered_render_operations,
            political_textures,
            tiled_lights,
            ordered_runtime_draws,
            m02_combat,
            authenticated_ui_pass,
            m03_arrows,
            rail_supply_arrows,
            o04_cities,
            o05_buildings,
            omitted_data_needs,
        })
    }

    fn load_selected_static_resources(&mut self, submit: &RenderSubmit) -> Result<()> {
        let mut selected = BTreeSet::new();
        for program in &submit.ordered_static_map_programs {
            for key in &program.ordered_content_keys {
                selected.insert(key.clone());
            }
        }
        let mut loaded = BTreeMap::new();
        for key in selected {
            let mut resource =
                self.resource_catalog.get(&key).cloned().ok_or_else(|| {
                    format!("static-map content key {key} is absent from manifest")
                })?;
            resource.bytes = self
                .resolver
                .read_admitted_used_content(&resource.content)?;
            loaded.insert(key, resource);
        }
        self.resources = loaded;
        Ok(())
    }

    fn validate_submit_content(&self, submit: &RenderSubmit) -> Result<()> {
        let mut keys = Vec::new();
        for particle in &submit.particles {
            collect_particle_content(particle, &mut keys);
        }
        for entity in &submit.entities {
            collect_entity_content(entity, &mut keys);
        }
        for key in keys {
            key.validate()
                .map_err(|err| format!("invalid runtime UsedContentKey: {err}"))?;
            self.resolver.read_admitted_used_content(key)?;
        }
        Ok(())
    }

    fn validate_m02_content(&self, frame: &M02CombatFrame) -> Result<()> {
        frame
            .validate()
            .map_err(|error| format!("invalid M02 carrier: {error}"))?;
        for indicator in &frame.indicators {
            let allowed = indicator.used_content.iter().collect::<BTreeSet<_>>();
            for content in &indicator.used_content {
                self.resolver.read_admitted_used_content(content)?;
            }
            let mut used = Vec::new();
            for packet in &indicator.draws {
                let draw = &packet.draw;
                used.push(&draw.shader_program.source);
                for binding in &draw.ordered_shader_resources {
                    if let StaticShaderResourceSource::UsedContent {
                        content, resource, ..
                    } = &binding.source
                    {
                        used.push(content);
                        for subresource in &resource.ordered_subresources {
                            collect_buffer_content(&subresource.source, &mut used);
                        }
                    }
                }
                let StaticGeometrySource::Exact { geometry } = &draw.geometry else {
                    return Err("M02 draw contains unauthenticated interpreter geometry".into());
                };
                collect_geometry_content(geometry, &mut used);
                collect_output_content(&draw.output_state, &mut used);
            }
            let used = used.into_iter().collect::<BTreeSet<_>>();
            for content in &used {
                if !allowed.contains(content) {
                    return Err(format!(
                        "M02 draw references content outside its indicator admission list: {}",
                        content.normalized_game_relative_path
                    )
                    .into());
                }
                self.resolver.read_admitted_used_content(content)?;
            }
        }
        Ok(())
    }

    fn validate_ui_pass_content(&self, pass: &AuthenticatedUiPass) -> Result<()> {
        pass.validate()
            .map_err(|error| format!("invalid authenticated UI-pass carrier: {error}"))?;
        for operation in &pass.ui_pass_operations {
            let UiPassOperation::HudCommand { command } = operation else {
                continue;
            };
            let allowed = command
                .ordered_effective_vfs_used_content
                .iter()
                .collect::<BTreeSet<_>>();
            for content in &command.ordered_effective_vfs_used_content {
                self.resolver.read_admitted_used_content(content)?;
            }
            let HudCommandExecution::ExactPipeline { draw } = &command.execution else {
                continue;
            };
            let draw = &draw.exact_draw;
            let mut used = vec![&draw.shader_program.source];
            for binding in &draw.ordered_shader_resources {
                if let StaticShaderResourceSource::UsedContent {
                    content, resource, ..
                } = &binding.source
                {
                    used.push(content);
                    for subresource in &resource.ordered_subresources {
                        collect_buffer_content(&subresource.source, &mut used);
                    }
                }
            }
            let StaticGeometrySource::Exact { geometry } = &draw.geometry else {
                return Err("authenticated exact UI draw contains interpreter geometry".into());
            };
            collect_geometry_content(geometry, &mut used);
            collect_output_content(&draw.output_state, &mut used);
            for content in used.into_iter().collect::<BTreeSet<_>>() {
                if !allowed.contains(content) {
                    return Err(format!(
                        "authenticated UI draw references content outside its command admission list: {}",
                        content.normalized_game_relative_path
                    )
                    .into());
                }
                self.resolver.read_admitted_used_content(content)?;
            }
        }
        Ok(())
    }

    fn validate_m03_content(&self, frame: &M03ArrowFrame) -> Result<()> {
        frame
            .validate()
            .map_err(|error| format!("invalid M03 carrier: {error}"))?;
        let allowed = frame.used_content.iter().collect::<BTreeSet<_>>();
        for content in &frame.used_content {
            self.resolver.read_admitted_used_content(content)?;
        }
        let mut used = Vec::new();
        for packet in &frame.draws {
            let draw = &packet.draw;
            used.push(&draw.shader_program.source);
            for binding in &draw.ordered_shader_resources {
                if let StaticShaderResourceSource::UsedContent {
                    content, resource, ..
                } = &binding.source
                {
                    used.push(content);
                    for subresource in &resource.ordered_subresources {
                        collect_buffer_content(&subresource.source, &mut used);
                    }
                }
            }
            let StaticGeometrySource::Exact { geometry } = &draw.geometry else {
                return Err("M03 draw contains unauthenticated interpreter geometry".into());
            };
            collect_geometry_content(geometry, &mut used);
            collect_output_content(&draw.output_state, &mut used);
        }
        for content in used.into_iter().collect::<BTreeSet<_>>() {
            if !allowed.contains(content) {
                return Err(format!(
                    "M03 draw references content outside its admission list: {}",
                    content.normalized_game_relative_path
                )
                .into());
            }
            self.resolver.read_admitted_used_content(content)?;
        }
        Ok(())
    }

    fn validate_rail_supply_content(&self, frame: &RailSupplyArrowFrame) -> Result<()> {
        frame
            .validate()
            .map_err(|error| format!("invalid rail supply-arrow carrier: {error}"))?;
        for content in &frame.used_content {
            self.resolver.read_admitted_used_content(content)?;
        }
        for content in &frame.draw_used_content {
            self.resolver.read_admitted_used_content(content)?;
        }
        Ok(())
    }

    fn validate_o04_content(
        &self,
        production: &ProductionRenderFrame,
        frame: &O04CityFrame,
    ) -> Result<()> {
        frame
            .validate()
            .map_err(|error| format!("invalid O04 carrier: {error}"))?;
        if frame.update_index != production.update_index
            || frame.first_sequence < production.first_sequence
            || frame.last_sequence > production.last_sequence
        {
            return Err(
                "O04 carrier hour/sequence authority differs from its production frame".into(),
            );
        }
        let submit = selected_submit(production)?
            .ok_or("O04 carrier has no authenticated display submit")?;

        let referenced = frame
            .ordered_outputs
            .iter()
            .cloned()
            .collect::<BTreeSet<_>>();
        if referenced.len() != frame.ordered_outputs.len() {
            return Err("O04 carrier repeats an entity output".into());
        }
        let actual_order = submit
            .ordered_draws
            .iter()
            .filter(|output| referenced.contains(*output))
            .cloned()
            .collect::<Vec<_>>();
        if actual_order != frame.ordered_outputs {
            return Err("O04 outputs are dangling or differ from display-submit order".into());
        }

        for city in &frame.cities {
            self.validate_o04_admitted_content(&city.entity_asset)?;
            self.validate_o04_admitted_content(&city.mesh)?;
            let entity_instance_id = match city.referenced_outputs.first() {
                Some(RenderOutputRef::Entity {
                    entity_instance_id, ..
                }) => *entity_instance_id,
                _ => return Err("O04 city has no entity output".into()),
            };
            let complete_entity_outputs = submit
                .ordered_draws
                .iter()
                .filter(|output| {
                    matches!(
                        output,
                        RenderOutputRef::Entity {
                            entity_instance_id: actual,
                            ..
                        } if *actual == entity_instance_id
                    )
                })
                .cloned()
                .collect::<Vec<_>>();
            if complete_entity_outputs != city.referenced_outputs {
                return Err("O04 city outputs are not the complete entity-instance cover".into());
            }
            for output in &city.referenced_outputs {
                let submission = entity_submission(submit, output)
                    .ok_or("O04 city references an absent entity submission")?;
                if submission.content_identity.resolved_entity_asset != city.entity_asset
                    || submission.content_identity.mesh != city.mesh
                    || submission.world_transform_4x4_f32_bits != city.world_transform_4x4_f32_bits
                {
                    return Err(
                        "O04 city semantic identity/transform differs from entity submission"
                            .into(),
                    );
                }
            }
        }
        Ok(())
    }

    fn validate_o04_admitted_content(&self, content: &UsedContentKey) -> Result<()> {
        let declared = self.resource_catalog.values().any(|resource| {
            resource.content == *content
                && resource
                    .consumers
                    .iter()
                    .any(|consumer| consumer == "production_render.o04_city")
        });
        if !declared {
            return Err(format!(
                "O04 content {} is not bound to production_render.o04_city in the portable manifest",
                content.normalized_game_relative_path
            )
            .into());
        }
        self.resolver.read_admitted_used_content(content)?;
        Ok(())
    }

    fn validate_o05_content(
        &self,
        production: &ProductionRenderFrame,
        frame: &O05BuildingFrame,
    ) -> Result<()> {
        frame
            .validate()
            .map_err(|error| format!("invalid O05 carrier: {error}"))?;
        if frame.update_index != production.update_index
            || frame.first_sequence != production.first_sequence
            || frame.last_sequence != production.last_sequence
        {
            return Err(
                "O05 carrier hour/sequence authority differs from its production frame".into(),
            );
        }
        self.validate_o05_admitted_content(&frame.map_buildings_source)?;
        let submit = selected_submit(production)?
            .ok_or("O05 carrier has no authenticated display submit")?;

        let referenced = frame
            .ordered_outputs
            .iter()
            .cloned()
            .collect::<BTreeSet<_>>();
        if referenced.len() != frame.ordered_outputs.len() {
            return Err("O05 carrier repeats an entity output".into());
        }
        let actual_order = submit
            .ordered_draws
            .iter()
            .filter(|output| referenced.contains(*output))
            .cloned()
            .collect::<Vec<_>>();
        if actual_order != frame.ordered_outputs {
            return Err("O05 outputs are dangling or differ from display-submit order".into());
        }

        for building in &frame.records {
            let Some(first_output) = building.referenced_outputs.first() else {
                continue;
            };
            let entity_asset = building.entity_asset.as_ref().expect("validated O05 asset");
            let mesh = building.mesh.as_ref().expect("validated O05 mesh");
            self.validate_o05_admitted_content(entity_asset)?;
            self.validate_o05_admitted_content(mesh)?;
            let entity_instance_id = match first_output {
                RenderOutputRef::Entity {
                    entity_instance_id, ..
                } => *entity_instance_id,
                _ => return Err("O05 building has no entity output".into()),
            };
            let complete_entity_outputs = submit
                .ordered_draws
                .iter()
                .filter(|output| {
                    matches!(
                        output,
                        RenderOutputRef::Entity {
                            entity_instance_id: actual,
                            ..
                        } if *actual == entity_instance_id
                    )
                })
                .cloned()
                .collect::<Vec<_>>();
            if complete_entity_outputs != building.referenced_outputs {
                return Err("O05 building outputs are not the complete entity-instance cover".into());
            }
            for output in &building.referenced_outputs {
                let submission = entity_submission(submit, output)
                    .ok_or("O05 building references an absent entity submission")?;
                if submission.content_identity.resolved_entity_asset != *entity_asset
                    || submission.content_identity.mesh != *mesh
                    || submission.world_transform_4x4_f32_bits
                        != building
                            .world_transform_4x4_f32_bits
                            .expect("validated O05 transform")
                {
                    return Err(
                        "O05 building semantic identity/transform differs from entity submission"
                            .into(),
                    );
                }
            }
        }
        Ok(())
    }

    fn validate_o05_admitted_content(&self, content: &UsedContentKey) -> Result<()> {
        content
            .validate()
            .map_err(|error| format!("invalid O05 UsedContentKey: {error}"))?;
        if content.effective_vfs_manifest_sha256 != self.effective_vfs_manifest_sha256 {
            return Err("O05 content belongs to a different effective VFS".into());
        }
        let declared = self.resource_catalog.values().any(|resource| {
            resource.content == *content
                && resource
                    .consumers
                    .iter()
                    .any(|consumer| consumer == "production_render.o05_building")
        });
        if !declared {
            return Err(format!(
                "O05 content {} is not bound to production_render.o05_building in the portable manifest",
                content.normalized_game_relative_path
            )
            .into());
        }
        self.resolver.read_admitted_used_content(content)?;
        Ok(())
    }

    fn validate_static_program_content(&self, submit: &RenderSubmit) -> Result<()> {
        for program in &submit.ordered_static_map_programs {
            let actual = program
                .ordered_content_keys
                .iter()
                .cloned()
                .collect::<BTreeSet<_>>();
            if actual.len() != program.ordered_content_keys.len() {
                return Err(format!(
                    "portable static-map program {} contains duplicate manifest keys",
                    program.pass_token
                )
                .into());
            }
            let allowed = program
                .ordered_content_keys
                .iter()
                .map(|key| {
                    let resource = self
                        .resources
                        .get(key)
                        .ok_or_else(|| format!("static-map content key {key} is not loaded"))?;
                    let expected_consumer =
                        format!("production_map.static_program/{}", program.payload_sha256);
                    if !resource
                        .consumers
                        .iter()
                        .any(|value| value == &expected_consumer)
                    {
                        return Err(format!(
                            "static-map content key {key} is not bound to sealed program {}",
                            program.payload_sha256
                        ));
                    }
                    Ok(resource.content.clone())
                })
                .collect::<std::result::Result<BTreeSet<_>, _>>()?;
            let mut used = Vec::new();
            for operation in &program.ordered_operations {
                if let StaticMapCommand::Draw { draw } = &operation.command {
                    used.push(&draw.shader_program.source);
                    for binding in &draw.ordered_shader_resources {
                        if let StaticShaderResourceSource::UsedContent {
                            content, resource, ..
                        } = &binding.source
                        {
                            used.push(content);
                            for subresource in &resource.ordered_subresources {
                                collect_buffer_content(&subresource.source, &mut used);
                            }
                        }
                    }
                    match &draw.geometry {
                        StaticGeometrySource::Exact { geometry } => {
                            collect_geometry_content(geometry, &mut used);
                        }
                        StaticGeometrySource::InterpreterOutput {
                            ordered_input_content_keys,
                            ..
                        } => {
                            for key in ordered_input_content_keys {
                                let resource = self.resources.get(key).ok_or_else(|| {
                                    format!("static geometry interpreter input key {key} is not admitted")
                                })?;
                                if !actual.contains(key) {
                                    return Err(format!(
                                        "static geometry interpreter input key {key} is absent from its pass program"
                                    )
                                    .into());
                                }
                                used.push(&resource.content);
                            }
                        }
                    }
                    collect_output_content(&draw.output_state, &mut used);
                } else {
                    let target = match &operation.command {
                        StaticMapCommand::ClearColor { target, .. }
                        | StaticMapCommand::ClearDepthStencil { target, .. } => target,
                        StaticMapCommand::Draw { .. } => unreachable!(),
                    };
                    collect_render_target_content(target, &mut used);
                }
            }
            let used = used.into_iter().cloned().collect::<BTreeSet<_>>();
            for content in &used {
                if !allowed.contains(content) {
                    return Err(format!(
                        "static-map program {} references content outside its ordered manifest keys: {}",
                        program.pass_token, content.normalized_game_relative_path
                    )
                    .into());
                }
                self.resolver.read_admitted_used_content(content)?;
            }
            if used != allowed {
                return Err(format!(
                    "static-map program {} declares an ordered content key unused by its authenticated operations",
                    program.pass_token
                )
                .into());
            }
        }
        Ok(())
    }
}

fn selected_submit(frame: &ProductionRenderFrame) -> Result<Option<&RenderSubmit>> {
    let Some(text) = frame.display_submit_index.as_deref() else {
        return Ok(None);
    };
    let index = text
        .parse::<u64>()
        .ok()
        .filter(|index| index.to_string() == text)
        .ok_or("display_submit_index is not canonical u64 text")?;
    frame
        .submits
        .iter()
        .find(|submit| submit.submit_index == index)
        .map(Some)
        .ok_or_else(|| format!("display_submit_index {index} is absent from submits").into())
}

fn ordered_draws(
    submit: &RenderSubmit,
    o04_cities: Option<&O04CityFrame>,
    o05_buildings: Option<&O05BuildingFrame>,
) -> Result<Vec<PreparedRuntimeDraw>> {
    let o04_outputs = o04_cities
        .map(|frame| {
            frame
                .cities
                .iter()
                .flat_map(|city| {
                    city.referenced_outputs
                        .iter()
                        .cloned()
                        .map(|output| (output, city.generator_ordinal))
                })
                .collect::<BTreeMap<_, _>>()
        })
        .unwrap_or_default();
    let o05_outputs = o05_buildings
        .map(|frame| {
            frame
                .records
                .iter()
                .flat_map(|building| {
                    building.referenced_outputs.iter().cloned().map(|output| {
                        (
                            output,
                            (building.runtime_object_id, building.update_ordinal),
                        )
                    })
                })
                .collect::<BTreeMap<_, _>>()
        })
        .unwrap_or_default();
    if o04_outputs.keys().any(|output| o05_outputs.contains_key(output)) {
        return Err("one entity output is classified as both O04 city and O05 building".into());
    }
    let mut particles = BTreeMap::new();
    for packet in &submit.particles {
        particles
            .entry((
                packet.effect_instance_id,
                packet.bucket,
                packet.packet_ordinal,
                packet.draw_order,
            ))
            .or_insert(packet);
    }
    let mut entities = BTreeMap::new();
    for entity in &submit.entities {
        entities
            .entry((
                entity.pass_ordinal,
                entity.entity_instance_id,
                entity.draw_order,
            ))
            .or_insert(entity);
    }
    let mut draws = Vec::with_capacity(submit.ordered_draws.len());
    for ordered in &submit.ordered_draws {
        let draw = match ordered {
            RenderOutputRef::Particle {
                effect_instance_id,
                bucket,
                packet_ordinal,
                draw_order,
            } => {
                let packet = *particles
                    .get(&(*effect_instance_id, *bucket, *packet_ordinal, *draw_order))
                    .ok_or("ordered runtime draw references a missing particle packet")?;
                PreparedRuntimeDraw::Particle {
                    ordered_draw: ordered.clone(),
                    packet: packet.clone(),
                }
            }
            RenderOutputRef::Entity {
                pass_ordinal,
                entity_instance_id,
                draw_order,
            } => {
                let submission = *entities
                    .get(&(*pass_ordinal, *entity_instance_id, *draw_order))
                    .ok_or("ordered runtime draw references a missing entity submission")?;
                if let Some(generator_ordinal) = o04_outputs.get(ordered) {
                    PreparedRuntimeDraw::O04City {
                        ordered_draw: ordered.clone(),
                        submission: submission.clone(),
                        generator_ordinal: *generator_ordinal,
                    }
                } else if let Some((runtime_object_id, update_ordinal)) = o05_outputs.get(ordered) {
                    PreparedRuntimeDraw::O05Building {
                        ordered_draw: ordered.clone(),
                        submission: submission.clone(),
                        runtime_object_id: *runtime_object_id,
                        update_ordinal: *update_ordinal,
                    }
                } else {
                    PreparedRuntimeDraw::Entity {
                        ordered_draw: ordered.clone(),
                        submission: submission.clone(),
                    }
                }
            }
        };
        draws.push(draw);
    }
    Ok(draws)
}

fn entity_submission<'a>(
    submit: &'a RenderSubmit,
    output: &RenderOutputRef,
) -> Option<&'a EntitySubmission> {
    let RenderOutputRef::Entity {
        pass_ordinal,
        entity_instance_id,
        draw_order,
    } = output
    else {
        return None;
    };
    submit.entities.iter().find(|entity| {
        entity.pass_ordinal == *pass_ordinal
            && entity.entity_instance_id == *entity_instance_id
            && entity.draw_order == *draw_order
    })
}

fn collect_particle_content<'a>(particle: &'a ParticlePacket, out: &mut Vec<&'a UsedContentKey>) {
    out.push(&particle.shader_program.source);
    for texture in &particle.ordered_texture_bindings {
        out.push(&texture.content);
        for subresource in &texture.resource.ordered_subresources {
            collect_buffer_content(&subresource.source, out);
        }
    }
    collect_geometry_content(&particle.geometry, out);
    collect_output_content(&particle.output_state, out);
}

fn collect_entity_content<'a>(entity: &'a EntitySubmission, out: &mut Vec<&'a UsedContentKey>) {
    out.push(&entity.content_identity.resolved_entity_asset);
    out.push(&entity.content_identity.mesh);
    out.push(&entity.shader_program.source);
    for texture in &entity.ordered_texture_bindings {
        out.push(&texture.content);
        for subresource in &texture.resource.ordered_subresources {
            collect_buffer_content(&subresource.source, out);
        }
    }
    collect_geometry_content(&entity.geometry, out);
    collect_output_content(&entity.output_state, out);
}

fn collect_geometry_content<'a>(
    geometry: &'a replay_core::GeometryBinding,
    out: &mut Vec<&'a UsedContentKey>,
) {
    for vertex in &geometry.ordered_vertex_buffers {
        collect_buffer_content(&vertex.source, out);
    }
    if let Some(index) = &geometry.index_buffer {
        collect_buffer_content(&index.source, out);
    }
}

fn collect_buffer_content<'a>(source: &'a BufferSource, out: &mut Vec<&'a UsedContentKey>) {
    if let BufferSource::UsedContentRange { content, .. } = source {
        out.push(content);
    }
}

fn collect_output_content<'a>(
    output: &'a replay_core::RenderOutputState,
    out: &mut Vec<&'a UsedContentKey>,
) {
    for target in &output.ordered_color_targets {
        collect_render_target_content(target, out);
    }
    if let Some(target) = &output.depth_target {
        collect_render_target_content(target, out);
    }
}

fn collect_render_target_content<'a>(
    target: &'a replay_core::RenderTargetDesc,
    out: &mut Vec<&'a UsedContentKey>,
) {
    if let Some(content) = &target.persistent_content {
        out.push(content);
    }
    if let Some(exact) = &target.exact_resource {
        for initial in &exact.ordered_initial_subresources {
            if let replay_core::RenderTargetInitialSource::UsedContentRange { content, .. } =
                &initial.source
            {
                out.push(content);
            }
        }
    }
}

#[cfg(test)]
mod tests;
