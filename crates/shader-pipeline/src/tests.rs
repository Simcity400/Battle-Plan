use super::*;
use std::collections::BTreeMap;
use std::path::Path;

static GPU_TESTS: std::sync::Mutex<()> = std::sync::Mutex::new(());

mod samplers;

#[test]
fn blend_defaults_inherit_alpha_and_masks_follow_native_substring_matching() {
    let source = b"BlendState BlendState { BlendEnable = yes SourceBlend = src_alpha DestBlend = inv_src_alpha WriteMask = \"RED|BLUE\" } Effect Up { }\n";
    let mut program = installed_content::pdxshader::program::read_with("custom.shader", &|path| {
        Some(if path == "custom.shader" {
            source.to_vec()
        } else {
            Vec::new()
        })
    })
    .unwrap();
    let (state, mask) = gpu::blend(&program, &program.techniques[0]).unwrap();
    let state = state.unwrap();
    assert_eq!(state.alpha, state.color);
    assert_eq!(state.alpha.src_factor, wgpu::BlendFactor::SrcAlpha);
    assert_eq!(mask, wgpu::ColorWrites::RED | wgpu::ColorWrites::BLUE);
    program.blends[0].value.source_alpha = 1;
    program.blends[0].value.dest_alpha = 1;
    program.blends[0].value.op_alpha = 4;
    program.blends[0].value.write_mask = 15;
    let (state, mask) = gpu::blend(&program, &program.techniques[0]).unwrap();
    assert_eq!(state.unwrap().alpha.operation, wgpu::BlendOperation::Max);
    assert_eq!(mask, wgpu::ColorWrites::ALL);
}

#[test]
fn depth_stencil_and_rasterizer_follow_native_lowering() {
    let source = b"DepthStencilState Stencil { DepthEnable = yes DepthWriteMask = depth_write_zero DepthFunction = comparison_greater_equal StencilEnable = yes StencilReadMask = 0x1F0 StencilWriteMask = 3 FrontStencilFailOp = stencil_op_zero FrontStencilDepthFailOp = stencil_op_incr FrontStencilPassOp = stencil_op_replace FrontStencilFunc = comparison_not_equal StencilRef = 5 }
DepthStencilState Off { DepthEnable = no }
RasterizerState Lines { FillMode = fill_wireframe CullMode = cull_front FrontCCW = yes DepthBias = -4 SlopeScaleDepthBias = 1.5 DepthBiasClamp = 0.25 Scissor = yes MultiSample = yes }
Effect Stencil { DepthStencilState = STENCIL RasterizerState = lines }
Effect Off { DepthStencilState = Off }
Effect Default { }
";
    let program = installed_content::pdxshader::program::read_with("states.shader", &|path| {
        Some(if path == "states.shader" {
            source.to_vec()
        } else {
            Vec::new()
        })
    })
    .unwrap();
    let [stencil, off, default] = [0, 1, 2].map(|index| &program.techniques[index]);
    let format = wgpu::TextureFormat::Depth24PlusStencil8;
    let (state, reference) = gpu::depth_stencil(
        &program.depth_stencil(stencil),
        &program.rasterizer(stencil),
        format,
    );
    assert_eq!(reference, 5);
    assert_eq!(state.depth_write_enabled, Some(false));
    assert_eq!(
        state.depth_compare,
        Some(wgpu::CompareFunction::GreaterEqual)
    );
    let face = wgpu::StencilFaceState {
        compare: wgpu::CompareFunction::NotEqual,
        fail_op: wgpu::StencilOperation::Zero,
        depth_fail_op: wgpu::StencilOperation::IncrementWrap,
        pass_op: wgpu::StencilOperation::Replace,
    };
    assert_eq!((state.stencil.front, state.stencil.back), (face, face));
    assert_eq!(
        (state.stencil.read_mask, state.stencil.write_mask),
        (0xF0, 3)
    );
    assert_eq!(
        state.bias,
        wgpu::DepthBiasState {
            constant: -4,
            slope_scale: 1.5,
            clamp: 0.25
        }
    );
    let (primitive, scissor) = gpu::primitive(
        &program.rasterizer(stencil),
        wgpu::PrimitiveTopology::TriangleList,
    );
    assert!(scissor);
    assert_eq!(primitive.polygon_mode, wgpu::PolygonMode::Line);
    assert_eq!(primitive.cull_mode, Some(wgpu::Face::Front));
    assert_eq!(primitive.front_face, wgpu::FrontFace::Ccw);
    assert!(!primitive.unclipped_depth);
    let (state, _) = gpu::depth_stencil(
        &program.depth_stencil(off),
        &program.rasterizer(off),
        format,
    );
    assert_eq!(state.depth_write_enabled, Some(false));
    assert_eq!(state.depth_compare, Some(wgpu::CompareFunction::Always));
    assert_eq!(state.stencil, wgpu::StencilState::default());
    // Reader defaults 1423F7140/1423F7180: depth LESS_EQUAL with writes, solid, back culled, CW.
    let (state, reference) = gpu::depth_stencil(
        &program.depth_stencil(default),
        &program.rasterizer(default),
        format,
    );
    assert_eq!(
        (state.depth_write_enabled, state.depth_compare, reference),
        (Some(true), Some(wgpu::CompareFunction::LessEqual), 0)
    );
    let (primitive, scissor) = gpu::primitive(
        &program.rasterizer(default),
        wgpu::PrimitiveTopology::TriangleStrip,
    );
    assert!(!scissor);
    assert_eq!(
        (
            primitive.polygon_mode,
            primitive.cull_mode,
            primitive.front_face
        ),
        (
            wgpu::PolygonMode::Fill,
            Some(wgpu::Face::Back),
            wgpu::FrontFace::Cw
        )
    );
}

