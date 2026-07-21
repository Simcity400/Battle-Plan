//! Fail-closed content admission from authenticated static-map programs.
//!
//! The selected frame's `ordered_content_keys` are the only content route.
//! `pass_token` is diagnostic and never selects a semantic role. Each admitted
//! key is bound to the sealed whole-program identity and reopened through the
//! selected launch VFS before promotion.

use portable_content_manifest::effective_vfs::EffectiveVfs;
use portable_content_manifest::{normalize_game_relative_path, ContentManifest, UsedContentKey};
use portable_render_stream::{
    BufferSource, ProductionRenderFrame, RenderTargetDesc, RenderTargetInitialSource,
    StaticGeometrySource, StaticMapCommand, StaticMapPassProgram, StaticShaderResourceSource,
    TextureResourceDesc,
};
use std::collections::{BTreeMap, BTreeSet};

pub const STATIC_PROGRAM_CONSUMER_PREFIX: &str = "production_map.static_program/";
pub const RUNTIME_UI_CONSUMER_PREFIX: &str = "production_ui.command/";
pub const O05_BUILDING_CONSUMER: &str = "production_render.o05_building";

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct StaticProgramContentObservation {
    pub consumer: String,
    pub content_key: String,
    pub content: UsedContentKey,
}

/// Resolve the exact file identities named by the selected-Present O05
/// carrier. The carrier may classify only entity submissions already present
/// in the production frame; this function merely adds their exact source,
/// entity-asset, and mesh winners to the portable content manifest under the
/// O05 consumer.
pub fn resolve_authenticated_o05_building_content<'a>(
    vfs: &EffectiveVfs,
    frames: impl IntoIterator<Item = &'a ProductionRenderFrame>,
    manifest: &ContentManifest,
) -> Result<Vec<StaticProgramContentObservation>, String> {
    manifest
        .validate()
        .map_err(|error| format!("O05 content manifest is invalid: {error}"))?;
    if manifest.manifest_digest_sha256.is_none() {
        return Err("O05 content manifest is not finalized".into());
    }
    let mut observations = Vec::new();
    let mut seen = BTreeSet::new();
    for frame in frames {
        frame
            .validate()
            .map_err(|error| format!("O05 production frame is invalid: {error}"))?;
        if frame.authority.effective_vfs_manifest_sha256
            != manifest.effective_vfs_manifest_sha256
            || frame.authority.effective_vfs_manifest_sha256 != vfs.authority_manifest_sha256()
        {
            return Err("O05 frame, manifest, and selected VFS authorities differ".into());
        }
        let Some(o05) = &frame.o05_buildings else {
            continue;
        };
        let content = std::iter::once(&o05.map_buildings_source).chain(
            o05.records.iter().flat_map(|record| {
                record.entity_asset.iter().chain(record.mesh.iter())
            }),
        );
        for expected in content {
            let resolved = vfs.resolve_required(
                &expected.normalized_game_relative_path,
                "selected-Present O05 building content",
            )?;
            if resolved.content != *expected {
                return Err(format!(
                    "selected-Present O05 winner changed for {}",
                    expected.normalized_game_relative_path
                ));
            }
            if seen.insert(expected.clone()) {
                observations.push(StaticProgramContentObservation {
                    consumer: O05_BUILDING_CONSUMER.into(),
                    content_key: expected.normalized_game_relative_path.clone(),
                    content: expected.clone(),
                });
            }
        }
    }
    Ok(observations)
}

/// Resolve the exact content already sealed into typed selected-Present HUD
/// commands. This is the recorder-side half of A25 font/icon portability; it
/// admits no stable-name or path-search fallback.
pub fn resolve_authenticated_ui_pass_content<'a>(
    vfs: &EffectiveVfs,
    frames: impl IntoIterator<Item = &'a ProductionRenderFrame>,
    manifest: &ContentManifest,
) -> Result<Vec<StaticProgramContentObservation>, String> {
    manifest
        .validate()
        .map_err(|error| format!("UI-pass content manifest is invalid: {error}"))?;
    if manifest.manifest_digest_sha256.is_none() {
        return Err("UI-pass content manifest is not finalized".into());
    }
    let mut observations = Vec::new();
    let mut winner_by_pair = BTreeMap::new();
    for frame in frames {
        frame
            .validate()
            .map_err(|error| format!("UI-pass frame is invalid: {error}"))?;
        if frame.authority.effective_vfs_manifest_sha256
            != manifest.effective_vfs_manifest_sha256
            || frame.authority.effective_vfs_manifest_sha256 != vfs.authority_manifest_sha256()
        {
            return Err("UI-pass frame, manifest, and selected VFS authorities differ".into());
        }
        let Some(pass) = &frame.authenticated_ui_pass else {
            continue;
        };
        for operation in &pass.ui_pass_operations {
            let portable_render_stream::UiPassOperation::HudCommand { command } = operation else {
                continue;
            };
            let consumer = format!(
                "{RUNTIME_UI_CONSUMER_PREFIX}{}",
                command.command_identity_sha256
            );
            for content in &command.ordered_effective_vfs_used_content {
                let resolved = vfs.resolve_required(
                    &content.normalized_game_relative_path,
                    "selected-Present UI command content",
                )?;
                if resolved.content != *content {
                    return Err(format!(
                        "selected-Present UI command winner changed for {}",
                        content.normalized_game_relative_path
                    ));
                }
                let pair = (
                    consumer.clone(),
                    content.normalized_game_relative_path.clone(),
                );
                match winner_by_pair.get(&pair) {
                    Some(existing) if existing != content => {
                        return Err("selected-Present UI command content changed across frames".into())
                    }
                    Some(_) => {}
                    None => {
                        winner_by_pair.insert(pair, content.clone());
                        observations.push(StaticProgramContentObservation {
                            consumer: consumer.clone(),
                            content_key: content.normalized_game_relative_path.clone(),
                            content: content.clone(),
                        });
                    }
                }
            }
        }
    }
    Ok(observations)
}

