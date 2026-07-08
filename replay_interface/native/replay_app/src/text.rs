//! D3D11 overlay drawing: bitmap-font text batches + solid quads.
//!
//! Follows the render.rs house patterns exactly: raw winapi COM behind
//! [`ComPtr`], `hr_ok` on every HRESULT, runtime-compiled HLSL via
//! `D3DCompile`. Glyph atlas pages are uploaded once as immutable
//! `R8G8B8A8` SRVs; every draw fills a dynamic vertex buffer with
//! screen-space pixel quads and renders them alpha-blended over whatever
//! is already in the target (the map).

use std::ffi::CString;
use std::mem;
use std::ptr;

use winapi::shared::dxgiformat::{
    DXGI_FORMAT_R32G32_FLOAT, DXGI_FORMAT_R8G8B8A8_UNORM,
};
use winapi::shared::minwindef::UINT;
use winapi::um::d3d11::{
    ID3D11BlendState, ID3D11Buffer, ID3D11InputLayout, ID3D11PixelShader,
    ID3D11RenderTargetView, ID3D11SamplerState,
    ID3D11ShaderResourceView, ID3D11Texture2D, ID3D11VertexShader, D3D11_BIND_CONSTANT_BUFFER,
    D3D11_BIND_VERTEX_BUFFER, D3D11_BLEND_DESC, D3D11_BLEND_INV_SRC_ALPHA, D3D11_BLEND_ONE,
    D3D11_BLEND_OP_ADD, D3D11_BLEND_SRC_ALPHA, D3D11_BUFFER_DESC, D3D11_COLOR_WRITE_ENABLE_ALL,
    D3D11_COMPARISON_NEVER, D3D11_CPU_ACCESS_WRITE, D3D11_FILTER_MIN_MAG_MIP_LINEAR,
    D3D11_FILTER_MIN_MAG_MIP_POINT, D3D11_INPUT_ELEMENT_DESC, D3D11_INPUT_PER_VERTEX_DATA,
    D3D11_MAPPED_SUBRESOURCE, D3D11_MAP_WRITE_DISCARD, D3D11_SAMPLER_DESC,
    D3D11_TEXTURE_ADDRESS_CLAMP, D3D11_USAGE_DYNAMIC, D3D11_VIEWPORT,
};
use winapi::um::d3dcommon::D3D11_PRIMITIVE_TOPOLOGY_TRIANGLELIST;

use crate::d3d::{hr_ok, ComPtr, Gfx};
use crate::font::{Font, GlyphQuad};
use crate::render::{compile_shader, create_immutable_texture};
use crate::Result;

/// Screen-space textured-quad shader shared by text and solid rects.
/// Vertices are window pixels; the VS converts to clip space; the PS
/// samples the bound page (glyph atlas or the 1x1 white texel) and
/// multiplies by the vertex color, so text tinting matches the game's
/// white-glyphs-times-font-color scheme observed in the real pages.
const OVERLAY_SHADER_HLSL: &str = r#"
cbuffer ScreenCB : register(b0) {
    float2 screen_size;
    float2 _pad;
};

Texture2D Page : register(t0);
SamplerState PointClamp : register(s0);

struct VsIn {
    float2 pos : POSITION;
    float2 uv : TEXCOORD0;
    float4 color : COLOR0;
};

struct VsOut {
    float4 pos : SV_Position;
    float2 uv : TEXCOORD0;
    float4 color : COLOR0;
};

VsOut vs_main(VsIn input) {
    VsOut output;
    float2 clip = float2(
        input.pos.x / screen_size.x * 2.0 - 1.0,
        1.0 - input.pos.y / screen_size.y * 2.0);
    output.pos = float4(clip, 0.0, 1.0);
    output.uv = input.uv;
    output.color = input.color;
    return output;
}

float4 ps_main(VsOut input) : SV_Target {
    return Page.Sample(PointClamp, input.uv) * input.color;
}
"#;

/// One overlay vertex: window-pixel position, page UV, RGBA color.
#[repr(C)]
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Vertex {
    pub pos: [f32; 2],
    pub uv: [f32; 2],
    /// `R8G8B8A8_UNORM` bytes r,g,b,a (see [`color_to_rgba_bytes`]).
    pub color: u32,
}

/// A solid rectangle request for [`QuadRenderer::draw_rects`].
#[derive(Clone, Copy, Debug)]
pub struct SolidRect {
    pub x: f32,
    pub y: f32,
    pub width: f32,
    pub height: f32,
    /// `0xAARRGGBB`, same convention as fonts.json colors.
    pub color: u32,
}

#[repr(C)]
struct ScreenConstants {
    screen_size: [f32; 2],
    _pad: [f32; 2],
}

/// Vertex capacity of the dynamic buffer; longer batches draw in chunks.
const VERTEX_CAPACITY: usize = 8192;

/// Convert an `0xAARRGGBB` color (fonts.json convention) into the
/// `R8G8B8A8_UNORM` little-endian vertex encoding (bytes r, g, b, a).
pub fn color_to_rgba_bytes(argb: u32) -> u32 {
    let a = (argb >> 24) & 0xFF;
    let r = (argb >> 16) & 0xFF;
    let g = (argb >> 8) & 0xFF;
    let b = argb & 0xFF;
    r | (g << 8) | (b << 16) | (a << 24)
}

