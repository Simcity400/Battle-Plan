//! Legacy BORD2 `--borders-from-files` diagnostic GPU pass.
//!
//! Renders the [`crate::border_world_synthesis`] mesh (all six classes:
//! province/state/country/coast/sea-region/impassable) over the production-map
//! terrain target with the shipped `gfx/FX/border.shader` `Effect border`
//! compiled byte-exact through the OFF-3 machinery, per-class `BorderDiffuse`
//! textures resolved from the shipped `map/terrain/border_*_0.dds` files, and
//! the camera-height fade from [`crate::border_law::border_transparency`].
//!
//! Everything here is files-first and fail-closed under the diagnostic flag.
//! The shipping route is the unconditional XMAP06 adapter and
//! [`crate::NativeXmap06ThinBorderHandoff`]; `replay_app` rejects this legacy
//! switch, so this module cannot replace or suppress either product phase.
//!
//! # Design decisions (each grounded in a shipped file or captured evidence)
//!
//! BORD5 (session_BORDERZOOM_3) captured a GENUINE border draw end-to-end;
//! every decision below now cites either that capture or shipped files.
//!
//! * **Vertex layout** — one interleaved vertex buffer using the captured
//!   stride 16 (`float3 position` at offset 0 + `R16G16_SINT uv` at offset 12),
//!   matching the shader's `VS_INPUT_BORDER` integer semantics.
//! * **Per-class draws** — the mesh's indices are reordered once into
//!   contiguous per-material runs ([`crate::BorderWorldMesh::class_index_partition`]);
//!   geometry uploads ONCE and each frame re-issues one `DrawIndexed` per class
//!   with only the tiny CB2 rewritten, so the fade can rescale every class per
//!   frame without touching the big buffers.
//! * **CB2** — captured 32-byte allocation containing `float4 vSelectionColor; float2
//!   vTime_Transparency; }` (border.shader:73-77). `vSelectionColor = 0` (no
//!   selection pulse in the replay), `vTime_Transparency.x` = map clock (the
//!   pulse time base), `.y` = the per-class transparency from the fade law.
//! * **Scene CB0/CB1** — the border VS reads `ViewProjectionMatrix` and
//!   `vCamLookAtDir`, and the PS reads distance-fog/day-night/`ShadowFadeFactor`
//!   constants; all live in the shared scene CB0 declared by the included
//!   `standardfuncsgfx.fxh`. The pass BORROWS the frame's terrain-draw CB0/CB1
//!   buffer pointers, so camera updates (including per-wrap-copy shifts) written
//!   by `native_d3d11_update_prepared_map_pass_scene` are seen automatically.
//! * **SRVs** — the shader DECLARES four samplers (BorderDiffuse 0 /
//!   SnowMudData 1 / IntelMap 2 / ShadowMap 3), but the compiled PS blob keeps
//!   exactly TWO in its DXBC reflection: `BorderDiffuse` (t0/s0) and
//!   `ShadowMap` (t3/s3) — verified empirically via `parse_dxbc_reflection` on
//!   the blob this module compiles (FXC strips the commented-out SnowMudData
//!   and the unreferenced IntelMap). BorderDiffuse binds the class's shipped
//!   DDS with sRGB sampling — the CAPTURED state-border SRV is
//!   `BC3_UNORM_SRGB` (78) and its payload is byte-identical to the shipped
//!   `map/terrain/border_state_0.dds` (all 7 mips), so the shipped per-class
//!   DDS is PROVEN to be the bound texture; its decoded cross-section law
//!   lives in `border_texture_law.rs`.
//!   Slots 1/2 still get the inert neutral
//!   texel defensively (a no-op for this blob). ShadowMap doubles as the
//!   shadow AND FOW
//!   input (`GetShadowScaled` reads `.r`, `ApplyFOW` reads `.g/.b`;
//!   shadow.fxh:44-53, fow.fxh:85-89) and is sampled as a PLAIN texture — the
//!   shipped DX11 header maps `sampler2DShadow` to a regular sampler and
//!   `tex2Dproj` to `SampleLevel(uv.xy/uv.w, 0)` (defines_hlsl_dx11.fxh:47,54).
//!   The neutral texel `(1, 1, 0, 1)` therefore yields shadow term 1.0 and FoW
//!   alpha 0. That is an intentional limitation of this legacy diagnostic.
//!   Product XMAP06 instead requires and binds the authenticated current
//!   XMAP10 `ShadowMap` for the replay camera.
//! * **Blend** — src_alpha / inv_src_alpha, add, RGB-only write mask: the
//!   shader's `BlendState` block (border.shader:160-167) AND the captured
//!   draw's live blend state agree exactly (write mask 7, blend 5/6/ADD).
//! * **Depth** — test `LESS_EQUAL`, write OFF — exactly the captured
//!   depth-stencil state; the VS nudges its Z toward the camera to clear the
//!   terrain (border.shader:96-99).
//! * **Rasterizer** — solid, CULL BACK, front CCW — the captured draw's live
//!   rasterizer state. Winding is direction-invariant for our chains (a chain
//!   reversal flips row order AND sides, preserving orientation).
//! * **SeaRegion** — rendered through the recovered south-up contour producer,
//!   literal f32 water smoothing, and closed-water loop law; it is no longer a
//!   typed omission.
//! * **Draw order** — production uses exact-a729's two phases. This legacy
//!   diagnostic hook remains non-authoritative.

use crate::Result;

/// Resolver-relative shipped shader for the border pass (same file the
/// synthesis module cites).
pub const BORDER_PASS_SHADER_SOURCE: &str = "gfx/FX/border.shader";
/// Effect name inside `border.shader`.
pub const BORDER_PASS_EFFECT_NAME: &str = "border";