#[test]
#[ignore = "requires installed content, a shader compiler and GPU; set BATTLE_PLAN_GAME_DIR"]
fn compile_installed_gui_effects() {
    let _serial = GPU_TESTS.lock().unwrap_or_else(|error| error.into_inner());
    let root = PathBuf::from(std::env::var_os("BATTLE_PLAN_GAME_DIR").or_else(|| std::env::var_os("HOI4_GAME_DIR")).expect("BATTLE_PLAN_GAME_DIR"));
    let compiler = Compiler::discover().unwrap();
    let (device, _queue) = device();
    let mut failures = Vec::new();
    let mut count = 0;
    let started = std::time::Instant::now();
    for file in std::fs::read_dir(root.join("gfx/FX")).unwrap() {
        let path = file.unwrap().path();
        let name = path.file_name().unwrap().to_string_lossy();
        if path.extension().is_none_or(|e| e != "shader") {
            continue;
        }
        // This sweep includes the GUI family and text. Other map effects use a different
        // geometry/constant ABI and are not draws submitted by the native GUI module.
        if !is_gui_effect(&name) {
            continue;
        }
        let effect = format!("gfx/FX/{name}");
        let program = match installed_content::pdxshader::program::read_with(&effect, &|p| {
            std::fs::read(root.join(p)).ok()
        }) {
            Ok(p) => p,
            Err(e) => {
                failures.push(e);
                continue;
            }
        };
        for defines in variants(&program) {
            let name = format!(
                "{name}-{}",
                if defines.is_empty() {
                    "plain".into()
                } else {
                    defines.join("-")
                }
            );
            for technique in &program.techniques {
                if let Some(output) = std::env::var_os("BATTLE_PLAN_SHADER_REPORT").or_else(|| std::env::var_os("HOI4_SHADER_REPORT")) {
                    std::fs::create_dir_all(&output).unwrap();
                    for (stage, suffix) in [(Stage::Vertex, "vs"), (Stage::Pixel, "ps")] {
                        if let Ok(hlsl) = program.hlsl(technique, stage, &defines) {
                            std::fs::write(
                                Path::new(&output)
                                    .join(format!("{name}-{}-{suffix}.hlsl", technique.name)),
                                hlsl,
                            )
                            .unwrap();
                        }
                    }
                }
                match compiler.compile(&program, technique, &defines) {
                    Ok(compiled) => {
                        if let Err(reason) = gpu::Pipeline::new(
                            &device,
                            &compiled,
                            &program,
                            technique,
                            wgpu::TextureFormat::Rgba8Unorm,
                            1,
                        ) {
                            failures.push(format!("{effect}:{} GPU {reason}", technique.name));
                            continue;
                        }
                        count += 1;
                        println!(
                            "PASS {effect}:{} {defines:?} vertex {} B pixel {} B inputs {:?}",
                            technique.name,
                            compiled.vertex.wgsl.len(),
                            compiled.pixel.wgsl.len(),
                            compiled.vertex.inputs
                        );
                        if let Some(output) = std::env::var_os("BATTLE_PLAN_SHADER_REPORT").or_else(|| std::env::var_os("HOI4_SHADER_REPORT")) {
                            std::fs::create_dir_all(&output).unwrap();
                            std::fs::write(
                                Path::new(&output)
                                    .join(format!("{name}-{}-vs.wgsl", technique.name)),
                                compiled.vertex.wgsl,
                            )
                            .unwrap();
                            std::fs::write(
                                Path::new(&output)
                                    .join(format!("{name}-{}-ps.wgsl", technique.name)),
                                compiled.pixel.wgsl,
                            )
                            .unwrap();
                        }
                    }
                    Err(e) => failures.push(format!("{e}, defines {defines:?}")),
                }
            }
        }
    }
    println!(
        "compiled {count} techniques in {:?}, {} failures",
        started.elapsed(),
        failures.len()
    );
    for failure in &failures {
        println!("FAIL {failure}");
    }
    assert!(count > 0);
    assert!(failures.is_empty(), "{} failed techniques", failures.len());
}