/// Append the two triangles of one glyph quad.
pub fn push_glyph_vertices(out: &mut Vec<Vertex>, quad: &GlyphQuad, rgba: u32) {
    let corners = [
        Vertex {
            pos: [quad.x0, quad.y0],
            uv: [quad.u0, quad.v0],
            color: rgba,
        },
        Vertex {
            pos: [quad.x1, quad.y0],
            uv: [quad.u1, quad.v0],
            color: rgba,
        },
        Vertex {
            pos: [quad.x1, quad.y1],
            uv: [quad.u1, quad.v1],
            color: rgba,
        },
        Vertex {
            pos: [quad.x0, quad.y1],
            uv: [quad.u0, quad.v1],
            color: rgba,
        },
    ];
    out.extend_from_slice(&[
        corners[0], corners[1], corners[2], corners[0], corners[2], corners[3],
    ]);
}

/// Append the two triangles of one solid rect (UV pinned to the center of
/// the 1x1 white texel).
pub fn push_rect_vertices(out: &mut Vec<Vertex>, rect: &SolidRect) {
    push_glyph_vertices(
        out,
        &GlyphQuad {
            page: 0,
            x0: rect.x,
            y0: rect.y,
            x1: rect.x + rect.width,
            y1: rect.y + rect.height,
            u0: 0.5,
            v0: 0.5,
            u1: 0.5,
            v1: 0.5,
        },
        color_to_rgba_bytes(rect.color),
    );
}

/// [`create_immutable_texture`] with an explicit mip chain: `levels[0]` is
/// mip 0, each subsequent level halves (validated). Needed because the render
/// helper is single-mip and the label atlas ships a real chain the engine
/// trilinear-samples (see the FontPage::mips law).
unsafe fn create_immutable_texture_mips(
    gfx: &Gfx,
    levels: &[(usize, usize, &[u8])],
    format: u32,
    role: &str,
) -> Result<(
    ComPtr<ID3D11Texture2D>,
    ComPtr<ID3D11ShaderResourceView>,
)> {
    use winapi::shared::dxgitype::DXGI_SAMPLE_DESC;
    use winapi::um::d3d11::{
        D3D11_BIND_SHADER_RESOURCE, D3D11_SUBRESOURCE_DATA, D3D11_TEXTURE2D_DESC,
        D3D11_USAGE_IMMUTABLE,
    };
    let (w0, h0, _) = levels.first().copied().ok_or("mip texture needs levels")?;
    let mut expect_w = w0;
    let mut expect_h = h0;
    let mut init: Vec<D3D11_SUBRESOURCE_DATA> = Vec::with_capacity(levels.len());
    for (li, (w, h, bytes)) in levels.iter().enumerate() {
        if *w != expect_w || *h != expect_h {
            return Err(format!(
                "{role}: mip {li} is {w}x{h}, expected {expect_w}x{expect_h}"
            )
            .into());
        }
        if bytes.len() != w * h * 4 {
            return Err(format!(
                "{role}: mip {li} has {} bytes, expected {}",
                bytes.len(),
                w * h * 4
            )
            .into());
        }
        init.push(D3D11_SUBRESOURCE_DATA {
            pSysMem: bytes.as_ptr() as *const _,
            SysMemPitch: (w * 4) as UINT,
            SysMemSlicePitch: 0,
        });
        expect_w = (expect_w / 2).max(1);
        expect_h = (expect_h / 2).max(1);
    }
    let desc = D3D11_TEXTURE2D_DESC {
        Width: w0 as UINT,
        Height: h0 as UINT,
        MipLevels: levels.len() as UINT,
        ArraySize: 1,
        Format: format,
        SampleDesc: DXGI_SAMPLE_DESC {
            Count: 1,
            Quality: 0,
        },
        Usage: D3D11_USAGE_IMMUTABLE,
        BindFlags: D3D11_BIND_SHADER_RESOURCE,
        CPUAccessFlags: 0,
        MiscFlags: 0,
    };
    let mut texture: *mut ID3D11Texture2D = ptr::null_mut();
    let hr = (*gfx.device.as_raw()).CreateTexture2D(&desc, init.as_ptr(), &mut texture);
    hr_ok(hr, &format!("CreateTexture2D (mips) for {role}"))?;
    let texture = ComPtr::from_raw(texture, role)?;
    let mut srv: *mut ID3D11ShaderResourceView = ptr::null_mut();
    let hr = (*gfx.device.as_raw()).CreateShaderResourceView(
        texture.as_raw() as *mut _,
        ptr::null(),
        &mut srv,
    );
    hr_ok(hr, &format!("CreateShaderResourceView (mips) for {role}"))?;
    let srv = ComPtr::from_raw(srv, role)?;
    Ok((texture, srv))
}

/// Atlas sampling filter for a [`Pipeline`]. Overlay text and solid rects use
/// `Point` (1:1 authored bitmap pixels, byte-stable). Map labels use `Linear` to
/// match `gfx/FX/mapname.shader` (MagFilter/MinFilter/MipFilter = "Linear"),
/// since they are scaled and rotated by the camera and point-sampling a scaled,
/// rotated pre-coloured atlas aliases the outline.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum TextureFilter {
    Point,
    Linear,
}