/// CPU-side stats for the prepared pass (VRAM + partition shape), for
/// reporting. Sizes are the exact byte widths of the created D3D11 buffers.
#[derive(Clone, Debug)]
pub struct BorderFilesPassStats {
    pub vertex_count: usize,
    pub index_count: usize,
    pub vertex_buffer_bytes: usize,
    pub index_buffer_bytes: usize,
    pub class_ranges: Vec<(String, u32)>,
    pub prepare_ms: f64,
}

/// Per-render outcome of the border pass: which classes drew, which were
/// skipped as fully faded (visually identical to an alpha-0 draw).
#[derive(Clone, Debug, Default, PartialEq)]
pub struct BorderFilesPassRenderReport {
    /// Classes that issued a DrawIndexed this render.
    pub drawn_class_count: usize,
    /// Classes skipped because [`crate::border_transparency`] returned `<= 0`
    /// at this camera height (the shader would multiply every fragment to
    /// alpha 0, so skipping is visually identical and saves fill rate).
    pub fully_faded_class_count: usize,
}

#[cfg(windows)]
mod windows_impl {
    use super::*;
    use crate::shader_compile::{
        build_stage_hlsl, d3d_compile_stage, ShaderStage, ENGINE_COMPILE_FLAGS,
    };
    use crate::{
        border_diffuse_source, border_fade_windows_from_resolver, border_transparency,
        load_dds_rgba_mips, native_d3d11_create_input_layout_from_elements, native_d3d11_hr,
        synthesize_border_world_mesh_from_resolver, BorderClassIndexRange, BorderFadeWindows,
        D3dPtr, GameFileResolver, NativeD3d11DepthStencil, NativeD3d11Device,
        NativeD3d11PreparedDraw, NativeD3d11RenderTarget, RgbaImage, RuntimeInputElementMeta,
        DXGI_FORMAT_R16G16_SINT, DXGI_FORMAT_R32G32B32_FLOAT, DXGI_FORMAT_R32_UINT,
        DXGI_FORMAT_R8G8B8A8_UNORM_SRGB,
    };
    use std::ptr;
    use std::time::Instant;
    use winapi::shared::dxgiformat::DXGI_FORMAT;
    use winapi::shared::minwindef::UINT;
    use winapi::um::d3d11::{
        ID3D11BlendState, ID3D11Buffer, ID3D11DepthStencilState, ID3D11InputLayout,
        ID3D11PixelShader, ID3D11RasterizerState, ID3D11SamplerState, ID3D11ShaderResourceView,
        ID3D11Texture2D, ID3D11VertexShader, D3D11_BIND_CONSTANT_BUFFER, D3D11_BIND_INDEX_BUFFER,
        D3D11_BIND_SHADER_RESOURCE, D3D11_BIND_VERTEX_BUFFER, D3D11_BLEND_DESC,
        D3D11_BLEND_INV_SRC_ALPHA, D3D11_BLEND_ONE, D3D11_BLEND_OP_ADD, D3D11_BLEND_SRC_ALPHA,
        D3D11_BLEND_ZERO, D3D11_BUFFER_DESC, D3D11_COMPARISON_LESS_EQUAL, D3D11_COMPARISON_NEVER,
        D3D11_CPU_ACCESS_WRITE, D3D11_CULL_BACK, D3D11_DEPTH_STENCILOP_DESC,
        D3D11_DEPTH_STENCIL_DESC, D3D11_DEPTH_WRITE_MASK_ZERO, D3D11_FILL_SOLID,
        D3D11_FILTER_MIN_MAG_MIP_LINEAR, D3D11_MAPPED_SUBRESOURCE, D3D11_MAP_WRITE_DISCARD,
        D3D11_RASTERIZER_DESC, D3D11_SAMPLER_DESC, D3D11_STENCIL_OP_KEEP, D3D11_SUBRESOURCE_DATA,
        D3D11_TEXTURE2D_DESC, D3D11_TEXTURE_ADDRESS_CLAMP, D3D11_TEXTURE_ADDRESS_WRAP,
        D3D11_USAGE_DYNAMIC, D3D11_USAGE_IMMUTABLE,
    };
    use winapi::um::d3dcommon::D3D11_PRIMITIVE_TOPOLOGY_TRIANGLELIST;

    /// Captured border vertex stride: `float3 position` + `R16G16_SINT uv`.
    const BORDER_PASS_VERTEX_STRIDE: u32 = 16;
    /// CAPTURE: `session_BORDERZOOM_3/map_frame_borders_pass_bundle.json`
    /// records the engine PS slot-2 buffer as 32 bytes. The float4 + float2
    /// payload occupies 24 bytes and D3D11 pads the allocation to 32.
    const BORDER_PASS_CB2_BYTES: usize = 32;
    /// `MipMapLodBias = -0.5` from the BorderDiffuse sampler block.
    const BORDER_DIFFUSE_MIP_LOD_BIAS: f32 = -0.5;
    /// `MipMapMaxLod = 4` from the BorderDiffuse sampler block.
    const BORDER_DIFFUSE_MAX_LOD: f32 = 4.0;

