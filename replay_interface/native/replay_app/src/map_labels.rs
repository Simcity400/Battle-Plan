//! World-space country-label vertex-buffer path for the captured the game
//! `mapname` pass.
//!
//! This module intentionally does not enable labels in product CLI. It provides
//! the product render surface that was missing: replay_app builds float3/float2
//! glyph vertices by calling the proven `map_viewer` mapname geometry law, then
//! draws them with the captured shader semantics (`ViewProjectionMatrix`,
//! `Transp_OffsetX.y` wrap offset, `Transp_OffsetX.x` alpha multiplier).
#![allow(dead_code)]

use std::ffi::CString;
use std::mem;
use std::ptr;

use map_viewer::{
    build_mapname_vertices, mapname_measured_glyph_parameters, mapname_tracked_advance_px,
    mapname_tracking_px, CountryLabelPlacement, MapLabelGlyphLayoutQuad,
};
use winapi::shared::dxgiformat::{
    DXGI_FORMAT_R32G32B32_FLOAT, DXGI_FORMAT_R32G32_FLOAT, DXGI_FORMAT_R8G8B8A8_UNORM,
};
use winapi::shared::dxgitype::DXGI_SAMPLE_DESC;
use winapi::shared::minwindef::UINT;
use winapi::um::d3d11::{
    ID3D11BlendState, ID3D11Buffer, ID3D11InputLayout, ID3D11PixelShader, ID3D11SamplerState,
    ID3D11ShaderResourceView, ID3D11Texture2D, ID3D11VertexShader, D3D11_BIND_CONSTANT_BUFFER,
    D3D11_BIND_SHADER_RESOURCE, D3D11_BIND_VERTEX_BUFFER, D3D11_BLEND_DESC,
    D3D11_BLEND_INV_SRC_ALPHA, D3D11_BLEND_ONE, D3D11_BLEND_OP_ADD, D3D11_BLEND_SRC_ALPHA,
    D3D11_BUFFER_DESC, D3D11_COLOR_WRITE_ENABLE_ALL, D3D11_COMPARISON_ALWAYS,
    D3D11_CPU_ACCESS_WRITE, D3D11_FILTER_MIN_MAG_MIP_LINEAR, D3D11_INPUT_ELEMENT_DESC,
    D3D11_INPUT_PER_VERTEX_DATA, D3D11_MAPPED_SUBRESOURCE, D3D11_MAP_WRITE_DISCARD,
    D3D11_SAMPLER_DESC, D3D11_SUBRESOURCE_DATA, D3D11_TEXTURE2D_DESC, D3D11_TEXTURE_ADDRESS_CLAMP,
    D3D11_USAGE_DYNAMIC, D3D11_USAGE_IMMUTABLE, D3D11_VIEWPORT,
};
use winapi::um::d3dcommon::D3D11_PRIMITIVE_TOPOLOGY_TRIANGLELIST;

use crate::d3d::{hr_ok, ComPtr, Gfx};
use crate::font::{load_game_font_preserving_color, Font};
use crate::render::{compile_shader, create_immutable_texture};
use crate::Result;

/// `interface/core.gfx:1679-1680` declares the map-name atlas as bitmapfont
/// `tahoma_60` backed by `gfx/fonts/hoi_mapfont4`.
pub const MAP_LABEL_FONT_NAME: &str = "tahoma_60";

const MAP_LABEL_SHADER_HLSL: &str = r#"
cbuffer SceneConstants : register(b0) {
    float4x4 ViewProjectionMatrix;
    float4 vCamLookAtDir;
};

cbuffer TranspOffset : register(b1) {
    float4 Transp_OffsetX;
    float4 _TranspOffsetPad;
};

Texture2D<float4> DiffuseTexture : register(t0);
SamplerState DiffuseSampler : register(s0);

struct VSIn {
    float3 vPosition : POSITION;
    float2 vTexCoord : TEXCOORD0;
};

struct VSOut {
    float4 position : SV_Position;
    float3 prepos : TEXCOORD0;
    float2 uv : TEXCOORD1;
};

