use super::*;
use crate::d3d11_device::{native_d3d11_check_void_upload_device, D3dPtr, NativeD3d11Device};
use crate::production_d3d::{native as targets, native_draw};
use crate::shader_compile::{self, DxbcReflection, ShaderStage as CompileStage};
use replay_core::{ShaderResourceViewDesc, ShaderStage};
use std::{
    marker::PhantomData,
    ptr,
    rc::Rc,
    thread::{self, ThreadId},
};
use winapi::um::{d3d11::*, unknwnbase::IUnknown};

pub struct NativeUiD3dExecutor {
    device: NativeD3d11Device,
    resolver: GameFileResolver,
    owner: ThreadId,
    _not_send_or_sync: PhantomData<Rc<()>>,
}

impl NativeUiD3dExecutor {
    /// Both pointers must be live interfaces on the owning UI thread. The
    /// executor AddRefs them and never borrows a presentation target.
    pub unsafe fn create_on_device(
        device: *mut ID3D11Device,
        context: *mut ID3D11DeviceContext,
        resolver: &GameFileResolver,
    ) -> Result<Self> {
        manifest_identity(resolver)?;
        if device.is_null() || context.is_null() {
            return Err("native UI executor received a null device/context".into());
        }
        if (*context).GetType() != D3D11_DEVICE_CONTEXT_IMMEDIATE {
            return Err("native UI executor requires the owning immediate context".into());
        }
        let mut actual_device = ptr::null_mut();
        (*context).GetDevice(&mut actual_device);
        let owned_device = D3dPtr::from_raw(actual_device, "native UI context device")?;
        if owned_device.as_raw() != device {
            return Err("native UI device and context do not match".into());
        }
        (*(context as *mut IUnknown)).AddRef();
        let owned_context = D3dPtr::from_raw(context, "native UI immediate context")?;
        native_d3d11_check_void_upload_device(&owned_device, "native UI executor creation")?;
        Ok(Self {
            device: NativeD3d11Device {
                device: owned_device,
                context: owned_context,
            },
            resolver: resolver.clone(),
            owner: thread::current().id(),
            _not_send_or_sync: PhantomData,
        })
    }
}

impl NativeUiDrawExecutor for NativeUiD3dExecutor {
    fn draw_over(
        &mut self,
        draw: &PreparedNativeUiDraw<'_>,
        destination: UiRgbaSurfaceMut<'_>,
    ) -> Result<NativeUiDrawExecution> {
        if thread::current().id() != self.owner {
            return Err("native UI immediate context used on another thread".into());
        }
        if manifest_identity(&self.resolver)? != draw.manifest_sha256 {
            return Err("native UI draw and executor admissions differ".into());
        }
        let input = &draw.inputs;
        destination.validate(input.target)?;
        // Recheck admitted files at the actual draw, not just packet creation.
        authenticate(&self.resolver, input)?;
        unsafe {
            native_d3d11_check_void_upload_device(&self.device.device, "native UI draw entry")?;
            let t = input.target;
            let desc = targets::texture_desc(
                t.width,
                t.height,
                t.dxgi_format_u32,
                1,
                0,
                D3D11_USAGE_DEFAULT,
                D3D11_BIND_RENDER_TARGET,
                0,
            );
            let texture =
                targets::create_texture(&self.device, &desc, "native UI private destination")?;
            let mut raw_view = ptr::null_mut();
            hr(
                (*self.device.device.as_raw()).CreateRenderTargetView(
                    texture.as_raw() as *mut _,
                    ptr::null(),
                    &mut raw_view,
                ),
                "CreateRenderTargetView(native UI)",
            )?;
            let view = D3dPtr::from_raw(raw_view, "native UI private RTV")?;
            let mut upload = destination.pixels.to_vec();
            channel_order(&mut upload, t.dxgi_format_u32);
            let pitch = t
                .width
                .checked_mul(4)
                .ok_or("native UI upload pitch overflows")?;
            let context = self.device.context.as_raw();
            (*context).OMSetRenderTargets(0, ptr::null(), ptr::null_mut());
            (*context).UpdateSubresource(
                texture.as_raw() as *mut _,
                0,
                ptr::null(),
                upload.as_ptr() as *const _,
                pitch,
                0,
            );
            native_d3d11_check_void_upload_device(
                &self.device.device,
                "native UI destination upload",
            )?;
            let gfx = native_draw::Gfx::borrowed(self.device.device.as_raw(), context)?;
            let result = native_draw::execute_ui_draw(&gfx, &self.resolver, input, view.as_raw());
            // execute_draw may fail after OM binding. Never leave our private
            // target bound across readback, failure, or destruction.
            (*context).OMSetRenderTargets(0, ptr::null(), ptr::null_mut());
            result?;
            native_d3d11_check_void_upload_device(&self.device.device, "native UI shader draw")?;
            let readback = targets::readback_texture(
                &self.device,
                texture.as_raw(),
                t.width,
                t.height,
                t.dxgi_format_u32,
                pitch,
            )?;
            native_d3d11_check_void_upload_device(&self.device.device, "native UI readback")?;
            install_readback(
                t,
                destination,
                readback.width,
                readback.height,
                readback.dxgi_format_u32,
                readback.tight_row_pitch,
                readback.bytes,
            )
        }
    }
}

