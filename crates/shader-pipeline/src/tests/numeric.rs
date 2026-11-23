use super::*;

#[test]
#[ignore = "requires a shader compiler and GPU"]
fn numeric_operations_preserve_native_nonfinite_rules() {
    initialize_runtime_tables();
    let _serial = GPU_TESTS.lock().unwrap_or_else(|error| error.into_inner());
    let (device, queue) = device();
    let arena = gpu::Arena::new(&device);
    let nan = f32::from_bits(0x7fc0_1234);
    let operands = [
        nan,
        0.8,
        -0.2,
        nan,
        0.25,
        nan,
        0.0,
        nan,
        0.75,
        0.9,
        nan,
        1.0,
        f32::INFINITY,
        f32::NEG_INFINITY,
        nan,
        1.0,
    ];
    for (body, expected) in [
        ("min(A,B)", [0.25, 0.8, -0.2, nan]),
        ("max(A,B)", [0.25, 0.8, 0.0, nan]),
        ("clamp(A,B,C)", [0.25, 0.8, 0.0, 1.0]),
        ("saturate(A)", [0.0, 0.8, 0.0, 0.0]),
        ("float4(A < B)", [0.0, 0.0, 1.0, 0.0]),
        ("float4(A <= B)", [0.0, 0.0, 1.0, 0.0]),
        ("float4(A > B)", [0.0; 4]),
        ("float4(A >= B)", [0.0; 4]),
        ("float4(A == B)", [0.0; 4]),
        ("float4(A != B)", [1.0; 4]),
        ("float4(A == A)", [0.0, 1.0, 1.0, 0.0]),
        ("float4(A != A)", [1.0, 0.0, 0.0, 1.0]),
        ("float4(isnan(A))", [1.0, 0.0, 0.0, 1.0]),
        ("float4(isinf(A))", [0.0; 4]),
        ("float4(isinf(D))", [1.0, 1.0, 0.0, 0.0]),
        ("float4(isnan(D))", [0.0, 0.0, 1.0, 0.0]),
        (
            "float4(A.x < B.x || A.x >= B.x, A.y < B.y || A.y >= B.y, A.z < B.z || A.z >= B.z, A.w < B.w || A.w >= B.w)",
            [0.0, 0.0, 1.0, 0.0],
        ),
        ("step(A,B)", [0.0, 0.0, 1.0, 0.0]),
        ("smoothstep(A,B,C)", [0.0; 4]),
        (
            "smoothstep(float4(0,0,1,1),float4(1,1,0,1),float4(0.5,0.25,0.25,1))",
            [0.5, 0.15625, 0.84375, 0.0],
        ),
        (
            "float4(min(A.x,B.x),max(A.x,B.x),saturate(A.x),clamp(A.x,B.x,C.x))",
            [0.25, 0.25, 0.0, 0.25],
        ),
    ] {
        let source = format!(
            r#"
ConstantBuffer(0,0) {{ float4 A; float4 B; float4 C; float4 D; }};
VertexStruct I {{ float3 position : POSITION; }};
VertexStruct O {{ float4 position : SV_POSITION; }};
VertexShader = {{ MainCode V [[ O main(I v) {{ O o; o.position=float4(v.position,1); return o; }} ]] }}
PixelShader = {{ MainCode P [[ float4 main(O v) : SV_TARGET {{ return {body}; }} ]] }}
Effect Numeric {{ VertexShader = V PixelShader = P }}
"#
        );
        let program = content_reader_gui::pdxshader::program::read_with(b"numeric.shader", &|path| {
            Some(if path == b"numeric.shader" {
                source.as_bytes().to_vec()
            } else {
                Vec::new()
            })
        })
        .unwrap();
        let compiled = Compiler::discover()
            .unwrap()
            .compile(&program, &program.techniques[0], &[])
            .unwrap();
        let state = gpu::State::authored(&program, &program.techniques[0]).unwrap();
        let effect = library::Effect::from_compiled(
            std::sync::Arc::new(program),
            Vec::new(),
            vec![Ok(std::sync::Arc::new(compiled))],
        );
        let selection = effect.select("Numeric").unwrap().unwrap();
        let pipeline = gpu::Cache::default()
            .get(
                &device,
                &selection,
                &state,
                wgpu::TextureFormat::Rgba32Float,
                1,
                wgpu::PrimitiveTopology::TriangleList,
            )
            .unwrap();
        let vertices: [[f32; 5]; 3] = [
            [-1., -1., 0., 0., 0.],
            [3., -1., 0., 0., 0.],
            [-1., 3., 0., 0., 0.],
        ];
        let draw = pipeline
            .prepare(
                &arena,
                &BTreeMap::from([(1, bytemuck::cast_slice(&operands).to_vec())]),
                &BTreeMap::new(),
                &vertices::Vertices {
                    declaration: vertices::Declaration::Sprite,
                    bytes: bytemuck::cast_slice(&vertices),
                    stride: 20,
                    count: 3,
                },
            )
            .unwrap();
        let pixels = render_format(
            &device,
            &queue,
            &arena,
            wgpu::TextureFormat::Rgba32Float,
            |pass| pipeline.draw(pass, &draw),
        );
        for pixel in pixels.chunks_exact(16) {
            for (bytes, expected) in pixel.chunks_exact(4).zip(expected) {
                let value = f32::from_le_bytes(bytes.try_into().unwrap());
                assert!(
                    if expected.is_nan() {
                        value.is_nan()
                    } else {
                        value == expected
                    },
                    "{body}: {value} != {expected}"
                );
            }
        }
    }
}