    /// A fully-prepared `--borders-from-files` pass: compiled shaders, the
    /// once-uploaded geometry, per-class textures/ranges, and the state
    /// objects. Rendered per frame with [`native_d3d11_render_border_files_pass`].
    pub(crate) struct NativeD3d11BorderFilesPass {
        pub(crate) vertex_shader: D3dPtr<ID3D11VertexShader>,
        pub(crate) pixel_shader: D3dPtr<ID3D11PixelShader>,
        pub(crate) input_layout: D3dPtr<ID3D11InputLayout>,
        pub(crate) vertex_buffer: D3dPtr<ID3D11Buffer>,
        pub(crate) index_buffer: D3dPtr<ID3D11Buffer>,
        pub(crate) cb2: D3dPtr<ID3D11Buffer>,
        /// One `(range, BorderDiffuse SRV)` per non-empty class, in draw order.
        pub(crate) class_draws: Vec<(BorderClassIndexRange, D3dPtr<ID3D11ShaderResourceView>)>,
        /// Keep the class textures alive for the SRV lifetimes.
        pub(crate) _class_textures: Vec<D3dPtr<ID3D11Texture2D>>,
        /// Neutral (1,1,0,1) texel for SnowMudData/IntelMap/ShadowMap.
        pub(crate) neutral_srv: D3dPtr<ID3D11ShaderResourceView>,
        pub(crate) _neutral_texture: D3dPtr<ID3D11Texture2D>,
        /// PS samplers 0..=3 per the shader's Samplers block.
        pub(crate) samplers: [D3dPtr<ID3D11SamplerState>; 4],
        pub(crate) blend_state: D3dPtr<ID3D11BlendState>,
        pub(crate) depth_state: D3dPtr<ID3D11DepthStencilState>,
        pub(crate) rasterizer_state: D3dPtr<ID3D11RasterizerState>,
        pub(crate) fade_windows: BorderFadeWindows,
        pub(crate) stats: BorderFilesPassStats,
    }

    impl NativeD3d11BorderFilesPass {
        pub(crate) fn stats(&self) -> &BorderFilesPassStats {
            &self.stats
        }
    }

    fn blocked(detail: impl std::fmt::Display) -> String {
        format!("--borders-from-files border pass blocked: {detail}")
    }

    /// Compile one stage of `Effect border` through the byte-exact machinery
    /// (resolver-routed so mods are honoured) at [`ENGINE_COMPILE_FLAGS`]
    /// (`OPTIMIZATION_LEVEL3`) — the level OFF-3 established reproduces the
    /// shipped map shader blobs byte-exact. FXC's default level-1 (flags 0) does
    /// NOT match the captured engine border DXBC (`d16f3aba`); O3 does.
    fn compile_border_stage(resolver: &GameFileResolver, stage: ShaderStage) -> Result<Vec<u8>> {
        let hlsl = build_stage_hlsl(
            resolver,
            BORDER_PASS_SHADER_SOURCE,
            BORDER_PASS_EFFECT_NAME,
            stage,
        )
        .map_err(|err| {
            blocked(format!(
                "failed to reconstruct {BORDER_PASS_SHADER_SOURCE}:{BORDER_PASS_EFFECT_NAME} \
                 {stage:?} HLSL: {err}"
            ))
        })?;
        d3d_compile_stage(&hlsl, stage, ENGINE_COMPILE_FLAGS).map_err(|err| {
            blocked(format!(
                "D3DCompile of {BORDER_PASS_SHADER_SOURCE}:{BORDER_PASS_EFFECT_NAME} {stage:?} \
                 failed: {err}"
            ))
            .into()
        })
    }

    unsafe fn create_immutable_buffer(
        device: &NativeD3d11Device,
        bytes: &[u8],
        bind_flags: UINT,
        role: &str,
    ) -> Result<D3dPtr<ID3D11Buffer>> {
        let desc = D3D11_BUFFER_DESC {
            ByteWidth: u32::try_from(bytes.len())
                .map_err(|_| blocked(format!("{role} exceeds u32 bytes")))?,
            Usage: D3D11_USAGE_IMMUTABLE,
            BindFlags: bind_flags,
            CPUAccessFlags: 0,
            MiscFlags: 0,
            StructureByteStride: 0,
        };
        let init = D3D11_SUBRESOURCE_DATA {
            pSysMem: bytes.as_ptr() as *const _,
            SysMemPitch: 0,
            SysMemSlicePitch: 0,
        };
        let mut buffer: *mut ID3D11Buffer = ptr::null_mut();
        native_d3d11_hr(
            (*device.device.as_raw()).CreateBuffer(&desc, &init, &mut buffer),
            &format!("CreateBuffer for {role}"),
        )?;
        D3dPtr::from_raw(buffer, role)
    }