fn hr(value: i32, role: &str) -> Result<()> {
    if value < 0 {
        Err(format!("{role} failed with HRESULT 0x{:08x}", value as u32).into())
    } else {
        Ok(())
    }
}

pub(super) fn authenticate(
    resolver: &GameFileResolver,
    input: &NativeUiDrawInputs<'_>,
) -> Result<()> {
    validate_bindings(input)?;
    resolver.read_admitted_used_content(&input.shader.source)?;
    let files = shader_compile::effect_translation_unit_files(
        resolver,
        &input.shader.source.normalized_game_relative_path,
    )?;
    let effect = files
        .last()
        .and_then(|(_, f)| f.effect(&input.shader.effect_token))
        .ok_or("native UI effect is absent from its admitted shader")?;
    if effect.vertex_shader.as_deref() != Some(input.shader.vertex_entry_token.as_str())
        || effect.pixel_shader.as_deref() != Some(input.shader.pixel_entry_token.as_str())
    {
        return Err("native UI effect entry tokens differ from admitted source".into());
    }
    let mut engine_defines = Vec::new();
    let mut effect_defines = Vec::new();
    for d in &input.shader.ordered_defines {
        match d.scope {
            replay_core::ShaderDefineScope::EngineGlobal => {
                engine_defines.push((d.name.clone(), d.value.clone()))
            }
            replay_core::ShaderDefineScope::Effect => effect_defines.push(if d.value.is_empty() {
                d.name.clone()
            } else {
                format!("{} {}", d.name, d.value)
            }),
            replay_core::ShaderDefineScope::EntryPoint => {
                return Err("native UI entry-point define binding is unsupported".into())
            }
        }
    }
    if effect_defines != effect.defines {
        return Err("native UI effect defines differ from admitted source".into());
    }
    for (compile_stage, stage, payload, profile) in [
        (
            CompileStage::Vertex,
            ShaderStage::Vertex,
            &input.shader.compiled_vertex_shader,
            "vs_5_0",
        ),
        (
            CompileStage::Pixel,
            ShaderStage::Pixel,
            &input.shader.compiled_pixel_shader,
            "ps_5_0",
        ),
    ] {
        if payload.target_profile != profile
            || payload.compiler_flags_u64 != shader_compile::ENGINE_COMPILE_FLAGS as u64
        {
            return Err(
                "native UI compiler target/flags differ from the shared engine compiler".into(),
            );
        }
        let hlsl = shader_compile::build_stage_hlsl_with_defines(
            resolver,
            &input.shader.source.normalized_game_relative_path,
            &input.shader.effect_token,
            compile_stage,
            &engine_defines,
        )?;
        let bytes = shader_compile::d3d_compile_stage(
            &hlsl,
            compile_stage,
            shader_compile::ENGINE_COMPILE_FLAGS,
        )?;
        if bytes != payload.bytecode {
            return Err("native UI bytecode differs from the admitted effect compilation".into());
        }
        validate_coverage(
            &shader_compile::parse_dxbc_reflection(&bytes)?,
            stage,
            input,
        )?;
    }
    for t in input.textures {
        resolver.read_admitted_used_content(&t.content)?;
        for subresource in &t.resource.ordered_subresources {
            let bytes = native_draw::materialize_buffer(&subresource.source, resolver, None)?;
            native_draw::validate_texture_subresource_payload(
                t.resource.dimension,
                t.resource.dxgi_format_u32,
                t.resource.width,
                t.resource.height,
                t.resource.depth,
                subresource.mip_level,
                subresource.row_pitch,
                subresource.slice_pitch,
                bytes.len(),
            )?;
        }
    }
    let (count, start) = match input.geometry.draw {
        replay_core::DrawCommand::Draw {
            vertex_count,
            start_vertex,
        } => (vertex_count, start_vertex),
        _ => return Err("native UI non-indexed draw required".into()),
    };
    for buffer in &input.geometry.ordered_vertex_buffers {
        let bytes = native_draw::materialize_buffer(&buffer.source, resolver, None)?;
        let mut element_end = 0u32;
        for element in input
            .pipeline
            .input_layout
            .iter()
            .filter(|e| e.input_slot == buffer.input_slot)
        {
            let width = match element.dxgi_format_u32 {
                2 => 16,
                6 => 12,
                16 => 8,
                41 => 4,
                28 => 4,
                _ => return Err("native UI vertex format size is not supported".into()),
            };
            element_end = element_end.max(
                element
                    .aligned_byte_offset
                    .checked_add(width)
                    .ok_or("native UI input offset overflows")?,
            );
        }
        if element_end > buffer.stride_bytes || element_end == 0 {
            return Err("native UI input elements exceed vertex stride".into());
        }
        let last = start
            .checked_add(count)
            .and_then(|n| n.checked_sub(1))
            .ok_or("native UI draw range overflows")?;
        let end = u64::from(buffer.byte_offset)
            + u64::from(last) * u64::from(buffer.stride_bytes)
            + u64::from(element_end);
        if end > bytes.len() as u64 {
            return Err("native UI draw reads beyond its actual vertex bytes".into());
        }
    }
    Ok(())
}