/// Shared pipeline objects for the overlay shader.
struct Pipeline {
    /// LABHDR1 target override: (raw RTV pointer, width, height). The RTV is
    /// owned by the renderer's captured pass and outlives the draw call; the
    /// pointer is only dereferenced inside `draw` on the same thread.
    target_override: std::cell::Cell<Option<(usize, f32, f32)>>,
    vs: ComPtr<ID3D11VertexShader>,
    ps: ComPtr<ID3D11PixelShader>,
    input_layout: ComPtr<ID3D11InputLayout>,
    sampler: ComPtr<ID3D11SamplerState>,
    blend: ComPtr<ID3D11BlendState>,
    screen_cb: ComPtr<ID3D11Buffer>,
    vertex_buffer: ComPtr<ID3D11Buffer>,
}

impl Pipeline {
    fn new(gfx: &Gfx) -> Result<Pipeline> {
        Self::with_filter(gfx, TextureFilter::Point)
    }

    fn with_filter(gfx: &Gfx, filter: TextureFilter) -> Result<Pipeline> {
        unsafe {
            let vs_blob = compile_shader(OVERLAY_SHADER_HLSL, "vs_main", "vs_4_0")?;
            let ps_blob = compile_shader(OVERLAY_SHADER_HLSL, "ps_main", "ps_4_0")?;

            let mut vs: *mut ID3D11VertexShader = ptr::null_mut();
            let hr = (*gfx.device.as_raw()).CreateVertexShader(
                (*vs_blob.as_raw()).GetBufferPointer(),
                (*vs_blob.as_raw()).GetBufferSize(),
                ptr::null_mut(),
                &mut vs,
            );
            hr_ok(hr, "CreateVertexShader for the overlay shader")?;
            let vs = ComPtr::from_raw(vs, "overlay vertex shader")?;

            let mut ps: *mut ID3D11PixelShader = ptr::null_mut();
            let hr = (*gfx.device.as_raw()).CreatePixelShader(
                (*ps_blob.as_raw()).GetBufferPointer(),
                (*ps_blob.as_raw()).GetBufferSize(),
                ptr::null_mut(),
                &mut ps,
            );
            hr_ok(hr, "CreatePixelShader for the overlay shader")?;
            let ps = ComPtr::from_raw(ps, "overlay pixel shader")?;

            let position = CString::new("POSITION").expect("static name");
            let texcoord = CString::new("TEXCOORD").expect("static name");
            let color = CString::new("COLOR").expect("static name");
            let elements = [
                D3D11_INPUT_ELEMENT_DESC {
                    SemanticName: position.as_ptr(),
                    SemanticIndex: 0,
                    Format: DXGI_FORMAT_R32G32_FLOAT,
                    InputSlot: 0,
                    AlignedByteOffset: 0,
                    InputSlotClass: D3D11_INPUT_PER_VERTEX_DATA,
                    InstanceDataStepRate: 0,
                },
                D3D11_INPUT_ELEMENT_DESC {
                    SemanticName: texcoord.as_ptr(),
                    SemanticIndex: 0,
                    Format: DXGI_FORMAT_R32G32_FLOAT,
                    InputSlot: 0,
                    AlignedByteOffset: 8,
                    InputSlotClass: D3D11_INPUT_PER_VERTEX_DATA,
                    InstanceDataStepRate: 0,
                },
                D3D11_INPUT_ELEMENT_DESC {
                    SemanticName: color.as_ptr(),
                    SemanticIndex: 0,
                    Format: DXGI_FORMAT_R8G8B8A8_UNORM,
                    InputSlot: 0,
                    AlignedByteOffset: 16,
                    InputSlotClass: D3D11_INPUT_PER_VERTEX_DATA,
                    InstanceDataStepRate: 0,
                },
            ];
            let mut input_layout: *mut ID3D11InputLayout = ptr::null_mut();
            let hr = (*gfx.device.as_raw()).CreateInputLayout(
                elements.as_ptr(),
                elements.len() as UINT,
                (*vs_blob.as_raw()).GetBufferPointer(),
                (*vs_blob.as_raw()).GetBufferSize(),
                &mut input_layout,
            );
            hr_ok(hr, "CreateInputLayout for the overlay shader")?;
            let input_layout = ComPtr::from_raw(input_layout, "overlay input layout")?;

            let mut sampler_desc: D3D11_SAMPLER_DESC = mem::zeroed();
            // Point for overlay/rects (1:1 authored bitmap pixels); Linear for
            // the scaled/rotated map labels (mapname.shader analog).
            sampler_desc.Filter = match filter {
                TextureFilter::Point => D3D11_FILTER_MIN_MAG_MIP_POINT,
                TextureFilter::Linear => D3D11_FILTER_MIN_MAG_MIP_LINEAR,
            };
            // FILE LAW: `gfx/FX/mapname.shader` Samplers declare
            // `MipMapLodBias = -0.4` for the label atlas; the overlay path
            // (single-mip Point pages) keeps 0.
            sampler_desc.MipLODBias = match filter {
                TextureFilter::Point => 0.0,
                TextureFilter::Linear => -0.4,
            };
            sampler_desc.MaxLOD = f32::MAX;
            sampler_desc.AddressU = D3D11_TEXTURE_ADDRESS_CLAMP;
            sampler_desc.AddressV = D3D11_TEXTURE_ADDRESS_CLAMP;
            sampler_desc.AddressW = D3D11_TEXTURE_ADDRESS_CLAMP;
            sampler_desc.ComparisonFunc = D3D11_COMPARISON_NEVER;
            let mut sampler: *mut ID3D11SamplerState = ptr::null_mut();
            let hr = (*gfx.device.as_raw()).CreateSamplerState(&sampler_desc, &mut sampler);
            hr_ok(hr, "CreateSamplerState for the overlay")?;
            let sampler = ComPtr::from_raw(sampler, "overlay sampler")?;

            let mut blend_desc: D3D11_BLEND_DESC = mem::zeroed();
            blend_desc.RenderTarget[0].BlendEnable = 1;
            blend_desc.RenderTarget[0].SrcBlend = D3D11_BLEND_SRC_ALPHA;
            blend_desc.RenderTarget[0].DestBlend = D3D11_BLEND_INV_SRC_ALPHA;
            blend_desc.RenderTarget[0].BlendOp = D3D11_BLEND_OP_ADD;
            blend_desc.RenderTarget[0].SrcBlendAlpha = D3D11_BLEND_ONE;
            blend_desc.RenderTarget[0].DestBlendAlpha = D3D11_BLEND_INV_SRC_ALPHA;
            blend_desc.RenderTarget[0].BlendOpAlpha = D3D11_BLEND_OP_ADD;
            blend_desc.RenderTarget[0].RenderTargetWriteMask = D3D11_COLOR_WRITE_ENABLE_ALL as u8;
            let mut blend: *mut ID3D11BlendState = ptr::null_mut();
            let hr = (*gfx.device.as_raw()).CreateBlendState(&blend_desc, &mut blend);
            hr_ok(hr, "CreateBlendState for the overlay")?;
            let blend = ComPtr::from_raw(blend, "overlay blend state")?;

            let cb_desc = D3D11_BUFFER_DESC {
                ByteWidth: mem::size_of::<ScreenConstants>() as UINT,
                Usage: D3D11_USAGE_DYNAMIC,
                BindFlags: D3D11_BIND_CONSTANT_BUFFER,
                CPUAccessFlags: D3D11_CPU_ACCESS_WRITE,
                MiscFlags: 0,
                StructureByteStride: 0,
            };
            let mut screen_cb: *mut ID3D11Buffer = ptr::null_mut();
            let hr = (*gfx.device.as_raw()).CreateBuffer(&cb_desc, ptr::null(), &mut screen_cb);
            hr_ok(hr, "CreateBuffer for overlay screen constants")?;
            let screen_cb = ComPtr::from_raw(screen_cb, "overlay screen constant buffer")?;

            let vb_desc = D3D11_BUFFER_DESC {
                ByteWidth: (VERTEX_CAPACITY * mem::size_of::<Vertex>()) as UINT,
                Usage: D3D11_USAGE_DYNAMIC,
                BindFlags: D3D11_BIND_VERTEX_BUFFER,
                CPUAccessFlags: D3D11_CPU_ACCESS_WRITE,
                MiscFlags: 0,
                StructureByteStride: 0,
            };
            let mut vertex_buffer: *mut ID3D11Buffer = ptr::null_mut();
            let hr = (*gfx.device.as_raw()).CreateBuffer(&vb_desc, ptr::null(), &mut vertex_buffer);
            hr_ok(hr, "CreateBuffer for the overlay vertex buffer")?;
            let vertex_buffer = ComPtr::from_raw(vertex_buffer, "overlay vertex buffer")?;

            Ok(Pipeline {
            target_override: std::cell::Cell::new(None),
                vs,
                ps,
                input_layout,
                sampler,
                blend,
                screen_cb,
                vertex_buffer,
            })
        }
    }