    /// Upload an RGBA8 mip chain as an immutable `R8G8B8A8_UNORM_SRGB`
    /// Texture2D + full SRV.
    ///
    /// BORD5: the REAL captured BorderDiffuse SRV (session_BORDERZOOM_3, the
    /// genuine state-border draw) is `BC3_UNORM_SRGB` (78) — sRGB sampling.
    /// The earlier non-sRGB reading came from diag23b, which is a proven
    /// contaminated capture. The CPU BC decode here follows the D3D BC spec,
    /// so decoding to RGBA8 and letting the GPU apply the sRGB-to-linear ramp
    /// on sample matches the engine's BC3_SRGB path (residual risk: BC
    /// interpolation rounding differences, an open unknown in
    /// `border_texture_law.rs`).
    unsafe fn create_rgba_mip_texture(
        device: &NativeD3d11Device,
        mips: &[RgbaImage],
        role: &str,
    ) -> Result<(D3dPtr<ID3D11Texture2D>, D3dPtr<ID3D11ShaderResourceView>)> {
        if mips.is_empty() {
            return Err(blocked(format!("{role} decoded zero mips")).into());
        }
        for mip in mips {
            if mip.pixels.len() != mip.width * mip.height * 4 {
                return Err(blocked(format!(
                    "{role} mip payload is {} bytes for {}x{} RGBA",
                    mip.pixels.len(),
                    mip.width,
                    mip.height
                ))
                .into());
            }
        }
        let desc = D3D11_TEXTURE2D_DESC {
            Width: mips[0].width as UINT,
            Height: mips[0].height as UINT,
            MipLevels: mips.len() as UINT,
            ArraySize: 1,
            Format: DXGI_FORMAT_R8G8B8A8_UNORM_SRGB as DXGI_FORMAT,
            SampleDesc: winapi::shared::dxgitype::DXGI_SAMPLE_DESC {
                Count: 1,
                Quality: 0,
            },
            Usage: D3D11_USAGE_IMMUTABLE,
            BindFlags: D3D11_BIND_SHADER_RESOURCE,
            CPUAccessFlags: 0,
            MiscFlags: 0,
        };
        let init: Vec<D3D11_SUBRESOURCE_DATA> = mips
            .iter()
            .map(|mip| D3D11_SUBRESOURCE_DATA {
                pSysMem: mip.pixels.as_ptr() as *const _,
                SysMemPitch: (mip.width * 4) as UINT,
                SysMemSlicePitch: 0,
            })
            .collect();
        let mut texture: *mut ID3D11Texture2D = ptr::null_mut();
        native_d3d11_hr(
            (*device.device.as_raw()).CreateTexture2D(&desc, init.as_ptr(), &mut texture),
            &format!("CreateTexture2D for {role}"),
        )?;
        let texture = D3dPtr::from_raw(texture, role)?;
        let mut srv: *mut ID3D11ShaderResourceView = ptr::null_mut();
        native_d3d11_hr(
            (*device.device.as_raw()).CreateShaderResourceView(
                texture.as_raw() as *mut _,
                ptr::null(),
                &mut srv,
            ),
            &format!("CreateShaderResourceView for {role}"),
        )?;
        let srv = D3dPtr::from_raw(srv, role)?;
        Ok((texture, srv))
    }

    /// The four PS samplers from the shader's `Samplers` block
    /// (border.shader:13-49): BorderDiffuse linear wrap/clamp with LOD bias
    /// -0.5 and max LOD 4; SnowMudData/IntelMap linear wrap; ShadowMap linear
    /// wrap — a REGULAR sampler, because the shipped DX11 header translates
    /// `sampler2DShadow` to a plain texture+sampler pair
    /// (defines_hlsl_dx11.fxh:47) and `tex2Dproj` to a plain `SampleLevel`
    /// (line 54); no comparison sampling exists in the compiled PS.
    unsafe fn create_border_samplers(
        device: &NativeD3d11Device,
    ) -> Result<[D3dPtr<ID3D11SamplerState>; 4]> {
        let make = |address_v: u32, lod_bias: f32, max_lod: f32, role: &str| {
            let desc = D3D11_SAMPLER_DESC {
                Filter: D3D11_FILTER_MIN_MAG_MIP_LINEAR,
                AddressU: D3D11_TEXTURE_ADDRESS_WRAP,
                AddressV: address_v,
                AddressW: D3D11_TEXTURE_ADDRESS_WRAP,
                MipLODBias: lod_bias,
                MaxAnisotropy: 1,
                ComparisonFunc: D3D11_COMPARISON_NEVER,
                BorderColor: [0.0; 4],
                MinLOD: 0.0,
                MaxLOD: max_lod,
            };
            let mut state: *mut ID3D11SamplerState = ptr::null_mut();
            native_d3d11_hr(
                (*device.device.as_raw()).CreateSamplerState(&desc, &mut state),
                &format!("CreateSamplerState for {role}"),
            )?;
            D3dPtr::from_raw(state, role)
        };
        Ok([
            // BorderDiffuse: AddressU Wrap / AddressV Clamp, bias -0.5, max LOD 4.
            make(
                D3D11_TEXTURE_ADDRESS_CLAMP,
                BORDER_DIFFUSE_MIP_LOD_BIAS,
                BORDER_DIFFUSE_MAX_LOD,
                "border BorderDiffuse sampler",
            )?,
            make(
                D3D11_TEXTURE_ADDRESS_WRAP,
                0.0,
                f32::MAX,
                "border SnowMudData sampler",
            )?,
            make(
                D3D11_TEXTURE_ADDRESS_WRAP,
                0.0,
                f32::MAX,
                "border IntelMap sampler",
            )?,
            make(
                D3D11_TEXTURE_ADDRESS_WRAP,
                0.0,
                f32::MAX,
                "border ShadowMap sampler",
            )?,
        ])
    }

