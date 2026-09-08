//! Actual fixed-state/sampler provider for hash-admitted ordinary CSprite shaders.
//! The original shader compiler, resource registry and GPU facade remain owning
//! providers. Missing native policy is an input error, never a renderer default.
//! Native fixed-state/sampler exports: portable_engine_evidence::{map_scene_completion,native_sampler_state}.
//! Caller initialization/order remains candidate as recorded in
//! extractor/native/runs/arrow_native_ui_20260908/CANDIDATE_EFFECT_CLOCK_LIFECYCLES.json.

use crate::container_sprite_fixed_states as fixed_states;
pub(crate) use fixed_states::NativeShaderStateTextPolicy;

use crate::assets::AssetResolver;
use crate::container_sprite_inputs::NativeButtonEffect;
use crate::gfx::SpriteDefinition;
use crate::runtime_graphics::authenticate_builtin_shader_content;
use map_viewer::{EffectFile, EffectSampler};
use portable_render_stream::{
    AddressMode, Filter, IndexFormat, InputElementDesc, PipelineDesc, PrimitiveTopology,
    SamplerDesc, ShaderStage,
};
use snapshot::sha256::Sha256;

#[derive(Clone, Copy, Debug)]
pub(crate) struct NativeUiPipelinePolicy {
    /// Actual reached GUI row +155 selects graphics +a8 or +b8.
    pub scissor_reached: Option<bool>,
    /// Creation-time request feeding renderer +54; target sample count is not it.
    pub renderer_creation_aa_request: Option<u32>,
    /// Owning admission of native CRT string comparison/uppercase semantics.
    pub state_text_policy: Option<NativeShaderStateTextPolicy>,
}

pub(crate) fn ordinary_pipeline(
    assets: &AssetResolver,
    sprite: &SpriteDefinition,
    effect: NativeButtonEffect,
    scissor: Option<bool>,
    renderer_creation_aa_request: Option<u32>,
    state_text_policy: Option<NativeShaderStateTextPolicy>,
) -> Result<PipelineDesc, String> {
    if assets.admitted_content_key(&sprite.source_file).is_none() {
        return Err("native UI pipeline sprite source identity unavailable".into());
    }
    if sprite.raw.values_ci("animation").next().is_some() {
        return Err("native UI pipeline animated pass/resource schedule unavailable".into());
    }
    let path = crate::gfx::a729_effect_shader_path(sprite.effect_file.as_deref())
        .map_err(|error| error.to_string())?;
    let read = |path: &str| -> Result<(_, Vec<u8>), String> {
        let key = assets
            .admitted_content_key(path)
            .ok_or_else(|| format!("native UI pipeline source identity unavailable: {path}"))?
            .clone();
        let resolved = assets
            .resolve_file(path)
            .map_err(|error| error.to_string())?;
        let bytes = assets
            .read_file(&resolved)
            .map_err(|error| error.to_string())?;
        if !resolved
            .rel_path
            .eq_ignore_ascii_case(&key.normalized_game_relative_path)
            || bytes.len() as u64 != key.payload_bytes
            || Sha256::hex(&bytes) != key.payload_sha256
        {
            return Err("native UI pipeline source bytes differ from admitted identity".into());
        }
        Ok((key, bytes))
    };
    let (key, bytes) = read(&path)?;
    let include_paths = crate::gfx::xui07_builtin_shader_includes(&path)
        .ok_or("native UI pipeline shader include law unavailable")?;
    let includes = include_paths
        .iter()
        .map(|path| read(path).map(|(key, _)| key))
        .collect::<Result<Vec<_>, _>>()?;
    let law = authenticate_builtin_shader_content(&path, &key, &includes)
        .map_err(|error| format!("native UI pipeline shader admission: {error:?}"))?;
    let effect_name = crate::container_sprite_draw::ordinary_effect(law, effect, 0)?;
    // The exact builtin hash admission above excludes duplicate named state
    // declarations. Generic EffectFile's BTreeMap is not that proof for mods.
    let text = std::str::from_utf8(&bytes)
        .map_err(|_| "hash-admitted ordinary UI shader has unsupported encoding")?;
    let file = map_viewer::parse_effect_file(text).map_err(|error| error.to_string())?;
    pipeline_from_authenticated_file(
        &file,
        effect_name,
        NativeUiPipelinePolicy {
            scissor_reached: scissor,
            renderer_creation_aa_request,
            state_text_policy,
        },
    )
}

