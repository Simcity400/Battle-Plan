//! Original ordinary CSprite VS/raster/PS packets; no CPU shader or sampler.
//! Native 14222d5a0 uploads matrix, color, offsets and two retained clocks.
//! The owner supplies the reached local strip/upload matrix, native pipeline,
//! viewport and authenticated resource descriptors. No missing input defaults.

use crate::assets::AssetResolver;
use crate::container_sprite_geometry::ContainerSpriteUvGeometry;
use crate::container_sprite_inputs::NativeButtonEffect;
use crate::container_sprite_resource_schedule::{
    InitializedOrdinarySpriteTypeResources, OrdinarySpriteResourceBinding,
};
use crate::container_sprite_resources::RegisteredOrdinaryTexture;
use crate::gfx::SpriteDefinition;
use crate::runtime_graphics::{
    admit_container_sprite_runtime_graphics, AdmittedRuntimeGraphics, AuthenticatedUiShaderLaw,
    ContainerSpriteRequest,
};
use map_viewer::native_ui::{
    NativeUiDrawInputs, NativeUiTargetDesc, PreparedNativeUiDraw,
};
use map_viewer::{DxbcReflection, GameFileResolver};
use portable_render_stream::{
    BufferSource, CompiledShaderPayload, ConstantBufferBinding, DrawCommand, GeometryBinding,
    IndexFormat, InputElementDesc, PipelineDesc, PrimitiveTopology, ScissorDesc, ShaderDefine,
    ShaderDefineScope, ShaderProgram, ShaderStage, TextureBinding, VertexBufferBinding,
    ViewportDesc,
};
use snapshot::sha256::Sha256;

type DrawResult<T> = Result<T, String>;

pub(crate) struct ContainerSpriteGpuInputs<'a> {
    /// Actual native vertices before projection, in original strip order.
    pub local_positions: &'a [[f32; 3]; 4],
    pub uv: &'a ContainerSpriteUvGeometry,
    /// Exact 64-byte native CPU upload, including its existing matrix packing.
    /// Do not transpose, multiply or project vertices in this packet builder.
    pub world_view_projection_bits: &'a [u32; 16],
    /// Explicit compiler environment; None is unknown, not an empty define set.
    pub engine_defines: Option<&'a [(String, String)]>,
    /// Source execution policy for the reached native CPU color/offset writers.
    /// None never inherits the current viewer thread's MXCSR.
    pub native_mxcsr: Option<u32>,
    /// The actual retained type resource bundle. Raster passes never register
    /// texture paths or repeat a generic primary_texture() selection.
    pub resources: &'a InitializedOrdinarySpriteTypeResources,
    pub pipeline: &'a PipelineDesc,
    pub viewport: &'a ViewportDesc,
    pub scissor: Option<&'a ScissorDesc>,
    pub target: &'a NativeUiTargetDesc,
}

pub(crate) struct ContainerSpriteDrawPacket<'a> {
    shader: ShaderProgram,
    geometry: GeometryBinding,
    constants: Vec<ConstantBufferBinding>,
    pipeline: &'a PipelineDesc,
    textures: Vec<TextureBinding>,
    viewport: &'a ViewportDesc,
    scissor: Option<&'a ScissorDesc>,
    target: &'a NativeUiTargetDesc,
}

impl<'a> ContainerSpriteDrawPacket<'a> {
    /// Reuses the established content/runtime admission before compilation.
    /// Source parser controls and matrix arithmetic remain owned upstream.
    #[cfg(windows)]
    pub(crate) fn build(
        assets: &AssetResolver,
        resolver: &GameFileResolver,
        sprite: &SpriteDefinition,
        request: &ContainerSpriteRequest,
        inputs: ContainerSpriteGpuInputs<'a>,
    ) -> DrawResult<Self> {
        require_cpu_controls(inputs.native_mxcsr)?;
        let reached = request
            .reached
            .ok_or("native CSprite effect/time receipt unavailable")?;
        let base = ordinary_base(inputs.resources)?;
        if !inputs.resources.requests.matches_definition(assets, sprite) {
            return Err("native UI type definition custody differs".into());
        }
        if reached.admitted_animation_count != inputs.resources.animations.len() {
            return Err("native UI reached animation census differs from retained type".into());
        }
        let admitted = admit_container_sprite_runtime_graphics(assets, sprite, request)
            .map_err(|e| format!("native UI runtime admission: {e:?}"))?;
        let law = admitted
            .shader_law()
            .ok_or("authenticated UI shader law unavailable")?;
        let effect_token =
            ordinary_effect(law, reached.effect, admitted.container_animation_count())?;
        if !sprite.kind.eq_ignore_ascii_case("spriteType") || sprite.border_size.is_some() {
            return Err("ordinary CSprite strip factory unavailable".into());
        }
        let path = crate::gfx::a729_effect_shader_path(sprite.effect_file.as_deref())
            .map_err(|e| format!("native UI effect path: {e:?}"))?;
        let source = assets
            .admitted_content_key(&path)
            .ok_or("native UI effect content identity unavailable")?
            .clone();
        if resolver
            .admitted_used_content_key(&path)
            .map_err(|e| e.to_string())?
            .as_ref()
            != Some(&source)
        {
            return Err("native UI asset and GPU effect admissions differ".into());
        }
        resolver
            .read_admitted_used_content(&source)
            .map_err(|e| e.to_string())?;
        // Check the exact same winner for every translated include/platform.
        let files = map_viewer::effect_translation_unit_files(resolver, &path)
            .map_err(|e| e.to_string())?;
        for file in files
            .iter()
            .map(|(path, _)| path.as_str())
            .chain(std::iter::once(map_viewer::DX11_PLATFORM_HEADER))
        {
            let key = resolver
                .admitted_used_content_key(file)
                .map_err(|e| e.to_string())?
                .ok_or_else(|| format!("native UI translation input not admitted: {file}"))?;
            if assets.admitted_content_key(file) != Some(&key) {
                return Err(format!("native UI translation admissions differ: {file}"));
            }
            resolver
                .read_admitted_used_content(&key)
                .map_err(|e| e.to_string())?;
        }
        let file = &files.last().ok_or("native UI translation unit empty")?.1;
        if file.effect_count(effect_token) != 1 {
            return Err(format!(
                "native UI effect {effect_token} does not resolve exactly once"
            ));
        }
        let effect = file.effect(effect_token).expect("count validated");
        let defines = inputs
            .engine_defines
            .ok_or("native UI compiler environment unavailable")?;
        // Animated/masking shader rows have separate producer and resource laws.
        if defines
            .iter()
            .any(|(name, _)| matches!(name.as_str(), "ANIMATED" | "MASKING"))
            || !effect.defines.is_empty()
        {
            return Err("ordinary native UI effect defines select an unsupported branch".into());
        }
        let compile =
            |stage, profile: &str| -> DrawResult<(CompiledShaderPayload, DxbcReflection)> {
                let hlsl = map_viewer::build_stage_hlsl_with_defines(
                    resolver,
                    &path,
                    effect_token,
                    stage,
                    defines,
                )
                .map_err(|e| e.to_string())?;
                let bytes =
                    map_viewer::d3d_compile_stage(&hlsl, stage, map_viewer::ENGINE_COMPILE_FLAGS)
                        .map_err(|e| e.to_string())?;
                let reflection =
                    map_viewer::parse_dxbc_reflection(&bytes).map_err(|e| e.to_string())?;
                Ok((
                    CompiledShaderPayload {
                        compiler_identity: format!("D3DCompile/map_viewer: {}", reflection.creator),
                        target_profile: profile.into(),
                        compiler_flags_u64: map_viewer::ENGINE_COMPILE_FLAGS as u64,
                        sha256: Sha256::hex(&bytes),
                        bytecode: bytes,
                    },
                    reflection,
                ))
            };
        let (vertex, vs) = compile(map_viewer::ShaderStage::Vertex, "vs_5_0")?;
        let (pixel, ps) = compile(map_viewer::ShaderStage::Pixel, "ps_5_0")?;
        let shader = ShaderProgram {
            source,
            producer_registration_root_rva: None,
            effect_token: effect_token.into(),
            vertex_entry_token: effect
                .vertex_shader
                .clone()
                .ok_or("native UI VS entry unavailable")?,
            pixel_entry_token: effect
                .pixel_shader
                .clone()
                .ok_or("native UI PS entry unavailable")?,
            ordered_defines: defines
                .iter()
                .map(|(name, value)| ShaderDefine {
                    name: name.clone(),
                    value: value.clone(),
                    scope: ShaderDefineScope::EngineGlobal,
                })
                .collect(),
            compiled_vertex_shader: vertex,
            compiled_pixel_shader: pixel,
            geometry_shader: None,
            hull_shader: None,
            domain_shader: None,
            compute_shader: None,
        };
        validate_ordinary_layout(inputs.pipeline)?;
        let texture = base
            .native_texture_binding(ShaderStage::Pixel, 0, "MapTexture")
            .map_err(|error| format!("native UI retained texture: {error:?}"))?;
        if assets.admitted_content_key(&base.content().normalized_game_relative_path)
            != Some(base.content())
            || resolver
                .admitted_used_content_key(&base.content().normalized_game_relative_path)
                .map_err(|error| error.to_string())?
                .as_ref()
                != Some(base.content())
        {
            return Err("native UI retained base resource admission differs".into());
        }
        let native = pack_native_constants(admitted, inputs.uv, inputs.world_view_projection_bits)?;
        let mut constants = reflected_constants(&vs, 1, law, &native)?;
        constants.extend(reflected_constants(&ps, 2, law, &native)?);
        let geometry = pack_local_strip(inputs.local_positions, &inputs.uv.vertex_uvs)?;
        let packet = Self {
            shader,
            geometry,
            constants,
            pipeline: inputs.pipeline,
            textures: vec![texture],
            viewport: inputs.viewport,
            scissor: inputs.scissor,
            target: inputs.target,
        };
        // Shared facade rechecks bytecode, every consumed CB/resource/sampler,
        // descriptors and draw ranges. No packet bypasses the actual compiler.
        packet.prepare(resolver)?;
        Ok(packet)
    }