// CSpriteType::initialize_animations@14222B5F0 supplies these native define families.
fn is_gui_effect(name: &str) -> bool {
    name.ends_with(".shader")
        && (name.starts_with("buttonstate")
            || matches!(
                name,
                "static_button.shader"
                    | "text.shader"
                    | "maskedflag.shader"
                    | "progress.shader"
                    | "progress_minmax.shader"
                    | "progress_startend.shader"
                    | "progress_reverse.shader"
                    | "progress_radial.shader"
                    | "circularprogressbar.shader"
            ))
}

// Compile each path only where the authored code tests it; names do not select a model.
fn variants(program: &Program) -> Vec<Vec<String>> {
    let source = [&program.common, &program.vertex, &program.pixel]
        .iter()
        .flat_map(|stage| {
            stage
                .code
                .iter()
                .chain(stage.entries.iter().map(|entry| &entry.source))
        })
        .map(|source| source.text.as_str())
        .collect::<Vec<_>>()
        .join("\n");
    let mut variants = vec![Vec::new()];
    if source.contains("ifdef ANIMATED") {
        variants.push(vec!["ANIMATED".into()]);
        variants.push(vec!["ANIMATED".into(), "NUM_ANIMATIONS_2".into()]);
    }
    if source.contains("ifdef MASKING") {
        let masked: Vec<_> = variants
            .iter()
            .map(|defines| {
                let mut defines = defines.clone();
                defines.push("MASKING".into());
                defines
            })
            .collect();
        variants.extend(masked);
    }
    variants
}

pub(crate) fn device() -> (wgpu::Device, wgpu::Queue) {
    let instance =
        wgpu::Instance::new(wgpu::InstanceDescriptor::new_without_display_handle_from_env());
    let adapter =
        pollster::block_on(instance.request_adapter(&wgpu::RequestAdapterOptions::default()))
            .unwrap();
    println!("native shader GPU: {:?}", adapter.get_info());
    pollster::block_on(adapter.request_device(&wgpu::DeviceDescriptor {
        required_features: adapter.features()
            & (wgpu::Features::ADDRESS_MODE_CLAMP_TO_BORDER | wgpu::Features::DUAL_SOURCE_BLENDING),
        ..Default::default()
    }))
    .unwrap()
}