    /// Blend state from the shader's `BlendState` block (border.shader:160-167):
    /// enabled, `src_alpha` / `inv_src_alpha`, add, `WriteMask = RED|GREEN|BLUE`
    /// (0x7 — the border never writes destination alpha). Alpha-channel blend is
    /// ONE/ZERO (unspecified by the block; the mask makes it inert).
    unsafe fn create_border_blend_state(
        device: &NativeD3d11Device,
    ) -> Result<D3dPtr<ID3D11BlendState>> {
        let mut desc: D3D11_BLEND_DESC = std::mem::zeroed();
        desc.AlphaToCoverageEnable = 0;
        desc.IndependentBlendEnable = 0;
        desc.RenderTarget[0].BlendEnable = 1;
        desc.RenderTarget[0].SrcBlend = D3D11_BLEND_SRC_ALPHA;
        desc.RenderTarget[0].DestBlend = D3D11_BLEND_INV_SRC_ALPHA;
        desc.RenderTarget[0].BlendOp = D3D11_BLEND_OP_ADD;
        desc.RenderTarget[0].SrcBlendAlpha = D3D11_BLEND_ONE;
        desc.RenderTarget[0].DestBlendAlpha = D3D11_BLEND_ZERO;
        desc.RenderTarget[0].BlendOpAlpha = D3D11_BLEND_OP_ADD;
        desc.RenderTarget[0].RenderTargetWriteMask = 0x7;
        let mut state: *mut ID3D11BlendState = ptr::null_mut();
        native_d3d11_hr(
            (*device.device.as_raw()).CreateBlendState(&desc, &mut state),
            "CreateBlendState for border pass (src_alpha/inv_src_alpha, RGB mask)",
        )?;
        D3dPtr::from_raw(state, "border blend state")
    }

    /// Depth: test `LESS_EQUAL`, write OFF. The border VS biases its clip-space
    /// Z toward the camera (border.shader:96-99) so the overlay clears the
    /// terrain surface; the test still culls fragments behind hills, and not
    /// writing depth keeps later passes (trees) unoccluded by border quads.
    unsafe fn create_border_depth_state(
        device: &NativeD3d11Device,
    ) -> Result<D3dPtr<ID3D11DepthStencilState>> {
        let stencil_op = D3D11_DEPTH_STENCILOP_DESC {
            StencilFailOp: D3D11_STENCIL_OP_KEEP,
            StencilDepthFailOp: D3D11_STENCIL_OP_KEEP,
            StencilPassOp: D3D11_STENCIL_OP_KEEP,
            StencilFunc: D3D11_COMPARISON_NEVER,
        };
        let desc = D3D11_DEPTH_STENCIL_DESC {
            DepthEnable: 1,
            DepthWriteMask: D3D11_DEPTH_WRITE_MASK_ZERO,
            DepthFunc: D3D11_COMPARISON_LESS_EQUAL,
            StencilEnable: 0,
            StencilReadMask: 0,
            StencilWriteMask: 0,
            FrontFace: stencil_op,
            BackFace: stencil_op,
        };
        let mut state: *mut ID3D11DepthStencilState = ptr::null_mut();
        native_d3d11_hr(
            (*device.device.as_raw()).CreateDepthStencilState(&desc, &mut state),
            "CreateDepthStencilState for border pass (LESS_EQUAL, write off)",
        )?;
        D3dPtr::from_raw(state, "border depth state")
    }

    /// Rasterizer: the CAPTURED border draw state (session_BORDERZOOM_3):
    /// solid fill, CULL BACK, `front_counter_clockwise = true`, depth clip on,
    /// no scissor/multisample/AA-line. Our chain emission replicates the
    /// engine's per-row index pattern and side assignment, and a full chain
    /// reversal flips both row order and the left/right sides (an
    /// orientation-preserving 180° parameter rotation), so winding — and
    /// therefore back-face culling — is direction-invariant.
    unsafe fn create_border_rasterizer_state(
        device: &NativeD3d11Device,
    ) -> Result<D3dPtr<ID3D11RasterizerState>> {
        let desc = D3D11_RASTERIZER_DESC {
            FillMode: D3D11_FILL_SOLID,
            CullMode: D3D11_CULL_BACK,
            FrontCounterClockwise: 1,
            DepthBias: 0,
            DepthBiasClamp: 0.0,
            SlopeScaledDepthBias: 0.0,
            DepthClipEnable: 1,
            ScissorEnable: 0,
            MultisampleEnable: 0,
            AntialiasedLineEnable: 0,
        };
        let mut state: *mut ID3D11RasterizerState = ptr::null_mut();
        native_d3d11_hr(
            (*device.device.as_raw()).CreateRasterizerState(&desc, &mut state),
            "CreateRasterizerState for border pass (captured: solid, cull back, front CCW)",
        )?;
        D3dPtr::from_raw(state, "border rasterizer state")
    }