    #[cfg(not(windows))]
    pub(crate) fn build(
        _assets: &AssetResolver,
        _resolver: &GameFileResolver,
        _sprite: &SpriteDefinition,
        _request: &ContainerSpriteRequest,
        _inputs: ContainerSpriteGpuInputs<'a>,
    ) -> DrawResult<Self> {
        Err("native UI packet compilation requires the Windows compiler".into())
    }

    pub(crate) fn inputs(&self) -> NativeUiDrawInputs<'_> {
        NativeUiDrawInputs {
            shader: &self.shader,
            textures: &self.textures,
            pipeline: self.pipeline,
            geometry: &self.geometry,
            scene_constants: &[],
            draw_constants: &self.constants,
            viewport: self.viewport,
            scissor: self.scissor,
            target: self.target,
        }
    }

    pub(crate) fn prepare(
        &self,
        resolver: &GameFileResolver,
    ) -> DrawResult<PreparedNativeUiDraw<'_>> {
        PreparedNativeUiDraw::admit(resolver, self.inputs()).map_err(|e| e.to_string())
    }
}

fn require_cpu_controls(native: Option<u32>) -> DrawResult<()> {
    if !native.is_some_and(|control| control & 0xffc0 == 0x1f80) {
        return Err("native UI source floating-point control unavailable".into());
    }
    #[cfg(any(target_arch = "x86", target_arch = "x86_64"))]
    {
        let mut control = 0_u32;
        unsafe {
            std::arch::asm!("stmxcsr [{out}]", out = in(reg) &mut control,
            options(nostack, preserves_flags));
        }
        if control & 0xffc0 == 0x1f80 {
            return Ok(());
        }
    }
    Err("native UI viewer floating-point control unsupported".into())
}

fn ordinary_base(
    resources: &InitializedOrdinarySpriteTypeResources,
) -> DrawResult<&RegisteredOrdinaryTexture> {
    if !resources.animations.is_empty() {
        return Err("native UI animated type resource/constant-buffer row unavailable".into());
    }
    match &resources.masking_texture {
        OrdinarySpriteResourceBinding::Absent
        | OrdinarySpriteResourceBinding::AuthoredEmpty { .. } => {}
        OrdinarySpriteResourceBinding::Loaded(_) => {
            return Err("native UI masking shader row unavailable".into());
        }
        other => {
            return Err(format!(
                "native UI masking resource state unavailable: {other:?}"
            ))
        }
    }
    match &resources.base {
        OrdinarySpriteResourceBinding::Loaded(base) => Ok(base.as_ref()),
        OrdinarySpriteResourceBinding::Absent => {
            Err("native UI base filename is witnessed absent".into())
        }
        OrdinarySpriteResourceBinding::AuthoredEmpty { .. } => {
            Err("native UI base filename is authored empty".into())
        }
        OrdinarySpriteResourceBinding::DeferredBase(_) => {
            Err("native UI base resource has not reached its deferred loader".into())
        }
        OrdinarySpriteResourceBinding::NotInitializedByDeferredSchedule { .. } => {
            Err("native UI base resource was not initialized by its deferred schedule".into())
        }
    }
}