#[test]
#[ignore = "requires a shader compiler"]
fn unlinkable_signatures_and_undeclared_registers_are_refused() {
    let source = r#"
VertexStruct VS_OUT { float4 p : PDX_POSITION;
float2 uv : TEXCOORD0;
float4 c : COLOR0;
};
VertexStruct REORDERED { float4 p : PDX_POSITION;
float4 c : COLOR0;
float2 uv : TEXCOORD0;
};
VertexStruct PREFIX { float4 p : PDX_POSITION;
float2 uv : texcoord;
};
VertexShader = { MainCode V [[ VS_OUT main(float3 pos : POSITION) { VS_OUT o; o.p = float4(pos, 1); o.uv = pos.xy; o.c = pos.xyzz; return o; } ]] }
PixelShader = {
MainCode Reordered [[ float4 main(REORDERED i) : PDX_COLOR { return i.c + i.uv.xyxy; } ]]
MainCode Prefix [[ float4 main(PREFIX i) : PDX_COLOR { return float4(i.uv, 0, 1); } ]]
MainCode Raw [[ cbuffer Raw : register(b1) { float4 k; }; float4 main(VS_OUT i) : PDX_COLOR { return k; } ]]
}
Effect Reordered { VertexShader = V PixelShader = Reordered }
Effect Prefix { VertexShader = V PixelShader = Prefix }
Effect Raw { VertexShader = V PixelShader = Raw }
"#;
    let program =
        installed_content::pdxshader::program::read_with("gfx/FX/linkage.lua", &|path| match path {
            "gfx/FX/linkage.shader" => Some(source.as_bytes().to_vec()),
            "gfx/FX/defines_hlsl_dx11.fxh" => {
                Some(b"#define PDX_POSITION SV_POSITION\n#define PDX_COLOR SV_TARGET\n".to_vec())
            }
            _ => None,
        })
        .unwrap();
    let compiler = Compiler::discover().unwrap();
    let compile = |name| compiler.compile(&program, program.technique(name).unwrap(), &[]);
    let reason = compile("Reordered").unwrap_err();
    assert!(reason.contains("pixel input COLOR0 at location 0 would read vertex output TEXCOORD0"));
    compile("Prefix").unwrap();
    let reason = compile("Raw").unwrap_err();
    assert!(reason.contains("Raw") && reason.contains("no PdxShader declaration"));
    // Registers past D3D11's slots are refused by name before any binding arithmetic.
    for (declaration, register) in [
        (
            "ConstantBuffer(2147483648,0) { float4 k;\n};",
            "b2147483648",
        ),
        ("ConstantBuffer(14,0) { float4 k;\n};", "b14"),
        ("Samplers = { Map { Index = 16 } }", "s16"),
    ] {
        let source = format!(
            "VertexShader = {{ MainCode V [[ float4 main() : SV_POSITION {{ return 0; }} ]] }}\nPixelShader = {{\n{declaration}\nMainCode P [[ float4 main() : SV_TARGET {{ return 1; }} ]] }}\nEffect Up {{ VertexShader = V PixelShader = P }}\n"
        );
        let program = installed_content::pdxshader::program::read_with("limits.shader", &|path| {
            Some(if path == "limits.shader" {
                source.as_bytes().to_vec()
            } else {
                Vec::new()
            })
        })
        .unwrap();
        let reason = compiler
            .compile(&program, &program.techniques[0], &[])
            .unwrap_err();
        assert!(
            reason.contains(register) && reason.contains("exceeds D3D11"),
            "{reason}"
        );
    }
}