    /// Draw `vertices` (triangle list) with `srv` bound as the page,
    /// chunking through the dynamic vertex buffer. Restores default blend
    /// state afterwards so the map pass is unaffected next frame.
    ///
    /// LABHDR1: when a target override is set (see
    /// [`TextRenderer::set_scene_target`]) the pass renders into that RTV at
    /// its dimensions instead of the window backbuffer — the path that puts
    /// country names INSIDE the HDR MainScene like the engine's mapname pass.
    fn draw(
        &self,
        gfx: &Gfx,
        srv: *mut ID3D11ShaderResourceView,
        vertices: &[Vertex],
    ) -> Result<()> {
        if vertices.is_empty() || gfx.width == 0 || gfx.height == 0 {
            return Ok(());
        }
        let over = self.target_override.get();
        let (tw, th) = match over {
            Some((_, w, h)) => (w, h),
            None => (gfx.width as f32, gfx.height as f32),
        };
        unsafe {
            let context = gfx.context.as_raw();

            let constants = ScreenConstants {
                screen_size: [tw, th],
                _pad: [0.0, 0.0],
            };
            let mut mapped: D3D11_MAPPED_SUBRESOURCE = mem::zeroed();
            let hr = (*context).Map(
                self.screen_cb.as_raw() as *mut _,
                0,
                D3D11_MAP_WRITE_DISCARD,
                0,
                &mut mapped,
            );
            hr_ok(hr, "Map overlay screen constants")?;
            ptr::copy_nonoverlapping(
                &constants as *const ScreenConstants as *const u8,
                mapped.pData as *mut u8,
                mem::size_of::<ScreenConstants>(),
            );
            (*context).Unmap(self.screen_cb.as_raw() as *mut _, 0);

            let rtvs = [match over {
                Some((rtv, _, _)) => rtv as *mut ID3D11RenderTargetView,
                None => gfx.rtv()?.as_raw(),
            }];
            (*context).OMSetRenderTargets(1, rtvs.as_ptr(), ptr::null_mut());
            let viewport = D3D11_VIEWPORT {
                TopLeftX: 0.0,
                TopLeftY: 0.0,
                Width: tw,
                Height: th,
                MinDepth: 0.0,
                MaxDepth: 1.0,
            };
            (*context).RSSetViewports(1, &viewport);
            (*context).IASetPrimitiveTopology(D3D11_PRIMITIVE_TOPOLOGY_TRIANGLELIST);
            (*context).IASetInputLayout(self.input_layout.as_raw());
            let stride = mem::size_of::<Vertex>() as UINT;
            let offset = 0u32;
            let buffers = [self.vertex_buffer.as_raw()];
            (*context).IASetVertexBuffers(0, 1, buffers.as_ptr(), &stride, &offset);
            (*context).VSSetShader(self.vs.as_raw(), ptr::null(), 0);
            let constant_buffers = [self.screen_cb.as_raw()];
            (*context).VSSetConstantBuffers(0, 1, constant_buffers.as_ptr());
            (*context).PSSetShader(self.ps.as_raw(), ptr::null(), 0);
            let samplers = [self.sampler.as_raw()];
            (*context).PSSetSamplers(0, 1, samplers.as_ptr());
            let srvs = [srv];
            (*context).PSSetShaderResources(0, 1, srvs.as_ptr());
            let blend_factor = [0.0f32; 4];
            (*context).OMSetBlendState(self.blend.as_raw(), &blend_factor, 0xFFFF_FFFF);

            // Whole triangles per chunk (capacity is a multiple of 3).
            for chunk in vertices.chunks(VERTEX_CAPACITY - VERTEX_CAPACITY % 3) {
                let mut mapped: D3D11_MAPPED_SUBRESOURCE = mem::zeroed();
                let hr = (*context).Map(
                    self.vertex_buffer.as_raw() as *mut _,
                    0,
                    D3D11_MAP_WRITE_DISCARD,
                    0,
                    &mut mapped,
                );
                hr_ok(hr, "Map overlay vertex buffer")?;
                ptr::copy_nonoverlapping(
                    chunk.as_ptr() as *const u8,
                    mapped.pData as *mut u8,
                    chunk.len() * mem::size_of::<Vertex>(),
                );
                (*context).Unmap(self.vertex_buffer.as_raw() as *mut _, 0);
                (*context).Draw(chunk.len() as UINT, 0);
            }

            // Back to default opaque blending for the next map pass.
            (*context).OMSetBlendState(ptr::null_mut(), &blend_factor, 0xFFFF_FFFF);
        }
        Ok(())
    }
}