pub(crate) fn ordinary_effect(
    law: AuthenticatedUiShaderLaw,
    effect: NativeButtonEffect,
    animations: usize,
) -> DrawResult<&'static str> {
    if !matches!(
        law,
        AuthenticatedUiShaderLaw::StaticButton | AuthenticatedUiShaderLaw::ButtonState
    ) {
        return Err("ordinary native UI shader family unavailable".into());
    }
    if animations != 0 {
        return Err("native UI animated shader row/constant buffers/resources unavailable".into());
    }
    // 14222d1a0 overrides default only for raw2/3/4. Keep raw0 in its owner.
    Ok(match effect {
        NativeButtonEffect::Unassigned | NativeButtonEffect::Up => "Up",
        NativeButtonEffect::Down => "Down",
        NativeButtonEffect::Over => "Over",
        NativeButtonEffect::Disable => "Disable",
    })
}

fn pack_local_strip(positions: &[[f32; 3]; 4], uvs: &[[f32; 2]; 4]) -> DrawResult<GeometryBinding> {
    let mut bytes = Vec::with_capacity(80);
    for (position, uv) in positions.iter().zip(uvs) {
        for value in position.iter().chain(uv) {
            if !value.is_finite() {
                return Err("native UI nonfinite local vertex".into());
            }
            bytes.extend_from_slice(&value.to_bits().to_le_bytes());
        }
    }
    Ok(GeometryBinding {
        ordered_vertex_buffers: vec![VertexBufferBinding {
            input_slot: 0,
            stride_bytes: 20,
            byte_offset: 0,
            source: BufferSource::Inline {
                sha256: Sha256::hex(&bytes),
                bytes,
            },
        }],
        index_buffer: None,
        stream_output_targets: vec![],
        draw: DrawCommand::Draw {
            vertex_count: 4,
            start_vertex: 0,
        },
    })
}

fn validate_ordinary_layout(pipeline: &PipelineDesc) -> DrawResult<()> {
    let element = |semantic: &str, format, offset| InputElementDesc {
        semantic_token: semantic.into(),
        semantic_index: 0,
        dxgi_format_u32: format,
        input_slot: 0,
        aligned_byte_offset: offset,
        per_instance: false,
        instance_step_rate: 0,
    };
    if pipeline.topology != PrimitiveTopology::TriangleStrip
        || pipeline.index_format != IndexFormat::None
        || !pipeline.static_indices_u16.is_empty()
        || pipeline.input_layout != [element("POSITION", 6, 0), element("TEXCOORD", 16, 12)]
    {
        return Err("native UI ordinary position3/uv2 strip input layout unavailable".into());
    }
    Ok(())
}

fn pack_native_constants(
    admitted: AdmittedRuntimeGraphics,
    uv: &ContainerSpriteUvGeometry,
    matrix: &[u32; 16],
) -> DrawResult<[u8; 112]> {
    // Explicit positive CIcon frames bypass the native count test. UV geometry
    // retains the original frame step; it does not clamp the selected offset.
    let index = i32::try_from(admitted.frame().zero_based_index).map_err(|e| e.to_string())?;
    let offset = uv
        .frame_offset(index)
        .map_err(|e| format!("native UI Offset: {e:?}"))?;
    let next = uv
        .frame_offset(index.wrapping_add(1))
        .map_err(|e| format!("native UI NextOffset: {e:?}"))?;
    pack_native_words(
        matrix,
        admitted.color().0.map(f32::to_bits),
        offset.map(f32::to_bits),
        next.map(f32::to_bits),
        admitted.time().to_bits(),
        admitted.animation_time().to_bits(),
    )
}

fn pack_native_words(
    matrix: &[u32; 16],
    color: [u32; 4],
    offset: [u32; 2],
    next: [u32; 2],
    time: u32,
    animation_time: u32,
) -> DrawResult<[u8; 112]> {
    let mut bytes = [0; 112];
    // 14222d5a0 writes offsets 0..103. Tail104..111 is HLSL alignment only;
    // reflected_constants refuses any shader that consumes an unknown tail.
    for (index, bits) in matrix
        .iter()
        .copied()
        .chain(color)
        .chain(offset)
        .chain(next)
        .chain([time, animation_time])
        .enumerate()
    {
        if !f32::from_bits(bits).is_finite() {
            return Err("native UI nonfinite constant operand".into());
        }
        bytes[index * 4..index * 4 + 4].copy_from_slice(&bits.to_le_bytes());
    }
    Ok(bytes)
}