    /// Prepare the whole pass ONCE per prepared production map: synthesize the
    /// mesh, compile both border shader stages, upload geometry, load per-class
    /// textures, and build the state objects. Fail-closed on every input.
    pub(crate) fn native_d3d11_prepare_border_files_pass(
        device: &NativeD3d11Device,
        resolver: &GameFileResolver,
    ) -> Result<NativeD3d11BorderFilesPass> {
        let started = Instant::now();
        let mesh = synthesize_border_world_mesh_from_resolver(resolver)?;
        let partition = mesh.class_index_partition();
        let fade_windows = border_fade_windows_from_resolver(resolver)?;

        let vs_bytes = compile_border_stage(resolver, ShaderStage::Vertex)?;
        let ps_bytes = compile_border_stage(resolver, ShaderStage::Pixel)?;

        unsafe {
            let mut vs: *mut ID3D11VertexShader = ptr::null_mut();
            native_d3d11_hr(
                (*device.device.as_raw()).CreateVertexShader(
                    vs_bytes.as_ptr() as *const _,
                    vs_bytes.len(),
                    ptr::null_mut(),
                    &mut vs,
                ),
                "CreateVertexShader for border.shader Effect border",
            )?;
            let vertex_shader = D3dPtr::from_raw(vs, "border vertex shader")?;

            let mut ps: *mut ID3D11PixelShader = ptr::null_mut();
            native_d3d11_hr(
                (*device.device.as_raw()).CreatePixelShader(
                    ps_bytes.as_ptr() as *const _,
                    ps_bytes.len(),
                    ptr::null_mut(),
                    &mut ps,
                ),
                "CreatePixelShader for border.shader Effect border",
            )?;
            let pixel_shader = D3dPtr::from_raw(ps, "border pixel shader")?;

            // VS_INPUT_BORDER as ONE interleaved stream (both elements slot 0).
            let elements = [
                RuntimeInputElementMeta {
                    semantic_name: "POSITION".to_string(),
                    semantic_index: 0,
                    format: DXGI_FORMAT_R32G32B32_FLOAT,
                    input_slot: 0,
                    aligned_byte_offset: 0,
                    input_slot_class: 0,
                    instance_data_step_rate: 0,
                },
                RuntimeInputElementMeta {
                    semantic_name: "TEXCOORD".to_string(),
                    semantic_index: 0,
                    format: DXGI_FORMAT_R16G16_SINT,
                    input_slot: 0,
                    aligned_byte_offset: 12,
                    input_slot_class: 0,
                    instance_data_step_rate: 0,
                },
            ];
            let input_layout = native_d3d11_create_input_layout_from_elements(
                &device.device,
                &elements,
                &vs_bytes,
            )?;

            let vertex_bytes = mesh.to_shader_vertex_layout_bytes()?;
            let vertex_buffer = create_immutable_buffer(
                device,
                &vertex_bytes,
                D3D11_BIND_VERTEX_BUFFER,
                "border pass vertex buffer",
            )?;
            let mut index_bytes = Vec::with_capacity(partition.indices.len() * 4);
            for index in &partition.indices {
                index_bytes.extend_from_slice(&index.to_le_bytes());
            }
            let index_buffer = create_immutable_buffer(
                device,
                &index_bytes,
                D3D11_BIND_INDEX_BUFFER,
                "border pass index buffer",
            )?;

            // Dynamic CB2, rewritten per class per frame.
            let cb2_desc = D3D11_BUFFER_DESC {
                ByteWidth: BORDER_PASS_CB2_BYTES as UINT,
                Usage: D3D11_USAGE_DYNAMIC,
                BindFlags: D3D11_BIND_CONSTANT_BUFFER,
                CPUAccessFlags: D3D11_CPU_ACCESS_WRITE,
                MiscFlags: 0,
                StructureByteStride: 0,
            };
            let cb2_zero = [0u8; BORDER_PASS_CB2_BYTES];
            let cb2_init = D3D11_SUBRESOURCE_DATA {
                pSysMem: cb2_zero.as_ptr() as *const _,
                SysMemPitch: 0,
                SysMemSlicePitch: 0,
            };
            let mut cb2_raw: *mut ID3D11Buffer = ptr::null_mut();
            native_d3d11_hr(
                (*device.device.as_raw()).CreateBuffer(&cb2_desc, &cb2_init, &mut cb2_raw),
                "CreateBuffer for border CB2 (vSelectionColor + vTime_Transparency)",
            )?;
            let cb2 = D3dPtr::from_raw(cb2_raw, "border CB2")?;

            // Per-class BorderDiffuse textures from the shipped DDS files.
            let mut class_draws = Vec::with_capacity(partition.ranges.len());
            let mut class_textures = Vec::with_capacity(partition.ranges.len());
            for range in &partition.ranges {
                let source = border_diffuse_source(range.material);
                let path = resolver
                    .resolve_required(source, "border diffuse DDS")
                    .map_err(|err| blocked(err))?;
                let mips = load_dds_rgba_mips(&path)
                    .map_err(|err| blocked(format!("failed to decode {source}: {err}")))?;
                let (texture, srv) =
                    create_rgba_mip_texture(device, &mips, &format!("border diffuse {source}"))?;
                class_textures.push(texture);
                class_draws.push((*range, srv));
            }

            // Neutral (1,1,0,1): shadow term 1.0 (no darkening), FOW alpha 0
            // (no fog). The compiled PS reflection keeps only BorderDiffuse
            // (t0) and ShadowMap (t3), so the slot-1/2 binds below are
            // defensive no-ops; slot 3 is the load-bearing neutral.
            let neutral_mip = RgbaImage {
                width: 1,
                height: 1,
                pixels: vec![255, 255, 0, 255],
            };
            let (neutral_texture, neutral_srv) = create_rgba_mip_texture(
                device,
                std::slice::from_ref(&neutral_mip),
                "border neutral shadow/fow texel",
            )?;

            let samplers = create_border_samplers(device)?;
            let blend_state = create_border_blend_state(device)?;
            let depth_state = create_border_depth_state(device)?;
            let rasterizer_state = create_border_rasterizer_state(device)?;

            let stats = BorderFilesPassStats {
                vertex_count: mesh.vertices.len(),
                index_count: partition.indices.len(),
                vertex_buffer_bytes: vertex_bytes.len(),
                index_buffer_bytes: index_bytes.len(),
                class_ranges: partition
                    .ranges
                    .iter()
                    .map(|range| (range.material.as_str().to_string(), range.index_count))
                    .collect(),
                prepare_ms: started.elapsed().as_secs_f64() * 1000.0,
            };

            Ok(NativeD3d11BorderFilesPass {
                vertex_shader,
                pixel_shader,
                input_layout,
                vertex_buffer,
                index_buffer,
                cb2,
                class_draws,
                _class_textures: class_textures,
                neutral_srv,
                _neutral_texture: neutral_texture,
                samplers,
                blend_state,
                depth_state,
                rasterizer_state,
                fade_windows,
                stats,
            })
        }
    }

