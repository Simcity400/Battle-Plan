use crate::{GameFileResolver, Result};
use snapshot::sha256::Sha256;

pub const SKY_SHADER: &str = "gfx/FX/sky.shader";
pub const SKY_VERTICES: [i16; 6] = [3, -1, -1, -1, -1, 3];

#[cfg(all(test, windows))]
mod tests {
    use super::*;
    unsafe fn draw_after_conflicting_rasterizer(
        sky: &mut NativeMapSky,
        device: *mut winapi::um::d3d11::ID3D11Device,
        context: *mut winapi::um::d3d11::ID3D11DeviceContext,
        expected: [u8; 4],
    ) {
        use crate::d3d11_device::{native_d3d11_hr, D3dPtr};
        use std::ptr;
        use winapi::shared::{dxgiformat::*, dxgitype::DXGI_SAMPLE_DESC};
        use winapi::um::d3d11::*;
        let ps = crate::d3d_compile_stage(
            "float4 main() : SV_Target { return float4(0.25, 0.5, 0.75, 1.0); }",
            crate::ShaderStage::Pixel,
            crate::shader_compile::ENGINE_COMPILE_FLAGS,
        )
        .unwrap();
        let mut pixel = ptr::null_mut();
        native_d3d11_hr(
            (*device).CreatePixelShader(ps.as_ptr().cast(), ps.len(), ptr::null_mut(), &mut pixel),
            "sky test pixel",
        )
        .unwrap();
        sky.pixel = D3dPtr::from_raw(pixel, "sky test pixel").unwrap();
        let color_desc = D3D11_TEXTURE2D_DESC {
            Width: 32,
            Height: 16,
            MipLevels: 1,
            ArraySize: 1,
            Format: DXGI_FORMAT_R8G8B8A8_UNORM,
            SampleDesc: DXGI_SAMPLE_DESC {
                Count: 1,
                Quality: 0,
            },
            Usage: D3D11_USAGE_DEFAULT,
            BindFlags: D3D11_BIND_RENDER_TARGET,
            CPUAccessFlags: 0,
            MiscFlags: 0,
        };
        let mut color = ptr::null_mut();
        native_d3d11_hr(
            (*device).CreateTexture2D(&color_desc, ptr::null(), &mut color),
            "sky test color",
        )
        .unwrap();
        let color = D3dPtr::from_raw(color, "sky test color").unwrap();
        let mut rtv = ptr::null_mut();
        native_d3d11_hr(
            (*device).CreateRenderTargetView(color.as_raw().cast(), ptr::null(), &mut rtv),
            "sky test RTV",
        )
        .unwrap();
        let rtv = D3dPtr::from_raw(rtv, "sky test RTV").unwrap();
        let depth_desc = D3D11_TEXTURE2D_DESC {
            Format: DXGI_FORMAT_D24_UNORM_S8_UINT,
            BindFlags: D3D11_BIND_DEPTH_STENCIL,
            ..color_desc
        };
        let mut depth = ptr::null_mut();
        native_d3d11_hr(
            (*device).CreateTexture2D(&depth_desc, ptr::null(), &mut depth),
            "sky test depth",
        )
        .unwrap();
        let depth = D3dPtr::from_raw(depth, "sky test depth").unwrap();
        let mut dsv = ptr::null_mut();
        native_d3d11_hr(
            (*device).CreateDepthStencilView(depth.as_raw().cast(), ptr::null(), &mut dsv),
            "sky test DSV",
        )
        .unwrap();
        let dsv = D3dPtr::from_raw(dsv, "sky test DSV").unwrap();
        (*context).ClearRenderTargetView(rtv.as_raw(), &[1.0, 0.0, 1.0, 0.25]);
        (*context).ClearDepthStencilView(
            dsv.as_raw(),
            D3D11_CLEAR_DEPTH | D3D11_CLEAR_STENCIL,
            1.0,
            0,
        );
        (*context).OMSetRenderTargets(1, &rtv.as_raw(), dsv.as_raw());
        (*context).RSSetViewports(
            1,
            &D3D11_VIEWPORT {
                TopLeftX: 0.0,
                TopLeftY: 0.0,
                Width: 32.0,
                Height: 16.0,
                MinDepth: 0.0,
                MaxDepth: 1.0,
            },
        );
        let mut hostile: D3D11_RASTERIZER_DESC = std::mem::zeroed();
        (*sky.rasterizer.as_raw()).GetDesc(&mut hostile);
        hostile.FillMode = D3D11_FILL_WIREFRAME;
        hostile.CullMode = D3D11_CULL_FRONT;
        hostile.ScissorEnable = 1;
        hostile.DepthBias = 10000;
        let mut prior = ptr::null_mut();
        native_d3d11_hr(
            (*device).CreateRasterizerState(&hostile, &mut prior),
            "sky test prior rasterizer",
        )
        .unwrap();
        let prior = D3dPtr::from_raw(prior, "sky test prior rasterizer").unwrap();
        (*context).RSSetState(prior.as_raw());
        (*context).RSSetScissorRects(
            1,
            &winapi::shared::windef::RECT {
                left: 0,
                top: 0,
                right: 1,
                bottom: 1,
            },
        );
        let gs = crate::d3d11_device::native_d3d11_compile_shader(
            "struct V { float4 p : SV_POSITION; }; [maxvertexcount(3)] void main(triangle V input[3], inout TriangleStream<V> stream) { }",
            "main", "gs_5_0",
        ).unwrap();
        let mut geometry = ptr::null_mut();
        native_d3d11_hr((*device).CreateGeometryShader((*gs.as_raw()).GetBufferPointer(),
            (*gs.as_raw()).GetBufferSize(), ptr::null_mut(), &mut geometry), "sky hostile GS").unwrap();
        let geometry = D3dPtr::from_raw(geometry, "sky hostile GS").unwrap();
        (*context).GSSetShader(geometry.as_raw(), ptr::null(), 0);
        sky.bind_and_draw(context);
        let mut bound_sampler = ptr::null_mut();
        (*context).PSGetSamplers(0, 1, &mut bound_sampler);
        assert_eq!(bound_sampler, sky.sampler.as_ref().map_or(ptr::null_mut(), |sampler| sampler.as_raw()));
        if !bound_sampler.is_null() { drop(D3dPtr::from_raw(bound_sampler, "sky bound sampler").unwrap()); }
        let staging_desc = D3D11_TEXTURE2D_DESC {
            Usage: D3D11_USAGE_STAGING,
            BindFlags: 0,
            CPUAccessFlags: D3D11_CPU_ACCESS_READ,
            ..color_desc
        };
        let mut staging = ptr::null_mut();
        native_d3d11_hr(
            (*device).CreateTexture2D(&staging_desc, ptr::null(), &mut staging),
            "sky test staging",
        )
        .unwrap();
        let staging = D3dPtr::from_raw(staging, "sky test staging").unwrap();
        (*context).CopyResource(staging.as_raw().cast(), color.as_raw().cast());
        let mut mapped: D3D11_MAPPED_SUBRESOURCE = std::mem::zeroed();
        native_d3d11_hr(
            (*context).Map(staging.as_raw().cast(), 0, D3D11_MAP_READ, 0, &mut mapped),
            "sky test readback",
        )
        .unwrap();
        for y in 0..16 {
            let row = std::slice::from_raw_parts(
                (mapped.pData as *const u8).add(y * mapped.RowPitch as usize),
                32 * 4,
            );
            assert!(
                row.chunks_exact(4)
                    .all(|pixel| pixel == expected),
                "sky row {y} inherited conflicting state"
            );
        }
        (*context).Unmap(staging.as_raw().cast(), 0);
    }