/// Resolve the exact ordered keys named by every authenticated selected-frame
/// program. No fixed role vocabulary, diagnostic token, fallback, or omission
/// row participates in selection.
pub fn resolve_authenticated_static_program_content<'a>(
    vfs: &EffectiveVfs,
    frames: impl IntoIterator<Item = &'a ProductionRenderFrame>,
    manifest: &ContentManifest,
) -> Result<Vec<StaticProgramContentObservation>, String> {
    manifest
        .validate()
        .map_err(|error| format!("static-program content manifest is invalid: {error}"))?;
    if manifest.manifest_digest_sha256.is_none() {
        return Err("static-program content manifest is not finalized".into());
    }
    let mut observations = Vec::new();
    let mut winner_by_pair = BTreeMap::new();
    let mut frame_count = 0usize;
    for frame in frames {
        frame_count += 1;
        frame
            .validate()
            .map_err(|error| format!("static-program frame is invalid: {error}"))?;
        let authority = &frame.authority.effective_vfs_manifest_sha256;
        if authority != &manifest.effective_vfs_manifest_sha256
            || authority != vfs.authority_manifest_sha256()
        {
            return Err(
                "static-program frame, manifest, and selected VFS authorities differ".into(),
            );
        }
        let display_submit_index = frame
            .display_submit_index
            .as_deref()
            .ok_or("static-program frame has no authenticated display submit")?
            .parse::<u64>()
            .map_err(|_| "static-program display submit index is not canonical u64")?;
        let submit = frame
            .submit(display_submit_index)
            .ok_or("static-program display submit is absent from the authenticated frame")?;
        let mut seen_programs = BTreeSet::new();
        for program in &submit.ordered_static_map_programs {
            let consumer = format!("{STATIC_PROGRAM_CONSUMER_PREFIX}{}", program.payload_sha256);
            if !seen_programs.insert(consumer.clone()) {
                return Err("selected submit repeats a sealed static-program identity".into());
            }
            let closure = resolve_program_closure(vfs, program)?;
            let closure_set = closure.iter().cloned().collect::<BTreeSet<_>>();
            for content in nested_program_content(vfs, program)? {
                if !closure_set.contains(&content) {
                    return Err(format!(
                        "static program {} nested dependency {} is outside its authenticated ordered closure",
                        program.payload_sha256,
                        content.normalized_game_relative_path
                    ));
                }
            }
            for content in closure {
                let content_key = content.normalized_game_relative_path.clone();
                let pair = (consumer.clone(), content_key.clone());
                match winner_by_pair.get(&pair) {
                    Some(existing) if existing != &content => {
                        return Err(format!(
                            "effective winner changed across static frames for {content_key}"
                        ));
                    }
                    Some(_) => {}
                    None => {
                        winner_by_pair.insert(pair, content.clone());
                        observations.push(StaticProgramContentObservation {
                            consumer: consumer.clone(),
                            content_key,
                            content,
                        });
                    }
                }
            }
        }
    }
    if frame_count == 0 {
        return Err("no authenticated static-program frame was supplied".into());
    }
    Ok(observations)
}

fn resolve_program_closure(
    vfs: &EffectiveVfs,
    program: &StaticMapPassProgram,
) -> Result<Vec<UsedContentKey>, String> {
    let mut closure = Vec::new();
    let mut seen_paths = BTreeSet::new();
    let mut seen_content = BTreeSet::new();
    for key in &program.ordered_content_keys {
        let normalized = normalize_game_relative_path(key)?;
        if normalized != *key || !seen_paths.insert(normalized.clone()) {
            return Err(format!(
                "static program {} repeats or does not canonically name content path {key}",
                program.payload_sha256
            ));
        }
        let resolved = vfs.resolve_required(&normalized, "static-map program content")?;
        if !seen_content.insert(resolved.content.clone()) {
            return Err(format!(
                "static program {} repeats one effective content identity",
                program.payload_sha256
            ));
        }
        closure.push(resolved.content);
    }
    Ok(closure)
}

fn nested_program_content(
    vfs: &EffectiveVfs,
    program: &StaticMapPassProgram,
) -> Result<BTreeSet<UsedContentKey>, String> {
    let mut nested = BTreeSet::new();
    for operation in &program.ordered_operations {
        match &operation.command {
            StaticMapCommand::ClearColor { target, .. }
            | StaticMapCommand::ClearDepthStencil { target, .. } => {
                append_target_content(target, &mut nested);
            }
            StaticMapCommand::Draw { draw } => {
                nested.insert(draw.shader_program.source.clone());
                for binding in &draw.ordered_shader_resources {
                    match &binding.source {
                        StaticShaderResourceSource::UsedContent {
                            content, resource, ..
                        } => {
                            nested.insert(content.clone());
                            append_texture_resource_content(resource, &mut nested);
                        }
                        StaticShaderResourceSource::PreparedTarget { resource, .. }
                        | StaticShaderResourceSource::SnowMudTexture { resource, .. }
                        | StaticShaderResourceSource::PoliticalTexture { resource, .. }
                        | StaticShaderResourceSource::TiledLightIndex { resource, .. }
                        | StaticShaderResourceSource::TiledLightData { resource, .. } => {
                            append_texture_resource_content(resource, &mut nested);
                        }
                    }
                }
                match &draw.geometry {
                    StaticGeometrySource::Exact { geometry } => {
                        for vertex in &geometry.ordered_vertex_buffers {
                            append_buffer_source_content(&vertex.source, &mut nested);
                        }
                        if let Some(index) = &geometry.index_buffer {
                            append_buffer_source_content(&index.source, &mut nested);
                        }
                    }
                    StaticGeometrySource::InterpreterOutput {
                        ordered_input_content_keys,
                        ..
                    } => {
                        for key in ordered_input_content_keys {
                            let normalized = normalize_game_relative_path(key)?;
                            if normalized != *key {
                                return Err(format!(
                                    "static geometry interpreter key is not canonical: {key}"
                                ));
                            }
                            nested.insert(
                                vfs.resolve_required(
                                    &normalized,
                                    "static geometry interpreter input",
                                )?
                                .content,
                            );
                        }
                    }
                }
                for target in draw
                    .output_state
                    .ordered_color_targets
                    .iter()
                    .chain(draw.output_state.depth_target.iter())
                {
                    append_target_content(target, &mut nested);
                }
            }
        }
    }
    Ok(nested)
}