fn validate_coverage(
    reflection: &DxbcReflection,
    stage: ShaderStage,
    input: &NativeUiDrawInputs<'_>,
) -> Result<()> {
    let stage_mask = match stage {
        ShaderStage::Vertex => 1,
        ShaderStage::Pixel => 2,
        _ => unreachable!(),
    };
    for resource in &reflection.resources {
        if resource.bind_count != 1 {
            return Err("native UI reflected resource array is unsupported".into());
        }
        match resource.resource_type {
            0 => {
                let reflected = reflection
                    .cbuffers
                    .iter()
                    .find(|b| b.name == resource.name)
                    .ok_or("native UI reflected cbuffer has no size")?;
                let bound = input
                    .scene_constants
                    .iter()
                    .chain(input.draw_constants)
                    .find(|c| {
                        c.stage_mask_u8 & stage_mask != 0
                            && u32::from(c.slot) == resource.bind_point
                    })
                    .ok_or("native UI consumed constant-buffer slot is unbound")?;
                if bound.byte_count != reflected.size {
                    return Err(
                        "native UI constant-buffer selected size differs from shader".into(),
                    );
                }
            }
            2 => {
                let bound = input
                    .textures
                    .iter()
                    .find(|t| t.stage == stage && u32::from(t.slot) == resource.bind_point)
                    .ok_or("native UI consumed texture slot is unbound")?;
                let dimension = match bound.view {
                    ShaderResourceViewDesc::Texture1d { .. } => 2,
                    ShaderResourceViewDesc::Texture1dArray { .. } => 3,
                    ShaderResourceViewDesc::Texture2d { .. } => 4,
                    ShaderResourceViewDesc::Texture2dArray { .. } => 5,
                    ShaderResourceViewDesc::Texture2dMultisampled { .. } => 6,
                    ShaderResourceViewDesc::Texture2dMultisampledArray { .. } => 7,
                    ShaderResourceViewDesc::Texture3d { .. } => 8,
                    ShaderResourceViewDesc::TextureCube { .. } => 9,
                    ShaderResourceViewDesc::TextureCubeArray { .. } => 10,
                    ShaderResourceViewDesc::Buffer { .. } => {
                        return Err("native UI buffer SRV is unsupported".into())
                    }
                };
                if resource.dimension != dimension {
                    return Err("native UI texture view dimension differs from shader".into());
                }
            }
            3 => {
                if !input
                    .pipeline
                    .samplers
                    .iter()
                    .any(|s| s.stage == stage && u32::from(s.slot) == resource.bind_point)
                {
                    return Err("native UI consumed sampler slot is unbound".into());
                }
            }
            _ => return Err("native UI consumed resource type is unsupported".into()),
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use replay_core::*;
    use std::collections::BTreeSet;

    struct Fixture {
        root: std::path::PathBuf,
        resolver: GameFileResolver,
        shader: ShaderProgram,
        pipeline: PipelineDesc,
        geometry: GeometryBinding,
        target: NativeUiTargetDesc,
        viewport: ViewportDesc,
    }
    impl Drop for Fixture {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.root);
        }
    }
    impl Fixture {
        fn new(pixel: &str) -> Self {
            Self::with_declarations("", pixel)
        }
        fn with_declarations(declarations: &str, pixel: &str) -> Self {
            let root = crate::test_support::temp_test_dir("native-ui-executor");
            let fx = root.join("gfx/FX");
            std::fs::create_dir_all(&fx).unwrap();
            std::fs::write(
                fx.join("defines_hlsl_dx11.fxh"),
                "// Authored fixture uses native HLSL semantics directly.\n",
            )
            .unwrap();
            let source = format!(
                r#"
{declarations}
VertexStruct Input {{ float2 p : POSITION; }};
VertexStruct Output {{ float4 p : SV_POSITION; }};
VertexShader = {{ MainCode VS [[
Output main(Input v) {{ Output o; o.p=float4(v.p,0.5,1); return o; }}
]] }}
PixelShader = {{ MainCode PS [[
float4 main(Output v) : SV_TARGET {{ {pixel} }}
]] }}
Effect exact {{ VertexShader = "VS" PixelShader = "PS" }}
"#
            );
            std::fs::write(fx.join("fixture.shader"), source).unwrap();
            let raw = GameFileResolver::vanilla(&root).unwrap();
            let paths = BTreeSet::from([
                "gfx/FX/fixture.shader".into(),
                "gfx/FX/defines_hlsl_dx11.fxh".into(),
            ]);
            let (resolver, admission) =
                crate::test_support::admitted_vanilla_resolver(&raw, &paths, "native-ui");
            let compile = |stage: CompileStage| {
                let hlsl = shader_compile::build_stage_hlsl_with_defines(
                    &resolver,
                    "gfx/FX/fixture.shader",
                    "exact",
                    stage,
                    &[],
                )
                .unwrap();
                let bytes = shader_compile::d3d_compile_stage(
                    &hlsl,
                    stage,
                    shader_compile::ENGINE_COMPILE_FLAGS,
                )
                .unwrap();
                CompiledShaderPayload {
                    compiler_identity: "fixture shared D3DCompile".into(),
                    target_profile: stage.profile().into(),
                    compiler_flags_u64: shader_compile::ENGINE_COMPILE_FLAGS as u64,
                    sha256: Sha256::hex(&bytes),
                    bytecode: bytes,
                }
            };
            let shader = ShaderProgram {
                source: admission
                    .require_used_content("gfx/FX/fixture.shader")
                    .unwrap()
                    .clone(),
                producer_registration_root_rva: None,
                effect_token: "exact".into(),
                vertex_entry_token: "VS".into(),
                pixel_entry_token: "PS".into(),
                ordered_defines: vec![],
                compiled_vertex_shader: compile(CompileStage::Vertex),
                compiled_pixel_shader: compile(CompileStage::Pixel),
                geometry_shader: None,
                hull_shader: None,
                domain_shader: None,
                compute_shader: None,
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
                samplers: vec![],
                input_layout: vec![InputElementDesc {
                    semantic_token: "POSITION".into(),
                    semantic_index: 0,
                    dxgi_format_u32: 16,
                    input_slot: 0,
                    aligned_byte_offset: 0,
                    per_instance: false,
                    instance_step_rate: 0,
                }],
                index_format: IndexFormat::None,
                static_indices_u16: vec![],
            };
            let vertices = [-1f32, 1., -1., -1., 0., 1., 0., -1.]
                .into_iter()
                .flat_map(f32::to_le_bytes)
                .collect::<Vec<_>>();
            let geometry = GeometryBinding {
                ordered_vertex_buffers: vec![VertexBufferBinding {
                    input_slot: 0,
                    stride_bytes: 8,
                    byte_offset: 0,
                    source: BufferSource::Inline {
                        sha256: Sha256::hex(&vertices),
                        bytes: vertices,
                    },
                }],
                index_buffer: None,
                stream_output_targets: vec![],
                draw: DrawCommand::Draw {
                    vertex_count: 4,
                    start_vertex: 0,
                },
            };
            Self {
                root,
                resolver,
                shader,
                pipeline,
                geometry,
                target: NativeUiTargetDesc {
                    width: 2,
                    height: 1,
                    dxgi_format_u32: 87,
                    sample_count: 1,
                    sample_quality: 0,
                },
                viewport: ViewportDesc {
                    top_left_x_f32_bits: 0,
                    top_left_y_f32_bits: 0,
                    width_f32_bits: 2f32.to_bits(),
                    height_f32_bits: 1f32.to_bits(),
                    min_depth_f32_bits: 0,
                    max_depth_f32_bits: 1f32.to_bits(),
                },
            }
        }
        fn inputs(&self) -> NativeUiDrawInputs<'_> {
            NativeUiDrawInputs {
                shader: &self.shader,
                textures: &[],
                pipeline: &self.pipeline,
                geometry: &self.geometry,
                scene_constants: &[],
                draw_constants: &[],
                viewport: &self.viewport,
                scissor: None,
                target: &self.target,
            }
        }
    }

    #[test]
    fn source_compilation_and_vertex_extent_are_required() {
        let mut fixture = Fixture::new("return float4(1,0,0,1);");
        PreparedNativeUiDraw::admit(&fixture.resolver, fixture.inputs()).unwrap();
        fixture.shader.compiled_pixel_shader.bytecode.push(0);
        fixture.shader.compiled_pixel_shader.sha256 =
            Sha256::hex(&fixture.shader.compiled_pixel_shader.bytecode);
        assert!(
            PreparedNativeUiDraw::admit(&fixture.resolver, fixture.inputs())
                .err()
                .unwrap()
                .to_string()
                .contains("admitted effect compilation")
        );
        let mut fixture = Fixture::new("return float4(1,0,0,1);");
        fixture.geometry.draw = DrawCommand::Draw {
            vertex_count: 5,
            start_vertex: 0,
        };
        assert!(
            PreparedNativeUiDraw::admit(&fixture.resolver, fixture.inputs())
                .err()
                .unwrap()
                .to_string()
                .contains("beyond its actual vertex bytes")
        );
    }

    #[test]
    fn rehashed_source_mutation_is_not_a_reused_admission() {
        // A unique source keeps this hostile snapshot mutation isolated from
        // other fixtures using the content-addressed admission cache.
        let fixture = Fixture::new("return float4(1,0,0,1); // snapshot mutation fixture\n");
        PreparedNativeUiDraw::admit(&fixture.resolver, fixture.inputs()).unwrap();
        let snapshot = fixture
            .resolver
            .resolve_required("gfx/FX/fixture.shader", "native UI fixture snapshot")
            .unwrap();
        let original = fixture
            .resolver
            .read_admitted_used_content(&fixture.shader.source)
            .unwrap();
        assert!(!snapshot.starts_with(&fixture.root));
        std::fs::write(
            fixture.root.join("gfx/FX/fixture.shader"),
            "Effect replaced {}\n",
        )
        .unwrap();
        // Production reads its authenticated snapshot, not the now-replaced
        // installation file. The actual consumed snapshot must still match.
        assert_eq!(
            fixture
                .resolver
                .read_admitted_used_content(&fixture.shader.source)
                .unwrap(),
            original
        );
        PreparedNativeUiDraw::admit(&fixture.resolver, fixture.inputs()).unwrap();
        crate::test_support::overwrite_readonly(&snapshot, b"Effect tampered {}\n");
        assert!(
            PreparedNativeUiDraw::admit(&fixture.resolver, fixture.inputs())
                .err()
                .unwrap()
                .to_string()
                .contains("immutable admitted content snapshot changed")
        );
    }

    #[test]
    fn selected_constant_ranges_cannot_alias_an_existing_stage_slot() {
        let fixture = Fixture::new("return float4(1,0,0,1);");
        let bytes = vec![0; 16];
        let constants = [ConstantBufferBinding {
            stage_mask_u8: 1,
            slot: 0,
            byte_offset: 0,
            byte_count: 16,
            sha256: Sha256::hex(&bytes),
            raw_bytes: bytes,
            semantic_layout: "actual fixture cb".into(),
        }];
        let mut input = fixture.inputs();
        input.scene_constants = &constants;
        input.draw_constants = &constants;
        assert!(PreparedNativeUiDraw::admit(&fixture.resolver, input)
            .err()
            .unwrap()
            .to_string()
            .contains("duplicates"));
    }

    #[test]
    fn compiled_consumed_slots_and_cbuffer_size_require_actual_providers() {
        let fixture =
            Fixture::with_declarations("ConstantBuffer(0,0) { float4 tint; };", "return tint;");
        assert!(
            PreparedNativeUiDraw::admit(&fixture.resolver, fixture.inputs())
                .err()
                .unwrap()
                .to_string()
                .contains("consumed constant-buffer slot is unbound")
        );
        let bytes = vec![0; 32];
        let mut constants = [ConstantBufferBinding {
            stage_mask_u8: 2,
            slot: 0,
            byte_offset: 0,
            byte_count: 32,
            sha256: Sha256::hex(&bytes),
            raw_bytes: bytes,
            semantic_layout: "fixture tint".into(),
        }];
        let mut input = fixture.inputs();
        input.draw_constants = &constants;
        assert!(PreparedNativeUiDraw::admit(&fixture.resolver, input)
            .err()
            .unwrap()
            .to_string()
            .contains("selected size differs"));
        constants[0].byte_count = 16;
        let mut input = fixture.inputs();
        input.draw_constants = &constants;
        PreparedNativeUiDraw::admit(&fixture.resolver, input).unwrap();
        let fixture = Fixture::with_declarations(
            "Code [[ Texture2D<float4> required_image : register(t0); ]]",
            "return required_image.Load(int3(0,0,0));",
        );
        assert!(
            PreparedNativeUiDraw::admit(&fixture.resolver, fixture.inputs())
                .err()
                .unwrap()
                .to_string()
                .contains("consumed texture slot is unbound")
        );
    }

    #[test]
    #[ignore = "requires actual D3D11 hardware; validates executor mechanics, not installed UI/native input admission"]
    fn hardware_draw_preserves_cpu_interleaving_and_refusal_destination() {
        let fixture =
            Fixture::new("return float4(1,0,0,1); // hardware snapshot mutation fixture\n");
        let device = unsafe { crate::d3d11_device::native_d3d11_create_device().unwrap() };
        let mut executor = unsafe {
            NativeUiD3dExecutor::create_on_device(
                device.device.as_raw(),
                device.context.as_raw(),
                &fixture.resolver,
            )
            .unwrap()
        };
        let prepared = PreparedNativeUiDraw::admit(&fixture.resolver, fixture.inputs()).unwrap();
        let mut pixels = [17, 29, 43, 255, 53, 67, 79, 255];
        let result = executor
            .draw_over(
                &prepared,
                UiRgbaSurfaceMut {
                    width: 2,
                    height: 1,
                    pixels: &mut pixels,
                },
            )
            .unwrap();
        assert_eq!(result.submitted_draw_count(), 1);
        assert_eq!(pixels, [255, 0, 0, 255, 53, 67, 79, 255]);
        pixels[4..].copy_from_slice(&[0, 255, 0, 255]); // actual next CPU paint
        executor
            .draw_over(
                &prepared,
                UiRgbaSurfaceMut {
                    width: 2,
                    height: 1,
                    pixels: &mut pixels,
                },
            )
            .unwrap();
        assert_eq!(pixels, [255, 0, 0, 255, 0, 255, 0, 255]);
        let before = pixels;
        assert!(executor
            .draw_over(
                &prepared,
                UiRgbaSurfaceMut {
                    width: 1,
                    height: 1,
                    pixels: &mut pixels
                }
            )
            .is_err());
        assert_eq!(pixels, before);
        let snapshot = fixture
            .resolver
            .resolve_required("gfx/FX/fixture.shader", "native UI hardware snapshot")
            .unwrap();
        assert!(!snapshot.starts_with(&fixture.root));
        crate::test_support::overwrite_readonly(&snapshot, b"Effect replaced {}\n");
        assert!(executor
            .draw_over(
                &prepared,
                UiRgbaSurfaceMut {
                    width: 2,
                    height: 1,
                    pixels: &mut pixels
                }
            )
            .is_err());
        assert_eq!(pixels, before);
    }
}