#[test]
#[ignore = "requires a shader compiler and GPU"]
fn executes_custom_vertex_and_pixel_code() {
    let _serial = GPU_TESTS.lock().unwrap_or_else(|error| error.into_inner());
    let effect = "gfx/FX/arbitrary-mod.lua";
    let source = r#"
VertexStruct INPUT { float3 position : POSITION;
float2 uv : TEXCOORD0;
};
VertexStruct OUTPUT { float4 p : PDX_POSITION;
float2 uv : TEXCOORD0;
};
ConstantBuffer(0,0) { float4x4 M;
float4 C;
};
VertexShader = { MainCode V [[
OUTPUT main(INPUT v) { OUTPUT o; o.p = mul(M, float4(v.position,1));
o.p.x *= 0.5; o.uv = v.uv; return o; }
]] }
PixelShader = { MainCode P [[
float4 main(OUTPUT v) : PDX_COLOR { return float4(v.uv.x, v.uv.y, C.b, 1); }
]] }
Effect Mod { VertexShader = V PixelShader = P }
"#;
    let program = installed_content::pdxshader::program::read_with(effect, &|path| match path {
        "gfx/FX/arbitrary-mod.shader" => Some(source.as_bytes().to_vec()),
        "gfx/FX/defines_hlsl_dx11.fxh" => {
            Some(b"#define PDX_POSITION SV_POSITION\n#define PDX_COLOR SV_TARGET\n".to_vec())
        }
        _ => None,
    })
    .unwrap();
    let compiler = Compiler::discover().unwrap();
    let technique = program.technique("Mod").unwrap();
    let compiled = compiler.compile(&program, technique, &[]).unwrap();
    let (device, queue) = device();
    let state = gpu::State::authored(&program, technique).unwrap();
    let effect = library::Effect::from_compiled(
        std::sync::Arc::new(program),
        Vec::new(),
        vec![Ok(std::sync::Arc::new(compiled))],
    );
    let selection = effect.select("Mod").unwrap().unwrap();
    let mut cache = gpu::Cache::default();
    let pipeline = cache
        .get(
            &device,
            &selection,
            &state,
            wgpu::TextureFormat::Rgba8Unorm,
            1,
        )
        .unwrap();
    let second = cache
        .get(
            &device,
            &selection,
            &state,
            wgpu::TextureFormat::Rgba8Unorm,
            1,
        )
        .unwrap();
    assert!(std::sync::Arc::ptr_eq(&pipeline, &second));
    assert_eq!(cache.created, 1);
    let mut c0 = vec![0_f32; 20];
    for i in [0, 5, 10, 15] {
        c0[i] = 1.0;
    }
    c0[18] = 0.25;
    let corners = [
        ([-1., 1.], [0., 0.]),
        ([1., 1.], [1., 0.]),
        ([1., -1.], [1., 1.]),
        ([-1., -1.], [0., 1.]),
    ];
    let vertices: Vec<[f32; 5]> = [0, 1, 2, 0, 2, 3]
        .iter()
        .map(|&i| {
            [
                corners[i].0[0],
                corners[i].0[1],
                0.,
                corners[i].1[0],
                corners[i].1[1],
            ]
        })
        .collect();
    let draw = pipeline
        .prepare(
            &device,
            &BTreeMap::from([
                (0, bytemuck::cast_slice(&c0).to_vec()),
                (1, bytemuck::cast_slice(&c0).to_vec()),
            ]),
            &BTreeMap::new(),
            &vertices::Vertices {
                declaration: vertices::Declaration::Sprite,
                bytes: bytemuck::cast_slice(&vertices),
                stride: 20,
                count: vertices.len(),
            },
        )
        .unwrap();
    let pixels = render(&device, &queue, &pipeline, &draw);
    for y in 0..16 {
        for x in 0..16 {
            let pixel = &pixels[(y * 16 + x) * 4..(y * 16 + x + 1) * 4];
            if !(4..12).contains(&x) {
                assert_eq!(pixel, [0, 0, 0, 0]);
            } else {
                let expected = [
                    (((x - 4) as f32 + 0.5) / 8. * 255.).round() as u8,
                    ((y as f32 + 0.5) / 16. * 255.).round() as u8,
                    64,
                    255,
                ];
                assert_eq!(pixel, expected, "pixel {x},{y}");
            }
        }
    }
    let mut stream = stream::Stream::default();
    stream
        .bind_effect(Some(selection), vertices::Declaration::Sprite)
        .unwrap();
    stream.bind_constants(0, Stage::Vertex, Some(bytemuck::cast_slice(&c0)));
    c0[18] = 1.0;
    stream.bind_constants(0, Stage::Pixel, Some(bytemuck::cast_slice(&c0)));
    // Missing technique keeps the input declaration, even when the caller is now text.
    stream
        .bind_effect(None, vertices::Declaration::Text)
        .unwrap();
    stream.bind_constants(0, Stage::Pixel, None);
    for _ in 0..2 {
        let submission = stream
            .prepare(
                &device,
                effect.source.effect.as_str(),
                bytemuck::cast_slice(&vertices),
                20,
                vertices.len(),
                wgpu::TextureFormat::Rgba8Unorm,
                1,
            )
            .unwrap();
        let actual = render(&device, &queue, &submission.pipeline, &submission.draw);
        let mut expected = pixels.clone();
        for pixel in expected.chunks_exact_mut(4).filter(|p| p[3] != 0) {
            pixel[2] = 255;
        }
        assert_eq!(
            actual, expected,
            "retained program and stage-separated constants"
        );
    }
    assert_eq!(stream.cache.created, 1);
    assert!(stream.refused.is_empty());
}

fn render(
    device: &wgpu::Device,
    queue: &wgpu::Queue,
    pipeline: &gpu::Pipeline,
    draw: &gpu::Draw,
) -> Vec<u8> {
    render_with(device, queue, |pass| pipeline.draw(pass, draw))
}