fn pipeline_from_authenticated_file(
    file: &EffectFile,
    effect_name: &str,
    policy: NativeUiPipelinePolicy,
) -> Result<PipelineDesc, String> {
    let effect = file
        .effect(effect_name)
        .ok_or("native UI pipeline effect is absent")?;
    if file.effect_count(effect_name) != 1 {
        return Err("native UI pipeline effect is ambiguous".into());
    }
    // SGfxShaderPassReader 142348f80 installs this literal before authored
    // overrides. 142344df0 selects the first matching named blend row.
    let blend_name = effect.blend_state.as_deref().unwrap_or("BlendState");
    if policy.state_text_policy.is_none() {
        return Err("native UI pipeline state-name comparison policy unavailable".into());
    }
    let mut matching = file
        .blend_states
        .iter()
        .filter(|(name, _)| name.eq_ignore_ascii_case(blend_name));
    let rows = matching.next().map(|(_, rows)| rows.as_slice());
    if matching.next().is_some() {
        return Err("native UI pipeline ordered blend declaration selection unavailable".into());
    }
    let fixed = fixed_states::ordinary_ui_fixed_states(
        rows,
        policy.scissor_reached,
        policy.renderer_creation_aa_request,
        policy.state_text_policy,
    )
    .map_err(|error| format!("native UI fixed state: {error:?}"))?;
    if !fixed.unchanged_invalid_enum_fields.is_empty() {
        return Err(format!(
            "native UI authored state enum rejected: {:?}",
            fixed.unchanged_invalid_enum_fields
        ));
    }
    // The hash-admitted initial ordinary shader files have no global/vertex
    // sampler declarations. Do not substitute vertex rows for the native
    // common-then-pixel descriptor array consumed by 142345450.
    if !file.vertex_stage.samplers.is_empty() {
        return Err("native UI ordinary vertex sampler binding unavailable".into());
    }
    // 142345450 replaces repeated physical slots; 14222d972 binds only base
    // descriptor zero to pixel stage (1424005a0, mask 1), not every declaration.
    let sampler = file
        .pixel_stage
        .samplers
        .iter()
        .filter(|sampler| sampler.index == 0)
        .last()
        .ok_or("native UI base sampler slot zero is unavailable")?;
    let sampler = ordinary_sampler(sampler, policy.state_text_policy)?;
    Ok(PipelineDesc {
        topology: PrimitiveTopology::TriangleStrip,
        blend: fixed.blend,
        depth: fixed.depth,
        rasterizer: fixed.rasterizer,
        samplers: vec![sampler],
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
        static_indices_u16: Vec::new(),
    })
}

fn ordinary_sampler(
    sampler: &EffectSampler,
    text_policy: Option<NativeShaderStateTextPolicy>,
) -> Result<SamplerDesc, String> {
    if sampler.index != 0 {
        return Err("native ordinary UI base sampler physical slot differs from zero".into());
    }
    if text_policy.is_none() {
        return Err(format!(
            "native sampler {} string policy unavailable",
            sampler.name
        ));
    }
    if sampler.sampler_type.is_some() {
        return Err(format!(
            "native sampler {} explicit Type loader selection unavailable",
            sampler.name
        ));
    }
    // Actual sampler reader uses %f for bias and %i for the LOD/unused aniso
    // operand. Do not replace those source conversions with Rust parse/defaults.
    for (name, value) in [
        ("MipMapLodBias", &sampler.mip_map_lod_bias),
        ("MipMapMinLod", &sampler.mip_map_min_lod),
        ("MipMapMaxLod", &sampler.mip_map_max_lod),
        ("MaxAnisotropy", &sampler.max_anisotropy),
    ] {
        if value.is_some() {
            return Err(format!(
                "native sampler {} {name} source conversion unavailable",
                sampler.name
            ));
        }
    }
    let mut min = sampler_filter(sampler.min_filter.as_deref(), "MinFilter")?;
    let mut mag = sampler_filter(sampler.mag_filter.as_deref(), "MagFilter")?;
    let mut mip = sampler_filter(sampler.mip_filter.as_deref(), "MipFilter")?;
    // 1424110f0 returns 0x55 if ANY filter is anisotropic. Lowering all three
    // to Anisotropic exactly represents that descriptor in the existing facade.
    if [min, mag, mip].contains(&Filter::Anisotropic) {
        min = Filter::Anisotropic;
        mag = Filter::Anisotropic;
        mip = Filter::Anisotropic;
    }
    Ok(SamplerDesc {
        stage: ShaderStage::Pixel,
        slot: 0,
        semantic_token: sampler.name.clone(),
        min_filter: min,
        mag_filter: mag,
        mip_filter: mip,
        address_u: sampler_address(sampler.address_u.as_deref(), "AddressU")?,
        address_v: sampler_address(sampler.address_v.as_deref(), "AddressV")?,
        address_w: AddressMode::Wrap,
        // D3D11 filter is noncomparison. None lowers to ComparisonFunc NEVER
        // while avoiding the comparison-filter bit in native_draw::filter.
        comparison: None,
        mip_lod_bias_f32_bits: 0,
        max_anisotropy: 16,
        border_color_rgba_f32_bits: [0; 4],
        min_lod_f32_bits: 0,
        max_lod_f32_bits: 1024_f32.to_bits(),
    })
}