    /// Find the scene constant buffer bound by the frame at `stage`/`slot`.
    fn borrowed_scene_cb(
        scene_draw: &NativeD3d11PreparedDraw,
        stage: &str,
        slot: u32,
    ) -> Option<*mut ID3D11Buffer> {
        scene_draw
            .constant_buffers
            .iter()
            .find(|cb| cb.stage == stage && cb.slot == slot)
            .map(|cb| cb.buffer.as_raw())
    }

    /// Render the prepared border pass over the production-map terrain target.
    ///
    /// `scene_draw` is a terrain draw of the SAME frame: its live CB0/CB1
    /// buffers (camera-patched by the scene updater, including per-wrap-copy
    /// shifts) and viewports are borrowed for the border VS/PS. `camera_height`
    /// is the world-Y camera eye height driving [`border_transparency`];
    /// `clock` feeds `vTime_Transparency.x` (selection-pulse time base — inert
    /// with `vSelectionColor = 0`, kept for shader fidelity).
    pub(crate) fn native_d3d11_render_border_files_pass(
        device: &NativeD3d11Device,
        pass: &NativeD3d11BorderFilesPass,
        render_target: &NativeD3d11RenderTarget,
        depth_target: &NativeD3d11DepthStencil,
        scene_draw: &NativeD3d11PreparedDraw,
        camera_height: f32,
        clock: f32,
    ) -> Result<BorderFilesPassRenderReport> {
        let mut report = BorderFilesPassRenderReport::default();
        let vs_cb0 = borrowed_scene_cb(scene_draw, "VS", 0).ok_or_else(|| {
            blocked(
                "the frame's terrain draw binds no VS constant buffer at slot 0; the border VS \
                 needs the scene CB0 (ViewProjectionMatrix / vCamLookAtDir)",
            )
        })?;
        let vs_cb1 = borrowed_scene_cb(scene_draw, "VS", 1);
        // The scene CB0 is one shared buffer; if the terrain draw lacks an
        // explicit PS slot-0 binding, the VS slot-0 buffer carries the same
        // scene constants (fog / day-night / ShadowFadeFactor).
        let ps_cb0 = borrowed_scene_cb(scene_draw, "PS", 0).unwrap_or(vs_cb0);
        let ps_cb1 = borrowed_scene_cb(scene_draw, "PS", 1);
        if scene_draw.viewports.is_empty() {
            return Err(blocked("the frame's terrain draw carries no viewport").into());
        }

        unsafe {
            let context = device.context.as_raw();

            // Per-frame CB2 base: vSelectionColor = 0 (no selection pulse in
            // the replay), vTime_Transparency = [clock, <per-class alpha>].
            // The per-class alpha is written inside the class loop below.
            (*context).VSSetShader(pass.vertex_shader.as_raw(), ptr::null_mut(), 0);
            (*context).PSSetShader(pass.pixel_shader.as_raw(), ptr::null_mut(), 0);
            (*context).IASetInputLayout(pass.input_layout.as_raw());
            (*context).IASetPrimitiveTopology(D3D11_PRIMITIVE_TOPOLOGY_TRIANGLELIST);
            let mut vb = pass.vertex_buffer.as_raw();
            let mut stride = BORDER_PASS_VERTEX_STRIDE;
            let mut offset = 0u32;
            (*context).IASetVertexBuffers(0, 1, &mut vb, &mut stride, &mut offset);
            (*context).IASetIndexBuffer(pass.index_buffer.as_raw(), DXGI_FORMAT_R32_UINT as _, 0);

            let mut vs_cb0_ptr = vs_cb0;
            (*context).VSSetConstantBuffers(0, 1, &mut vs_cb0_ptr);
            if let Some(vs_cb1) = vs_cb1 {
                let mut ptr1 = vs_cb1;
                (*context).VSSetConstantBuffers(1, 1, &mut ptr1);
            }
            let mut ps_cb0_ptr = ps_cb0;
            (*context).PSSetConstantBuffers(0, 1, &mut ps_cb0_ptr);
            if let Some(ps_cb1) = ps_cb1 {
                let mut ptr1 = ps_cb1;
                (*context).PSSetConstantBuffers(1, 1, &mut ptr1);
            }

            for (slot, sampler) in pass.samplers.iter().enumerate() {
                let mut raw = sampler.as_raw();
                (*context).PSSetSamplers(slot as UINT, 1, &mut raw);
            }
            // Neutral SnowMudData(1) / IntelMap(2) / ShadowMap(3).
            for slot in 1..=3u32 {
                let mut raw = pass.neutral_srv.as_raw();
                (*context).PSSetShaderResources(slot, 1, &mut raw);
            }

            (*context).RSSetState(pass.rasterizer_state.as_raw());
            (*context).RSSetViewports(
                scene_draw.viewports.len() as UINT,
                scene_draw.viewports.as_ptr(),
            );
            (*context).RSSetScissorRects(0, ptr::null());
            (*context).OMSetBlendState(
                pass.blend_state.as_raw(),
                &[1.0, 1.0, 1.0, 1.0],
                0xffff_ffff,
            );
            (*context).OMSetDepthStencilState(pass.depth_state.as_raw(), 0);
            let mut rtv = render_target.view.as_raw();
            (*context).OMSetRenderTargets(1, &mut rtv, depth_target.view.as_raw());

            for (range, diffuse_srv) in &pass.class_draws {
                let transparency =
                    border_transparency(&pass.fade_windows, range.material, camera_height)?;
                if transparency <= 0.0 {
                    // Fully faded at this camera height: nothing to draw. The
                    // shader would multiply every fragment's alpha to zero, so
                    // skipping is visually identical and saves the fill rate.
                    report.fully_faded_class_count += 1;
                    continue;
                }
                // CB2 = { vSelectionColor: float4 = 0; vTime_Transparency:
                // float2 = [clock, transparency]; pad to the captured 32 B }.
                let mut mapped: D3D11_MAPPED_SUBRESOURCE = std::mem::zeroed();
                native_d3d11_hr(
                    (*context).Map(
                        pass.cb2.as_raw() as *mut _,
                        0,
                        D3D11_MAP_WRITE_DISCARD,
                        0,
                        &mut mapped,
                    ),
                    "Map border CB2",
                )?;
                let mut cb2_bytes = [0u8; BORDER_PASS_CB2_BYTES];
                cb2_bytes[16..20].copy_from_slice(&clock.to_le_bytes());
                cb2_bytes[20..24].copy_from_slice(&transparency.to_le_bytes());
                ptr::copy_nonoverlapping(
                    cb2_bytes.as_ptr(),
                    mapped.pData as *mut u8,
                    BORDER_PASS_CB2_BYTES,
                );
                (*context).Unmap(pass.cb2.as_raw() as *mut _, 0);
                let mut cb2_ptr = pass.cb2.as_raw();
                (*context).PSSetConstantBuffers(2, 1, &mut cb2_ptr);

                let mut diffuse = diffuse_srv.as_raw();
                (*context).PSSetShaderResources(0, 1, &mut diffuse);
                (*context).DrawIndexed(range.index_count, range.start_index, 0);
                report.drawn_class_count += 1;
            }

            // Unbind the borrowed scene CBs and our SRVs so later passes see a
            // clean slate (they re-bind their own state anyway; this guards
            // against dangling references to the border textures).
            let mut null_srv: *mut ID3D11ShaderResourceView = ptr::null_mut();
            for slot in 0..=3u32 {
                (*context).PSSetShaderResources(slot, 1, &mut null_srv);
            }
        }
        Ok(report)
    }
}

