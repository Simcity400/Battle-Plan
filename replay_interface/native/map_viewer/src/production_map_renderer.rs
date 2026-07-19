//! Capture-independent production-map admission and execution planning.
//!
//! Production content is selected by the portable replay's content manifest,
//! resolved through the launcher-equivalent VFS, and checked again at use.
//! Paths are data carried by `UsedContentKey`; this module has no vanilla asset
//! catalog, fallback path, or capture-bundle constructor.

use crate::{GameFileResolver, Result};
use replay_core::{
    AuthenticatedUiPass, BufferSource, ConstantBufferBinding, ContentManifest, EntitySubmission,
    HudCommandExecution, M02CombatFrame, M03ArrowFrame, O04CityFrame, ParticlePacket,
    PoliticalTexturePacket, ProductionRenderFrame, RenderExecutionRef, RenderOutputRef,
    RenderSubmit, RequiredContentKind, StaticGeometrySource, StaticMapCommand,
    StaticMapPassProgram, StaticShaderResourceSource, TiledLightSubmission, UiPassOperation,
    UsedContentKey, ValidatedContentAdmission,
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
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ProductionMapDataNeed {
    pub packet: String,
    pub feature: String,
    pub reason: String,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct PreparedProductionMapFrame {
    pub content_manifest_sha256: String,
    pub effective_vfs_manifest_sha256: String,
    pub selected_submit_index: Option<u64>,
    pub selected_submit_global_order: Option<u32>,
    /// Exact scene bindings from the selected portable submit. Static map
    /// interpreters consume these directly; they must never recover camera,
    /// clock, or other scene state from a retained capture bundle.
    pub ordered_scene_constant_buffers: Vec<ConstantBufferBinding>,
    /// Complete static map programs carried by the selected portable submit.
    /// An empty vector is authoritative unavailability, never permission to
    /// synthesize pipeline state from vanilla paths or retained captures.
    pub ordered_static_map_programs: Vec<StaticMapPassProgram>,
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
    pub o04_cities: Option<O04CityFrame>,
    pub omitted_data_needs: Vec<ProductionMapDataNeed>,
}

/// Capture-independent renderer front-end. A D3D backend can execute the
/// static law interpreters and the exact prepared runtime draws directly.
pub struct ProductionMapRenderer {
    resolver: GameFileResolver,
    manifest_sha256: String,
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
        self.resources.clear();
        let mut omitted_data_needs = Vec::new();
        let (
            selected_submit_index,
            selected_submit_global_order,
            ordered_scene_constant_buffers,
            ordered_static_map_programs,
            ordered_render_operations,
            political_textures,
            tiled_lights,
            ordered_runtime_draws,
            m02_combat,
            authenticated_ui_pass,
            m03_arrows,
            o04_cities,
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
                    Vec::new(),
                    Vec::new(),
                    Vec::new(),
                    Vec::new(),
                    Vec::new(),
                    Vec::new(),
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
                if let Some(o04) = &frame.o04_cities {
                    self.validate_o04_content(frame, o04)?;
                }
                match selected_submit(frame)? {
                    None => (
                        None,
                        None,
                        Vec::new(),
                        Vec::new(),
                        Vec::new(),
                        Vec::new(),
                        Vec::new(),
                        Vec::new(),
                        frame.m02_combat.clone(),
                        frame.authenticated_ui_pass.clone(),
                        frame.m03_arrows.clone(),
                        frame.o04_cities.clone(),
                    ),
                    Some(submit) => {
                        self.load_selected_static_resources(submit)?;
                        self.validate_static_program_content(submit)?;
                        self.validate_submit_content(submit)?;
                        let draws = ordered_draws(submit, frame.o04_cities.as_ref())?;
                        (
                            Some(submit.submit_index),
                            Some(submit.global_order),
                            submit.ordered_scene_constant_buffers.clone(),
                            submit.ordered_static_map_programs.clone(),
                            submit.ordered_render_operations.clone(),
                            submit.political_textures.clone(),
                            submit.tiled_lights.clone(),
                            draws,
                            frame.m02_combat.clone(),
                            frame.authenticated_ui_pass.clone(),
                            frame.m03_arrows.clone(),
                            frame.o04_cities.clone(),
                        )
                    }
                }
            }
        };

        if m02_combat.is_none() {
            omitted_data_needs.push(ProductionMapDataNeed {
                packet: "M02".into(),
                feature: "combat indicators".into(),
                reason:
                    "portable production frame has no authenticated M02 carrier; feature omitted"
                        .into(),
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
        if o04_cities.is_none() {
            omitted_data_needs.push(ProductionMapDataNeed {
                packet: "O04".into(),
                feature: "city entity submissions".into(),
                reason:
                    "portable production frame has no authenticated O04 carrier; feature omitted"
                        .into(),
            });
        }

        Ok(PreparedProductionMapFrame {
            content_manifest_sha256: self.manifest_sha256.clone(),
            effective_vfs_manifest_sha256: self.effective_vfs_manifest_sha256.clone(),
            selected_submit_index,
            selected_submit_global_order,
            ordered_scene_constant_buffers,
            ordered_static_map_programs,
            ordered_render_operations,
            political_textures,
            tiled_lights,
            ordered_runtime_draws,
            m02_combat,
            authenticated_ui_pass,
            m03_arrows,
            o04_cities,
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
        .map_err(|_| "display_submit_index is not canonical u64 text")?;
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
    let mut draws = Vec::with_capacity(submit.ordered_draws.len());
    for ordered in &submit.ordered_draws {
        let draw = match ordered {
            RenderOutputRef::Particle {
                effect_instance_id,
                bucket,
                packet_ordinal,
                draw_order,
            } => {
                let packet = submit
                    .particles
                    .iter()
                    .find(|packet| {
                        packet.effect_instance_id == *effect_instance_id
                            && packet.bucket == *bucket
                            && packet.packet_ordinal == *packet_ordinal
                            && packet.draw_order == *draw_order
                    })
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
                let submission = submit
                    .entities
                    .iter()
                    .find(|entity| {
                        entity.pass_ordinal == *pass_ordinal
                            && entity.entity_instance_id == *entity_instance_id
                            && entity.draw_order == *draw_order
                    })
                    .ok_or("ordered runtime draw references a missing entity submission")?;
                if let Some(generator_ordinal) = o04_outputs.get(ordered) {
                    PreparedRuntimeDraw::O04City {
                        ordered_draw: ordered.clone(),
                        submission: submission.clone(),
                        generator_ordinal: *generator_ordinal,
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