fn sampler_filter(value: Option<&str>, field: &str) -> Result<Filter, String> {
    let Some(value) = value else {
        return Ok(Filter::Linear);
    };
    if !value.is_ascii() {
        return Err(format!("native sampler {field} token encoding unavailable"));
    }
    match value.to_ascii_lowercase().as_str() {
        "none" | "point" => Ok(Filter::Point),
        "linear" => Ok(Filter::Linear),
        "anisotropic" => Ok(Filter::Anisotropic),
        _ => Err(format!("native sampler {field} enum rejected: {value}")),
    }
}
fn sampler_address(value: Option<&str>, field: &str) -> Result<AddressMode, String> {
    let Some(value) = value else {
        return Ok(AddressMode::Wrap);
    };
    if !value.is_ascii() {
        return Err(format!("native sampler {field} token encoding unavailable"));
    }
    match value.to_ascii_lowercase().as_str() {
        "wrap" => Ok(AddressMode::Wrap),
        "mirror" => Ok(AddressMode::Mirror),
        "clamp" => Ok(AddressMode::Clamp),
        "border" => Ok(AddressMode::Border),
        "mirror_once" => Err(format!(
            "native sampler {field} MIRROR_ONCE descriptor representation unavailable"
        )),
        _ => Err(format!("native sampler {field} enum rejected: {value}")),
    }
}

#[cfg(test)]
pub(crate) const STATIC_SHADER: &str = "Includes = {\n}\n\nPixelShader =\n{\n\tSamplers =\n\t{\n\t\tMapTexture =\n\t\t{\n\t\t\tIndex = 0\n\t\t\tMagFilter = \"Point\"\n\t\t\tMinFilter = \"Point\"\n\t\t\tMipFilter = \"None\"\n\t\t\tAddressU = \"Wrap\"\n\t\t\tAddressV = \"Wrap\"\n\t\t}\n\t}\n}\n\n\nVertexStruct VS_INPUT\n{\n\tfloat3 vPosition  : POSITION;\n\tfloat2 vTexCoord  : TEXCOORD0;\n};\n\nVertexStruct VS_OUTPUT\n{\n\tfloat4  vPosition : PDX_POSITION;\n\tfloat2  vTexCoord : TEXCOORD0;\n};\n\n\nConstantBuffer( 0, 0 )\n{\nfloat4x4 WorldViewProjectionMatrix\t;\nfloat4 Color \t\t\t\t\t\t;\nfloat vXOffset\t\t\t\t\t\t;\t// For textures with more than one frame\n};\n\n\n\nVertexShader =\n{\n\tMainCode VertexShader\n\t[[\n\t\t\n\t\tVS_OUTPUT main(const VS_INPUT v )\n\t\t{\n\t\t    VS_OUTPUT Out;\n\t\t    Out.vPosition  = mul( WorldViewProjectionMatrix, float4( v.vPosition.xyz, 1 ) );\n\t\t\n\t\t    Out.vTexCoord  = v.vTexCoord;\n\t\t\tOut.vTexCoord.x += vXOffset;\n\t\t    return Out;\n\t\t}\n\t\t\n\t\t\n\t]]\n}\n\nPixelShader =\n{\n\tMainCode PixelShaderUp\n\t[[\n\t\t\t\n\t\tfloat4 main( VS_OUTPUT v ) : PDX_COLOR\n\t\t{\n\t\t    float4 OutColor = tex2D( MapTexture, v.vTexCoord );\n\t\t\tOutColor *= Color;\n\t\t \n\t\t    return OutColor;\n\t\t}\n\t\t\n\t\t\t\n\t\t\n\t]]\n}\n\n\nBlendState BlendState\n{\n\tBlendEnable = yes\n\tAlphaTest = no\n\tSourceBlend = \"src_alpha\"\n\tDestBlend = \"inv_src_alpha\"\n}\n\n\nEffect Up\n{\n\tVertexShader = \"VertexShader\"\n\tPixelShader = \"PixelShaderUp\"\n}\n\nEffect Down\n{\n\tVertexShader = \"VertexShader\"\n\tPixelShader = \"PixelShaderUp\"\n}\n\nEffect Disable\n{\n\tVertexShader = \"VertexShader\"\n\tPixelShader = \"PixelShaderUp\"\n}\n\nEffect Over\n{\n\tVertexShader = \"VertexShader\"\n\tPixelShader = \"PixelShaderUp\"\n}\n\n";