/// Solid-rect renderer (the scrub bar path). Independent of any font so
/// the bar can still draw when font loading failed closed.
pub struct QuadRenderer {
    pipeline: Pipeline,
    _white_texture: ComPtr<ID3D11Texture2D>,
    white_srv: ComPtr<ID3D11ShaderResourceView>,
}

impl QuadRenderer {
    pub fn new(gfx: &Gfx) -> Result<QuadRenderer> {
        let pipeline = Pipeline::new(gfx)?;
        let (white_texture, white_srv) = unsafe {
            create_immutable_texture(
                gfx,
                1,
                1,
                DXGI_FORMAT_R8G8B8A8_UNORM,
                &[0xFF, 0xFF, 0xFF, 0xFF],
                "overlay 1x1 white texel",
            )?
        };
        Ok(QuadRenderer {
            pipeline,
            _white_texture: white_texture,
            white_srv,
        })
    }

    /// Draw all rects in a single batch.
    pub fn draw_rects(&self, gfx: &Gfx, rects: &[SolidRect]) -> Result<()> {
        let mut vertices = Vec::with_capacity(rects.len() * 6);
        for rect in rects {
            push_rect_vertices(&mut vertices, rect);
        }
        self.pipeline.draw(gfx, self.white_srv.as_raw(), &vertices)
    }
}

/// One screen-space sprite blit request for [`SpriteRenderer::draw`]:
/// destination rect in window pixels, source rect in normalized UVs, and an
/// `0xAARRGGBB` tint (white = pass the texture through, the GUI convention).
#[derive(Clone, Copy, Debug)]
pub struct SpriteQuad {
    pub x0: f32,
    pub y0: f32,
    pub x1: f32,
    pub y1: f32,
    pub u0: f32,
    pub v0: f32,
    pub u1: f32,
    pub v1: f32,
    pub color: u32,
}

/// Screen-space sprite renderer over ONE immutable RGBA texture (e.g. the VP
/// map-icon strip). Linear-sampled: the engine renders mapicon sprites
/// smoothly at non-integer scales (the measured on-screen VP dot is ~1.25x
/// the authored frame), so point sampling would alias the scaled blit.
pub struct SpriteRenderer {
    pipeline: Pipeline,
    _texture: ComPtr<ID3D11Texture2D>,
    srv: ComPtr<ID3D11ShaderResourceView>,
}

impl SpriteRenderer {
    pub fn new(
        gfx: &Gfx,
        width: usize,
        height: usize,
        rgba: &[u8],
        label: &str,
    ) -> Result<SpriteRenderer> {
        let pipeline = Pipeline::with_filter(gfx, TextureFilter::Linear)?;
        let (texture, srv) = unsafe {
            create_immutable_texture(gfx, width, height, DXGI_FORMAT_R8G8B8A8_UNORM, rgba, label)?
        };
        Ok(SpriteRenderer {
            pipeline,
            _texture: texture,
            srv,
        })
    }