VSOut vs_main(VSIn input) {
    VSOut output;
    float4 pos = float4(input.vPosition, 1.0f);
    pos.x += Transp_OffsetX.y;
    output.position = mul(ViewProjectionMatrix, pos) + vCamLookAtDir * 0.0f;
    output.prepos = input.vPosition.xyz;
    output.uv = input.vTexCoord;
    return output;
}

float4 ps_main(VSOut input) : SV_Target {
    float4 sample = DiffuseTexture.Sample(DiffuseSampler, input.uv);
    sample.a *= Transp_OffsetX.x;
    return sample;
}
"#;

const VERTEX_CAPACITY: usize = 6 * 4096;

#[repr(C)]
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct MapLabelVertex {
    pub position: [f32; 3],
    pub uv: [f32; 2],
}

#[derive(Clone, Debug, PartialEq)]
pub struct MapLabelVertexBatch {
    pub page: usize,
    pub vertices: Vec<MapLabelVertex>,
}

#[repr(C)]
#[derive(Clone, Copy)]
struct MapLabelSceneConstants {
    view_projection: [[f32; 4]; 4],
    cam_look_at_dir: [f32; 4],
}

#[repr(C)]
#[derive(Clone, Copy)]
struct MapLabelTranspOffset {
    transp_offset_x: [f32; 4],
    _pad: [f32; 4],
}

pub fn load_map_label_font(resolver: &map_viewer::GameFileResolver) -> Result<Font> {
    load_game_font_preserving_color(resolver, MAP_LABEL_FONT_NAME)
        .map_err(|err| format!("map labels: load {MAP_LABEL_FONT_NAME}: {err}").into())
}

pub fn build_label_vertex_batches(
    font: &Font,
    placement: &CountryLabelPlacement,
    text: &str,
    map_height: f32,
    world_y: f32,
) -> Result<Vec<MapLabelVertexBatch>> {
    let metrics = font.label_text_metrics(text)?;
    let tracking = mapname_tracking_px(placement, &metrics);
    let tracked_advance = mapname_tracked_advance_px(&metrics, tracking);
    let natural_width: f32 = metrics.advances.iter().map(|(advance, _)| *advance).sum();
    let layout = font.layout_text_tracked(0.0, 0.0, text, tracking);
    if !layout.missing.is_empty() {
        return Err(format!(
            "map label {:?} cannot build VB: missing glyphs {:?}",
            text, layout.missing
        )
        .into());
    }

    // Recover the engine's direct spline parameter over the complete laid-out
    // glyph sequence before atlas-page batching. Page filtering must not alter
    // Euclidean station spacing (including spaces and cross-page glyphs).
    let mut all_glyphs: Vec<MapLabelGlyphLayoutQuad> = layout
        .quads
        .iter()
        .map(|quad| MapLabelGlyphLayoutQuad {
            x0: quad.x0,
            y0: quad.y0,
            x1: quad.x1,
            y1: quad.y1,
            u0: quad.u0,
            v0: quad.v0,
            u1: quad.u1,
            v1: quad.v1,
            measured_t: 0.0,
        })
        .collect();
    let measured = mapname_measured_glyph_parameters(&all_glyphs)
        .ok_or_else(|| format!("map label {:?} has invalid straight-layout stations", text))?;
    for (glyph, measured_t) in all_glyphs.iter_mut().zip(measured) {
        glyph.measured_t = measured_t;
    }

    let mut batches = Vec::new();
    for page in 0..font.pages.len() {
        let glyphs: Vec<MapLabelGlyphLayoutQuad> = layout
            .quads
            .iter()
            .zip(&all_glyphs)
            .filter_map(|(quad, glyph)| (quad.page == page).then_some(*glyph))
            .collect();
        if glyphs.is_empty() {
            continue;
        }
        let law_vertices = build_mapname_vertices(
            placement,
            &glyphs,
            natural_width,
            tracked_advance,
            metrics.line_height_px,
            map_height,
            world_y,
        );
        batches.push(MapLabelVertexBatch {
            page,
            vertices: law_vertices
                .into_iter()
                .map(|vertex| MapLabelVertex {
                    position: [vertex.x, vertex.y, vertex.z],
                    uv: [vertex.u, vertex.v],
                })
                .collect(),
        });
    }
    Ok(batches)
}