#[cfg(test)]
mod tests {
    use super::*;

    fn policy() -> NativeUiPipelinePolicy {
        NativeUiPipelinePolicy {
            scissor_reached: Some(true),
            renderer_creation_aa_request: Some(0),
            state_text_policy: Some(NativeShaderStateTextPolicy::AsciiCaseFoldAndUppercase),
        }
    }
    fn file() -> EffectFile {
        map_viewer::parse_effect_file(STATIC_SHADER).unwrap()
    }
    fn sampler() -> EffectSampler {
        EffectSampler {
            name: "Test".into(),
            index: 0,
            ..Default::default()
        }
    }

    #[test]
    fn shipped_static_pass_uses_default_blend_name_and_gui_depth_raster() {
        let p = pipeline_from_authenticated_file(&file(), "Down", policy()).unwrap();
        assert!(p.blend.enabled);
        assert_eq!(
            p.blend.source_color,
            portable_render_stream::BlendFactor::SourceAlpha
        );
        assert_eq!(p.blend.source_alpha, p.blend.source_color);
        assert_eq!(p.blend.blend_factor_rgba_f32_bits, [1_f32.to_bits(); 4]);
        assert!(!p.depth.enabled && !p.depth.write_enabled);
        assert_eq!(p.rasterizer.cull, portable_render_stream::CullMode::Back);
        assert!(p.rasterizer.scissor_enabled);
        assert_eq!(p.input_layout[0].dxgi_format_u32, 6);
        assert_eq!(p.input_layout[1].aligned_byte_offset, 12);
        assert_eq!(p.topology, PrimitiveTopology::TriangleStrip);
        assert_eq!(p.index_format, IndexFormat::None);
    }
    #[test]
    fn native_sampler_ctor_and_none_filter_lower_without_d3d_defaults() {
        let mut s = sampler();
        let p = ordinary_sampler(&s, policy().state_text_policy).unwrap();
        assert_eq!(
            (p.min_filter, p.mag_filter, p.mip_filter),
            (Filter::Linear, Filter::Linear, Filter::Linear)
        );
        assert_eq!(p.address_w, AddressMode::Wrap);
        assert_eq!(p.max_lod_f32_bits, 1024_f32.to_bits());
        assert_eq!(p.max_anisotropy, 16);
        assert_eq!(p.border_color_rgba_f32_bits, [0; 4]);
        assert_eq!(p.comparison, None);
        s.min_filter = Some("None".into());
        s.mag_filter = Some("Point".into());
        let p = ordinary_sampler(&s, policy().state_text_policy).unwrap();
        assert_eq!((p.min_filter, p.mag_filter), (Filter::Point, Filter::Point));
    }
    #[test]
    fn any_native_anisotropic_filter_lowers_to_the_one_exact_d3d_filter() {
        for selected in 0..3 {
            let mut s = sampler();
            match selected {
                0 => s.min_filter = Some("Anisotropic".into()),
                1 => s.mag_filter = Some("Anisotropic".into()),
                _ => s.mip_filter = Some("Anisotropic".into()),
            };
            let p = ordinary_sampler(&s, policy().state_text_policy).unwrap();
            assert_eq!(
                (p.min_filter, p.mag_filter, p.mip_filter),
                (
                    Filter::Anisotropic,
                    Filter::Anisotropic,
                    Filter::Anisotropic
                )
            );
        }
    }
    #[test]
    fn authored_numeric_unknowns_and_mirror_once_are_named_per_operand() {
        for (field, ordinal) in [
            ("MipMapLodBias", 0),
            ("MipMapMinLod", 1),
            ("MipMapMaxLod", 2),
            ("MaxAnisotropy", 3),
        ] {
            let mut s = sampler();
            let target = match ordinal {
                0 => &mut s.mip_map_lod_bias,
                1 => &mut s.mip_map_min_lod,
                2 => &mut s.mip_map_max_lod,
                _ => &mut s.max_anisotropy,
            };
            *target = Some("-0.8".into());
            assert!(ordinary_sampler(&s, policy().state_text_policy)
                .unwrap_err()
                .contains(field));
        }
        let mut s = sampler();
        s.address_u = Some("Mirror_Once".into());
        assert!(ordinary_sampler(&s, policy().state_text_policy)
            .unwrap_err()
            .contains("MIRROR_ONCE"));
        s.address_u = Some(" Clamp ".into());
        assert!(ordinary_sampler(&s, policy().state_text_policy)
            .unwrap_err()
            .contains("enum rejected"));
    }
    #[test]
    fn scoped_guards_preserve_unknown_vs_false_and_sampler_slot_replacement() {
        let mut f = file();
        let mut second = f.pixel_stage.samplers[0].clone();
        second.name = "ActualLastSlot".into();
        second.address_u = Some("Clamp".into());
        f.pixel_stage.samplers.push(second);
        let p = pipeline_from_authenticated_file(&f, "Up", policy()).unwrap();
        assert_eq!(p.samplers.len(), 1);
        assert_eq!(p.samplers[0].semantic_token, "ActualLastSlot");
        assert_eq!(p.samplers[0].address_u, AddressMode::Clamp);
        let mut missing = policy();
        missing.scissor_reached = None;
        assert!(pipeline_from_authenticated_file(&f, "Up", missing)
            .unwrap_err()
            .contains("ScissorSelectionUnknown"));
        let mut no_scissor = policy();
        no_scissor.scissor_reached = Some(false);
        assert!(
            !pipeline_from_authenticated_file(&f, "Up", no_scissor)
                .unwrap()
                .rasterizer
                .scissor_enabled
        );
        let mut missing = policy();
        missing.renderer_creation_aa_request = None;
        assert!(pipeline_from_authenticated_file(&f, "Up", missing)
            .unwrap_err()
            .contains("RendererCreationAaUnknown"));
        let mut missing = policy();
        missing.state_text_policy = None;
        assert!(pipeline_from_authenticated_file(&f, "Up", missing)
            .unwrap_err()
            .contains("state-name comparison"));
        f.vertex_stage.samplers.push(sampler());
        assert!(pipeline_from_authenticated_file(&f, "Up", policy())
            .unwrap_err()
            .contains("vertex sampler"));
    }
    #[test]
    fn actual_admitted_static_asset_reaches_pipeline_and_rejects_changed_shader_identity() {
        let root = std::env::temp_dir().join(format!(
            "arrow-native-ui-pipeline-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        std::fs::create_dir_all(root.join("interface")).unwrap();
        std::fs::create_dir_all(root.join("gfx/FX")).unwrap();
        let gfx = r#"spriteTypes={spriteType={name=test texturefile="gfx/test.dds" noOfFrames=1 effectFile="gfx/FX/static_button.lua"}}"#;
        std::fs::write(root.join("interface/test.gfx"), gfx).unwrap();
        std::fs::write(root.join("gfx/FX/static_button.shader"), STATIC_SHADER).unwrap();
        let (files, admission) = crate::test_admission::admit_vanilla(
            &root,
            "l_english",
            &["interface/test.gfx", "gfx/FX/static_button.shader"],
            "ordinary-pipeline-static",
        );
        let assets = AssetResolver::from_content_admission(&files, &admission).unwrap();
        let mut registry = crate::gfx::SpriteRegistry::new();
        registry
            .parse_content(gfx.as_bytes(), "interface/test.gfx")
            .unwrap();
        let sprite = registry.get("test").unwrap();
        let p = ordinary_pipeline(
            &assets,
            sprite,
            NativeButtonEffect::Down,
            Some(true),
            Some(0),
            policy().state_text_policy,
        )
        .unwrap();
        assert_eq!(p.samplers[0].min_filter, Filter::Point);
        assert_eq!(p.samplers[0].mip_filter, Filter::Point);
        // A changed authored shader gets a new admission, which must still
        // satisfy the same exact builtin law. Do not damage shared snapshots.
        std::fs::write(
            root.join("gfx/FX/static_button.shader"),
            format!(
                "{}\nBlendState BlendState {{ BlendEnable=no }}",
                STATIC_SHADER
            ),
        )
        .unwrap();
        let (files2, admission2) = crate::test_admission::admit_vanilla(
            &root,
            "l_english",
            &["interface/test.gfx", "gfx/FX/static_button.shader"],
            "ordinary-pipeline-changed",
        );
        let assets2 = AssetResolver::from_content_admission(&files2, &admission2).unwrap();
        assert!(ordinary_pipeline(
            &assets2,
            sprite,
            NativeButtonEffect::Down,
            Some(true),
            Some(0),
            policy().state_text_policy
        )
        .unwrap_err()
        .contains("BuiltinShaderLawHashMismatch"));
        std::fs::remove_dir_all(root).unwrap();
    }
}