    /// Draw all sprites in one alpha-blended batch.
    pub fn draw(&self, gfx: &Gfx, quads: &[SpriteQuad]) -> Result<()> {
        let mut vertices = Vec::with_capacity(quads.len() * 6);
        for quad in quads {
            push_glyph_vertices(
                &mut vertices,
                &GlyphQuad {
                    page: 0,
                    x0: quad.x0,
                    y0: quad.y0,
                    x1: quad.x1,
                    y1: quad.y1,
                    u0: quad.u0,
                    v0: quad.v0,
                    u1: quad.u1,
                    v1: quad.v1,
                },
                color_to_rgba_bytes(quad.color),
            );
        }
        self.pipeline.draw(gfx, self.srv.as_raw(), &vertices)
    }
}

/// Bitmap-font text renderer over a loaded game [`Font`].
pub struct TextRenderer {
    pipeline: Pipeline,
    font: Font,
    /// One immutable texture + SRV per atlas page.
    pages: Vec<(ComPtr<ID3D11Texture2D>, ComPtr<ID3D11ShaderResourceView>)>,
}

impl TextRenderer {
    pub fn new(gfx: &Gfx, font: &Font) -> Result<TextRenderer> {
        // Pre-coloured atlases (map labels) are scaled/rotated by the camera, so
        // they sample Linear like mapname.shader; coverage atlases (overlay) stay
        // Point so their 1:1 dumps are byte-stable.
        let filter = match font.atlas_mode {
            crate::font::AtlasMode::PreservedColor => TextureFilter::Linear,
            crate::font::AtlasMode::Coverage => TextureFilter::Point,
        };
        // ENGINE-MEASURED (frame4700, NAME2 weight fix): the engine binds the
        // hoi_mapfont4 atlas as DXGI fmt 87 = B8G8R8A8_UNORM — NOT _SRGB —
        // at its mapname draws, so the sRGB-encoded glyph bytes enter the
        // linear HDR MainScene WITHOUT hardware linearization. LABHDR1's
        // first guess (an _SRGB view, "linearize like any sRGB source")
        // rendered the dark glyph bodies ~2.2-gamma darker than the engine —
        // the user-visible heavy/bold titles. Reproducing vanilla means
        // sampling raw UNORM exactly like the engine, even though that is
        // colorimetrically "wrong".
        let atlas_format = DXGI_FORMAT_R8G8B8A8_UNORM;
        let pipeline = Pipeline::with_filter(gfx, filter)?;
        let mut pages = Vec::with_capacity(font.pages.len());
        for (index, page) in font.pages.iter().enumerate() {
            let label = format!("font {} atlas page {index}", font.name);
            let uploaded = unsafe {
                if page.mips.is_empty() {
                    create_immutable_texture(
                        gfx,
                        page.width as usize,
                        page.height as usize,
                        atlas_format,
                        &page.rgba,
                        &label,
                    )?
                } else {
                    // The SHIPPED mip chain (FontPage::mips) — the engine
                    // trilinear-samples it with the -0.4 LOD bias above.
                    let mut levels: Vec<(usize, usize, &[u8])> =
                        vec![(page.width as usize, page.height as usize, &page.rgba)];
                    levels.extend(
                        page.mips
                            .iter()
                            .map(|(w, h, rgba)| (*w as usize, *h as usize, rgba.as_slice())),
                    );
                    create_immutable_texture_mips(gfx, &levels, atlas_format, &label)?
                }
            };
            pages.push(uploaded);
        }
        Ok(TextRenderer {
            pipeline,
            font: font.clone(),
            pages,
        })
    }

    pub fn font(&self) -> &Font {
        &self.font
    }

    /// LABHDR1: route subsequent text draws into `rtv` (dimensions `w`x`h`)
    /// instead of the window backbuffer. The RTV must stay alive for the
    /// duration; callers MUST call [`Self::clear_scene_target`] afterwards.
    pub fn set_scene_target(&self, rtv: *mut ID3D11RenderTargetView, w: f32, h: f32) {
        self.pipeline.target_override.set(Some((rtv as usize, w, h)));
    }

    pub fn clear_scene_target(&self) {
        self.pipeline.target_override.set(None);
    }

    /// Measure one line of text: (width, line height) in pixels.
    /// The overlay currently only needs [`TextRenderer::draw_text`]; this
    /// is the other half of the API surface for upcoming counter/panel
    /// layout work.
    #[allow(dead_code)]
    pub fn measure(&self, text: &str) -> (u32, u32) {
        self.font.measure(text)
    }