fn reflected_constants(
    reflection: &DxbcReflection,
    stage_mask: u8,
    law: AuthenticatedUiShaderLaw,
    native: &[u8; 112],
) -> DrawResult<Vec<ConstantBufferBinding>> {
    let expected_size = if law == AuthenticatedUiShaderLaw::StaticButton {
        96
    } else {
        112
    };
    let mut out = Vec::new();
    for resource in &reflection.resources {
        if resource.bind_count != 1 {
            return Err(format!(
                "native UI resource array unavailable: {}",
                resource.name
            ));
        }
        match resource.resource_type {
            0 => {
                let cb = reflection
                    .cbuffers
                    .iter()
                    .find(|cb| cb.name == resource.name)
                    .ok_or("native UI reflected CB definition unavailable")?;
                if resource.bind_point != 0 || cb.size != expected_size {
                    return Err(format!(
                        "native UI unbound constant buffer {} at b{} size{}",
                        cb.name, resource.bind_point, cb.size
                    ));
                }
                for variable in &cb.vars {
                    let expected = match variable.name.as_str() {
                        "WorldViewProjectionMatrix" => Some((0, 64)),
                        "Color" => Some((64, 16)),
                        "vXOffset" if law == AuthenticatedUiShaderLaw::StaticButton => {
                            Some((80, 4))
                        }
                        "Offset" if law == AuthenticatedUiShaderLaw::ButtonState => Some((80, 8)),
                        "NextOffset" if law == AuthenticatedUiShaderLaw::ButtonState => {
                            Some((88, 8))
                        }
                        "Time" if law == AuthenticatedUiShaderLaw::ButtonState => Some((96, 4)),
                        "AnimationTime" if law == AuthenticatedUiShaderLaw::ButtonState => {
                            Some((100, 4))
                        }
                        _ => None,
                    };
                    if expected != Some((variable.offset, variable.size)) {
                        return Err(format!(
                            "native UI reflected operand has no native producer: {}",
                            variable.name
                        ));
                    }
                }
                let raw_bytes = native[..cb.size as usize].to_vec();
                out.push(ConstantBufferBinding {
                    stage_mask_u8: stage_mask,
                    slot: 0,
                    byte_offset: 0,
                    byte_count: cb.size,
                    sha256: Sha256::hex(&raw_bytes),
                    raw_bytes,
                    semantic_layout: format!(
                        "native CSprite14222d5a0;{}:{}:{}",
                        cb.name,
                        cb.size,
                        cb.vars
                            .iter()
                            .map(|v| format!("{}@{}+{}", v.name, v.offset, v.size))
                            .collect::<Vec<_>>()
                            .join(",")
                    ),
                });
            }
            2 if stage_mask == 2 && resource.bind_point == 0 && resource.dimension == 4 => {}
            3 if stage_mask == 2 && resource.bind_point == 0 => {}
            _ => {
                return Err(format!(
                    "native UI resource has no ordinary owner: {} type{} slot{}",
                    resource.name, resource.resource_type, resource.bind_point
                ))
            }
        }
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    #[cfg(windows)]
    mod reached_compositor;

    use super::*;
    use map_viewer::{DxbcCbuffer, DxbcCbufferVar, DxbcResource};

    #[test]
    fn raw_zero_dispatch_does_not_change_widget_effect_and_animated_rows_refuse() {
        let raw = NativeButtonEffect::Unassigned;
        assert_eq!(
            ordinary_effect(AuthenticatedUiShaderLaw::ButtonState, raw, 0).unwrap(),
            "Up"
        );
        assert_eq!(raw as i32, 0);
        assert_eq!(
            ordinary_effect(
                AuthenticatedUiShaderLaw::StaticButton,
                NativeButtonEffect::Disable,
                0
            )
            .unwrap(),
            "Disable"
        );
        assert!(ordinary_effect(AuthenticatedUiShaderLaw::ButtonState, raw, 1).is_err());
        assert!(ordinary_effect(AuthenticatedUiShaderLaw::ButtonStateBlendFrames, raw, 0).is_err());
    }

    #[test]
    fn original_fractional_strip_and_signed_zero_are_copied_without_projection() {
        let positions = [
            [0.0, -0.0, 1.0],
            [0.0, 13.0 / 3.0, 1.0],
            [7.0, 0.0, 1.0],
            [7.0, 13.0 / 3.0, 1.0],
        ];
        let uvs = [[0.0, 1.0], [0.0, 0.0], [1.0, 1.0], [1.0, 0.0]];
        let packed = pack_local_strip(&positions, &uvs).unwrap();
        let BufferSource::Inline { bytes, .. } = &packed.ordered_vertex_buffers[0].source else {
            panic!()
        };
        assert_eq!(bytes.len(), 80);
        assert_eq!(&bytes[4..8], &(-0.0_f32).to_le_bytes());
        assert_eq!(&bytes[24..28], &(13.0_f32 / 3.0).to_le_bytes());
        assert_eq!(
            packed.draw,
            DrawCommand::Draw {
                vertex_count: 4,
                start_vertex: 0
            }
        );
    }

    #[test]
    fn native_cb_offsets_copy_already_modulated_color_and_distinct_clocks() {
        let matrix = std::array::from_fn(|i| (i as f32).to_bits());
        let color = [0.25, 0.5, 0.75, 0.125].map(f32::to_bits);
        let raw = pack_native_words(
            &matrix,
            color,
            [0.25, 0.5].map(f32::to_bits),
            [0.75, 1.0].map(f32::to_bits),
            7.0_f32.to_bits(),
            3.0_f32.to_bits(),
        )
        .unwrap();
        assert_eq!(&raw[76..80], &0.125_f32.to_le_bytes());
        assert_eq!(&raw[80..84], &0.25_f32.to_le_bytes());
        assert_eq!(&raw[88..92], &0.75_f32.to_le_bytes());
        assert_eq!(&raw[96..100], &7.0_f32.to_le_bytes());
        assert_eq!(&raw[100..104], &3.0_f32.to_le_bytes());
    }

    fn reflection() -> DxbcReflection {
        DxbcReflection {
            target: 0,
            creator: "test".into(),
            resources: vec![DxbcResource {
                name: "PdxConstantBuffer0".into(),
                resource_type: 0,
                dimension: 0,
                bind_point: 0,
                bind_count: 1,
            }],
            cbuffers: vec![DxbcCbuffer {
                name: "PdxConstantBuffer0".into(),
                size: 112,
                vars: vec![DxbcCbufferVar {
                    name: "Color".into(),
                    offset: 64,
                    size: 16,
                    used: true,
                }],
            }],
            inputs: vec![],
            outputs: vec![],
        }
    }

    #[test]
    fn reflected_layout_and_resource_gaps_refuse_instead_of_receiving_defaults() {
        let mut r = reflection();
        assert_eq!(
            reflected_constants(&r, 2, AuthenticatedUiShaderLaw::ButtonState, &[0; 112]).unwrap()
                [0]
            .byte_count,
            112
        );
        r.cbuffers[0].vars[0].offset = 80;
        assert!(
            reflected_constants(&r, 2, AuthenticatedUiShaderLaw::ButtonState, &[0; 112]).is_err()
        );
        r = reflection();
        r.cbuffers[0].vars.push(DxbcCbufferVar {
            name: "UnknownTail".into(),
            offset: 104,
            size: 4,
            used: true,
        });
        assert!(
            reflected_constants(&r, 2, AuthenticatedUiShaderLaw::ButtonState, &[0; 112]).is_err()
        );
        r = reflection();
        r.resources.push(DxbcResource {
            name: "MaskTexture_Texture".into(),
            resource_type: 2,
            dimension: 4,
            bind_point: 1,
            bind_count: 1,
        });
        assert!(
            reflected_constants(&r, 2, AuthenticatedUiShaderLaw::ButtonState, &[0; 112]).is_err()
        );
    }

    #[cfg(windows)]
    #[test]
    fn real_compiler_reflection_covers_original_cb_ranges_and_names_new_operands() {
        let source = "cbuffer PdxConstantBuffer0 : register(b0) { float4x4 WorldViewProjectionMatrix; float4 Color; float2 Offset; float2 NextOffset; float Time; float AnimationTime; }; float4 main(float4 p:SV_Position):SV_Target { return Color * (Time - AnimationTime); }";
        let bytes = map_viewer::d3d_compile_stage(
            source,
            map_viewer::ShaderStage::Pixel,
            map_viewer::ENGINE_COMPILE_FLAGS,
        )
        .unwrap();
        let reflection = map_viewer::parse_dxbc_reflection(&bytes).unwrap();
        let raw = pack_native_words(
            &[0; 16],
            [1.0_f32.to_bits(); 4],
            [0; 2],
            [0; 2],
            7.0_f32.to_bits(),
            3.0_f32.to_bits(),
        )
        .unwrap();
        let bindings =
            reflected_constants(&reflection, 2, AuthenticatedUiShaderLaw::ButtonState, &raw)
                .unwrap();
        assert_eq!(bindings.len(), 1);
        assert_eq!(
            &bindings[0].raw_bytes[96..104],
            &[7.0_f32.to_le_bytes(), 3.0_f32.to_le_bytes()].concat()
        );
        let changed = source
            .replace(
                "float AnimationTime;",
                "float AnimationTime; float UnknownOperand;",
            )
            .replace(
                "(Time - AnimationTime)",
                "(Time - AnimationTime + UnknownOperand)",
            );
        let bytes = map_viewer::d3d_compile_stage(
            &changed,
            map_viewer::ShaderStage::Pixel,
            map_viewer::ENGINE_COMPILE_FLAGS,
        )
        .unwrap();
        assert!(reflected_constants(
            &map_viewer::parse_dxbc_reflection(&bytes).unwrap(),
            2,
            AuthenticatedUiShaderLaw::ButtonState,
            &raw
        )
        .unwrap_err()
        .contains("UnknownOperand"));
    }
    #[cfg(windows)]
    struct PacketFixture {
        root: std::path::PathBuf,
        files: GameFileResolver,
        assets: AssetResolver,
        sprite: SpriteDefinition,
        registry: crate::gfx::SpriteRegistry,
        records: crate::container_animation_records::AdmittedContainerAnimations,
        resources: InitializedOrdinarySpriteTypeResources,
        request: ContainerSpriteRequest,
        uv: ContainerSpriteUvGeometry,
        positions: [[f32; 3]; 4],
        matrix: [u32; 16],
        pipeline: PipelineDesc,
        target: NativeUiTargetDesc,
        viewport: ViewportDesc,
    }
    #[cfg(windows)]
    impl PacketFixture {
        fn new() -> Self {
            Self::with_frame_count(1)
        }
        fn with_frame_count(frame_count: u32) -> Self {
            Self::with_options(frame_count, false)
        }
        fn with_static_shader(use_static_shader: bool) -> Self {
            Self::with_options(1, use_static_shader)
        }
        fn with_options(frame_count: u32, use_static_shader: bool) -> Self {
            Self::with_options_and_gui(frame_count, use_static_shader, None)
        }
        fn with_options_and_gui(frame_count: u32, use_static_shader: bool, gui: Option<&str>) -> Self {
            Self::with_case(frame_count, use_static_shader, gui, false)
        }
        fn with_vertical_frame_gui(gui: &str) -> Self {
            Self::with_case(2, false, Some(gui), true)
        }
        fn with_case(frame_count: u32, use_static_shader: bool, gui: Option<&str>, vertical: bool) -> Self {
            use crate::container_animation_records::AdmittedContainerAnimations;
            use crate::container_sprite_geometry::{
                ContainerSpriteGeometryState, SpriteTextureNormalization,
            };
            use crate::container_sprite_resources::{
                OrdinarySpriteLoadSchedule, OrdinarySpriteTextureRegistry,
                OrdinarySpriteTextureRequests,
            };
            use crate::runtime_graphics::RgbaF32;
            use crate::runtime_graphics::{ClipRect, ContainerSpriteReachedState};
            use portable_render_stream::*;
            let root = std::env::temp_dir().join(format!(
                "native-ui-packet-{}-{}",
                std::process::id(),
                std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .unwrap()
                    .as_nanos()
            ));
            std::fs::create_dir_all(root.join("interface")).unwrap();
            std::fs::create_dir_all(root.join("gfx/FX")).unwrap();
            let source = format!(
                r#"spriteTypes = {{ spriteType = {{ name = test
                textureFile = "gfx/not-the-winner.dds"
                textureFile = "gfx/actual.dds"
                noOfFrames = {frame_count} effectFile = "gfx/FX/buttonstate.lua"
            }} }}"#
            );
            let source = if vertical {
                source.replace("noOfFrames =", "frameDirectionY = yes noOfFrames =")
            } else {
                source
            };
            let source = if use_static_shader {
                source.replace("buttonstate.lua", "static_button.lua")
            } else {
                source
            };
            std::fs::write(root.join("interface/test.gfx"), &source).unwrap();
            let dimensions = if vertical { [1_u32, 4] } else { [2_u32, 2] };
            let mut bytes = vec![0_u8; 128 + (dimensions[0] * dimensions[1] * 4) as usize];
            bytes[..4].copy_from_slice(b"DDS ");
            for (offset, value) in [
                (4, 124_u32),
                (8, 0x100f),
                (12, dimensions[1]),
                (16, dimensions[0]),
                (28, 1),
                (76, 32),
                (80, 0x41),
                (88, 32),
                (92, 0xff0000),
                (96, 0xff00),
                (100, 0xff),
                (104, 0xff000000),
                (108, 0x1000),
            ] {
                bytes[offset..offset + 4].copy_from_slice(&value.to_le_bytes());
            }
            for pixel in bytes[128..].chunks_exact_mut(4) {
                pixel.copy_from_slice(&[7, 13, 29, 127]);
            }
            if vertical {
                // Native DDS BGRA rows: red, green, blue, white.
                bytes[128..].copy_from_slice(&[
                    0, 0, 255, 255, 0, 255, 0, 255,
                    255, 0, 0, 255, 255, 255, 255, 255,
                ]);
            }
            std::fs::write(root.join("gfx/actual.dds"), bytes).unwrap();
            for (name, bytes) in [
                (
                    "buttonstate.shader",
                    include_bytes!("../../replay_app/tests/fixtures/exact_a729/buttonstate.shader")
                        .as_slice(),
                ),
                (
                    "buttonstate.fxh",
                    include_bytes!("../../replay_app/tests/fixtures/exact_a729/buttonstate.fxh")
                        .as_slice(),
                ),
                (
                    "sprite_animation.fxh",
                    include_bytes!(
                        "../../replay_app/tests/fixtures/exact_a729/sprite_animation.fxh"
                    )
                    .as_slice(),
                ),
            ] {
                // Git may check these shared text fixtures out as CRLF. The
                // actual built-in files and runtime law own these exact LF
                // bytes; assert the full identity before admitting the fixture.
                let bytes = std::str::from_utf8(bytes).unwrap().replace("\r\n", "\n");
                let (expected_len, expected_sha256) = match name {
                    "buttonstate.shader" => (4867, "1eddb371702289f4cc9d9d565aef21cf77c9261bd4324194295eeea6309f413a"),
                    "buttonstate.fxh" => (243, "8a0e70eb222f1d71b1660fad3c19625859e95e72959995b3f26d478a8b95dd93"),
                    "sprite_animation.fxh" => (4204, "b32e1007b2e41df9ad3c41f49c6df32b7f7fce58bf0f86f55207505a1382238f"),
                    _ => unreachable!("the fixture's exact three authored sources"),
                };
                assert_eq!(bytes.len(), expected_len, "{name}");
                assert_eq!(Sha256::hex(bytes.as_bytes()), expected_sha256, "{name}");
                std::fs::write(root.join("gfx/FX").join(name), bytes).unwrap();
            }
            // This authored fixture platform is admitted with the shader source;
            // it tests the actual compiler/packet/strict facade, not a captured
            // game's platform identity or rendered pixel qualification.
            let platform = format!(
                "// Unique admitted fixture {}\n{}",
                root.file_name().unwrap().to_string_lossy(),
                r#"
#define PDX_POSITION SV_POSITION
#define PDX_COLOR SV_TARGET
float2 vec2(float vValue) { return float2(vValue, vValue); }
float3 vec3(float vValue) { return float3(vValue, vValue, vValue); }
float4 vec4(float vValue) { return float4(vValue, vValue, vValue, vValue); }
#define sampler2D STextureSampler2D
struct STextureSampler2D { Texture2D t; SamplerState s; };
STextureSampler2D CreateSampler2D(Texture2D t,SamplerState s) { STextureSampler2D r; r.t=t; r.s=s; return r; }
float4 tex2D(STextureSampler2D r,float2 uv) { return r.t.Sample(r.s,uv); }
"#
            );
            std::fs::write(root.join("gfx/FX/defines_hlsl_dx11.fxh"), platform).unwrap();
            let mut required = vec![
                "interface/test.gfx",
                "gfx/actual.dds",
                "gfx/FX/buttonstate.shader",
                "gfx/FX/buttonstate.fxh",
                "gfx/FX/sprite_animation.fxh",
                "gfx/FX/defines_hlsl_dx11.fxh",
            ];
            if use_static_shader {
                std::fs::write(
                    root.join("gfx/FX/static_button.shader"),
                    crate::container_sprite_pipeline::STATIC_SHADER,
                )
                .unwrap();
                required.push("gfx/FX/static_button.shader");
            }
            if let Some(gui) = gui {
                let path = crate::peace_conference_ui::PEACE_CONFERENCE_GUI;
                std::fs::write(root.join(path), gui).unwrap();
                required.push(path);
            }
            let (files, admission) = crate::test_admission::admit_vanilla(
                &root,
                "l_english",
                &required,
                "native-ui-packet",
            );
            let assets = AssetResolver::from_content_admission(&files, &admission).unwrap();
            let mut sprites = crate::gfx::SpriteRegistry::new();
            sprites
                .parse_content(source.as_bytes(), "interface/test.gfx")
                .unwrap();
            let sprite = sprites.get("test").unwrap().clone();
            assert_eq!(sprite.primary_texture(), Some("gfx/not-the-winner.dds"));
            let requests =
                OrdinarySpriteTextureRequests::from_definition(&assets, &sprite).unwrap();
            let records =
                AdmittedContainerAnimations::parse(&sprite.raw, requests.source.clone(), None)
                    .unwrap();
            let (_, resources) = InitializedOrdinarySpriteTypeResources::stage_type_initialization(
                &OrdinarySpriteTextureRegistry::new_viewer_session(),
                &assets,
                &requests,
                &records,
                Some(OrdinarySpriteLoadSchedule::Eager),
                Some(false),
                None,
            )
            .unwrap();
            let uv = ContainerSpriteUvGeometry::from_definition(
                &sprite,
                dimensions,
                ContainerSpriteGeometryState::from_csprite_constructor(),
                SpriteTextureNormalization::ActualDimensions,
            )
            .unwrap();
            let positions = uv.uncropped_local_quad_at_unit_scale();
            let matrix = [
                1., 0., 0., 0., 0., 1., 0., 0., 0., 0., 1., 0., 0., 0., 0., 1.,
            ]
            .map(f32::to_bits);
            let request = ContainerSpriteRequest {
                source_identity: "test actual type".into(),
                frame_count: usize::try_from(frame_count).unwrap(),
                frame: crate::runtime_graphics::ContainerSpriteFrame::NativeIcon(1),
                interaction: None,
                color: RgbaF32([0.25, 0.5, 0.75, 0.5]),
                clip: ClipRect {
                    left: 0,
                    top: 0,
                    right: 8,
                    bottom: 8,
                },
                opacity: 0.5,
                reached: Some(ContainerSpriteReachedState {
                    effect: NativeButtonEffect::Down,
                    gui_time: 7.,
                    effect_started_at: 3.,
                    admitted_animation_count: 0,
                }),
            };
            let stencil = StencilFaceDesc {
                fail_operation: StencilOperation::Keep,
                depth_fail_operation: StencilOperation::Keep,
                pass_operation: StencilOperation::Keep,
                comparison: Comparison::Always,
            };
            let pipeline = PipelineDesc {
                topology: PrimitiveTopology::TriangleStrip,
                blend: BlendDesc {
                    alpha_to_coverage_enabled: false,
                    independent_blend_enabled: false,
                    enabled: true,
                    source_color: BlendFactor::SourceAlpha,
                    destination_color: BlendFactor::InverseSourceAlpha,
                    color_operation: BlendOperation::Add,
                    source_alpha: BlendFactor::One,
                    destination_alpha: BlendFactor::Zero,
                    alpha_operation: BlendOperation::Add,
                    write_mask_rgba_u8: 15,
                    additional_targets: vec![],
                    blend_factor_rgba_f32_bits: [0; 4],
                    sample_mask_u32: u32::MAX,
                },
                depth: DepthDesc {
                    enabled: false,
                    write_enabled: false,
                    comparison: Comparison::Always,
                    stencil_enabled: false,
                    stencil_read_mask_u8: 255,
                    stencil_write_mask_u8: 255,
                    front_face: stencil.clone(),
                    back_face: stencil,
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
                samplers: vec![SamplerDesc {
                    stage: portable_render_stream::ShaderStage::Pixel,
                    slot: 0,
                    semantic_token: "MapTexture".into(),
                    min_filter: Filter::Linear,
                    mag_filter: Filter::Linear,
                    mip_filter: Filter::Point,
                    address_u: AddressMode::Clamp,
                    address_v: AddressMode::Clamp,
                    address_w: AddressMode::Clamp,
                    comparison: None,
                    mip_lod_bias_f32_bits: (-0.8_f32).to_bits(),
                    max_anisotropy: 1,
                    border_color_rgba_f32_bits: [0; 4],
                    min_lod_f32_bits: 0,
                    max_lod_f32_bits: f32::MAX.to_bits(),
                }],
                input_layout: vec![
                    InputElementDesc {
                        semantic_token: "POSITION".into(),
                        semantic_index: 0,
                        dxgi_format_u32: 6,
                        input_slot: 0,
                        aligned_byte_offset: 0,
                        per_instance: false,
                        instance_step_rate: 0,
                    },
                    InputElementDesc {
                        semantic_token: "TEXCOORD".into(),
                        semantic_index: 0,
                        dxgi_format_u32: 16,
                        input_slot: 0,
                        aligned_byte_offset: 12,
                        per_instance: false,
                        instance_step_rate: 0,
                    },
                ],
                index_format: IndexFormat::None,
                static_indices_u16: vec![],
            };

            Self {
                root,
                files,
                assets,
                sprite,
                registry: sprites,
                records,
                resources,
                request,
                uv,
                positions,
                matrix,
                pipeline,
                target: NativeUiTargetDesc {
                    width: 8,
                    height: 8,
                    dxgi_format_u32: 87,
                    sample_count: 1,
                    sample_quality: 0,
                },
                viewport: ViewportDesc {
                    top_left_x_f32_bits: 0,
                    top_left_y_f32_bits: 0,
                    width_f32_bits: 8_f32.to_bits(),
                    height_f32_bits: 8_f32.to_bits(),
                    min_depth_f32_bits: 0,
                    max_depth_f32_bits: 1_f32.to_bits(),
                },
            }
        }
        fn inputs(&self) -> ContainerSpriteGpuInputs<'_> {
            ContainerSpriteGpuInputs {
                local_positions: &self.positions,
                uv: &self.uv,
                world_view_projection_bits: &self.matrix,
                engine_defines: Some(&[]),
                native_mxcsr: Some(0x1f80),
                resources: &self.resources,
                pipeline: &self.pipeline,
                viewport: &self.viewport,
                scissor: None,
                target: &self.target,
            }
        }
        fn packet(&self) -> DrawResult<ContainerSpriteDrawPacket<'_>> {
            ContainerSpriteDrawPacket::build(
                &self.assets,
                &self.files,
                &self.sprite,
                &self.request,
                self.inputs(),
            )
        }
    }
    #[cfg(windows)]
    impl Drop for PacketFixture {
        fn drop(&mut self) {
            std::fs::remove_dir_all(&self.root).unwrap();
        }
    }

    #[cfg(windows)]
    #[test]
    fn authenticated_shader_retained_last_texture_and_actual_facade_form_one_packet() {
        let fixture = PacketFixture::new();
        let packet = fixture.packet().unwrap();
        packet.prepare(&fixture.files).unwrap();
        assert_eq!(packet.shader.effect_token, "Down");
        assert_eq!(packet.textures.len(), 1);
        assert_eq!(
            packet.textures[0].content.normalized_game_relative_path,
            "gfx/actual.dds"
        );
        assert_eq!(packet.textures[0].resource.dxgi_format_u32, 87);
        assert!(packet.constants.iter().all(|cb| cb.slot == 0));
        let pixel = packet
            .constants
            .iter()
            .find(|cb| cb.stage_mask_u8 == 2)
            .unwrap();
        assert_eq!(&pixel.raw_bytes[76..80], &0.25_f32.to_le_bytes());
        assert_eq!(
            &pixel.raw_bytes[96..104],
            &[7_f32.to_le_bytes(), 3_f32.to_le_bytes()].concat()
        );
        let BufferSource::Inline { bytes, .. } = &packet.geometry.ordered_vertex_buffers[0].source
        else {
            panic!()
        };
        assert_eq!(&bytes[8..12], &1_f32.to_le_bytes());
    }

    #[cfg(windows)]
    #[test]
    fn actual_admitted_static_pipeline_compiles_into_the_retained_resource_facade_packet() {
        let mut fixture = PacketFixture::with_static_shader(true);
        fixture.pipeline = crate::container_sprite_pipeline::ordinary_pipeline(
            &fixture.assets, &fixture.sprite, NativeButtonEffect::Down,
            Some(false), Some(0),
            Some(crate::container_sprite_pipeline::NativeShaderStateTextPolicy::AsciiCaseFoldAndUppercase),
        ).unwrap();
        let packet = fixture.packet().unwrap();
        packet.prepare(&fixture.files).unwrap();
        assert_eq!(packet.shader.effect_token, "Down");
        assert_eq!(packet.textures.len(), 1);
        assert_eq!(
            packet.textures[0].content.normalized_game_relative_path,
            "gfx/actual.dds"
        );
        assert_eq!(
            fixture.pipeline.blend.source_alpha,
            fixture.pipeline.blend.source_color
        );
        assert_eq!(
            fixture.pipeline.depth.comparison,
            portable_render_stream::Comparison::LessEqual
        );
        assert_eq!(
            fixture.pipeline.rasterizer.cull,
            portable_render_stream::CullMode::Back
        );
        assert_eq!(
            fixture.pipeline.samplers[0].mip_filter,
            portable_render_stream::Filter::Point
        );
        assert!(packet.constants.iter().all(|row| row.byte_count <= 96));
        let pixel = packet
            .constants
            .iter()
            .find(|row| row.stage_mask_u8 == 2)
            .unwrap();
        assert_eq!(&pixel.raw_bytes[76..80], &0.25_f32.to_le_bytes());
    }

    #[cfg(windows)]
    #[test]
    fn actual_packet_refuses_missing_sampler_changed_matrix_and_unknown_effect_inputs() {
        let mut fixture = PacketFixture::new();
        fixture.pipeline.samplers.clear();
        assert!(fixture
            .packet()
            .err()
            .unwrap()
            .contains("sampler slot is unbound"));
        let mut fixture = PacketFixture::new();
        fixture.matrix[0] = f32::NAN.to_bits();
        assert!(fixture
            .packet()
            .err()
            .unwrap()
            .contains("nonfinite constant"));
        let mut fixture = PacketFixture::new();
        fixture.request.reached = None;
        assert!(fixture
            .packet()
            .err()
            .unwrap()
            .contains("effect/time receipt unavailable"));
        let fixture = PacketFixture::new();
        let mut missing_control = fixture.inputs();
        missing_control.native_mxcsr = None;
        assert!(ContainerSpriteDrawPacket::build(
            &fixture.assets,
            &fixture.files,
            &fixture.sprite,
            &fixture.request,
            missing_control
        )
        .err()
        .unwrap()
        .contains("source floating-point control unavailable"));
        let mut inputs = fixture.inputs();
        inputs.engine_defines = None;
        assert!(ContainerSpriteDrawPacket::build(
            &fixture.assets,
            &fixture.files,
            &fixture.sprite,
            &fixture.request,
            inputs
        )
        .err()
        .unwrap()
        .contains("compiler environment unavailable"));
    }

    #[cfg(windows)]
    #[test]
    fn actual_type_identity_and_resource_states_are_not_collapsed_into_a_base_texture() {
        let mut fixture = PacketFixture::new();
        fixture.sprite.name = "different row".into();
        assert!(fixture
            .packet()
            .err()
            .unwrap()
            .contains("type definition custody"));
        let mut fixture = PacketFixture::new();
        fixture.resources.base = OrdinarySpriteResourceBinding::Absent;
        assert!(fixture.packet().err().unwrap().contains("witnessed absent"));
        fixture.resources.base = OrdinarySpriteResourceBinding::AuthoredEmpty { field_ordinal: 2 };
        assert!(fixture.packet().err().unwrap().contains("authored empty"));
        let mut fixture = PacketFixture::new();
        let crate::container_sprite_resources::OrdinarySpriteTextureField::File(base) =
            fixture.resources.requests.base.clone()
        else {
            panic!()
        };
        fixture.resources.base = OrdinarySpriteResourceBinding::DeferredBase(base);
        assert!(fixture.packet().err().unwrap().contains("deferred loader"));
    }

    #[cfg(windows)]
    #[test]
    fn packet_uses_admitted_platform_snapshot_and_refuses_actual_snapshot_corruption() {
        let fixture = PacketFixture::new();
        let packet = fixture.packet().unwrap();
        let key = fixture
            .files
            .admitted_used_content_key(map_viewer::DX11_PLATFORM_HEADER)
            .unwrap()
            .unwrap();
        let snapshot = fixture
            .files
            .resolve_required(
                map_viewer::DX11_PLATFORM_HEADER,
                "native UI packet immutable platform snapshot",
            )
            .unwrap();
        let admitted_bytes = fixture.files.read_admitted_used_content(&key).unwrap();
        assert!(!snapshot.starts_with(&fixture.root));
        std::fs::write(
            fixture.root.join(map_viewer::DX11_PLATFORM_HEADER),
            b"// live platform replacement\n",
        )
        .unwrap();
        assert_eq!(
            fixture.files.read_admitted_used_content(&key).unwrap(),
            admitted_bytes
        );
        packet.prepare(&fixture.files).unwrap();
        crate::test_admission::overwrite_readonly(&snapshot, b"// corrupt admitted platform\n");
        let error = packet.prepare(&fixture.files).err().unwrap();
        assert!(
            error.contains("immutable admitted content snapshot changed"),
            "{error}"
        );
    }

    #[cfg(windows)]
    #[test]
    fn actual_native_icon_packet_preserves_signed_frames_and_out_of_count_offsets() {
        use crate::runtime_graphics::ContainerSpriteFrame;
        let mut fixture = PacketFixture::with_frame_count(4);
        // Expected CPU-upload words follow 14222d5a0's raw-frame branches,
        // signed CVTDQ2PS and MULSS with the admitted quarter-width step.
        // In particular, raw MAX selects MAX-1; both MAX-1 and MAX round
        // to 2^31 before multiplication. This is not an INC-overflow case.
        for (raw, effect, expected_index, offset_words) in [
            (0, NativeButtonEffect::Disable, 0, [0, 0, 0x3e80_0000, 0]),
            (
                -1,
                NativeButtonEffect::Down,
                1,
                [0x3e80_0000, 0, 0x3f00_0000, 0],
            ),
            (
                i32::MIN,
                NativeButtonEffect::Over,
                2,
                [0x3f00_0000, 0, 0x3f40_0000, 0],
            ),
            (
                -1,
                NativeButtonEffect::Disable,
                3,
                [0x3f40_0000, 0, 0x3f80_0000, 0],
            ),
            (
                -1,
                NativeButtonEffect::Unassigned,
                0,
                [0, 0, 0x3e80_0000, 0],
            ),
            (
                5,
                NativeButtonEffect::Unassigned,
                4,
                [0x3f80_0000, 0, 0x3fa0_0000, 0],
            ),
            (
                255,
                NativeButtonEffect::Down,
                254,
                [0x427e_0000, 0, 0x427f_0000, 0],
            ),
            (
                i32::MAX,
                NativeButtonEffect::Down,
                2_147_483_646,
                [0x4e00_0000, 0, 0x4e00_0000, 0],
            ),
        ] {
            fixture.request.frame = ContainerSpriteFrame::NativeIcon(raw);
            fixture.request.reached.as_mut().unwrap().effect = effect;
            let admitted = admit_container_sprite_runtime_graphics(
                &fixture.assets,
                &fixture.sprite,
                &fixture.request,
            )
            .unwrap();
            assert_eq!(
                admitted.frame().zero_based_index,
                expected_index,
                "raw={raw}, effect={effect:?}"
            );
            let packet = fixture.packet().unwrap();
            packet.prepare(&fixture.files).unwrap();
            let pixel = packet
                .constants
                .iter()
                .find(|cb| cb.stage_mask_u8 == 2)
                .unwrap();
            let actual: Vec<u32> = pixel.raw_bytes[80..96]
                .chunks_exact(4)
                .map(|word| u32::from_le_bytes(word.try_into().unwrap()))
                .collect();
            assert_eq!(
                actual.as_slice(),
                &offset_words,
                "raw={raw}, effect={effect:?}"
            );
            assert_eq!(&pixel.raw_bytes[76..80], &0.25_f32.to_le_bytes());
            assert_eq!(
                &pixel.raw_bytes[96..104],
                &[7_f32.to_le_bytes(), 3_f32.to_le_bytes()].concat()
            );
        }
    }

    #[cfg(windows)]
    #[test]
    fn actual_native_icon_packet_keeps_negative_effect_fallback_distinct_from_positive_override() {
        use crate::runtime_graphics::ContainerSpriteFrame;
        let mut fixture = PacketFixture::new();
        // The actual one-frame roster rejects effect index 1 for negative
        // raw frames, while an explicit positive frame bypasses that check.
        for (raw, expected_index, offset_words) in [
            (-1, 0, [0, 0, 0x3f80_0000, 0]),
            (2, 1, [0x3f80_0000, 0, 0x4000_0000, 0]),
        ] {
            fixture.request.frame = ContainerSpriteFrame::NativeIcon(raw);
            let admitted = admit_container_sprite_runtime_graphics(
                &fixture.assets,
                &fixture.sprite,
                &fixture.request,
            )
            .unwrap();
            assert_eq!(admitted.frame().zero_based_index, expected_index);
            let packet = fixture.packet().unwrap();
            packet.prepare(&fixture.files).unwrap();
            let pixel = packet
                .constants
                .iter()
                .find(|cb| cb.stage_mask_u8 == 2)
                .unwrap();
            let actual: Vec<u32> = pixel.raw_bytes[80..96]
                .chunks_exact(4)
                .map(|word| u32::from_le_bytes(word.try_into().unwrap()))
                .collect();
            assert_eq!(actual.as_slice(), &offset_words, "raw={raw}");
        }
    }

    #[cfg(windows)]
    #[test]
    fn actual_packet_keeps_preview_frame_bounds_separate_from_native_raw_frames() {
        use crate::runtime_graphics::ContainerSpriteFrame;
        let mut fixture = PacketFixture::new();
        for raw in [0, 2, u32::MAX] {
            fixture.request.frame = ContainerSpriteFrame::PreviewOneBased(raw);
            let error = fixture.packet().err().unwrap();
            assert!(error.contains("InvalidFrame"), "raw={raw}: {error}");
        }
        fixture.request.frame = ContainerSpriteFrame::PreviewOneBased(1);
        fixture.packet().unwrap().prepare(&fixture.files).unwrap();
    }
}
