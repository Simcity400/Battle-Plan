use super::*;
use std::collections::BTreeMap;

pub(crate) fn initialize_runtime_tables() {
    content_reader_script::test_game::enter();
}

pub(crate) static GPU_TESTS: std::sync::Mutex<()> = std::sync::Mutex::new(());

#[test]
fn the_compiler_is_the_named_dll_and_nothing_else() {
    let scratch = Scratch::new().unwrap();
    let library = scratch.0.join("dxcompiler.dll");
    assert_eq!(Compiler::at(&library).err().as_deref(), Some(MISSING_COMPILER));
    std::fs::write(&library, b"not a library").unwrap();
    assert_eq!(Compiler::at(&library).err().as_deref(), Some("dxcompiler.dll carries no product version"));
    if let Some(root) = std::env::var_os("SystemRoot") {
        let system_dll = std::path::PathBuf::from(root).join("System32").join("kernel32.dll");
        if system_dll.is_file() {
            let version = dxc::product_version(&system_dll).unwrap();
            assert!(!version.is_empty());
            assert_eq!(
                Compiler::at(&system_dll).err().as_deref(),
                Some(format!("dxcompiler.dll is {version:?}, not the pinned {DXC_VERSION}").as_str())
            );
        }
    }
}

#[test]
fn shader_module_labels_avoid_dxc_path_and_flag_characters() {
    assert_eq!(gpu::shader_module_label("named.shader:Water"), "named.shader_Water");
    assert_eq!(
        gpu::shader_module_label("gfx/FX/river.shader:Water"),
        "gfx_FX_river.shader_Water"
    );
    assert_eq!(gpu::shader_module_label("-flag.shader:Up"), "_-flag.shader_Up");
    assert_eq!(gpu::shader_module_label(""), "_");
}

mod arena;
mod dimensions;
mod numeric;
mod samplers;

#[test]
fn unknown_constant_bytes_are_distinct_from_out_of_range_reads() {
    let buffer = Buffer {
        binding: 5,
        size: 48,
        reads: vec![0..12, 16..24, 32..48],
    };
    assert!(buffer.check_initialized(32, &[0..12, 16..24]).is_ok());
    assert!(
        buffer
            .check_initialized(48, &[0..12, 16..24])
            .unwrap_err()
            .contains("pixel constant buffer b2 reads unavailable bytes 32..48")
    );
    assert!(buffer.check_initialized(20, &[0..12, 16..20]).is_ok());
    assert!(buffer.check_initialized(32, &[0..12, 16..20]).is_err());
    assert!(
        buffer
            .check_initialized(48, &[32..40, 0..12, 40..48, 16..24])
            .is_ok()
    );
}

#[test]
#[ignore = "requires the pinned shader compiler"]
fn constant_reads_exclude_padding_and_include_dynamic_array_components() {
    crate::tests::initialize_runtime_tables();
    let compile = |body: &str| {
        let source = format!(
            r#"ConstantBuffer(2,0) {{
float4 value;
float3 direction;
float unused;
float4 elements[3];
float4x4 transform;
}};
VertexStruct I {{
float3 position : POSITION;
}};
VertexStruct O {{
float4 position : SV_POSITION;
}};
VertexShader = {{ MainCode V [[ O main(I v) {{ O o; o.position=float4(v.position,1); return o; }} ]] }}
PixelShader = {{ MainCode P [[ float4 main(O v) : SV_TARGET {{ {body} }} ]] }}
Effect Test {{ VertexShader = V PixelShader = P }}
"#
        );
        let program = content_reader_gui::pdxshader::program::read_with(b"reads.shader", &|path| {
            Some(if path == b"reads.shader" {
                source.as_bytes().to_vec()
            } else {
                Vec::new()
            })
        })
        .unwrap();
        Compiler::discover()
            .unwrap()
            .compile(&program, &program.techniques[0], &[])
            .unwrap()
    };
    let simple = compile("return float4(value.xyz + direction, 1);");
    assert_eq!(simple.pixel.buffers[0].reads, vec![0..12, 16..28]);
    let component = compile("return value.z;");
    assert_eq!(component.pixel.buffers[0].reads, vec![8..12]);
    let dynamic = compile("return float4(elements[uint(v.position.x)%3].xyz, 1);");
    assert_eq!(dynamic.pixel.buffers[0].reads, vec![32..44, 48..60, 64..76]);
    let matrix = compile("return mul(transform,float4(v.position.xyz,1));");
    assert_eq!(matrix.pixel.buffers[0].reads, vec![80..144]);
    let gap = compile("return unused;");
    assert_eq!(gap.pixel.buffers[0].reads, vec![28..32]);
    assert!(
        gap.pixel.buffers[0]
            .check_initialized(32, std::slice::from_ref(&(0..28)))
            .is_err()
    );
    assert!(
        gap.pixel.buffers[0]
            .check_initialized(16, std::slice::from_ref(&(0..16)))
            .is_ok()
    );
}