    /// Draw one line centered on (`center_x`, `center_y`) with glyph quads
    /// scaled by `scale` (map-label fallback: label size tracks the map
    /// zoom instead of the fixed overlay pixel size). `color` is
    /// `0xAARRGGBB`; alpha rides the same src_alpha/inv_src_alpha blend the
    /// overlay text uses (the mapname.shader analog).
    pub fn draw_text_scaled_centered(
        &self,
        gfx: &Gfx,
        center_x: f32,
        center_y: f32,
        scale: f32,
        text: &str,
        color: u32,
    ) -> Result<Vec<char>> {
        let layout = self.font.layout_text(0.0, 0.0, text);
        let (width, height) = self.font.measure(text);
        let origin_x = center_x - width as f32 * scale * 0.5;
        let origin_y = center_y - height as f32 * scale * 0.5;
        let rgba = color_to_rgba_bytes(color);
        for (page_index, (_, srv)) in self.pages.iter().enumerate() {
            let mut vertices = Vec::new();
            for quad in layout.quads.iter().filter(|quad| quad.page == page_index) {
                let scaled = GlyphQuad {
                    page: quad.page,
                    x0: origin_x + quad.x0 * scale,
                    y0: origin_y + quad.y0 * scale,
                    x1: origin_x + quad.x1 * scale,
                    y1: origin_y + quad.y1 * scale,
                    u0: quad.u0,
                    v0: quad.v0,
                    u1: quad.u1,
                    v1: quad.v1,
                };
                push_glyph_vertices(&mut vertices, &scaled, rgba);
            }
            self.pipeline.draw(gfx, srv.as_raw(), &vertices)?;
        }
        Ok(layout.missing)
    }

    /// Draw one line of text as a curved, oriented baseline: each glyph is
    /// placed at, and rotated to, a point supplied by `place`. This backs the
    /// LAB1 country-label law (`map_labels.rs`) where the baseline follows the
    /// country's principal axis and bows along its shape.
    ///
    /// `place(u)` maps a horizontal pen offset `u` (in scaled screen pixels,
    /// measured from the baseline's left end) to a screen-space
    /// `(x, y, angle)`: the glyph origin on the baseline and the local tangent
    /// angle (radians, screen space, +angle rotates clockwise). Glyphs are
    /// scaled by `scale` and vertically centred on the baseline (the font's
    /// mid-line sits on the path). `color` is `0xAARRGGBB`. Returns the
    /// characters that had no glyph.
    pub fn draw_text_curved(
        &self,
        gfx: &Gfx,
        scale: f32,
        text: &str,
        color: u32,
        place: impl Fn(f32) -> (f32, f32, f32),
    ) -> Result<Vec<char>> {
        self.draw_text_curved_tracked(gfx, scale, text, color, 0.0, place)
    }

    /// [`Self::draw_text_curved`] with uniform additive letter tracking
    /// (`tracking` in FONT px per character slot — the engine's country-name
    /// justification law; see `BmFont::layout_text_tracked`).
    pub fn draw_text_curved_tracked(
        &self,
        gfx: &Gfx,
        scale: f32,
        text: &str,
        color: u32,
        tracking: f32,
        place: impl Fn(f32) -> (f32, f32, f32),
    ) -> Result<Vec<char>> {
        let layout = self.font.layout_text_tracked(0.0, 0.0, text, tracking);
        // Vertically centre glyphs on the baseline: shift so the font's
        // mid-line sits on the path (BMFont lays out from the line top).
        let half_line = self.font.line_height as f32 * 0.5;
        let rgba = color_to_rgba_bytes(color);
        for (page_index, (_, srv)) in self.pages.iter().enumerate() {
            let mut vertices = Vec::new();
            for quad in layout.quads.iter().filter(|quad| quad.page == page_index) {
                // Pen offset of this glyph's box centre along the baseline.
                let pen_u = (quad.x0 + quad.x1) * 0.5 * scale;
                let (bx, by, angle) = place(pen_u);
                let (sin, cos) = angle.sin_cos();
                // Glyph-local corners relative to the pen point along the
                // baseline: x measured from the glyph box centre, y from the
                // font vertical centre (so the mid-line rides the path).
                let cx = (quad.x0 + quad.x1) * 0.5;
                let corners = [
                    (quad.x0 - cx, quad.y0 - half_line),
                    (quad.x1 - cx, quad.y0 - half_line),
                    (quad.x1 - cx, quad.y1 - half_line),
                    (quad.x0 - cx, quad.y1 - half_line),
                ];
                let mut screen = [[0.0f32; 2]; 4];
                for (i, (lx, ly)) in corners.iter().enumerate() {
                    let sx = lx * scale;
                    let sy = ly * scale;
                    screen[i] = [bx + sx * cos - sy * sin, by + sx * sin + sy * cos];
                }
                let uvs = [
                    [quad.u0, quad.v0],
                    [quad.u1, quad.v0],
                    [quad.u1, quad.v1],
                    [quad.u0, quad.v1],
                ];
                let mk = |i: usize| Vertex {
                    pos: screen[i],
                    uv: uvs[i],
                    color: rgba,
                };
                vertices.extend_from_slice(&[mk(0), mk(1), mk(2), mk(0), mk(2), mk(3)]);
            }
            self.pipeline.draw(gfx, srv.as_raw(), &vertices)?;
        }
        Ok(layout.missing)
    }