#[cfg(windows)]
pub(crate) use windows_impl::{
    native_d3d11_prepare_border_files_pass, native_d3d11_render_border_files_pass,
    NativeD3d11BorderFilesPass,
};

// Compile-time reference so the constants are used on all platforms.
#[allow(dead_code)]
const _: fn() -> Result<()> = || {
    let _ = (BORDER_PASS_SHADER_SOURCE, BORDER_PASS_EFFECT_NAME);
    Ok(())
};

#[cfg(all(test, windows))]
mod tests {
    use crate::test_support::temp_test_dir;
    use crate::{GameFileResolver, NativeD3d11TerrainRenderer};
    use std::fs;

    /// The spec's fail-closed no-resolver gate: `--borders-from-files` set on
    /// the renderer WITHOUT a resolver must be a hard error at prepare time,
    /// and flag-OFF must be `Ok(None)` (no work at all — the byte-identical
    /// path).
    #[test]
    fn borders_from_files_without_resolver_fails_closed() {
        let mut renderer = NativeD3d11TerrainRenderer::new().expect("headless D3D11 device");
        // Flag OFF (default): no resolver required, no work requested.
        assert!(renderer
            .border_files_resolver_for_prepare()
            .expect("flag off is Ok")
            .is_none());
        // Flag ON without a resolver: hard error naming the fix.
        renderer.set_borders_from_files(true);
        let err = renderer
            .border_files_resolver_for_prepare()
            .expect_err("flag on without resolver must fail closed");
        assert!(
            err.to_string()
                .contains("--borders-from-files border pass blocked"),
            "unexpected error: {err}"
        );
        assert!(
            err.to_string().contains("with_border_resolver"),
            "error must name the fix: {err}"
        );
        // Flag ON with a resolver: the gate passes (prepare itself is
        // exercised by the missing-game-files test below).
        let root = temp_test_dir("border_pass_resolver_ok");
        fs::create_dir_all(root.join("map")).expect("create map dir");
        fs::write(
            root.join("map").join("default.map"),
            "definitions = \"definition.csv\"\nprovinces = \"provinces.bmp\"\nheightmap = \"heightmap.bmp\"\n",
        )
        .expect("write default.map");
        let resolver = GameFileResolver::vanilla(&root).expect("resolver");
        let renderer = renderer.with_border_resolver(resolver);
        assert!(renderer
            .border_files_resolver_for_prepare()
            .expect("flag on with resolver is Ok")
            .is_some());
        fs::remove_dir_all(root).ok();
    }

    /// Full prepare against a resolver whose game dir lacks the border inputs:
    /// the pass must fail closed (via the synthesis fail-closed chain), never
    /// produce an empty/partial pass.
    #[test]
    fn prepare_border_files_pass_fails_closed_on_missing_game_files() {
        let renderer = NativeD3d11TerrainRenderer::new().expect("headless D3D11 device");
        let root = temp_test_dir("border_pass_missing_files");
        fs::create_dir_all(root.join("map")).expect("create map dir");
        fs::write(
            root.join("map").join("default.map"),
            "definitions = \"definition.csv\"\nprovinces = \"provinces.bmp\"\nheightmap = \"heightmap.bmp\"\n",
        )
        .expect("write default.map");
        let resolver = GameFileResolver::vanilla(&root).expect("resolver");
        let err = match super::native_d3d11_prepare_border_files_pass(&renderer.device, &resolver) {
            Ok(_) => panic!("missing installed content must block the border pass"),
            Err(err) => err,
        };
        assert!(
            err.to_string().contains("definition.csv"),
            "unexpected error: {err}"
        );
        fs::remove_dir_all(root).ok();
    }
}