pub(crate) fn render_with(
    device: &wgpu::Device,
    queue: &wgpu::Queue,
    draw: impl FnOnce(&mut wgpu::RenderPass<'_>),
) -> Vec<u8> {
    let target = device.create_texture(&wgpu::TextureDescriptor {
        label: Some("native effect pixel comparison"),
        size: wgpu::Extent3d {
            width: 16,
            height: 16,
            depth_or_array_layers: 1,
        },
        mip_level_count: 1,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format: wgpu::TextureFormat::Rgba8Unorm,
        usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::COPY_SRC,
        view_formats: &[],
    });
    let read = device.create_buffer(&wgpu::BufferDescriptor {
        label: Some("shader pixels"),
        size: 256 * 16,
        usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
        mapped_at_creation: false,
    });
    let mut encoder = device.create_command_encoder(&Default::default());
    {
        let view = target.create_view(&Default::default());
        let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
            label: Some("native shader"),
            color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                view: &view,
                resolve_target: None,
                depth_slice: None,
                ops: wgpu::Operations {
                    load: wgpu::LoadOp::Clear(wgpu::Color::TRANSPARENT),
                    store: wgpu::StoreOp::Store,
                },
            })],
            depth_stencil_attachment: None,
            timestamp_writes: None,
            occlusion_query_set: None,
            multiview_mask: None,
        });
        draw(&mut pass);
    }
    encoder.copy_texture_to_buffer(
        wgpu::TexelCopyTextureInfo {
            texture: &target,
            mip_level: 0,
            origin: wgpu::Origin3d::ZERO,
            aspect: wgpu::TextureAspect::All,
        },
        wgpu::TexelCopyBufferInfo {
            buffer: &read,
            layout: wgpu::TexelCopyBufferLayout {
                offset: 0,
                bytes_per_row: Some(256),
                rows_per_image: None,
            },
        },
        target.size(),
    );
    queue.submit([encoder.finish()]);
    let slice = read.slice(..);
    slice.map_async(wgpu::MapMode::Read, |result| result.unwrap());
    device.poll(wgpu::PollType::wait_indefinitely()).unwrap();
    let bytes = slice.get_mapped_range().unwrap();
    bytes
        .chunks_exact(256)
        .flat_map(|row| row[..64].iter().copied())
        .collect()
}