pub fn label_vertices_to_bytes(vertices: &[MapLabelVertex]) -> Vec<u8> {
    let mut bytes = Vec::with_capacity(vertices.len() * map_viewer::MAPNAME_VERTEX_STRIDE);
    for vertex in vertices {
        for value in vertex.position.into_iter().chain(vertex.uv) {
            bytes.extend_from_slice(&value.to_le_bytes());
        }
    }
    bytes
}

pub struct MapLabelRenderer {
    vs: ComPtr<ID3D11VertexShader>,
    ps: ComPtr<ID3D11PixelShader>,
    input_layout: ComPtr<ID3D11InputLayout>,
    sampler: ComPtr<ID3D11SamplerState>,
    blend: ComPtr<ID3D11BlendState>,
    scene_cb: ComPtr<ID3D11Buffer>,
    transp_cb: ComPtr<ID3D11Buffer>,
    vertex_buffer: ComPtr<ID3D11Buffer>,
    pages: Vec<(ComPtr<ID3D11Texture2D>, ComPtr<ID3D11ShaderResourceView>)>,
}

impl MapLabelRenderer {
    pub fn new(gfx: &Gfx, font: &Font) -> Result<Self> {
        unsafe {
            let vs_blob = compile_shader(MAP_LABEL_SHADER_HLSL, "vs_main", "vs_4_0")?;
            let ps_blob = compile_shader(MAP_LABEL_SHADER_HLSL, "ps_main", "ps_4_0")?;

            let mut vs: *mut ID3D11VertexShader = ptr::null_mut();
            let hr = (*gfx.device.as_raw()).CreateVertexShader(
                (*vs_blob.as_raw()).GetBufferPointer(),
                (*vs_blob.as_raw()).GetBufferSize(),
                ptr::null_mut(),
                &mut vs,
            );
            hr_ok(hr, "CreateVertexShader for map labels")?;
            let vs = ComPtr::from_raw(vs, "map-label vertex shader")?;

            let mut ps: *mut ID3D11PixelShader = ptr::null_mut();
            let hr = (*gfx.device.as_raw()).CreatePixelShader(
                (*ps_blob.as_raw()).GetBufferPointer(),
                (*ps_blob.as_raw()).GetBufferSize(),
                ptr::null_mut(),
                &mut ps,
            );
            hr_ok(hr, "CreatePixelShader for map labels")?;
            let ps = ComPtr::from_raw(ps, "map-label pixel shader")?;

            let position = CString::new("POSITION").expect("static semantic");
            let texcoord = CString::new("TEXCOORD").expect("static semantic");
            let elements = [
                D3D11_INPUT_ELEMENT_DESC {
                    SemanticName: position.as_ptr(),
                    SemanticIndex: 0,
                    Format: DXGI_FORMAT_R32G32B32_FLOAT,
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
                    AlignedByteOffset: 12,
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
            hr_ok(hr, "CreateInputLayout for map labels")?;
            let input_layout = ComPtr::from_raw(input_layout, "map-label input layout")?;

            let sampler = create_map_label_sampler(gfx)?;
            let blend = create_map_label_blend(gfx)?;
            let scene_cb = create_dynamic_constant_buffer::<MapLabelSceneConstants>(
                gfx,
                "map-label scene constants",
            )?;
            let transp_cb = create_dynamic_constant_buffer::<MapLabelTranspOffset>(
                gfx,
                "map-label Transp_OffsetX constants",
            )?;
            let vertex_buffer = create_dynamic_vertex_buffer(gfx)?;

            let mut pages = Vec::with_capacity(font.pages.len());
            for (index, page) in font.pages.iter().enumerate() {
                let label = format!("map-label font {} page {index} UNORM atlas", font.name);
                let uploaded = if page.mips.is_empty() {
                    create_immutable_texture(
                        gfx,
                        page.width as usize,
                        page.height as usize,
                        DXGI_FORMAT_R8G8B8A8_UNORM,
                        &page.rgba,
                        &label,
                    )?
                } else {
                    let mut levels: Vec<(usize, usize, &[u8])> =
                        vec![(page.width as usize, page.height as usize, &page.rgba)];
                    levels.extend(
                        page.mips
                            .iter()
                            .map(|(w, h, rgba)| (*w as usize, *h as usize, rgba.as_slice())),
                    );
                    create_immutable_texture_mips(gfx, &levels, DXGI_FORMAT_R8G8B8A8_UNORM, &label)?
                };
                pages.push(uploaded);
            }

            Ok(Self {
                vs,
                ps,
                input_layout,
                sampler,
                blend,
                scene_cb,
                transp_cb,
                vertex_buffer,
                pages,
            })
        }
    }

    pub fn draw_batches(
        &self,
        gfx: &Gfx,
        batches: &[MapLabelVertexBatch],
        view_projection: [[f32; 4]; 4],
        transparency: f32,
        wrap_offset_x: f32,
    ) -> Result<()> {
        if batches.is_empty() {
            return Ok(());
        }
        unsafe {
            let context = gfx.context.as_raw();
            write_dynamic_buffer(
                context,
                &self.scene_cb,
                &MapLabelSceneConstants {
                    view_projection,
                    cam_look_at_dir: [0.0; 4],
                },
                "Map map-label scene constants",
            )?;
            write_dynamic_buffer(
                context,
                &self.transp_cb,
                &MapLabelTranspOffset {
                    transp_offset_x: [transparency.clamp(0.0, 1.0), wrap_offset_x, 0.0, 0.0],
                    _pad: [0.0; 4],
                },
                "Map map-label Transp_OffsetX constants",
            )?;

            let rtvs = [gfx.rtv()?.as_raw()];
            (*context).OMSetRenderTargets(1, rtvs.as_ptr(), ptr::null_mut());
            let viewport = D3D11_VIEWPORT {
                TopLeftX: 0.0,
                TopLeftY: 0.0,
                Width: gfx.width as f32,
                Height: gfx.height as f32,
                MinDepth: 0.0,
                MaxDepth: 1.0,
            };
            (*context).RSSetViewports(1, &viewport);
            (*context).IASetPrimitiveTopology(D3D11_PRIMITIVE_TOPOLOGY_TRIANGLELIST);
            (*context).IASetInputLayout(self.input_layout.as_raw());
            let stride = mem::size_of::<MapLabelVertex>() as UINT;
            let offset = 0u32;
            let buffers = [self.vertex_buffer.as_raw()];
            (*context).IASetVertexBuffers(0, 1, buffers.as_ptr(), &stride, &offset);
            (*context).VSSetShader(self.vs.as_raw(), ptr::null(), 0);
            (*context).PSSetShader(self.ps.as_raw(), ptr::null(), 0);
            (*context).VSSetConstantBuffers(0, 1, [self.scene_cb.as_raw()].as_ptr());
            (*context).VSSetConstantBuffers(1, 1, [self.transp_cb.as_raw()].as_ptr());
            (*context).PSSetConstantBuffers(1, 1, [self.transp_cb.as_raw()].as_ptr());
            (*context).PSSetSamplers(0, 1, [self.sampler.as_raw()].as_ptr());
            let blend_factor = [0.0f32; 4];
            (*context).OMSetBlendState(self.blend.as_raw(), &blend_factor, u32::MAX);

            for batch in batches {
                let Some((_, srv)) = self.pages.get(batch.page) else {
                    return Err(format!(
                        "map-label batch references missing font page {}",
                        batch.page
                    )
                    .into());
                };
                (*context).PSSetShaderResources(0, 1, [srv.as_raw()].as_ptr());
                for chunk in batch.vertices.chunks(VERTEX_CAPACITY - VERTEX_CAPACITY % 3) {
                    write_dynamic_vertex_chunk(context, &self.vertex_buffer, chunk)?;
                    (*context).Draw(chunk.len() as UINT, 0);
                }
            }
            let null_srv: [*mut ID3D11ShaderResourceView; 1] = [ptr::null_mut()];
            (*context).PSSetShaderResources(0, 1, null_srv.as_ptr());
            (*context).OMSetBlendState(ptr::null_mut(), &blend_factor, u32::MAX);
        }
        Ok(())
    }
}

unsafe fn create_map_label_sampler(gfx: &Gfx) -> Result<ComPtr<ID3D11SamplerState>> {
    let desc = D3D11_SAMPLER_DESC {
        Filter: D3D11_FILTER_MIN_MAG_MIP_LINEAR,
        AddressU: D3D11_TEXTURE_ADDRESS_CLAMP,
        AddressV: D3D11_TEXTURE_ADDRESS_CLAMP,
        AddressW: D3D11_TEXTURE_ADDRESS_CLAMP,
        MipLODBias: -0.4,
        MaxAnisotropy: 1,
        ComparisonFunc: D3D11_COMPARISON_ALWAYS,
        BorderColor: [0.0; 4],
        MinLOD: 0.0,
        MaxLOD: f32::MAX,
    };
    let mut sampler: *mut ID3D11SamplerState = ptr::null_mut();
    let hr = (*gfx.device.as_raw()).CreateSamplerState(&desc, &mut sampler);
    hr_ok(hr, "CreateSamplerState for map labels")?;
    ComPtr::from_raw(sampler, "map-label sampler")
}

unsafe fn create_map_label_blend(gfx: &Gfx) -> Result<ComPtr<ID3D11BlendState>> {
    let mut desc: D3D11_BLEND_DESC = mem::zeroed();
    desc.RenderTarget[0].BlendEnable = 1;
    desc.RenderTarget[0].SrcBlend = D3D11_BLEND_SRC_ALPHA;
    desc.RenderTarget[0].DestBlend = D3D11_BLEND_INV_SRC_ALPHA;
    desc.RenderTarget[0].BlendOp = D3D11_BLEND_OP_ADD;
    desc.RenderTarget[0].SrcBlendAlpha = D3D11_BLEND_ONE;
    desc.RenderTarget[0].DestBlendAlpha = D3D11_BLEND_INV_SRC_ALPHA;
    desc.RenderTarget[0].BlendOpAlpha = D3D11_BLEND_OP_ADD;
    desc.RenderTarget[0].RenderTargetWriteMask = D3D11_COLOR_WRITE_ENABLE_ALL as u8;
    let mut blend: *mut ID3D11BlendState = ptr::null_mut();
    let hr = (*gfx.device.as_raw()).CreateBlendState(&desc, &mut blend);
    hr_ok(hr, "CreateBlendState for map labels")?;
    ComPtr::from_raw(blend, "map-label blend state")
}

unsafe fn create_dynamic_constant_buffer<T>(gfx: &Gfx, role: &str) -> Result<ComPtr<ID3D11Buffer>> {
    let size = mem::size_of::<T>();
    if size % 16 != 0 {
        return Err(format!("{role} size {size} is not 16-byte aligned").into());
    }
    let desc = D3D11_BUFFER_DESC {
        ByteWidth: size as UINT,
        Usage: D3D11_USAGE_DYNAMIC,
        BindFlags: D3D11_BIND_CONSTANT_BUFFER,
        CPUAccessFlags: D3D11_CPU_ACCESS_WRITE,
        MiscFlags: 0,
        StructureByteStride: 0,
    };
    let mut buffer: *mut ID3D11Buffer = ptr::null_mut();
    let hr = (*gfx.device.as_raw()).CreateBuffer(&desc, ptr::null(), &mut buffer);
    hr_ok(hr, &format!("CreateBuffer for {role}"))?;
    ComPtr::from_raw(buffer, role)
}

unsafe fn create_dynamic_vertex_buffer(gfx: &Gfx) -> Result<ComPtr<ID3D11Buffer>> {
    let desc = D3D11_BUFFER_DESC {
        ByteWidth: (VERTEX_CAPACITY * mem::size_of::<MapLabelVertex>()) as UINT,
        Usage: D3D11_USAGE_DYNAMIC,
        BindFlags: D3D11_BIND_VERTEX_BUFFER,
        CPUAccessFlags: D3D11_CPU_ACCESS_WRITE,
        MiscFlags: 0,
        StructureByteStride: 0,
    };
    let mut buffer: *mut ID3D11Buffer = ptr::null_mut();
    let hr = (*gfx.device.as_raw()).CreateBuffer(&desc, ptr::null(), &mut buffer);
    hr_ok(hr, "CreateBuffer for map-label vertex buffer")?;
    ComPtr::from_raw(buffer, "map-label vertex buffer")
}

unsafe fn write_dynamic_buffer<T: Copy>(
    context: *mut winapi::um::d3d11::ID3D11DeviceContext,
    buffer: &ComPtr<ID3D11Buffer>,
    value: &T,
    role: &str,
) -> Result<()> {
    let mut mapped: D3D11_MAPPED_SUBRESOURCE = mem::zeroed();
    let hr = (*context).Map(
        buffer.as_raw() as *mut _,
        0,
        D3D11_MAP_WRITE_DISCARD,
        0,
        &mut mapped,
    );
    hr_ok(hr, role)?;
    ptr::copy_nonoverlapping(
        value as *const T as *const u8,
        mapped.pData as *mut u8,
        mem::size_of::<T>(),
    );
    (*context).Unmap(buffer.as_raw() as *mut _, 0);
    Ok(())
}

unsafe fn write_dynamic_vertex_chunk(
    context: *mut winapi::um::d3d11::ID3D11DeviceContext,
    buffer: &ComPtr<ID3D11Buffer>,
    vertices: &[MapLabelVertex],
) -> Result<()> {
    let mut mapped: D3D11_MAPPED_SUBRESOURCE = mem::zeroed();
    let hr = (*context).Map(
        buffer.as_raw() as *mut _,
        0,
        D3D11_MAP_WRITE_DISCARD,
        0,
        &mut mapped,
    );
    hr_ok(hr, "Map map-label vertex buffer")?;
    ptr::copy_nonoverlapping(
        vertices.as_ptr() as *const u8,
        mapped.pData as *mut u8,
        vertices.len() * mem::size_of::<MapLabelVertex>(),
    );
    (*context).Unmap(buffer.as_raw() as *mut _, 0);
    Ok(())
}

unsafe fn create_immutable_texture_mips(
    gfx: &Gfx,
    levels: &[(usize, usize, &[u8])],
    format: u32,
    role: &str,
) -> Result<(ComPtr<ID3D11Texture2D>, ComPtr<ID3D11ShaderResourceView>)> {
    let (w0, h0, _) = levels.first().copied().ok_or("mip texture needs levels")?;
    let mut expect_w = w0;
    let mut expect_h = h0;
    let mut init = Vec::with_capacity(levels.len());
    for (li, (w, h, bytes)) in levels.iter().enumerate() {
        if *w != expect_w || *h != expect_h {
            return Err(
                format!("{role}: mip {li} is {w}x{h}, expected {expect_w}x{expect_h}").into(),
            );
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

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use std::path::Path;

    fn write_font_tga(path: &Path) {
        let mut tga = vec![0u8; 18];
        tga[2] = 2;
        tga[12] = 16;
        tga[14] = 16;
        tga[16] = 32;
        tga[17] = 0x20;
        for _ in 0..(16 * 16) {
            tga.extend_from_slice(&[0x40, 0x40, 0x40, 0xFF]);
        }
        fs::write(path, tga).expect("write tga");
    }

    fn write_label_font_fixture() -> std::path::PathBuf {
        let dir = crate::bundle::tests::unique_temp_dir("map_label_font");
        let fonts = dir.join("gfx").join("fonts");
        fs::create_dir_all(&fonts).expect("create fonts dir");
        fs::create_dir_all(dir.join("interface")).expect("create interface dir");
        fs::write(
            dir.join("interface").join("core.gfx"),
            r#"
bitmapfonts = {
    bitmapfont = {
        name = "tahoma_60"
        path = "gfx/fonts/hoi_mapfont4"
        color = 0xffffffff
    }
}
"#,
        )
        .expect("write core.gfx");
        fs::write(
            fonts.join("hoi_mapfont4.fnt"),
            "\
info face=\"test\" size=16 bold=0 italic=0 charset=\"\" unicode=0 stretchH=100 smooth=1 aa=1 padding=0,0,0,0 spacing=1,1 outline=0
common lineHeight=16 base=12 scaleW=16 scaleH=16 pages=1 packed=0 alphaChnl=1 redChnl=0 greenChnl=0 blueChnl=0
page id=0 file=\"hoi_mapfont4.tga\"
chars count=3
char id=32 x=0 y=0 width=0 height=0 xoffset=0 yoffset=0 xadvance=4 page=0 chnl=15
char id=65 x=0 y=0 width=4 height=8 xoffset=0 yoffset=4 xadvance=5 page=0 chnl=15
char id=66 x=4 y=0 width=4 height=8 xoffset=0 yoffset=4 xadvance=5 page=0 chnl=15
kernings count=0
",
        )
        .expect("write fnt");
        write_font_tga(&fonts.join("hoi_mapfont4.tga"));
        dir
    }

    fn placement() -> CountryLabelPlacement {
        CountryLabelPlacement {
            tag: "TST".to_string(),
            is_home: true,
            component_id: 1,
            anchor_x: 100.0,
            anchor_y: 50.0,
            angle: 0.0,
            half_length: 50.0,
            bow: 0.0,
            world_height: 10.0,
            pixel_count: 2000,
            major_extent: 100.0,
            minor_extent: 20.0,
            minor_extent_incl: 20.0,
            breadth_span: 20.0,
            breadth_p2_98: 20.0,
            curve: None,
        }
    }

    #[test]
    fn replay_label_vb_is_float_exact_with_map_viewer_law_output() {
        let root = write_label_font_fixture();
        let resolver = map_viewer::GameFileResolver::vanilla(root).expect("resolver");
        let font = load_map_label_font(&resolver).expect("label font");
        let placement = placement();
        let batches = build_label_vertex_batches(&font, &placement, "AB", 200.0, 9.5).expect("vb");
        assert_eq!(batches.len(), 1);
        assert_eq!(batches[0].vertices.len(), 12);

        let metrics = font.label_text_metrics("AB").expect("metrics");
        assert_eq!(metrics.straight_layout_stations.len(), 2);
        assert_eq!(metrics.straight_layout_stations[0].slot_index, 0);
        assert_eq!(
            metrics.straight_layout_stations[0].x.to_bits(),
            2.0f32.to_bits()
        );
        assert_eq!(
            metrics.straight_layout_stations[0].y.to_bits(),
            4.0f32.to_bits()
        );
        assert_eq!(metrics.straight_layout_stations[1].slot_index, 1);
        assert_eq!(
            metrics.straight_layout_stations[1].x.to_bits(),
            7.0f32.to_bits()
        );
        assert_eq!(
            metrics.straight_layout_stations[1].y.to_bits(),
            4.0f32.to_bits()
        );
        let tracking = mapname_tracking_px(&placement, &metrics);
        let tracked_advance = mapname_tracked_advance_px(&metrics, tracking);
        let layout = font.layout_text_tracked(0.0, 0.0, "AB", tracking);
        let mut glyphs: Vec<MapLabelGlyphLayoutQuad> = layout
            .quads
            .iter()
            .map(|quad| MapLabelGlyphLayoutQuad {
                x0: quad.x0,
                y0: quad.y0,
                x1: quad.x1,
                y1: quad.y1,
                u0: quad.u0,
                v0: quad.v0,
                u1: quad.u1,
                v1: quad.v1,
                measured_t: 0.0,
            })
            .collect();
        let measured = mapname_measured_glyph_parameters(&glyphs).expect("measured stations");
        for (glyph, measured_t) in glyphs.iter_mut().zip(measured) {
            glyph.measured_t = measured_t;
        }
        let natural_width: f32 = metrics.advances.iter().map(|(advance, _)| *advance).sum();
        let law = build_mapname_vertices(
            &placement,
            &glyphs,
            natural_width,
            tracked_advance,
            metrics.line_height_px,
            200.0,
            9.5,
        );
        let replay: Vec<_> = batches[0]
            .vertices
            .iter()
            .map(|vertex| (vertex.position, vertex.uv))
            .collect();
        let law: Vec<_> = law
            .iter()
            .map(|vertex| ([vertex.x, vertex.y, vertex.z], [vertex.u, vertex.v]))
            .collect();
        assert_eq!(replay, law);
        assert_eq!(
            label_vertices_to_bytes(&batches[0].vertices).len(),
            batches[0].vertices.len() * map_viewer::MAPNAME_VERTEX_STRIDE
        );
    }

    #[test]
    fn replay_label_vb_matches_map_viewer_law_for_oracle63_rows() {
        let oracle_path = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("../map_viewer/tests/fixtures/mapname/engine_label_oracle.json");
        if !oracle_path.is_file() {
            eprintln!(
                "SKIP replay label oracle63 VB gate: fixture absent at {}",
                oracle_path.display()
            );
            return;
        }
        let raw = fs::read_to_string(&oracle_path).expect("read oracle fixture");
        let v: serde_json::Value = serde_json::from_str(&raw).expect("oracle parses");
        let rows = v["labels"].as_array().expect("labels array");
        assert_eq!(rows.len(), 63, "oracle fixture row count");

        let root = write_label_font_fixture();
        let resolver = map_viewer::GameFileResolver::vanilla(root).expect("resolver");
        let font = load_map_label_font(&resolver).expect("label font");
        let mut checked = 0usize;
        for row in rows {
            let glyph_count = row["glyphs"].as_u64().expect("glyphs") as usize;
            let text = "A".repeat(glyph_count);
            let angle =
                row["angle_deg"].as_f64().expect("angle") as f32 * std::f32::consts::PI / 180.0;
            let placement = CountryLabelPlacement {
                tag: row["tag"].as_str().expect("tag").to_string(),
                is_home: true,
                component_id: checked as u32,
                anchor_x: row["anchor_x"].as_f64().expect("anchor_x") as f32,
                anchor_y: row["anchor_y"].as_f64().expect("anchor_y") as f32,
                angle,
                half_length: row["arc"].as_f64().expect("arc") as f32 * 0.5,
                bow: 0.0,
                world_height: 10.0,
                pixel_count: 2000,
                major_extent: 100.0,
                minor_extent: 20.0,
                minor_extent_incl: 20.0,
                breadth_span: 20.0,
                breadth_p2_98: 20.0,
                curve: None,
            };
            let batches =
                build_label_vertex_batches(&font, &placement, &text, 2048.0, 9.5).expect("vb");
            assert_eq!(batches.len(), 1, "single-page fixture font");

            let metrics = font.label_text_metrics(&text).expect("metrics");
            let tracking = mapname_tracking_px(&placement, &metrics);
            let tracked_advance = mapname_tracked_advance_px(&metrics, tracking);
            let layout = font.layout_text_tracked(0.0, 0.0, &text, tracking);
            let mut glyphs: Vec<MapLabelGlyphLayoutQuad> = layout
                .quads
                .iter()
                .map(|quad| MapLabelGlyphLayoutQuad {
                    x0: quad.x0,
                    y0: quad.y0,
                    x1: quad.x1,
                    y1: quad.y1,
                    u0: quad.u0,
                    v0: quad.v0,
                    u1: quad.u1,
                    v1: quad.v1,
                    measured_t: 0.0,
                })
                .collect();
            let measured = mapname_measured_glyph_parameters(&glyphs).expect("measured stations");
            for (glyph, measured_t) in glyphs.iter_mut().zip(measured) {
                glyph.measured_t = measured_t;
            }
            let natural_width: f32 = metrics.advances.iter().map(|(advance, _)| *advance).sum();
            let law = build_mapname_vertices(
                &placement,
                &glyphs,
                natural_width,
                tracked_advance,
                metrics.line_height_px,
                2048.0,
                9.5,
            );
            let replay: Vec<_> = batches[0]
                .vertices
                .iter()
                .map(|vertex| {
                    (
                        [vertex.position[0], vertex.position[1], vertex.position[2]],
                        vertex.uv,
                    )
                })
                .collect();
            let law: Vec<_> = law
                .iter()
                .map(|vertex| ([vertex.x, vertex.y, vertex.z], [vertex.u, vertex.v]))
                .collect();
            assert_eq!(replay, law, "{}", placement.tag);
            checked += 1;
        }
        assert_eq!(checked, 63);
    }
}