    /// [`Self::draw_text_curved_tracked`] with the glyphs emitted as
    /// caller-projected quads — the engine `mapname` VB semantics: glyph quads
    /// are laid out in WORLD space flat on the map plane (LAB3: the captured
    /// TL→BL cap edge lies in the XZ plane, origins terrain-draped) and every
    /// corner runs through the 3D view-projection, so text foreshortens with
    /// camera tilt and shrinks with distance — a uniform screen `scale`
    /// cannot reproduce either.
    ///
    /// `corner(pen_u, local_x, local_y) -> Option<[x, y]>` maps a glyph-corner
    /// in FONT px — `pen_u` the glyph box centre's offset from the text start
    /// (tracking included), `local_x`/`local_y` the corner offset from the box
    /// centre / font mid-line — to screen px on the current target. `None`
    /// skips the whole glyph (fail closed off-view). `color` is `0xAARRGGBB`.
    /// Returns the characters that had no glyph.
    pub fn draw_text_curved_projected(
        &self,
        gfx: &Gfx,
        text: &str,
        color: u32,
        tracking: f32,
        corner: impl Fn(f32, f32, f32) -> Option<[f32; 2]>,
    ) -> Result<Vec<char>> {
        let layout = self.font.layout_text_tracked(0.0, 0.0, text, tracking);
        // Same baseline convention as the screen-space path: the font's
        // mid-line rides the baseline path.
        let half_line = self.font.line_height as f32 * 0.5;
        let rgba = color_to_rgba_bytes(color);
        for (page_index, (_, srv)) in self.pages.iter().enumerate() {
            let mut vertices = Vec::new();
            'quads: for quad in layout.quads.iter().filter(|quad| quad.page == page_index) {
                let cx = (quad.x0 + quad.x1) * 0.5;
                let corners = [
                    (quad.x0 - cx, quad.y0 - half_line),
                    (quad.x1 - cx, quad.y0 - half_line),
                    (quad.x1 - cx, quad.y1 - half_line),
                    (quad.x0 - cx, quad.y1 - half_line),
                ];
                let mut screen = [[0.0f32; 2]; 4];
                for (i, (lx, ly)) in corners.iter().enumerate() {
                    match corner(cx, *lx, *ly) {
                        Some(point) => screen[i] = point,
                        None => continue 'quads,
                    }
                }
                let uvs = [
                    [quad.u0, quad.v0],
                    [quad.u1, quad.v0],
                    [quad.u1, quad.v1],
                    [quad.u0, quad.v1],
                ];
                let mk = |i: usize| Vertex {
                    pos: screen[i],
                    uv: uvs[i],
                    color: rgba,
                };
                vertices.extend_from_slice(&[mk(0), mk(1), mk(2), mk(0), mk(2), mk(3)]);
            }
            self.pipeline.draw(gfx, srv.as_raw(), &vertices)?;
        }
        Ok(layout.missing)
    }

    /// Draw one line of text with its top-left at (`x`, `y`) window
    /// pixels; `color` is `0xAARRGGBB`. Returns the characters that had no
    /// glyph (skipped and recorded, never substituted).
    pub fn draw_text(
        &self,
        gfx: &Gfx,
        x: f32,
        y: f32,
        text: &str,
        color: u32,
    ) -> Result<Vec<char>> {
        let layout = self.font.layout_text(x, y, text);
        let rgba = color_to_rgba_bytes(color);
        // Batch per page (the fonts in use today have a single page).
        for (page_index, (_, srv)) in self.pages.iter().enumerate() {
            let mut vertices = Vec::new();
            for quad in layout.quads.iter().filter(|quad| quad.page == page_index) {
                push_glyph_vertices(&mut vertices, quad, rgba);
            }
            self.pipeline.draw(gfx, srv.as_raw(), &vertices)?;
        }
        Ok(layout.missing)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn color_converts_argb_to_rgba_byte_order() {
        // 0xAARRGGBB -> bytes r,g,b,a (little-endian u32).
        assert_eq!(color_to_rgba_bytes(0xFF00_0000), 0xFF00_0000);
        assert_eq!(color_to_rgba_bytes(0x0000_00FF), 0x00FF_0000);
        assert_eq!(color_to_rgba_bytes(0x00FF_0000), 0x0000_00FF);
        assert_eq!(
            color_to_rgba_bytes(0x8010_2030),
            0x10 | (0x20 << 8) | (0x30 << 16) | (0x80 << 24)
        );
    }

    #[test]
    fn glyph_quad_expands_to_two_ccw_screen_triangles() {
        let mut vertices = Vec::new();
        push_glyph_vertices(
            &mut vertices,
            &GlyphQuad {
                page: 0,
                x0: 1.0,
                y0: 2.0,
                x1: 3.0,
                y1: 6.0,
                u0: 0.1,
                v0: 0.2,
                u1: 0.3,
                v1: 0.4,
            },
            0xAABB_CCDD,
        );
        assert_eq!(vertices.len(), 6);
        assert_eq!(vertices[0].pos, [1.0, 2.0]);
        assert_eq!(vertices[0].uv, [0.1, 0.2]);
        assert_eq!(vertices[2].pos, [3.0, 6.0]);
        assert_eq!(vertices[2].uv, [0.3, 0.4]);
        // Both triangles share the top-left and bottom-right corners.
        assert_eq!(vertices[3], vertices[0]);
        assert_eq!(vertices[4], vertices[2]);
        assert_eq!(vertices[5].pos, [1.0, 6.0]);
        assert!(vertices.iter().all(|vertex| vertex.color == 0xAABB_CCDD));
    }

    #[test]
    fn rect_vertices_pin_uv_to_white_texel_center() {
        let mut vertices = Vec::new();
        push_rect_vertices(
            &mut vertices,
            &SolidRect {
                x: 10.0,
                y: 20.0,
                width: 5.0,
                height: 2.0,
                color: 0xFF11_2233,
            },
        );
        assert_eq!(vertices.len(), 6);
        assert!(vertices.iter().all(|vertex| vertex.uv == [0.5, 0.5]));
        assert_eq!(vertices[2].pos, [15.0, 22.0]);
        assert_eq!(vertices[0].color, color_to_rgba_bytes(0xFF11_2233));
    }
}