    #[test]
    fn sky_authored_red_write_mask_preserves_other_target_channels() {
        use crate::d3d11_device::{native_d3d11_hr, D3dPtr};
        use std::{fs, ptr};
        use winapi::um::{d3d11::*, d3dcommon::*};
        let root = crate::test_support::temp_test_dir("sky_red_write_mask");
        let fx = root.join("gfx/FX");
        fs::create_dir_all(&fx).unwrap();
        fs::write(fx.join("sky.shader"), r#"
VertexStruct VS_INPUT { int2 position : POSITION; }
VertexStruct VS_OUTPUT { float4 position : SV_POSITION; }
VertexShader = {
    MainCode VS [[ VS_OUTPUT main(VS_INPUT i) { VS_OUTPUT o; o.position = float4(i.position, 1, 1); return o; } ]]
}
PixelShader = {
    Samplers = {
        ReflectionCubeMap = { Index = 0 Type = "Cube" MagFilter = "Point" MinFilter = "Point" MipFilter = "Point" AddressU = "Clamp" AddressV = "Clamp" }
    }
    MainCode PS [[ float4 main() : SV_Target { return ReflectionCubeMap_Texture.Sample(ReflectionCubeMap_Sampler, float3(1, 0, 0)); } ]]
}
BlendState BlendState { BlendEnable = no WriteMask = "ALL" }
BlendState SelectedRed { BlendEnable = no WriteMask = "RED" }
Effect sky { VertexShader = "VS" PixelShader = "PS" BlendState = "SelectedRed" }
"#).unwrap();
        fs::write(fx.join("defines_hlsl_dx11.fxh"), r#"
struct STextureSamplerCube { TextureCube cube_texture; SamplerState cube_sampler; };
STextureSamplerCube CreateSamplerCube(TextureCube t, SamplerState s) { STextureSamplerCube r = { t, s }; return r; }
"#).unwrap();
        let source = GameFileResolver::vanilla(&root).unwrap();
        let paths = [SKY_SHADER.to_ascii_lowercase(), crate::shader_compile::DX11_PLATFORM_HEADER.to_ascii_lowercase()].into_iter().collect();
        let (resolver, _) = crate::test_support::admitted_vanilla_resolver(&source, &paths, "sky-red-mask");
        unsafe {
            let mut device = ptr::null_mut();
            let mut context = ptr::null_mut();
            native_d3d11_hr(D3D11CreateDevice(ptr::null_mut(), D3D_DRIVER_TYPE_WARP, ptr::null_mut(), 0,
                &D3D_FEATURE_LEVEL_11_0, 1, D3D11_SDK_VERSION, &mut device, ptr::null_mut(), &mut context), "sky red WARP").unwrap();
            let device = D3dPtr::from_raw(device, "sky red device").unwrap();
            let context = D3dPtr::from_raw(context, "sky red context").unwrap();
            let mut sky = NativeMapSky::create(device.as_raw(), &resolver, 5632, 2048, &"a".repeat(64)).unwrap();
            let mut sampler: D3D11_SAMPLER_DESC = std::mem::zeroed();
            (*sky.sampler.as_ref().unwrap().as_raw()).GetDesc(&mut sampler);
            assert_eq!(sampler.Filter, D3D11_FILTER_MIN_MAG_MIP_POINT);
            assert_eq!(sampler.AddressU, D3D11_TEXTURE_ADDRESS_CLAMP);
            let linear_root = crate::test_support::temp_test_dir("sky_linear_sampler");
            let linear_fx = linear_root.join("gfx/FX");
            fs::create_dir_all(&linear_fx).unwrap();
            fs::write(linear_fx.join("sky.shader"), fs::read_to_string(fx.join("sky.shader")).unwrap().replace("\"Point\"", "\"Linear\"")).unwrap();
            fs::copy(fx.join("defines_hlsl_dx11.fxh"), linear_fx.join("defines_hlsl_dx11.fxh")).unwrap();
            let linear_source = GameFileResolver::vanilla(&linear_root).unwrap();
            let (linear_resolver, _) = crate::test_support::admitted_vanilla_resolver(&linear_source, &paths, "sky-linear");
            let linear = NativeMapSky::create(device.as_raw(), &linear_resolver, 5632, 2048, &"a".repeat(64)).unwrap();
            assert_eq!(sky.program_sha256, linear.program_sha256);
            assert_eq!(sky.blend_sha256, linear.blend_sha256);
            assert_ne!(sky.sampler_sha256, linear.sampler_sha256);
            assert_ne!(sky.draw_digest(&"b".repeat(64), &"c".repeat(64), &"d".repeat(64)).unwrap(),
                linear.draw_digest(&"b".repeat(64), &"c".repeat(64), &"d".repeat(64)).unwrap());
            drop(linear); drop(linear_resolver); drop(linear_source); fs::remove_dir_all(linear_root).unwrap();
            let mut desc: D3D11_BLEND_DESC = std::mem::zeroed();
            (*sky.blend.as_raw()).GetDesc(&mut desc);
            assert_eq!(desc.RenderTarget[0].BlendEnable, 0);
            assert_eq!(desc.RenderTarget[0].RenderTargetWriteMask, D3D11_COLOR_WRITE_ENABLE_RED as u8);
            draw_after_conflicting_rasterizer(&mut sky, device.as_raw(), context.as_raw(), [64, 0, 255, 64]);
            let rows = [("BlendEnable", "yes"), ("SourceBlend", "SRC_ALPHA"),
                ("DestBlend", "INV_SRC_ALPHA"), ("BlendOp", "REV_SUBTRACT")]
                .map(|(k, v)| (k.to_owned(), v.to_owned()));
            let blend = crate::xmap12_d3d_handoff::create_blend_state(device.as_raw(), &rows).unwrap();
            (*blend.as_raw()).GetDesc(&mut desc);
            let target = desc.RenderTarget[0];
            assert_eq!((target.SrcBlendAlpha, target.DestBlendAlpha, target.BlendOpAlpha),
                (D3D11_BLEND_SRC_ALPHA, D3D11_BLEND_INV_SRC_ALPHA, D3D11_BLEND_OP_REV_SUBTRACT));
            let mut explicit = rows.to_vec();
            explicit.push(("SourceBlendAlpha".into(), "DEST_ALPHA".into()));
            let blend = crate::xmap12_d3d_handoff::create_blend_state(device.as_raw(), &explicit).unwrap();
            (*blend.as_raw()).GetDesc(&mut desc);
            assert_eq!((desc.RenderTarget[0].SrcBlendAlpha, desc.RenderTarget[0].DestBlendAlpha,
                desc.RenderTarget[0].BlendOpAlpha),
                (D3D11_BLEND_DEST_ALPHA, D3D11_BLEND_ZERO, D3D11_BLEND_OP_ADD));
            let mut files = crate::shader_compile::effect_translation_unit_files(&resolver, SKY_SHADER).unwrap();
            for (_, file) in &mut files { file.blend_states.clear(); }
            let (blend, _) = create_sky_blend(device.as_raw(), &files).unwrap();
            (*blend.as_raw()).GetDesc(&mut desc);
            assert_eq!((desc.RenderTarget[0].BlendEnable, desc.RenderTarget[0].RenderTargetWriteMask,
                desc.RenderTarget[0].SrcBlendAlpha, desc.RenderTarget[0].DestBlendAlpha),
                (0, 15, D3D11_BLEND_ONE, D3D11_BLEND_ZERO));
        }
        drop(resolver);
        drop(source);
        fs::remove_dir_all(&root).unwrap();
    }

    #[test]
    fn sky_constant_effect_creates_without_sampler_and_clears_prior_binding() {
        use crate::d3d11_device::{native_d3d11_hr, D3dPtr};
        use std::{fs, ptr};
        use winapi::um::{d3d11::*, d3dcommon::*};
        let root = crate::test_support::temp_test_dir("sky_without_sampler");
        let fx = root.join("gfx/FX");
        fs::create_dir_all(&fx).unwrap();
        fs::write(fx.join("sky.shader"), r#"
VertexStruct Input { int2 p : POSITION; }
VertexShader = { MainCode VS [[ float4 main(Input i) : SV_POSITION { return float4(i.p, 1, 1); } ]] }
PixelShader = { MainCode PS [[ float4 main() : SV_Target { return float4(0.25, 0.5, 0.75, 1); } ]] }
Effect sky { VertexShader = "VS" PixelShader = "PS" }
"#).unwrap();
        fs::write(fx.join("defines_hlsl_dx11.fxh"), "").unwrap();
        let source = GameFileResolver::vanilla(&root).unwrap();
        let paths = [SKY_SHADER.to_ascii_lowercase(), crate::shader_compile::DX11_PLATFORM_HEADER.to_ascii_lowercase()].into_iter().collect();
        let (resolver, _) = crate::test_support::admitted_vanilla_resolver(&source, &paths, "sky-no-sampler");
        unsafe {
            let mut device = ptr::null_mut(); let mut context = ptr::null_mut();
            native_d3d11_hr(D3D11CreateDevice(ptr::null_mut(), D3D_DRIVER_TYPE_WARP, ptr::null_mut(), 0,
                &D3D_FEATURE_LEVEL_11_0, 1, D3D11_SDK_VERSION, &mut device, ptr::null_mut(), &mut context), "sky constant WARP").unwrap();
            let device = D3dPtr::from_raw(device, "sky constant device").unwrap();
            let context_owner = D3dPtr::from_raw(context, "sky constant context").unwrap();
            let mut sky = NativeMapSky::create(device.as_raw(), &resolver, 5632, 2048, &"a".repeat(64)).unwrap();
            assert!(sky.sampler.is_none());
            assert!(sky.sampler_sha256.is_none());
            let descriptor = D3D11_SAMPLER_DESC { Filter: D3D11_FILTER_MIN_MAG_MIP_POINT,
                AddressU: D3D11_TEXTURE_ADDRESS_WRAP, AddressV: D3D11_TEXTURE_ADDRESS_WRAP,
                AddressW: D3D11_TEXTURE_ADDRESS_WRAP, MipLODBias: 0.0, MaxAnisotropy: 1,
                ComparisonFunc: D3D11_COMPARISON_NEVER, BorderColor: [0.0; 4], MinLOD: 0.0, MaxLOD: f32::MAX };
            let mut previous = ptr::null_mut();
            native_d3d11_hr((*device.as_raw()).CreateSamplerState(&descriptor, &mut previous), "sky previous sampler").unwrap();
            let previous = D3dPtr::from_raw(previous, "sky previous sampler").unwrap();
            (*context).PSSetSamplers(0, 1, &previous.as_raw());
            draw_after_conflicting_rasterizer(&mut sky, device.as_raw(), context, [64, 128, 191, 255]);
            drop(context_owner);
        }
        drop(resolver); drop(source); fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn sky_reflected_inputs_require_exact_bound_providers() {
        let vertex = "cbuffer Scene : register(b0) { float4 offset; }; float4 main(int2 p : POSITION) : SV_POSITION { return float4(p, 1, 1) + offset; }";
        let pixel = "cbuffer Scene : register(b0) { float4 direction; }; TextureCube cube : register(t0); SamplerState sample_state : register(s0); float4 main() : SV_Target { return cube.Sample(sample_state, direction.xyz); }";
        let check = |stage, source: &str| {
            let bytes = crate::d3d_compile_stage(source, stage, crate::shader_compile::ENGINE_COMPILE_FLAGS).unwrap();
            require_sky_shader_providers(stage, &bytes)
        };
        check(crate::ShaderStage::Vertex, vertex).unwrap();
        check(crate::ShaderStage::Pixel, pixel).unwrap();
        for (stage, source) in [
            (crate::ShaderStage::Vertex, vertex.replace("b0", "b1")),
            (crate::ShaderStage::Pixel, pixel.replace("b0", "b1")),
            (crate::ShaderStage::Pixel, pixel.replace("t0", "t1")),
            (crate::ShaderStage::Pixel, pixel.replace("s0", "s1")),
            (crate::ShaderStage::Pixel, pixel.replace("TextureCube", "Texture2D").replace("direction.xyz", "direction.xy")),
            (crate::ShaderStage::Vertex, vertex.replace("float4 offset;", "float4 padding[26]; float4 offset;")),
            (crate::ShaderStage::Vertex, "Texture2D tex : register(t0); float4 main(int2 p : POSITION) : SV_POSITION { return tex.Load(int3(p, 0)); }".into()),
        ] {
            assert!(check(stage, &source).unwrap_err().to_string().contains("sky-reflected-provider-unavailable"));
        }
    }

    #[test]
    #[ignore = "requires an authenticated the game content install"]
    fn sky_authored_program_creates_native_far_depth_triangle() {
        use crate::d3d11_device::{native_d3d11_hr, D3dPtr};
        use std::ptr;
        use winapi::um::{d3d11::*, d3dcommon::*};
        let resolver =
            GameFileResolver::vanilla(replay_test_gates::require_vanilla_game_root()).unwrap();
        let paths = crate::shader_compile::effect_translation_unit_files(&resolver, SKY_SHADER)
            .unwrap()
            .into_iter()
            .map(|row| row.0.to_ascii_lowercase())
            .chain([crate::shader_compile::DX11_PLATFORM_HEADER.to_ascii_lowercase()])
            .collect();
        let (resolver, _) =
            crate::test_support::admitted_vanilla_resolver(&resolver, &paths, "sky-native-test");
        unsafe {
            let mut device = ptr::null_mut();
            let mut context = ptr::null_mut();
            native_d3d11_hr(
                D3D11CreateDevice(
                    ptr::null_mut(),
                    D3D_DRIVER_TYPE_WARP,
                    ptr::null_mut(),
                    0,
                    &D3D_FEATURE_LEVEL_11_0,
                    1,
                    D3D11_SDK_VERSION,
                    &mut device,
                    ptr::null_mut(),
                    &mut context,
                ),
                "sky test WARP",
            )
            .unwrap();
            let device = D3dPtr::from_raw(device, "sky test device").unwrap();
            let _context = D3dPtr::from_raw(context, "sky test context").unwrap();
            let mut sky =
                NativeMapSky::create(device.as_raw(), &resolver, 5632, 2048, &"a".repeat(64))
                    .unwrap();
            let mut depth: D3D11_DEPTH_STENCIL_DESC = std::mem::zeroed();
            (*sky.depth.as_raw()).GetDesc(&mut depth);
            assert_eq!(
                (
                    depth.DepthEnable,
                    depth.DepthWriteMask,
                    depth.DepthFunc,
                    depth.StencilEnable
                ),
                (
                    1,
                    D3D11_DEPTH_WRITE_MASK_ZERO,
                    D3D11_COMPARISON_LESS_EQUAL,
                    0
                )
            );
            let mut buffer: D3D11_BUFFER_DESC = std::mem::zeroed();
            (*sky.vertices.as_raw()).GetDesc(&mut buffer);
            assert_eq!(
                (buffer.ByteWidth, buffer.BindFlags),
                (12, D3D11_BIND_VERTEX_BUFFER)
            );
            let mut sampler: D3D11_SAMPLER_DESC = std::mem::zeroed();
            (*sky.sampler.as_ref().unwrap().as_raw()).GetDesc(&mut sampler);
            assert_eq!(sampler.Filter, D3D11_FILTER_MIN_MAG_MIP_LINEAR);
            assert_eq!(sampler.AddressU, D3D11_TEXTURE_ADDRESS_CLAMP);
            draw_after_conflicting_rasterizer(&mut sky, device.as_raw(), context, [64, 128, 191, 255]);
        }
    }
}

#[cfg(windows)]
pub struct NativeMapSky {
    source_identity: String,
    rasterizer: crate::d3d11_device::D3dPtr<winapi::um::d3d11::ID3D11RasterizerState>,
    vertex: crate::d3d11_device::D3dPtr<winapi::um::d3d11::ID3D11VertexShader>,
    pixel: crate::d3d11_device::D3dPtr<winapi::um::d3d11::ID3D11PixelShader>,
    layout: crate::d3d11_device::D3dPtr<winapi::um::d3d11::ID3D11InputLayout>,
    vertices: crate::d3d11_device::D3dPtr<winapi::um::d3d11::ID3D11Buffer>,
    depth: crate::d3d11_device::D3dPtr<winapi::um::d3d11::ID3D11DepthStencilState>,
    sampler: Option<crate::d3d11_device::D3dPtr<winapi::um::d3d11::ID3D11SamplerState>>,
    sampler_sha256: Option<String>,
    blend: crate::d3d11_device::D3dPtr<winapi::um::d3d11::ID3D11BlendState>,
    blend_sha256: String,
    program_sha256: String,
}

#[cfg(windows)]
impl NativeMapSky {
    pub fn source_identity(&self) -> &str {
        &self.source_identity
    }

    pub unsafe fn create(
        device: *mut winapi::um::d3d11::ID3D11Device,
        resolver: &GameFileResolver,
        map_width: u32,
        map_height: u32,
        source_identity: &str,
    ) -> Result<Self> {
        use crate::d3d11_device::{native_d3d11_hr, D3dPtr};
        use std::ptr;
        use winapi::um::d3d11::*;
        if device.is_null() || source_identity.len() != 64 || !resolver.has_content_admission() {
            return Err("sky received no device or authenticated source identity".into());
        }
        let defines = crate::engine_map_shader_defines(
            crate::derive_a729_engine_map_shader_constants(map_width, map_height, false, 0)?,
        );
        let files = crate::shader_compile::effect_translation_unit_files(resolver, SKY_SHADER)?;
        let (blend, blend_sha256) = create_sky_blend(device, &files)?;
        let compile = |stage| -> Result<Vec<u8>> {
            let hlsl = crate::shader_compile::build_stage_hlsl_with_defines(
                resolver, SKY_SHADER, "sky", stage, &defines,
            )?;
            crate::shader_compile::d3d_compile_stage(
                &hlsl,
                stage,
                crate::shader_compile::ENGINE_COMPILE_FLAGS,
            )
        };
        let vs = compile(crate::ShaderStage::Vertex)?;
        let ps = compile(crate::ShaderStage::Pixel)?;
        require_sky_shader_providers(crate::ShaderStage::Vertex, &vs)?;
        require_sky_shader_providers(crate::ShaderStage::Pixel, &ps)?;
        let mut vertex = ptr::null_mut();
        native_d3d11_hr(
            (*device).CreateVertexShader(
                vs.as_ptr() as *const _,
                vs.len(),
                ptr::null_mut(),
                &mut vertex,
            ),
            "CreateVertexShader(sky)",
        )?;
        let vertex = D3dPtr::from_raw(vertex, "sky vertex shader")?;
        let mut pixel = ptr::null_mut();
        native_d3d11_hr(
            (*device).CreatePixelShader(
                ps.as_ptr() as *const _,
                ps.len(),
                ptr::null_mut(),
                &mut pixel,
            ),
            "CreatePixelShader(sky)",
        )?;
        let pixel = D3dPtr::from_raw(pixel, "sky pixel shader")?;
        let input = D3D11_INPUT_ELEMENT_DESC {
            SemanticName: b"POSITION\0".as_ptr() as *const _,
            SemanticIndex: 0,
            Format: winapi::shared::dxgiformat::DXGI_FORMAT_R16G16_SINT,
            InputSlot: 0,
            AlignedByteOffset: 0,
            InputSlotClass: D3D11_INPUT_PER_VERTEX_DATA,
            InstanceDataStepRate: 0,
        };
        let mut layout = ptr::null_mut();
        native_d3d11_hr(
            (*device).CreateInputLayout(&input, 1, vs.as_ptr() as *const _, vs.len(), &mut layout),
            "CreateInputLayout(sky)",
        )?;
        let layout = D3dPtr::from_raw(layout, "sky input layout")?;
        let desc = D3D11_BUFFER_DESC {
            ByteWidth: 12,
            Usage: D3D11_USAGE_IMMUTABLE,
            BindFlags: D3D11_BIND_VERTEX_BUFFER,
            CPUAccessFlags: 0,
            MiscFlags: 0,
            StructureByteStride: 0,
        };
        let data = D3D11_SUBRESOURCE_DATA {
            pSysMem: SKY_VERTICES.as_ptr() as *const _,
            SysMemPitch: 0,
            SysMemSlicePitch: 0,
        };
        let mut vertices = ptr::null_mut();
        native_d3d11_hr(
            (*device).CreateBuffer(&desc, &data, &mut vertices),
            "CreateBuffer(sky)",
        )?;
        let vertices = D3dPtr::from_raw(vertices, "sky vertices")?;
        let stencil = D3D11_DEPTH_STENCILOP_DESC {
            StencilFailOp: D3D11_STENCIL_OP_KEEP,
            StencilDepthFailOp: D3D11_STENCIL_OP_KEEP,
            StencilPassOp: D3D11_STENCIL_OP_KEEP,
            StencilFunc: D3D11_COMPARISON_ALWAYS,
        };
        let depth_desc = D3D11_DEPTH_STENCIL_DESC {
            DepthEnable: 1,
            DepthWriteMask: D3D11_DEPTH_WRITE_MASK_ZERO,
            DepthFunc: D3D11_COMPARISON_LESS_EQUAL,
            StencilEnable: 0,
            StencilReadMask: 255,
            StencilWriteMask: 255,
            FrontFace: stencil,
            BackFace: stencil,
        };
        let mut depth = ptr::null_mut();
        native_d3d11_hr(
            (*device).CreateDepthStencilState(&depth_desc, &mut depth),
            "CreateDepthStencilState(sky)",
        )?;
        let depth = D3dPtr::from_raw(depth, "sky depth")?;
        let (sampler, sampler_sha256) = if crate::parse_dxbc_reflection(&ps)?.resources.iter().any(|resource| resource.resource_type == 3) {
            let (sampler, identity) = crate::xmap12_d3d_handoff::create_effect_sampler_on_device(device, resolver, SKY_SHADER, 0)?;
            (Some(sampler), Some(identity))
        } else { (None, None) };
        Ok(Self {
            source_identity: source_identity.into(),
            rasterizer: crate::xmap16_supply_arrow_handoff::create_supply_arrow_rasterizer_state(
                device,
            )?,
            vertex,
            pixel,
            layout,
            vertices,
            depth,
            sampler,
            sampler_sha256,
            blend,
            blend_sha256,
            program_sha256: Sha256::hex(&[vs, ps].concat()),
        })
    }

    pub unsafe fn draw<S: crate::NativeXmap16SceneConstantBufferView>(
        &self,
        context: *mut winapi::um::d3d11::ID3D11DeviceContext,
        target: &dyn crate::NativeXmap16MainSceneTargetView,
        target_sha256: &str,
        scene: &S,
        cube: &crate::NativeXmap07ResourceView<'_>,
    ) -> Result<String> {
        use std::ptr;
        if context.is_null()
            || target.color_rtv().is_null()
            || target.depth_dsv().is_null()
            || scene.raw_buffer().is_null()
            || cube.as_raw().is_null()
            || cube.binding.symbol != "ReflectionCubeMap"
            || cube.binding.resource_type != "TextureCube"
            || target_sha256.len() != 64
            || scene.payload_sha256().len() != 64
        {
            return Err("sky draw lacks its scene, target, or reflection cube".into());
        }
        let desc = target.desc();
        desc.validate()?;
        let mut rtv = target.color_rtv();
        (*context).OMSetRenderTargets(1, &mut rtv, target.depth_dsv());
        (*context).RSSetViewports(
            1,
            &winapi::um::d3d11::D3D11_VIEWPORT {
                TopLeftX: 0.0,
                TopLeftY: 0.0,
                Width: desc.color_width as f32,
                Height: desc.color_height as f32,
                MinDepth: 0.0,
                MaxDepth: 1.0,
            },
        );
        let mut cb = scene.raw_buffer();
        (*context).VSSetConstantBuffers(0, 1, &mut cb);
        (*context).PSSetConstantBuffers(0, 1, &mut cb);
        let mut srv = cube.as_raw();
        (*context).PSSetShaderResources(0, 1, &mut srv);
        self.bind_and_draw(context);
        (*context).PSSetShaderResources(0, 1, &mut ptr::null_mut());
        self.draw_digest(target_sha256, scene.payload_sha256(), &cube.binding.resource_sha256)
    }

    fn draw_digest(&self, target_sha256: &str, scene_sha256: &str, cube_sha256: &str) -> Result<String> {
        Ok(Sha256::hex(&serde_json::to_vec(&(
            self.source_identity.as_str(),
            self.program_sha256.as_str(),
            self.blend_sha256.as_str(),
            self.sampler_sha256.as_deref(),
            target_sha256,
            scene_sha256,
            cube_sha256,
            3u32,
        ))?))
    }
    unsafe fn bind_and_draw(&self, context: *mut winapi::um::d3d11::ID3D11DeviceContext) {
        use std::ptr;
        (*context).RSSetState(self.rasterizer.as_raw());
        (*context).OMSetDepthStencilState(self.depth.as_raw(), 0);
        (*context).OMSetBlendState(self.blend.as_raw(), &[0.0; 4], u32::MAX);
        let sampler = self.sampler.as_ref().map_or(ptr::null_mut(), |sampler| sampler.as_raw());
        (*context).PSSetSamplers(0, 1, &sampler);
        (*context).VSSetShader(self.vertex.as_raw(), ptr::null_mut(), 0);
        (*context).PSSetShader(self.pixel.as_raw(), ptr::null_mut(), 0);
        (*context).GSSetShader(ptr::null_mut(), ptr::null(), 0);
        (*context).HSSetShader(ptr::null_mut(), ptr::null(), 0);
        (*context).DSSetShader(ptr::null_mut(), ptr::null(), 0);
        (*context).SOSetTargets(0, ptr::null(), ptr::null());
        (*context).IASetInputLayout(self.layout.as_raw());
        (*context)
            .IASetPrimitiveTopology(winapi::um::d3dcommon::D3D11_PRIMITIVE_TOPOLOGY_TRIANGLELIST);
        let mut vertices = self.vertices.as_raw();
        (*context).IASetVertexBuffers(0, 1, &mut vertices, &4, &0);
        (*context).Draw(3, 0);
    }
}

#[cfg(windows)]
fn require_sky_shader_providers(stage: crate::ShaderStage, bytecode: &[u8]) -> Result<()> {
    let reflection = crate::parse_dxbc_reflection(bytecode)?;
    for resource in &reflection.resources {
        let supplied = resource.bind_count == 1 && resource.bind_point == 0
            && match resource.resource_type {
                0 => reflection.cbuffers.iter().any(|buffer|
                    buffer.name == resource.name && buffer.size <= 416),
                2 => stage == crate::ShaderStage::Pixel && resource.dimension == 9,
                3 => stage == crate::ShaderStage::Pixel,
                _ => false,
            };
        if !supplied {
            return Err(format!(
                "sky-reflected-provider-unavailable: {stage:?} {} type {} register {} count {} dimension {}",
                resource.name, resource.resource_type, resource.bind_point, resource.bind_count,
                resource.dimension,
            ).into());
        }
    }
    Ok(())
}

#[cfg(windows)]
unsafe fn create_sky_blend(
    device: *mut winapi::um::d3d11::ID3D11Device,
    files: &[(String, crate::EffectFile)],
) -> Result<(crate::d3d11_device::D3dPtr<winapi::um::d3d11::ID3D11BlendState>, String)> {
    let root = &files.last().ok_or("sky translation unit is empty")?.1;
    if root.effect_count("sky") != 1 {
        return Err("sky effect selection is absent or ambiguous".into());
    }
    let effect = root.effect("sky").ok_or("sky effect is absent")?;
    let name = effect.blend_state.as_deref().unwrap_or("BlendState");
    let rows = files.iter().rev().find_map(|(_, file)| file.blend_states.get(name))
        .map(Vec::as_slice).unwrap_or(&[]);
    let identity = Sha256::hex(&serde_json::to_vec(&(name, rows))?);
    Ok((crate::xmap12_d3d_handoff::create_blend_state(device, rows)?, identity))
}