#[test]
#[ignore = "requires DXC and GPU"]
fn arbitrary_map_pass_layouts_registers_and_volume_texture() {
    use wgpu::util::DeviceExt;
    let _serial = GPU_TESTS.lock().unwrap_or_else(|e| e.into_inner());
    let program = installed_content::pdxshader::program::read_with("arbitrary.shader", &|p| Some(if p == "arbitrary.shader" {
        br#"VertexStruct I { float2 position : POSITION;
uint flag : FLAGS;
};
VertexStruct O { float4 p : SV_POSITION;
float amount : TEXCOORD0;
};
VertexShader = { MainCode V [[ O main(I v) { O o; o.p=float4(v.position,0,1); o.amount=float(v.flag); return o; } ]] }
PixelShader = {
ConstantBuffer(7,99) { float4 tint;
};
Samplers = { Fog { Type = 3d Index = 4 AddressU = Mirror_Once } }
Code [[ Texture3D FogVolume : register(t4); ]]
MainCode P [[ float4 main(O v) : SV_TARGET { return FogVolume.SampleLevel(Fog_Sampler,float3(-0.5,0.5,1.25),0) * tint * v.amount; } ]] }
Effect Map { VertexShader = V PixelShader = P }
"#.to_vec()
    } else { Vec::new() })).unwrap();
    // 142411AB0 declares `Type = 3d` as Texture2D; a volume is the authored code's own view.
    assert!(
        program
            .hlsl(&program.techniques[0], Stage::Pixel, &[])
            .unwrap()
            .contains("Texture2D Fog_Texture : register(t4);")
    );
    let compiled = Compiler::discover()
        .unwrap()
        .compile(&program, &program.techniques[0], &[])
        .unwrap();
    assert_eq!(
        compiled.pixel.textures,
        [(binding(4, Stage::Pixel), TextureKind::D3)]
    );
    assert_eq!(
        compiled
            .vertex
            .inputs
            .iter()
            .find(|i| i.semantic == "FLAGS")
            .unwrap()
            .kind,
        ScalarKind::Uint
    );
    assert_eq!(
        compiled
            .vertex
            .inputs
            .iter()
            .find(|i| i.semantic == "POSITION")
            .unwrap()
            .kind,
        ScalarKind::Float
    );
    let (device, queue) = device();
    let state = gpu::State::authored(&program, &program.techniques[0]).unwrap();
    let mut pass = gpu::Pass::packed(&compiled, &state, wgpu::TextureFormat::Rgba8Unorm, 1);
    let location = |semantic: &str| {
        compiled
            .vertex
            .inputs
            .iter()
            .find(|i| i.semantic.eq_ignore_ascii_case(semantic))
            .unwrap()
            .location
    };
    pass.vertices = vec![
        gpu::VertexLayout {
            stride: 8,
            step: wgpu::VertexStepMode::Vertex,
            attributes: vec![wgpu::VertexAttribute {
                format: wgpu::VertexFormat::Float32x2,
                offset: 0,
                shader_location: location("POSITION"),
            }],
        },
        gpu::VertexLayout {
            stride: 4,
            step: wgpu::VertexStepMode::Instance,
            attributes: vec![wgpu::VertexAttribute {
                format: wgpu::VertexFormat::Uint32,
                offset: 0,
                shader_location: location("FLAGS"),
            }],
        },
    ];
    pass.primitive.topology = wgpu::PrimitiveTopology::TriangleStrip;
    let effect = library::Effect::from_compiled(
        std::sync::Arc::new(program),
        vec![],
        vec![Ok(std::sync::Arc::new(compiled))],
    );
    let selection = effect.select("Map").unwrap().unwrap();
    let mut cache = gpu::Cache::default();
    let pipeline = cache.get_pass(&device, &selection, &state, &pass).unwrap();
    let tex = device.create_texture(&wgpu::TextureDescriptor {
        label: None,
        size: wgpu::Extent3d {
            width: 1,
            height: 1,
            depth_or_array_layers: 2,
        },
        mip_level_count: 1,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D3,
        format: wgpu::TextureFormat::Rgba8Unorm,
        usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
        view_formats: &[],
    });
    // W = 1.25 wraps to the first slice's centre (1423FD550); a clamp would read the second.
    queue.write_texture(
        tex.as_image_copy(),
        &[255, 128, 64, 255, 0, 0, 0, 255],
        wgpu::TexelCopyBufferLayout {
            offset: 0,
            bytes_per_row: Some(4),
            rows_per_image: Some(1),
        },
        tex.size(),
    );
    let bindings = pipeline
        .bindings(
            &device,
            &BTreeMap::from([(
                binding(7, Stage::Pixel),
                bytemuck::cast_slice(&[1_f32; 4]).to_vec(),
            )]),
            &BTreeMap::from([(
                binding(4, Stage::Pixel),
                tex.create_view(&Default::default()),
            )]),
        )
        .unwrap();
    let verts = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
        label: None,
        contents: bytemuck::cast_slice(&[[-1_f32, 1.], [-1., -1.], [1., 1.], [1., -1.]]),
        usage: wgpu::BufferUsages::VERTEX,
    });
    let flags = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
        label: None,
        contents: bytemuck::cast_slice(&[1_u32]),
        usage: wgpu::BufferUsages::VERTEX,
    });
    let pixels = render_with(&device, &queue, |p| {
        p.set_pipeline(&pipeline.pipeline);
        bindings.bind(p);
        p.set_vertex_buffer(0, verts.slice(..));
        p.set_vertex_buffer(1, flags.slice(..));
        p.draw(0..4, 0..1);
    });
    assert!(pixels.chunks_exact(4).all(|p| p == [255, 128, 64, 255]));
    assert!(std::sync::Arc::ptr_eq(
        &pipeline,
        &cache.get_pass(&device, &selection, &state, &pass).unwrap()
    ));
    pass.targets[0].as_mut().unwrap().write_mask = wgpu::ColorWrites::RED;
    cache.get_pass(&device, &selection, &state, &pass).unwrap();
    assert_eq!(
        cache.created, 2,
        "render state is part of the cache identity"
    );
    println!(
        "PASS arbitrary pass: 256 exact pixels; split/instanced integer layout, b7, t4/s4 volume wrapping W, triangle strip and state-key invalidation"
    );
}