#[test]
fn blend_defaults_inherit_alpha_and_masks_follow_native_substring_matching() {
    crate::tests::initialize_runtime_tables();
    let source = b"BlendState BlendState { BlendEnable = yes SourceBlend = src_alpha DestBlend = inv_src_alpha WriteMask = \"RED|BLUE\" } Effect Up { }\n";
    let mut program = content_reader_gui::pdxshader::program::read_with(b"custom.shader", &|path| {
        Some(if path == b"custom.shader" {
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
    crate::tests::initialize_runtime_tables();
    let source = b"DepthStencilState Stencil { DepthEnable = yes DepthWriteMask = depth_write_zero DepthFunction = comparison_greater_equal StencilEnable = yes StencilReadMask = 0x1F0 StencilWriteMask = 3 FrontStencilFailOp = stencil_op_zero FrontStencilDepthFailOp = stencil_op_incr FrontStencilPassOp = stencil_op_replace FrontStencilFunc = comparison_not_equal StencilRef = 5 }
DepthStencilState Off { DepthEnable = no }
RasterizerState Lines { FillMode = fill_wireframe CullMode = cull_front FrontCCW = yes DepthBias = -4 SlopeScaleDepthBias = 1.5 DepthBiasClamp = 0.25 Scissor = yes MultiSample = yes }
Effect Stencil { DepthStencilState = STENCIL RasterizerState = lines }
Effect Off { DepthStencilState = Off }
Effect Default { }
";
    let program = content_reader_gui::pdxshader::program::read_with(b"states.shader", &|path| {
        Some(if path == b"states.shader" {
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

pub(crate) fn device() -> (wgpu::Device, wgpu::Queue) {
    let instance =
        crate::gpu::instance();
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
    crate::tests::initialize_runtime_tables();
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
        content_reader_gui::pdxshader::program::read_with(b"gfx/FX/linkage.lua", &|path| match path {
            b"gfx/FX/linkage.shader" => Some(source.as_bytes().to_vec()),
            b"gfx/FX/defines_hlsl_dx11.fxh" => {
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
        let program = content_reader_gui::pdxshader::program::read_with(b"limits.shader", &|path| {
            Some(if path == b"limits.shader" {
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

const CLOCK_CONSTANTS: &str = "ConstantBuffer( 2, 7 ) { float4x4 Transform;\nfloat Restart;\nfloat Clock;\nfloat2 Shift;\nfloat4 Tint;\n};\n";

#[test]
#[ignore = "requires a shader compiler"]
fn compiled_programs_answer_how_long_they_change() {
    crate::tests::initialize_runtime_tables();
    let source = format!(
        r#"
VertexStruct CORNER_IN {{ float3 at : POSITION;
float2 uv : TEXCOORD0;
}};
VertexStruct CORNER_OUT {{ float4 at : PDX_POSITION;
float2 uv : TEXCOORD0;
}};
{CLOCK_CONSTANTS}
VertexShader = {{
MainCode Plain [[ CORNER_OUT main(const CORNER_IN i) {{ CORNER_OUT o; o.at = mul(Transform, float4(i.at, 1)); o.uv = i.uv + Shift; return o; }} ]]
MainCode Wobble [[ CORNER_OUT main(const CORNER_IN i) {{ CORNER_OUT o; o.at = mul(Transform, float4(i.at.x + cos(Clock), i.at.yz, 1)); o.uv = i.uv; return o; }} ]]
}}
PixelShader = {{
MainCode Still [[ float4 main(CORNER_OUT i) : PDX_COLOR {{ return Tint * Restart; }} ]]
MainCode Shade [[ float4 main(CORNER_OUT i) : PDX_COLOR {{ float e = saturate((Clock - Restart) * 8); e = e * e * (3 - 2 * e); return Tint * (1 - e); }} ]]
MainCode Fade [[ float4 main(CORNER_OUT i) : PDX_COLOR {{ return Tint * (1 - smoothstep(0.0, 3.0, Clock - Restart)); }} ]]
MainCode Looped [[ float4 main(CORNER_OUT i) : PDX_COLOR {{ float4 c = Tint; float s = saturate((Clock - Restart) * 32); for (int n = 0; n < 2; ++n) {{ if (i.uv.y < 0.25) c.rgb *= s; }} return c; }} ]]
MainCode Pulse [[ float4 main(CORNER_OUT i) : PDX_COLOR {{ return Tint * cos(Clock); }} ]]
MainCode Moment [[ float4 main(CORNER_OUT i) : PDX_COLOR {{ return Tint * saturate(Clock * 2); }} ]]
MainCode Gate [[ float4 main(CORNER_OUT i) : PDX_COLOR {{ if (Clock - Restart > 1.5) discard; return Tint; }} ]]
MainCode Drop [[ float4 main(CORNER_OUT i) : PDX_COLOR {{ return Tint * step(-0.25, Restart - Clock); }} ]]
MainCode Rise [[ float4 main(CORNER_OUT i) : PDX_COLOR {{ return Tint * step(0.75, Clock - Restart); }} ]]
}}
Effect Still {{ VertexShader = Plain PixelShader = Still }}
Effect Shade {{ VertexShader = Plain PixelShader = Shade }}
Effect Fade {{ VertexShader = Plain PixelShader = Fade }}
Effect Looped {{ VertexShader = Plain PixelShader = Looped }}
Effect Pulse {{ VertexShader = Plain PixelShader = Pulse }}
Effect Moment {{ VertexShader = Plain PixelShader = Moment }}
Effect Gate {{ VertexShader = Plain PixelShader = Gate }}
Effect Wobble {{ VertexShader = Wobble PixelShader = Still }}
Effect Drop {{ VertexShader = Plain PixelShader = Drop }}
Effect Rise {{ VertexShader = Plain PixelShader = Rise }}
"#
    );
    let program =
        content_reader_gui::pdxshader::program::read_with(b"gfx/FX/clock.lua", &|path| match path {
            b"gfx/FX/clock.shader" => Some(source.as_bytes().to_vec()),
            b"gfx/FX/defines_hlsl_dx11.fxh" => {
                Some(b"#define PDX_POSITION SV_POSITION\n#define PDX_COLOR SV_TARGET\n".to_vec())
            }
            _ => None,
        })
        .unwrap();
    let compiler = Compiler::discover().unwrap();
    let settles = |name| {
        compiler
            .compile(&program, program.technique(name).unwrap(), &[])
            .unwrap()
            .settles(2, 68, 64)
    };
    assert_eq!(settles("Still"), Some(clock::Settling::NOW));
    let near = |name, seconds: f32| {
        let settling = settles(name).unwrap_or_else(|| panic!("{name} never settles"));
        assert!(
            seconds <= settling.after && settling.after <= seconds.next_up().next_up(),
            "{name}: {settling:?}"
        );
        assert!(settling.per_clock < 1e-5, "{name}: {settling:?}");
    };
    near("Shade", 0.125);
    near("Fade", 3.0);
    near("Looped", 1.0 / 32.0);
    near("Drop", 0.25_f32.next_up());
    near("Rise", 0.75);
    for name in ["Pulse", "Moment", "Gate", "Wobble"] {
        assert_eq!(settles(name), None, "{name}");
    }
}

#[test]
#[ignore = "requires a shader compiler"]
fn synthetic_button_states_settle_with_or_without_masking() {
    crate::tests::initialize_runtime_tables();
    let source = r#"
VertexShader = { MainCode V [[ float4 main() : SV_POSITION { return float4(0, 0, 0, 1); } ]] }
PixelShader = {
    ConstantBuffer(0, 0) { float Clock;
float Restart;
};
    Code [[
        float shade(float v) {
        #ifdef MASKING
            return v * 0.5;
        #else
            return v;
        #endif
        }
    ]]
    MainCode Still [[ float4 main() : SV_TARGET { return shade(1); } ]]
    MainCode Over [[ float4 main() : SV_TARGET { return shade(saturate((Clock - Restart) * 2)); } ]]
    MainCode Down [[ float4 main() : SV_TARGET { return shade(saturate((Clock - Restart) * 8)); } ]]
}
Effect Up { VertexShader = V PixelShader = Still }
Effect Over { VertexShader = V PixelShader = Over }
Effect Down { VertexShader = V PixelShader = Down }
"#;
    let program = content_reader_gui::pdxshader::program::read_with(b"button.shader", &|path| {
        Some(if path == b"button.shader" {
            source.as_bytes().to_vec()
        } else {
            Vec::new()
        })
    })
    .unwrap();
    let compiler = Compiler::discover().unwrap();
    for defines in [vec![], vec!["MASKING".to_owned()]] {
        let settles = |name| {
            compiler
                .compile(&program, program.technique(name).unwrap(), &defines)
                .unwrap()
                .settles(0, 0, 4)
        };
        assert_eq!(settles("Up"), Some(clock::Settling::NOW));
        for (name, seconds) in [("Over", 0.5_f32), ("Down", 0.125)] {
            let settling = settles(name).unwrap();
            assert!(
                seconds <= settling.after && settling.after <= seconds.next_up().next_up(),
                "{name}: {settling:?}"
            );
            assert!(settling.per_clock < 1e-5, "{name}: {settling:?}");
        }
    }
}

#[test]
#[ignore = "requires a shader compiler and GPU"]
fn executes_custom_vertex_and_pixel_code() {
    crate::tests::initialize_runtime_tables();
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
    let program =
        content_reader_gui::pdxshader::program::read_with(effect.as_bytes(), &|path| match path {
            b"gfx/FX/arbitrary-mod.shader" => Some(source.as_bytes().to_vec()),
            b"gfx/FX/defines_hlsl_dx11.fxh" => {
                Some(b"#define PDX_POSITION SV_POSITION\n#define PDX_COLOR SV_TARGET\n".to_vec())
            }
            _ => None,
        })
        .unwrap();
    let compiler = Compiler::discover().unwrap();
    let technique = program.technique("Mod").unwrap();
    let compiled = compiler.compile(&program, technique, &[]).unwrap();
    let (device, queue) = device();
    let arena = gpu::Arena::new(&device);
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
            wgpu::PrimitiveTopology::TriangleList,
        )
        .unwrap();
    let second = cache
        .get(
            &device,
            &selection,
            &state,
            wgpu::TextureFormat::Rgba8Unorm,
            1,
            wgpu::PrimitiveTopology::TriangleList,
        )
        .unwrap();
    assert!(std::sync::Arc::ptr_eq(&pipeline, &second));
    assert_eq!(cache.created, 1);
    for variant in 0..6 {
        let mut changed = state.clone();
        let mut format = wgpu::TextureFormat::Rgba8Unorm;
        let mut samples = 1;
        let mut topology = wgpu::PrimitiveTopology::TriangleList;
        match variant {
            0 => changed.write_mask = wgpu::ColorWrites::RED,
            1 => changed.blend = Some(wgpu::BlendState::ALPHA_BLENDING),
            2 => format = wgpu::TextureFormat::Rgba8UnormSrgb,
            3 => samples = 4,
            4 => topology = wgpu::PrimitiveTopology::TriangleStrip,
            _ => {
                changed.samplers.insert(
                    0,
                    content_reader_gui::pdxshader::program::Sampler {
                        name: "unused".into(),
                        index: 0,
                        cube: false,
                        mag: content_reader_gui::pdxshader::program::Filter::Point,
                        min: content_reader_gui::pdxshader::program::Filter::Point,
                        mip: content_reader_gui::pdxshader::program::Filter::Point,
                        u: content_reader_gui::pdxshader::Address::Clamp,
                        v: content_reader_gui::pdxshader::Address::Clamp,
                        w: content_reader_gui::pdxshader::Address::Wrap,
                        lod_bias: -0.0,
                        min_lod: 0,
                        max_lod: 1000,
                    },
                );
            }
        }
        let selected = cache
            .get(&device, &selection, &changed, format, samples, topology)
            .unwrap();
        assert!(!std::sync::Arc::ptr_eq(&pipeline, &selected));
        let compiled = selection.effect.compiled(selection.index).unwrap();
        let mut pass = gpu::Pass::packed(&compiled, &changed, format, samples);
        pass.primitive.topology = topology;
        assert!(std::sync::Arc::ptr_eq(
            &selected,
            &cache
                .get_pass(&device, &selection, &changed, &pass)
                .unwrap()
        ));
        assert!(std::sync::Arc::ptr_eq(
            &pipeline,
            &cache
                .get(
                    &device,
                    &selection,
                    &state,
                    wgpu::TextureFormat::Rgba8Unorm,
                    1,
                    wgpu::PrimitiveTopology::TriangleList
                )
                .unwrap()
        ));
    }
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
            &arena,
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
    let pixels = render(&device, &queue, &arena, &pipeline, &draw);
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
    stream
        .bind_effect(None, vertices::Declaration::Text)
        .unwrap();
    stream.bind_constants(0, Stage::Pixel, None);
    for _ in 0..2 {
        let submission = stream
            .prepare(
                &arena,
                effect.source.effect.as_str(),
                bytemuck::cast_slice(&vertices),
                20,
                vertices.len(),
                wgpu::PrimitiveTopology::TriangleList,
                wgpu::TextureFormat::Rgba8Unorm,
                1,
            )
            .unwrap();
        let actual = render(
            &device,
            &queue,
            &arena,
            &submission.pipeline,
            &submission.draw,
        );
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
    arena: &gpu::Arena,
    pipeline: &gpu::Pipeline,
    draw: &gpu::Draw,
) -> Vec<u8> {
    render_with(device, queue, arena, |pass| pipeline.draw(pass, draw))
}

pub(crate) fn render_with(
    device: &wgpu::Device,
    queue: &wgpu::Queue,
    arena: &gpu::Arena,
    draw: impl FnOnce(&mut wgpu::RenderPass<'_>),
) -> Vec<u8> {
    render_format(device, queue, arena, wgpu::TextureFormat::Rgba8Unorm, draw)
}

fn render_format(
    device: &wgpu::Device,
    queue: &wgpu::Queue,
    arena: &gpu::Arena,
    format: wgpu::TextureFormat,
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
        format,
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
    arena.flush(queue);
    queue.submit([encoder.finish()]);
    arena.reset();
    let slice = read.slice(..);
    slice.map_async(wgpu::MapMode::Read, |result| result.unwrap());
    device.poll(wgpu::PollType::wait_indefinitely()).unwrap();
    let bytes = slice.get_mapped_range().unwrap();
    bytes
        .chunks_exact(256)
        .flat_map(|row| {
            row[..16 * format.block_copy_size(None).unwrap() as usize]
                .iter()
                .copied()
        })
        .collect()
}

#[test]
#[ignore = "requires DXC and GPU"]
fn arbitrary_map_pass_layouts_registers_and_volume_texture() {
    crate::tests::initialize_runtime_tables();
    use wgpu::util::DeviceExt;
    let _serial = GPU_TESTS.lock().unwrap_or_else(|e| e.into_inner());
    let program = content_reader_gui::pdxshader::program::read_with(b"arbitrary.shader", &|p| Some(if p == b"arbitrary.shader" {
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
    let arena = gpu::Arena::new(&device);
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
            &arena,
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
    let pixels = render_with(&device, &queue, &arena, |p| {
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
    let mut stream = stream::Stream::default();
    stream.bind_program(Some(selection.clone()), pass.vertices.clone());
    stream.bind_constants(7, Stage::Pixel, Some(bytemuck::cast_slice(&[1_f32; 4])));
    stream.bind_constants(7, Stage::Vertex, Some(bytemuck::cast_slice(&[0_f32; 4])));
    stream.bind_texture(
        4,
        Stage::Pixel,
        Some(tex.create_view(&Default::default())),
        Some(&state.samplers[&binding(4, Stage::Pixel)]),
    );
    stream.bind_program(None, Vec::new());
    stream.bind_constants(7, Stage::Pixel, None);
    stream.bind_partial_constants(
        7,
        Stage::Pixel,
        Some(bytemuck::cast_slice(&[1_f32; 4])),
        std::slice::from_ref(&(0..12)),
    );
    stream.bind_partial_constants(7, Stage::Pixel, None, std::slice::from_ref(&(0..16)));
    assert!(
        stream
            .prepare_pass(&arena, "unknown alpha", &pass)
            .err()
            .unwrap()
            .contains("unavailable bytes 12..16")
    );
    let mut inherited = stream::Stream::default();
    inherited.inherit_bindings(&stream);
    assert!(
        inherited
            .prepare_pass(&arena, "inherited unknown alpha", &pass)
            .is_err()
    );
    stream.bind_constants(7, Stage::Pixel, Some(bytemuck::cast_slice(&[1_f32; 4])));
    let padded = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
        label: Some("retained input declaration with replacement buffer stride"),
        contents: bytemuck::cast_slice(&[
            [-1_f32, 1., 91., 92.],
            [-1., -1., 93., 94.],
            [1., 1., 95., 96.],
            [1., -1., 97., 98.],
        ]),
        usage: wgpu::BufferUsages::VERTEX,
    });
    for (null, changed_stride) in [(false, false), (false, true), (true, true)] {
        if changed_stride {
            stream.bind_vertex_stride(0, 16);
            stream.bind_program(None, Vec::new());
            let layout = stream.vertex_layouts().unwrap();
            assert_eq!(layout[0].stride, 16);
            assert_eq!(layout[1], pass.vertices[1]);
        }
        if null {
            stream.bind_texture(4, Stage::Pixel, None, None);
        }
        let submission = stream
            .prepare_pass(&arena, "retained volume", &pass)
            .unwrap();
        let actual = render_with(&device, &queue, &arena, |p| {
            p.set_pipeline(&submission.pipeline.pipeline);
            submission.bindings.bind(p);
            p.set_vertex_buffer(0, if changed_stride { &padded } else { &verts }.slice(..));
            p.set_vertex_buffer(1, flags.slice(..));
            p.draw(0..4, 0..1);
        });
        assert!(
            actual
                .chunks_exact(4)
                .all(|p| p == if null { [0; 4] } else { [255, 128, 64, 255] })
        );
    }
    assert_eq!(
        stream.cache.created, 2,
        "buffer stride changes pipeline identity; null SRV and template RED mask do not"
    );
    let mut overlap = pass.vertices.clone();
    overlap[0].attributes[0].offset = 16;
    stream.bind_program(Some(selection), overlap);
    stream.bind_program(None, Vec::new());
    stream.bind_vertex_stride(0, 8);
    stream.bind_texture(
        4,
        Stage::Pixel,
        Some(tex.create_view(&Default::default())),
        None,
    );
    let source = [vec![91_f32; 5], vec![-1., 1., -1., -1., 1., 1., 1., -1.]].concat();
    let submission = stream
        .prepare_native_pass(
            &arena,
            "overlapping native declaration",
            &pass,
            &BTreeMap::from([
                (
                    0,
                    Some(vertices::NativeBuffer {
                        bytes: bytemuck::cast_slice(&source),
                        size: 52,
                        offset: 4,
                        known: std::slice::from_ref(&(0..52)),
                    }),
                ),
                (
                    1,
                    Some(vertices::NativeBuffer {
                        bytes: bytemuck::cast_slice(&[1_u32]),
                        size: 4,
                        offset: 0,
                        known: std::slice::from_ref(&(0..4)),
                    }),
                ),
            ]),
            4,
            1,
        )
        .unwrap();
    let actual = render_with(&device, &queue, &arena, |p| {
        p.set_pipeline(&submission.pass.pipeline.pipeline);
        submission.pass.bindings.bind(p);
        for (slot, buffer) in submission.vertices.iter().enumerate() {
            p.set_vertex_buffer(slot as u32, buffer.slice());
        }
        p.draw(0..4, 0..1);
    });
    assert!(
        actual
            .chunks_exact(4)
            .all(|pixel| pixel == [255, 128, 64, 255])
    );
    assert_eq!(stream.vertex_layouts().unwrap()[0].attributes[0].offset, 16);
    assert_eq!(stream.vertex_layouts().unwrap()[0].stride, 8);
    stream.invalidate_bindings();
    assert!(
        stream
            .prepare_pass(&arena, "invalidated", &pass)
            .err()
            .unwrap()
            .contains("preceding native program")
    );
    assert_eq!(stream.refused.values().sum::<u64>(), 2);
    println!(
        "PASS arbitrary pass: 1280 exact pixels; split/instanced integer layout, independent and overlapping buffer strides, b7, t4/s4 volume wrapping W, triangle strip, retained bindings/null SRV and state-key invalidation"
    );
}

#[test]
fn authored_samplers_preserve_independent_vertex_and_pixel_slots() {
    crate::tests::initialize_runtime_tables();
    let program = content_reader_gui::pdxshader::program::read_with(b"stages.shader", &|p| {
        Some(if p == b"stages.shader" {
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
        content_reader_gui::pdxshader::Address::Wrap
    );
    assert_eq!(
        state.samplers[&binding(2, Stage::Pixel)].u,
        content_reader_gui::pdxshader::Address::Clamp
    );
}