fn append_target_content(target: &RenderTargetDesc, out: &mut BTreeSet<UsedContentKey>) {
    if let Some(content) = &target.persistent_content {
        out.insert(content.clone());
    }
    if let Some(exact) = &target.exact_resource {
        for initial in &exact.ordered_initial_subresources {
            if let RenderTargetInitialSource::UsedContentRange { content, .. } = &initial.source {
                out.insert(content.clone());
            }
        }
    }
}

fn append_texture_resource_content(
    resource: &TextureResourceDesc,
    out: &mut BTreeSet<UsedContentKey>,
) {
    for subresource in &resource.ordered_subresources {
        append_buffer_source_content(&subresource.source, out);
    }
}

fn append_buffer_source_content(source: &BufferSource, out: &mut BTreeSet<UsedContentKey>) {
    if let BufferSource::UsedContentRange { content, .. } = source {
        out.insert(content.clone());
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use portable_content_manifest::effective_vfs::EffectiveVfsLayerMount;
    use portable_content_manifest::{
        compute_selection_sha256, ContentLayerKind, LayerStorageKind, RecordedContentLayer,
        RequiredContent, RequiredContentKind, SelectionLayerIdentity, SourceGameIdentity,
        SourceProfileIdentity,
    };
    use portable_render_stream::{
        BlendDesc, BlendFactor, BlendOperation, Comparison, CompiledShaderPayload,
        ConstantBufferBinding, CullMode, DepthDesc, DrawCommand, FillMode, IndexFormat,
        InputElementDesc, InterpreterVertexBufferDesc, ParticleRuntimeMode, PipelineDesc,
        PreparedRenderTarget, PrimitiveTopology, RasterizerDesc, RenderAuthority, RenderCheckpoint,
        RenderExecutionRef, RenderOutputState, RenderSubmit, RenderTargetAttachmentView,
        RenderTargetExactResource, RenderTargetInitialSubresource, RenderTargetResourceDesc,
        RenderTargetViewDesc, ShaderProgram, ShaderResourceViewDesc, StaticMapDraw,
        StaticMapOperation, StaticShaderResourceBinding, StencilFaceDesc, StencilOperation,
        TextureResourceDimension, ViewportDesc,
    };
    use snapshot::sha256::{to_hex, Sha256};
    use std::fs;
    use std::path::{Path, PathBuf};

    use crate::early_render_launch::{
        EarlyHookCompleteness, EarlyLaunchLayer, EarlyLaunchLayerKind, EarlyRenderCaptureReceipt,
        EarlyRenderLaunchManifest, EARLY_CAPTURE_RECEIPT_SCHEMA, EARLY_LAUNCH_MANIFEST_SCHEMA,
    };

    fn hash(bytes: &[u8]) -> String {
        let mut value = Sha256::new();
        value.update(bytes);
        to_hex(&value.finalize())
    }

    fn temp(label: &str) -> PathBuf {
        std::env::temp_dir().join(format!(
            "recorder_d80_static_{label}_{}_{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ))
    }

    fn write(root: &Path, path: &str, bytes: &[u8]) {
        let target = path
            .split('/')
            .fold(root.to_path_buf(), |path, part| path.join(part));
        fs::create_dir_all(target.parent().unwrap()).unwrap();
        fs::write(target, bytes).unwrap();
    }

    struct Fixture {
        root: PathBuf,
        vfs: EffectiveVfs,
        frame: ProductionRenderFrame,
        manifest: ContentManifest,
    }

    impl Drop for Fixture {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.root);
        }
    }

    fn fixture(label: &str) -> Fixture {
        let root = temp(label);
        let game = root.join("game");
        let active_mod = root.join("profile/mod/active");
        fs::create_dir_all(&game).unwrap();
        fs::create_dir_all(&active_mod).unwrap();
        for index in 0..10 {
            let path = format!("gfx/role/role_{index:02}.asset");
            write(&game, &path, format!("stale-base-{index}").as_bytes());
            write(
                &active_mod,
                &path,
                format!("mod-defined-role-{index}").as_bytes(),
            );
        }
        write(
            &active_mod,
            "gfx/role/custom_post.shader",
            b"mod-defined-shader-program",
        );
        write(
            &active_mod,
            "gfx/role/custom_post.mesh",
            b"mod-defined-transitive-geometry",
        );
        let authority = "a".repeat(64);
        let vfs = EffectiveVfs::new(
            authority.clone(),
            [
                EffectiveVfsLayerMount {
                    ordinal: 0,
                    layer_kind: ContentLayerKind::Base,
                    layer_id: "base".into(),
                    storage_kind: LayerStorageKind::Directory,
                    local_source: game,
                    source_container_sha256: None,
                    replace_paths: Vec::new(),
                },
                EffectiveVfsLayerMount {
                    ordinal: 1,
                    layer_kind: ContentLayerKind::Mod,
                    layer_id: "mod/active.mod".into(),
                    storage_kind: LayerStorageKind::Directory,
                    local_source: active_mod,
                    source_container_sha256: None,
                    replace_paths: vec!["gfx/role".into()],
                },
            ],
        )
        .unwrap();

        let mut required_content = Vec::new();
        let mut content_by_key = BTreeMap::new();
        for file in vfs.list_recursive("gfx/role", None).unwrap().into_iter() {
            let key = file.logical_path.clone();
            content_by_key.insert(
                file.logical_path.clone(),
                (key.clone(), file.content.clone()),
            );
            required_content.push(RequiredContent {
                content_key: key,
                kind: RequiredContentKind::Other,
                content: file.content,
                observation_count: 1,
                consumers: vec!["authenticated_static_map_program".into()],
            });
        }
        required_content.sort_by(|left, right| left.content_key.cmp(&right.content_key));
        let required_layer = SelectionLayerIdentity {
            ordinal: 1,
            layer_kind: ContentLayerKind::Mod,
            layer_id: "mod/active.mod".into(),
            descriptor_sha256: Some("b".repeat(64)),
            source_container_sha256: None,
            storage_kind: LayerStorageKind::Directory,
            replace_paths: vec!["gfx/role".into()],
        };
        let selection_sha256 = compute_selection_sha256(
            &authority,
            std::slice::from_ref(&required_layer),
            &[],
            "english",
            &BTreeMap::new(),
        )
        .unwrap();
        let manifest = ContentManifest {
            schema: portable_content_manifest::SCHEMA.into(),
            product_modset_sha256: "9".repeat(64),
            source_game: SourceGameIdentity {
                version: "1.19.2".into(),
                build: "a729".into(),
                executable_sha256: "c".repeat(64),
                build_profile: "hoi4.exe-1.19.2.0.a729".into(),
            },
            source_profile: SourceProfileIdentity {
                profile_id: "test-profile".into(),
                playset_id: "test-playset".into(),
                selection_sha256,
            },
            effective_vfs_manifest_sha256: authority.clone(),
            ordered_layers: vec![
                RecordedContentLayer {
                    ordinal: 0,
                    layer_kind: ContentLayerKind::Base,
                    layer_id: "base".into(),
                    display_name: "base".into(),
                    descriptor_sha256: None,
                    source_container_sha256: None,
                    storage_kind: LayerStorageKind::Directory,
                    replace_paths: Vec::new(),
                    required_for_playback: false,
                },
                RecordedContentLayer {
                    ordinal: required_layer.ordinal,
                    layer_kind: required_layer.layer_kind,
                    layer_id: required_layer.layer_id.clone(),
                    display_name: "Active Mod".into(),
                    descriptor_sha256: required_layer.descriptor_sha256.clone(),
                    source_container_sha256: None,
                    storage_kind: required_layer.storage_kind,
                    replace_paths: required_layer.replace_paths.clone(),
                    required_for_playback: true,
                },
            ],
            dlc_selection: Vec::new(),
            localisation_language: "english".into(),
            required_settings: BTreeMap::new(),
            required_content,
            manifest_digest_sha256: None,
        }
        .with_computed_digest()
        .unwrap();

        let target = |persistent_content: Option<UsedContentKey>, prepared_target| {
            let exact_resource =
                persistent_content
                    .as_ref()
                    .map(|content| RenderTargetExactResource {
                        resource: RenderTargetResourceDesc {
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
                            bind_flags_u32: 0x20,
                            cpu_access_flags_u32: 0,
                            misc_flags_u32: 0,
                        },
                        ordered_views: vec![RenderTargetViewDesc::RenderTarget {
                            dxgi_format_u32: 28,
                            view: RenderTargetAttachmentView::Texture2d { mip_slice: 0 },
                        }],
                        ordered_initial_subresources: vec![RenderTargetInitialSubresource {
                            subresource_ordinal: 0,
                            row_pitch: 1,
                            slice_pitch: 1,
                            source: RenderTargetInitialSource::UsedContentRange {
                                content: content.clone(),
                                source_byte_offset: 0,
                                byte_count: content.payload_bytes,
                                range_sha256: content.payload_sha256.clone(),
                            },
                        }],
                    });
            RenderTargetDesc {
                prepared_target,
                width: 64,
                height: 64,
                dxgi_format_u32: 28,
                mip_level: 0,
                sample_count: 1,
                sample_quality: 0,
                persistent_content,
                exact_resource,
            }
        };
        let mut programs = Vec::new();
        for index in 0..10 {
            let path = format!("gfx/role/role_{index:02}.asset");
            let (key, content) = content_by_key.get(&path).unwrap();
            let mut program = StaticMapPassProgram {
                program_ordinal: index as u32,
                pass_token: format!("opaque-pass:{:064x}", index + 1),
                ordered_content_keys: vec![key.clone()],
                ordered_operations: vec![StaticMapOperation {
                    operation_ordinal: index as u32,
                    payload_sha256: String::new(),
                    command: StaticMapCommand::ClearColor {
                        target: target(
                            Some(content.clone()),
                            PreparedRenderTarget::OpaqueRenderTarget {
                                attachment_ordinal: 0,
                            },
                        ),
                        rgba_f32_bits: [0; 4],
                    },
                }],
                payload_sha256: String::new(),
            };
            program.seal_payload_hashes().unwrap();
            programs.push(program);
        }
        let (shader_key, shader_content) =
            content_by_key.get("gfx/role/custom_post.shader").unwrap();
        let (mesh_key, _mesh_content) = content_by_key.get("gfx/role/custom_post.mesh").unwrap();
        let prepared = TextureResourceDesc {
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
        };
        let bytecode_vs = b"mod-vs".to_vec();
        let bytecode_ps = b"mod-ps".to_vec();
        let shader_program = ShaderProgram {
            source: shader_content.clone(),
            effect_token: "mod_effect.custom_post".into(),
            vertex_entry_token: "ModVertex".into(),
            pixel_entry_token: "ModPixel".into(),
            ordered_defines: Vec::new(),
            compiled_vertex_shader: CompiledShaderPayload {
                compiler_identity: "authenticated-test-compiler".into(),
                target_profile: "vs_5_0".into(),
                compiler_flags_u64: 0,
                sha256: hash(&bytecode_vs),
                bytecode: bytecode_vs,
            },
            compiled_pixel_shader: CompiledShaderPayload {
                compiler_identity: "authenticated-test-compiler".into(),
                target_profile: "ps_5_0".into(),
                compiler_flags_u64: 0,
                sha256: hash(&bytecode_ps),
                bytecode: bytecode_ps,
            },
            geometry_shader: None,
            hull_shader: None,
            domain_shader: None,
            compute_shader: None,
        };
        let face = StencilFaceDesc {
            fail_operation: StencilOperation::Keep,
            depth_fail_operation: StencilOperation::Keep,
            pass_operation: StencilOperation::Keep,
            comparison: Comparison::Always,
        };
        let pipeline = PipelineDesc {
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
            input_layout: vec![InputElementDesc {
                semantic_token: "POSITION".into(),
                semantic_index: 0,
                dxgi_format_u32: 6,
                input_slot: 0,
                aligned_byte_offset: 0,
                per_instance: false,
                instance_step_rate: 0,
            }],
            index_format: IndexFormat::None,
            static_indices_u16: Vec::new(),
        };
        let output_state = RenderOutputState {
            ordered_color_targets: vec![target(None, PreparedRenderTarget::SwapchainBackBuffer)],
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
        };
        let mut post = StaticMapPassProgram {
            program_ordinal: 10,
            pass_token: format!("opaque-pass:{:064x}", 11),
            ordered_content_keys: vec![mesh_key.clone(), shader_key.clone()],
            ordered_operations: vec![StaticMapOperation {
                operation_ordinal: 10,
                payload_sha256: String::new(),
                command: StaticMapCommand::Draw {
                    draw: StaticMapDraw {
                        draw_token: "mod_draw.custom_post".into(),
                        shader_program,
                        ordered_shader_resources: vec![StaticShaderResourceBinding {
                            stage: portable_render_stream::ShaderStage::Pixel,
                            slot: 0,
                            semantic_token: "ModMainScene".into(),
                            source: StaticShaderResourceSource::PreparedTarget {
                                target: PreparedRenderTarget::OpaqueRenderTarget {
                                    attachment_ordinal: 0,
                                },
                                resource: prepared,
                                view: ShaderResourceViewDesc::Texture2d {
                                    dxgi_format_u32: 28,
                                    most_detailed_mip: 0,
                                    mip_levels: 1,
                                },
                                producer: RenderExecutionRef::Static {
                                    operation_ordinal: 9,
                                },
                            },
                        }],
                        pipeline,
                        geometry: StaticGeometrySource::InterpreterOutput {
                            interpreter_token: "mod_geometry_interpreter.v1".into(),
                            ordered_input_content_keys: vec![mesh_key.clone()],
                            ordered_vertex_buffers: vec![InterpreterVertexBufferDesc {
                                input_slot: 0,
                                stride_bytes: 12,
                                byte_offset: 0,
                                vertex_count: 3,
                                payload_sha256: "d".repeat(64),
                            }],
                            index_buffer: None,
                            stream_output_targets: Vec::new(),
                            draw: DrawCommand::Draw {
                                vertex_count: 3,
                                start_vertex: 0,
                            },
                        },
                        ordered_constant_buffers: Vec::new(),
                        output_state,
                        payload_sha256: String::new(),
                    },
                },
            }],
            payload_sha256: String::new(),
        };
        post.seal_payload_hashes().unwrap();
        programs.push(post);

        let cb = vec![0u8; 16];
        let submit = RenderSubmit {
            submit_index: 1,
            global_order: 0,
            presentation_qpc_ticks: 1,
            camera_carrier_ordinal: 0,
            camera_carrier: None,
            ordered_scene_constant_buffers: vec![ConstantBufferBinding {
                stage_mask_u8: 1,
                slot: 0,
                byte_offset: 0,
                byte_count: cb.len() as u32,
                raw_bytes: cb.clone(),
                sha256: hash(&cb),
                semantic_layout: "mod_scene_constants".into(),
            }],
            ordered_static_map_programs: programs,
            tree_runtime: None,
            s1_shadow: None,
            effect_lifecycle_events: Vec::new(),
            particles: Vec::new(),
            entity_presentations: Vec::new(),
            particle_packet_set_sha256: "e".repeat(64),
            entities: Vec::new(),
            ordered_draws: Vec::new(),
            ordered_render_operations: (0..11)
                .map(|operation_ordinal| RenderExecutionRef::Static { operation_ordinal })
                .collect(),
            direct_visuals: Vec::new(),
            light_lifecycle_events: Vec::new(),
            tiled_lights: Vec::new(),
            weather_membership: Vec::new(),
            political_textures: Vec::new(),
        };
        let frame = ProductionRenderFrame {
            schema: portable_render_stream::SCHEMA.into(),
            authority: RenderAuthority {
                process_id: 7,
                process_start_nonce: 8,
                observer_installed_qpc: 9,
                d3d_device_created_qpc: 12,
                first_creation_sequence: 1,
                hook_epoch_nonce: 11,
                hook_set_receipt_sha256: "a".repeat(64),
                capture_receipt_sha256: "b".repeat(64),
                launch_epoch_sha256: portable_render_stream::compute_render_launch_epoch_sha256(
                    7,
                    8,
                    9,
                    12,
                    1,
                    11,
                    &"a".repeat(64),
                    &"b".repeat(64),
                ),
                effective_vfs_manifest_sha256: authority,
                effect_catalog_sha256: "1".repeat(64),
                entity_animation_catalog_sha256: "2".repeat(64),
                recorded_input_variant: None,
                recorded_input_receipt_sha256: None,
                particle_runtime_mode: ParticleRuntimeMode::SubmittedPacketReplay,
            },
            update_index: 16_959_360,
            first_sequence: 0,
            last_sequence: 0,
            presentation_qpc_frequency_hz: 1,
            display_submit_index: Some("1".into()),
            selected_present: portable_render_stream::SelectedPresentReceipt::sealed(
                1,
                "c".repeat(64),
                1,
                0,
                60_759_359,
                1,
                2,
            ),
            snow_mud_texture: None,
            submits: vec![submit],
            m02_combat: None,
            m03_arrows: None,
            o04_cities: None,
            o05_buildings: None,
            authenticated_ui_pass: None,
            checkpoint: RenderCheckpoint {
                last_sequence: 0,
                last_update_index: 16_959_360,
                last_submit_index: Some("1".into()),
                next_stable_ids: BTreeMap::new(),
                active_effect_lifecycle: Vec::new(),
                active_entity_presentation: Vec::new(),
                active_direct_visual_producers: Vec::new(),
                active_light_lifecycle: Vec::new(),
                latest_tiled_light_payload: None,
                weather_state_and_ordered_membership: Vec::new(),
                active_effect_set_sha256: "3".repeat(64),
                ordered_particle_packets_sha256: "4".repeat(64),
                ordered_entity_submissions_sha256: "5".repeat(64),
                ordered_direct_visual_producers_sha256: "6".repeat(64),
                ordered_light_submissions_sha256: "7".repeat(64),
                weather_state_sha256: "4f53cda18c2baa0c0354bb5f9a3ecbe5ed12ab4d8e11ba873c2f11161202b945".into(),
                weather_membership_sha256: "4f53cda18c2baa0c0354bb5f9a3ecbe5ed12ab4d8e11ba873c2f11161202b945".into(),
                weather_lifecycle_sha256: "4f53cda18c2baa0c0354bb5f9a3ecbe5ed12ab4d8e11ba873c2f11161202b945".into(),
                weather_visibility_sha256: "4f53cda18c2baa0c0354bb5f9a3ecbe5ed12ab4d8e11ba873c2f11161202b945".into(),
                ordered_weather_packets_sha256: "4f53cda18c2baa0c0354bb5f9a3ecbe5ed12ab4d8e11ba873c2f11161202b945".into(),
                weather_composition_sha256: "4f53cda18c2baa0c0354bb5f9a3ecbe5ed12ab4d8e11ba873c2f11161202b945".into(),
            },
        };
        frame.validate().unwrap();
        Fixture {
            root,
            vfs,
            frame,
            manifest,
        }
    }

    #[test]
    fn opaque_program_closure_replace_path_and_relocation_are_exact() {
        let mut observations_by_relocation = Vec::new();
        for label in ["first", "second"] {
            let mut fixture = fixture(label);
            fixture.manifest.required_content.retain(|row| {
                row.content.normalized_game_relative_path != "gfx/role/custom_post.mesh"
            });
            fixture.manifest.manifest_digest_sha256 = None;
            fixture.manifest = fixture.manifest.clone().with_computed_digest().unwrap();
            assert!(!fixture.manifest.required_content.iter().any(|row| {
                row.content.normalized_game_relative_path == "gfx/role/custom_post.mesh"
            }));
            let observations = resolve_authenticated_static_program_content(
                &fixture.vfs,
                [&fixture.frame],
                &fixture.manifest,
            )
            .unwrap();
            assert_eq!(
                observations
                    .iter()
                    .map(|row| row.consumer.as_str())
                    .collect::<BTreeSet<_>>()
                    .len(),
                fixture.frame.submits[0].ordered_static_map_programs.len()
            );
            assert!(observations.iter().all(|row| {
                row.content.winning_layer_ordinal == 1
                    && row.content.layer_kind == ContentLayerKind::Mod
            }));
            assert!(observations.iter().any(|row| {
                row.content.normalized_game_relative_path == "gfx/role/custom_post.mesh"
                    && row.consumer.starts_with(STATIC_PROGRAM_CONSUMER_PREFIX)
            }));
            assert!(observations
                .iter()
                .all(|row| { row.consumer.starts_with(STATIC_PROGRAM_CONSUMER_PREFIX) }));
            observations_by_relocation.push(observations);
        }
        assert_eq!(observations_by_relocation[0], observations_by_relocation[1]);
    }

    #[test]
    fn opaque_program_duplicate_or_incomplete_content_fails_closed() {
        let mut ambiguous = fixture("ambiguous");
        let duplicate = ambiguous.frame.submits[0].ordered_static_map_programs[0].clone();
        ambiguous.frame.submits[0]
            .ordered_static_map_programs
            .insert(1, duplicate);
        assert!(resolve_authenticated_static_program_content(
            &ambiguous.vfs,
            [&ambiguous.frame],
            &ambiguous.manifest,
        )
        .is_err());

        let mut incomplete = fixture("incomplete");
        let post = incomplete.frame.submits[0]
            .ordered_static_map_programs
            .last_mut()
            .unwrap();
        post.ordered_content_keys.remove(0);
        post.seal_payload_hashes().unwrap();
        assert!(resolve_authenticated_static_program_content(
            &incomplete.vfs,
            [&incomplete.frame],
            &incomplete.manifest,
        )
        .is_err());
    }

    #[test]
    fn exact_render_target_initial_content_enters_the_program_closure() {
        let fixture = fixture("exact-target-content");
        let content = fixture
            .vfs
            .resolve_required("gfx/role/role_00.asset", "test target initialization")
            .unwrap()
            .content;
        let target = RenderTargetDesc {
            prepared_target: PreparedRenderTarget::OpaqueRenderTarget {
                attachment_ordinal: 0,
            },
            width: 1,
            height: 1,
            dxgi_format_u32: 28,
            mip_level: 0,
            sample_count: 1,
            sample_quality: 0,
            persistent_content: None,
            exact_resource: Some(RenderTargetExactResource {
                resource: RenderTargetResourceDesc {
                    dimension: TextureResourceDimension::Texture2d,
                    width: 1,
                    height: 1,
                    depth: 1,
                    mip_levels: 1,
                    array_size: 1,
                    dxgi_format_u32: 28,
                    sample_count: 1,
                    sample_quality: 0,
                    usage_u32: 0,
                    bind_flags_u32: 0x20,
                    cpu_access_flags_u32: 0,
                    misc_flags_u32: 0,
                },
                ordered_views: Vec::new(),
                ordered_initial_subresources: vec![RenderTargetInitialSubresource {
                    subresource_ordinal: 0,
                    row_pitch: 4,
                    slice_pitch: 4,
                    source: RenderTargetInitialSource::UsedContentRange {
                        content: content.clone(),
                        source_byte_offset: 0,
                        byte_count: 4,
                        range_sha256: "9".repeat(64),
                    },
                }],
            }),
        };
        let mut nested = BTreeSet::new();
        append_target_content(&target, &mut nested);
        assert_eq!(nested, BTreeSet::from([content]));
    }

    fn early_admission_for(
        frame: &mut ProductionRenderFrame,
    ) -> (EarlyRenderLaunchManifest, EarlyRenderCaptureReceipt) {
        let manifest = EarlyRenderLaunchManifest {
            schema: EARLY_LAUNCH_MANIFEST_SCHEMA.into(),
            executable_sha256: "9".repeat(64),
            collector_sha256: "8".repeat(64),
            effective_vfs_manifest_sha256: frame.authority.effective_vfs_manifest_sha256.clone(),
            ordered_content_layers: vec![EarlyLaunchLayer {
                kind: EarlyLaunchLayerKind::Base,
                layer_id: "base".into(),
                descriptor_sha256: None,
                content_sha256: "7".repeat(64),
                storage_kind: LayerStorageKind::Directory,
            }],
            requested_profile_selection:
                crate::early_render_launch::RequestedProfileSelectionReceipt {
                    schema: crate::runtime_dlc_roster::REQUESTED_PROFILE_SELECTION_SCHEMA.into(),
                    dlc_load_sha256: "6".repeat(64),
                    disabled_dlc_descriptor_ids: Vec::new(),
                    requested_enabled_dlc_ids: Vec::new(),
                },
            effective_runtime_dlc_candidate_ids: Vec::new(),
            localisation_language: "english".into(),
            required_settings: BTreeMap::from([("multi_sampling".into(), "4".into())]),
            semantic_arguments: Vec::new(),
        };
        manifest.validate().unwrap();

        let mut receipt = EarlyRenderCaptureReceipt {
            schema: EARLY_CAPTURE_RECEIPT_SCHEMA.into(),
            launch_manifest_sha256: manifest.sha256().unwrap(),
            effective_vfs_manifest_sha256: manifest.effective_vfs_manifest_sha256.clone(),
            effective_vfs_index_sha256: "8".repeat(64),
            effective_vfs_index_row_count: 3,
            effective_vfs_index_chunk_count: 2,
            content_manifest_bytes_sha256: "9".repeat(64),
            content_manifest_byte_count: 512,
            content_manifest_chunk_count: 1,
            process_id: frame.authority.process_id,
            process_start_nonce: frame.authority.process_start_nonce,
            executable_sha256: manifest.executable_sha256.clone(),
            collector_sha256: manifest.collector_sha256.clone(),
            created_suspended: true,
            hook_epoch_nonce: frame.authority.hook_epoch_nonce,
            hook_set_sha256: frame.authority.hook_set_receipt_sha256.clone(),
            hook_ready_sequence: 1,
            resume_sequence: 2,
            hook_ready_qpc: 9,
            resume_qpc: 10,
            first_creation_sequence: frame.authority.first_creation_sequence,
            first_game_d3d_creation_qpc: frame.authority.d3d_device_created_qpc,
            staged_hook_generation:
                compact_contract::early_render_control::EARLY_STAGED_HOOK_GENERATION,
            staged_hook_count: compact_contract::early_render_control::EARLY_STAGED_HOOK_COUNT,
            staged_hook_set_sha256: "a".repeat(64),
            diagnostic_staged_hook_mask: None,
            game_device_ptr: 0x1000,
            immediate_context_ptr: 0x2000,
            effective_runtime_dlc: {
                let names = Vec::<String>::new();
                crate::early_render_launch::EffectiveRuntimeDlcReceipt {
                    schema: crate::runtime_dlc_roster::EFFECTIVE_RUNTIME_DLC_SCHEMA.into(),
                    source: "engine_active_dlc_manager_v1".into(),
                    manager_getter_rva: compact_contract::early_render_runtime_dlc::A729_DLC_MANAGER_GETTER_RVA,
                    active_dlc_enumerator_rva: compact_contract::early_render_runtime_dlc::A729_ACTIVE_DLC_ENUMERATOR_RVA,
                    string_vector_teardown_rva: compact_contract::early_render_runtime_dlc::A729_STRING_VECTOR_TEARDOWN_RVA,
                    sample_count: 2,
                    double_stable: true,
                    active_display_names_engine_order: names.clone(),
                    enabled_dlc_ids: Vec::new(),
                    display_name_to_layer_id: BTreeMap::new(),
                    engine_order_sha256: snapshot::sha256::to_hex(
                        &compact_contract::early_render_runtime_dlc::canonical_runtime_dlc_roster_sha256(&names),
                    ),
                }
            },
            game_d3d_creation_count_at_hook_ready: 0,
            preexisting_game_d3d_object_count: 0,
            completeness: EarlyHookCompleteness {
                all_shader_create: true,
                input_layout_create: true,
                blend_state_create: true,
                depth_stencil_state_create: true,
                rasterizer_state_create: true,
                sampler_state_create: true,
                buffer_create_update: true,
                texture_create_update: true,
                resource_view_create: true,
                draw_state_readback: true,
            },
            transcript_sha256: String::new(),
        };
        receipt.transcript_sha256 = early_transcript_sha256(&receipt);
        receipt.validate().unwrap();
        frame.authority.capture_receipt_sha256 = receipt.sha256().unwrap();
        reseal_launch_epoch(frame);
        frame.validate().unwrap();
        (manifest, receipt)
    }

    fn early_transcript_sha256(receipt: &EarlyRenderCaptureReceipt) -> String {
        #[derive(serde::Serialize)]
        struct Transcript<'a> {
            events: [&'static str; 8],
            launch_manifest_sha256: &'a str,
            effective_vfs_index_sha256: &'a str,
            effective_vfs_index_row_count: u32,
            effective_vfs_index_chunk_count: u32,
            content_manifest_bytes_sha256: &'a str,
            content_manifest_byte_count: u64,
            content_manifest_chunk_count: u32,
            process_id: u32,
            process_start_nonce: u64,
            hook_epoch_nonce: u64,
            hook_set_sha256: &'a str,
            hook_ready_sequence: u64,
            resume_sequence: u64,
            hook_ready_qpc: u64,
            resume_qpc: u64,
            first_creation_sequence: u64,
            first_game_d3d_creation_qpc: u64,
            staged_hook_generation: u64,
            staged_hook_count: u32,
            staged_hook_set_sha256: &'a str,
            game_device_ptr: u64,
            immediate_context_ptr: u64,
            effective_runtime_dlc: &'a crate::early_render_launch::EffectiveRuntimeDlcReceipt,
        }
        let transcript = Transcript {
            events: [
                "created_suspended",
                "injection_committed",
                "collector_epoch_bound",
                "hooks_ready",
                "primary_thread_resumed",
                "first_game_d3d_creation",
                "staged_hook_set_ready",
                "runtime_dlc_roster_validated",
            ],
            launch_manifest_sha256: &receipt.launch_manifest_sha256,
            effective_vfs_index_sha256: &receipt.effective_vfs_index_sha256,
            effective_vfs_index_row_count: receipt.effective_vfs_index_row_count,
            effective_vfs_index_chunk_count: receipt.effective_vfs_index_chunk_count,
            content_manifest_bytes_sha256: &receipt.content_manifest_bytes_sha256,
            content_manifest_byte_count: receipt.content_manifest_byte_count,
            content_manifest_chunk_count: receipt.content_manifest_chunk_count,
            process_id: receipt.process_id,
            process_start_nonce: receipt.process_start_nonce,
            hook_epoch_nonce: receipt.hook_epoch_nonce,
            hook_set_sha256: &receipt.hook_set_sha256,
            hook_ready_sequence: receipt.hook_ready_sequence,
            resume_sequence: receipt.resume_sequence,
            hook_ready_qpc: receipt.hook_ready_qpc,
            resume_qpc: receipt.resume_qpc,
            first_creation_sequence: receipt.first_creation_sequence,
            first_game_d3d_creation_qpc: receipt.first_game_d3d_creation_qpc,
            staged_hook_generation: receipt.staged_hook_generation,
            staged_hook_count: receipt.staged_hook_count,
            staged_hook_set_sha256: &receipt.staged_hook_set_sha256,
            game_device_ptr: receipt.game_device_ptr,
            immediate_context_ptr: receipt.immediate_context_ptr,
            effective_runtime_dlc: &receipt.effective_runtime_dlc,
        };
        hash(&serde_json::to_vec(&transcript).unwrap())
    }

    fn reseal_launch_epoch(frame: &mut ProductionRenderFrame) {
        let authority = &mut frame.authority;
        authority.launch_epoch_sha256 = portable_render_stream::compute_render_launch_epoch_sha256(
            authority.process_id,
            authority.process_start_nonce,
            authority.observer_installed_qpc,
            authority.d3d_device_created_qpc,
            authority.first_creation_sequence,
            authority.hook_epoch_nonce,
            &authority.hook_set_receipt_sha256,
            &authority.capture_receipt_sha256,
        );
    }

    #[test]
    fn production_frame_requires_exact_authenticated_early_launch_epoch() {
        let mut fixture = fixture("early-admission");
        let (manifest, receipt) = early_admission_for(&mut fixture.frame);
        crate::portable_content_manifest::validate_render_frame_early_admission(
            &fixture.frame,
            &manifest,
            &receipt,
        )
        .unwrap();

        let mut foreign_process = fixture.frame.clone();
        foreign_process.authority.process_id += 1;
        reseal_launch_epoch(&mut foreign_process);
        foreign_process.validate().unwrap();
        assert!(
            crate::portable_content_manifest::validate_render_frame_early_admission(
                &foreign_process,
                &manifest,
                &receipt,
            )
            .unwrap_err()
            .contains("foreign")
        );

        let mut late_observer = fixture.frame.clone();
        late_observer.authority.observer_installed_qpc = receipt.hook_ready_qpc + 1;
        reseal_launch_epoch(&mut late_observer);
        late_observer.validate().unwrap();
        assert!(
            crate::portable_content_manifest::validate_render_frame_early_admission(
                &late_observer,
                &manifest,
                &receipt,
            )
            .unwrap_err()
            .contains("foreign")
        );

        let mut early_observer = fixture.frame.clone();
        early_observer.authority.observer_installed_qpc = receipt.hook_ready_qpc - 1;
        reseal_launch_epoch(&mut early_observer);
        early_observer.validate().unwrap();
        assert!(
            crate::portable_content_manifest::validate_render_frame_early_admission(
                &early_observer,
                &manifest,
                &receipt,
            )
            .unwrap_err()
            .contains("foreign")
        );

        let mut foreign_first_creation = fixture.frame.clone();
        foreign_first_creation.authority.first_creation_sequence += 1;
        reseal_launch_epoch(&mut foreign_first_creation);
        foreign_first_creation.validate().unwrap();
        assert!(
            crate::portable_content_manifest::validate_render_frame_early_admission(
                &foreign_first_creation,
                &manifest,
                &receipt,
            )
            .unwrap_err()
            .contains("foreign")
        );
    }
}