/// Every installed vanilla GUI effect, technique and define variant through one persistent
/// cache: the first process compiles, the second must not even discover DXC. Map effects also
/// need the renderer's map-derived global defines, which their consumer supplies.
#[test]
#[ignore = "run twice in separate processes with BATTLE_PLAN_CACHE_TEST_DIR and BATTLE_PLAN_EXPECT_COMPILES=1 then 0"]
fn installed_vanilla_persistent_cache_across_processes() {
    let root = PathBuf::from(std::env::var_os("BATTLE_PLAN_GAME_DIR").or_else(|| std::env::var_os("HOI4_GAME_DIR")).expect("BATTLE_PLAN_GAME_DIR"));
    let dir = PathBuf::from(std::env::var_os("BATTLE_PLAN_CACHE_TEST_DIR").or_else(|| std::env::var_os("HOI4_CACHE_TEST_DIR")).expect("BATTLE_PLAN_CACHE_TEST_DIR"));
    let first = std::env::var("BATTLE_PLAN_EXPECT_COMPILES").or_else(|_| std::env::var("HOI4_EXPECT_COMPILES")).unwrap() != "0";
    struct Checked {
        first: bool,
    }
    impl cache::Compile for Checked {
        fn compile(
            &self,
            source: &Program,
            technique: &Technique,
            defines: &[String],
        ) -> Result<Compiled, String> {
            assert!(self.first, "second process must not even discover DXC");
            Compiler::discover()?.compile(source, technique, defines)
        }
    }
    let cache = std::sync::Arc::new(cache::Cache::new(
        Some(dir.clone()),
        std::sync::Arc::new(Checked { first }),
    ));
    let started = std::time::Instant::now();
    let mut files: Vec<_> = std::fs::read_dir(root.join("gfx/FX"))
        .unwrap()
        .map(|entry| entry.unwrap().file_name().to_string_lossy().into_owned())
        .filter(|name| is_gui_effect(name))
        .collect();
    files.sort();
    let (mut selected, mut failures) = (0, Vec::new());
    for name in &files {
        let program = std::sync::Arc::new(
            installed_content::pdxshader::program::read_with(&format!("gfx/FX/{name}"), &|p| {
                std::fs::read(root.join(p)).ok()
            })
            .unwrap(),
        );
        for defines in variants(&program) {
            let effect = library::Effect::new(program.clone(), defines, cache.clone());
            for technique in &program.techniques {
                match effect.select(&technique.name) {
                    Ok(_) => selected += 1,
                    Err(reason) => failures.push(reason),
                }
            }
        }
    }
    let diagnostics = cache.diagnostics();
    let bytes: u64 = std::fs::read_dir(&dir)
        .unwrap()
        .map(|entry| entry.unwrap().metadata().unwrap().len())
        .sum();
    println!(
        "vanilla process cache: {} files, {selected} selections, compiled={} disk_hits={} in {:?}; {} entries, {bytes} bytes at {}",
        files.len(),
        diagnostics.compiled,
        diagnostics.disk_hits,
        started.elapsed(),
        std::fs::read_dir(&dir).unwrap().count(),
        dir.display()
    );
    assert!(failures.is_empty(), "{failures:#?}");
    assert!(
        diagnostics.warnings.is_empty(),
        "{:?}",
        diagnostics.warnings
    );
    if first {
        assert_eq!(diagnostics.disk_hits, 0);
        assert!(diagnostics.compiled > 0);
    } else {
        assert_eq!(diagnostics.compiled, 0);
        assert!(diagnostics.disk_hits > 0);
    }
}

#[test]
fn authored_samplers_preserve_independent_vertex_and_pixel_slots() {
    let program = installed_content::pdxshader::program::read_with("stages.shader", &|p| {
        Some(if p == "stages.shader" {
            br#"VertexShader = { Samplers = { Height { Index = 2 AddressU = Wrap } } }
PixelShader = { Samplers = { Color { Index = 2 AddressU = Clamp } } }
Effect Pass { VertexShader = V PixelShader = P }
"#
            .to_vec()
        } else {
            Vec::new()
        })
    })
    .unwrap();
    let state = gpu::State::authored(&program, &program.techniques[0]).unwrap();
    assert_eq!(
        state.samplers[&binding(2, Stage::Vertex)].u,
        installed_content::pdxshader::Address::Wrap
    );
    assert_eq!(
        state.samplers[&binding(2, Stage::Pixel)].u,
        installed_content::pdxshader::Address::Clamp
    );
}
