//! Product D3D11 pass for VP/capital map furniture.
//!
//! The CPU law and fail-closed frame cache live in `map_furniture`; this file
//! only uploads the resolver-decoded strip/font atlases and submits the emitted
//! screen-space sprite triangles. City-name text is deliberately fail-closed:
//! FILE declares `hoi_20bs`, but the retained draw fixture has no texture,
//! vertex/index, or shader payload with which to prove that event's identity.

use std::ffi::CString;
use std::mem;
use std::ptr;

use map_viewer::{GameFileResolver, ProductionMapCamera};
use replay_core::WorldFrame;
use winapi::shared::dxgiformat::{DXGI_FORMAT_R32G32_FLOAT, DXGI_FORMAT_R8G8B8A8_UNORM};
use winapi::shared::minwindef::UINT;
use winapi::um::d3d11::{
    ID3D11BlendState, ID3D11Buffer, ID3D11InputLayout, ID3D11PixelShader, ID3D11SamplerState,
    ID3D11ShaderResourceView, ID3D11Texture2D, ID3D11VertexShader, D3D11_BIND_CONSTANT_BUFFER,
    D3D11_BIND_VERTEX_BUFFER, D3D11_BLEND_DESC, D3D11_BLEND_INV_SRC_ALPHA, D3D11_BLEND_OP_ADD,
    D3D11_BLEND_SRC_ALPHA, D3D11_BUFFER_DESC, D3D11_COLOR_WRITE_ENABLE_ALL, D3D11_COMPARISON_NEVER,
    D3D11_CPU_ACCESS_WRITE, D3D11_FILTER_MIN_MAG_LINEAR_MIP_POINT, D3D11_INPUT_ELEMENT_DESC,
    D3D11_INPUT_PER_VERTEX_DATA, D3D11_MAPPED_SUBRESOURCE, D3D11_MAP_WRITE_DISCARD,
    D3D11_SAMPLER_DESC, D3D11_TEXTURE_ADDRESS_CLAMP, D3D11_TEXTURE_ADDRESS_WRAP,
    D3D11_USAGE_DYNAMIC, D3D11_VIEWPORT,
};
use winapi::um::d3dcommon::D3D11_PRIMITIVE_TOPOLOGY_TRIANGLELIST;

use crate::bundle::MapBundle;
use crate::d3d::{hr_ok, ComPtr, Gfx};
use crate::map_furniture::{
    compose_furniture_draw_plan, ProjectedFurnitureSprite, ReplayFurnitureAssets,
    ReplayFurnitureScene,
};
use crate::render::{compile_shader, create_immutable_texture};
use crate::Result;
use map_viewer::ResourceStateOverlay;

const FURNITURE_ICON_SHADER_HLSL: &str = r#"
cbuffer ScreenCB : register(b0) {
    float2 screen_size;
    float2 _pad;
};

Texture2D Strip : register(t0);
SamplerState FurnitureSampler : register(s0);

struct VsIn {
    float2 pos : POSITION;
    float2 uv : TEXCOORD0;
};

struct VsOut {
    float4 pos : SV_Position;
    float2 uv : TEXCOORD0;
};

VsOut vs_main(VsIn input) {
    VsOut output;
    // DERIVED coordinate transform: window pixels (Y down) to D3D clip space.
    float2 clip = float2(
        input.pos.x / screen_size.x * 2.0f - 1.0f,
        1.0f - input.pos.y / screen_size.y * 2.0f);
    output.pos = float4(clip, 0.0f, 1.0f);
    output.uv = input.uv;
    return output;
}

float4 ps_main(VsOut input) : SV_Target {
    return Strip.Sample(FurnitureSampler, input.uv);
}
"#;

#[repr(C)]
#[derive(Clone, Copy)]
struct FurnitureIconVertex {
    position: [f32; 2],
    uv: [f32; 2],
}

#[repr(C)]
#[derive(Clone, Copy)]
struct ScreenConstants {
    screen_size: [f32; 2],
    _pad: [f32; 2],
}

/// Fail-closed Resource-map-mode furniture plan.
///
/// FILE resolves icon frames and CAPTURE proves a screen-space GUI icon plus
/// amount-text composition, but the state/deposit world anchor and exact local
/// offsets are still unavailable. The renderer therefore receives identities
/// for diagnostics and capacity accounting while draw_count remains zero.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct ResourceIconRenderPlan {
    pub(crate) resolved_identity_count: usize,
    pub(crate) invalid_frame_count: usize,
    pub(crate) draw_count: usize,
    pub(crate) placement_gap: ResourceIconRenderGap,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum ResourceIconRenderGap {
    ResourceIconPlacementUnavailable,
}

impl ResourceIconRenderGap {
    pub(crate) const fn code(self) -> &'static str {
        match self {
            Self::ResourceIconPlacementUnavailable => "ResourceIconPlacementUnavailable",
        }
    }
}

pub(crate) fn compose_resource_icon_render_plan(
    overlays: &[ResourceStateOverlay],
) -> ResourceIconRenderPlan {
    let mut resolved_identity_count = 0usize;
    let mut invalid_frame_count = 0usize;
    for (_, icon_frame, _) in overlays.iter().flat_map(|overlay| &overlay.deposits) {
        if (1..=7).contains(icon_frame) {
            resolved_identity_count += 1;
        } else {
            invalid_frame_count += 1;
        }
    }
    ResourceIconRenderPlan {
        resolved_identity_count,
        invalid_frame_count,
        draw_count: 0,
        placement_gap: ResourceIconRenderGap::ResourceIconPlacementUnavailable,
    }
}

struct FurnitureIconRenderer {
    vs: ComPtr<ID3D11VertexShader>,
    ps: ComPtr<ID3D11PixelShader>,
    input_layout: ComPtr<ID3D11InputLayout>,
    sampler: ComPtr<ID3D11SamplerState>,
    blend: ComPtr<ID3D11BlendState>,
    screen_cb: ComPtr<ID3D11Buffer>,
    vertex_buffer: ComPtr<ID3D11Buffer>,
    vertex_capacity: usize,
    _strip_texture: ComPtr<ID3D11Texture2D>,
    strip_srv: ComPtr<ID3D11ShaderResourceView>,
}

impl FurnitureIconRenderer {
    fn new(
        gfx: &Gfx,
        atlas: &map_viewer::RgbaFontAtlas,
        maximum_marker_count: usize,
    ) -> Result<Self> {
        unsafe {
            let vs_blob = compile_shader(FURNITURE_ICON_SHADER_HLSL, "vs_main", "vs_4_0")?;
            let ps_blob = compile_shader(FURNITURE_ICON_SHADER_HLSL, "ps_main", "ps_4_0")?;

            let mut vs: *mut ID3D11VertexShader = ptr::null_mut();
            let hr = (*gfx.device.as_raw()).CreateVertexShader(
                (*vs_blob.as_raw()).GetBufferPointer(),
                (*vs_blob.as_raw()).GetBufferSize(),
                ptr::null_mut(),
                &mut vs,
            );
            hr_ok(hr, "CreateVertexShader for map furniture icons")?;
            let vs = ComPtr::from_raw(vs, "map-furniture icon vertex shader")?;

            let mut ps: *mut ID3D11PixelShader = ptr::null_mut();
            let hr = (*gfx.device.as_raw()).CreatePixelShader(
                (*ps_blob.as_raw()).GetBufferPointer(),
                (*ps_blob.as_raw()).GetBufferSize(),
                ptr::null_mut(),
                &mut ps,
            );
            hr_ok(hr, "CreatePixelShader for map furniture icons")?;
            let ps = ComPtr::from_raw(ps, "map-furniture icon pixel shader")?;

            let position = CString::new("POSITION").expect("static semantic");
            let texcoord = CString::new("TEXCOORD").expect("static semantic");
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
            ];
            let mut input_layout: *mut ID3D11InputLayout = ptr::null_mut();
            let hr = (*gfx.device.as_raw()).CreateInputLayout(
                elements.as_ptr(),
                elements.len() as UINT,
                (*vs_blob.as_raw()).GetBufferPointer(),
                (*vs_blob.as_raw()).GetBufferSize(),
                &mut input_layout,
            );
            hr_ok(hr, "CreateInputLayout for map furniture icons")?;
            let input_layout = ComPtr::from_raw(input_layout, "map-furniture icon input layout")?;

            let mut sampler_desc: D3D11_SAMPLER_DESC = mem::zeroed();
            // CAPTURE: retained hoi4_frame68489.rdc event 19603/chunk 23249,
            // event-scoped pipeline state for the Berlin VP draw.
            sampler_desc.Filter = D3D11_FILTER_MIN_MAG_LINEAR_MIP_POINT;
            sampler_desc.AddressU = D3D11_TEXTURE_ADDRESS_CLAMP;
            sampler_desc.AddressV = D3D11_TEXTURE_ADDRESS_CLAMP;
            sampler_desc.AddressW = D3D11_TEXTURE_ADDRESS_WRAP;
            sampler_desc.MipLODBias = -0.8;
            sampler_desc.ComparisonFunc = D3D11_COMPARISON_NEVER;
            sampler_desc.MinLOD = 0.0;
            sampler_desc.MaxLOD = 1024.0;
            let mut sampler: *mut ID3D11SamplerState = ptr::null_mut();
            let hr = (*gfx.device.as_raw()).CreateSamplerState(&sampler_desc, &mut sampler);
            hr_ok(hr, "CreateSamplerState for map furniture icons")?;
            let sampler = ComPtr::from_raw(sampler, "map-furniture icon sampler")?;

            let mut blend_desc: D3D11_BLEND_DESC = mem::zeroed();
            // CAPTURE: same retained draw; RGB and alpha use the same
            // SrcAlpha/InvSrcAlpha/Add factors.
            blend_desc.RenderTarget[0].BlendEnable = 1;
            blend_desc.RenderTarget[0].SrcBlend = D3D11_BLEND_SRC_ALPHA;
            blend_desc.RenderTarget[0].DestBlend = D3D11_BLEND_INV_SRC_ALPHA;
            blend_desc.RenderTarget[0].BlendOp = D3D11_BLEND_OP_ADD;
            blend_desc.RenderTarget[0].SrcBlendAlpha = D3D11_BLEND_SRC_ALPHA;
            blend_desc.RenderTarget[0].DestBlendAlpha = D3D11_BLEND_INV_SRC_ALPHA;
            blend_desc.RenderTarget[0].BlendOpAlpha = D3D11_BLEND_OP_ADD;
            blend_desc.RenderTarget[0].RenderTargetWriteMask = D3D11_COLOR_WRITE_ENABLE_ALL as u8;
            let mut blend: *mut ID3D11BlendState = ptr::null_mut();
            let hr = (*gfx.device.as_raw()).CreateBlendState(&blend_desc, &mut blend);
            hr_ok(hr, "CreateBlendState for map furniture icons")?;
            let blend = ComPtr::from_raw(blend, "map-furniture icon blend state")?;

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
            hr_ok(hr, "CreateBuffer for map furniture screen constants")?;
            let screen_cb = ComPtr::from_raw(screen_cb, "map-furniture screen constants")?;

            // DERIVED capacity: six vertices per FILE victory-point entry; no
            // fixed visual/content ceiling is introduced.
            let vertex_capacity = maximum_marker_count
                .max(1)
                .checked_mul(6)
                .ok_or("map-furniture icon vertex capacity overflow")?;
            let byte_width = vertex_capacity
                .checked_mul(mem::size_of::<FurnitureIconVertex>())
                .and_then(|bytes| u32::try_from(bytes).ok())
                .ok_or("map-furniture icon vertex buffer exceeds D3D11 limits")?;
            let vb_desc = D3D11_BUFFER_DESC {
                ByteWidth: byte_width,
                Usage: D3D11_USAGE_DYNAMIC,
                BindFlags: D3D11_BIND_VERTEX_BUFFER,
                CPUAccessFlags: D3D11_CPU_ACCESS_WRITE,
                MiscFlags: 0,
                StructureByteStride: 0,
            };
            let mut vertex_buffer: *mut ID3D11Buffer = ptr::null_mut();
            let hr = (*gfx.device.as_raw()).CreateBuffer(&vb_desc, ptr::null(), &mut vertex_buffer);
            hr_ok(hr, "CreateBuffer for map furniture icon vertices")?;
            let vertex_buffer =
                ComPtr::from_raw(vertex_buffer, "map-furniture icon vertex buffer")?;

            let (strip_texture, strip_srv) = create_immutable_texture(
                gfx,
                atlas.width,
                atlas.height,
                DXGI_FORMAT_R8G8B8A8_UNORM,
                &atlas.pixels,
                "map-furniture VP strip atlas",
            )?;

            Ok(Self {
                vs,
                ps,
                input_layout,
                sampler,
                blend,
                screen_cb,
                vertex_buffer,
                vertex_capacity,
                _strip_texture: strip_texture,
                strip_srv,
            })
        }
    }

    fn draw(&self, gfx: &Gfx, quads: &[ProjectedFurnitureSprite]) -> Result<()> {
        if quads.is_empty() || gfx.width == 0 || gfx.height == 0 {
            return Ok(());
        }
        let mut vertices = Vec::with_capacity(quads.len() * 6);
        for quad in quads {
            let vertex = |index: usize| FurnitureIconVertex {
                position: quad.corners_px[index],
                uv: quad.uvs[index],
            };
            // W7/NAME2 triangle order over TL,TR,BL,BR corners.
            vertices.extend([
                vertex(0),
                vertex(1),
                vertex(2),
                vertex(1),
                vertex(3),
                vertex(2),
            ]);
        }
        if vertices.len() > self.vertex_capacity {
            return Err(format!(
                "map-furniture icon batch has {} vertices, capacity is {}",
                vertices.len(),
                self.vertex_capacity
            )
            .into());
        }

        unsafe {
            let context = gfx.context.as_raw();
            let constants = ScreenConstants {
                screen_size: [gfx.width as f32, gfx.height as f32],
                _pad: [0.0; 2],
            };
            write_dynamic_buffer(
                context,
                &self.screen_cb,
                &constants,
                "Map map-furniture screen constants",
            )?;
            write_dynamic_vertices(context, &self.vertex_buffer, &vertices)?;

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
            let stride = mem::size_of::<FurnitureIconVertex>() as UINT;
            let offset = 0u32;
            let buffers = [self.vertex_buffer.as_raw()];
            (*context).IASetVertexBuffers(0, 1, buffers.as_ptr(), &stride, &offset);
            (*context).VSSetShader(self.vs.as_raw(), ptr::null(), 0);
            (*context).VSSetConstantBuffers(0, 1, [self.screen_cb.as_raw()].as_ptr());
            (*context).PSSetShader(self.ps.as_raw(), ptr::null(), 0);
            (*context).PSSetSamplers(0, 1, [self.sampler.as_raw()].as_ptr());
            (*context).PSSetShaderResources(0, 1, [self.strip_srv.as_raw()].as_ptr());
            let blend_factor = [0.0f32; 4];
            (*context).OMSetBlendState(self.blend.as_raw(), &blend_factor, u32::MAX);
            (*context).Draw(vertices.len() as UINT, 0);

            let null_srv: [*mut ID3D11ShaderResourceView; 1] = [ptr::null_mut()];
            (*context).PSSetShaderResources(0, 1, null_srv.as_ptr());
            (*context).OMSetBlendState(ptr::null_mut(), &blend_factor, u32::MAX);
        }
        Ok(())
    }
}

pub(crate) struct FurnitureRenderPass {
    scene: ReplayFurnitureScene,
    icons: FurnitureIconRenderer,
}

impl FurnitureRenderPass {
    pub(crate) fn new(gfx: &Gfx, resolver: &GameFileResolver, bundle: &MapBundle) -> Result<Self> {
        let assets = ReplayFurnitureAssets::load(resolver, bundle)?;
        let scene = ReplayFurnitureScene::new(assets);
        let icons = FurnitureIconRenderer::new(gfx, scene.strip(), scene.maximum_marker_count())?;
        Ok(Self { scene, icons })
    }

    /// Recompose VP/capital furniture for the selected viewed country.
    ///
    /// CAPTURE (2026-07-10 whole-frame corpus): the byte-exact VP-strip draws
    /// confirm enemy column 0 (Warsaw), self column 2 (Berlin), and neutral
    /// column 4 (Kaunas), with zero column-1/3 draws among 1,722 samples. The
    /// same-faction branch remains RE-only. Product diplomacy inputs still fail
    /// closed in `ReplayFurnitureScene`: a foreign owner without BOTH measured
    /// `at_war` and `same_faction` leaves emits no sprite.
    pub(crate) fn update_world_frame(
        &mut self,
        frame: Option<&WorldFrame>,
        viewed_tag: Option<&str>,
    ) -> Result<()> {
        self.scene.update_world_frame(frame, viewed_tag)
    }

    pub(crate) fn draw(&self, gfx: &Gfx, cameras: &[ProductionMapCamera]) -> Result<()> {
        for camera in cameras {
            let plan = compose_furniture_draw_plan(
                self.scene.current(),
                self.scene.vp_icon_defines(),
                true,
                camera,
                gfx.width as f32,
                gfx.height as f32,
            );
            self.icons.draw(gfx, &plan.sprite_quads)?;
        }
        Ok(())
    }
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

unsafe fn write_dynamic_vertices(
    context: *mut winapi::um::d3d11::ID3D11DeviceContext,
    buffer: &ComPtr<ID3D11Buffer>,
    vertices: &[FurnitureIconVertex],
) -> Result<()> {
    let mut mapped: D3D11_MAPPED_SUBRESOURCE = mem::zeroed();
    let hr = (*context).Map(
        buffer.as_raw() as *mut _,
        0,
        D3D11_MAP_WRITE_DISCARD,
        0,
        &mut mapped,
    );
    hr_ok(hr, "Map map-furniture icon vertex buffer")?;
    ptr::copy_nonoverlapping(
        vertices.as_ptr() as *const u8,
        mapped.pData as *mut u8,
        vertices.len() * mem::size_of::<FurnitureIconVertex>(),
    );
    (*context).Unmap(buffer.as_raw() as *mut _, 0);
    Ok(())
}

#[cfg(test)]
mod resource_icon_tests {
    use super::*;

    #[test]
    fn renderer_keeps_resource_identities_but_submits_zero_unanchored_draws() {
        let overlays = vec![
            ResourceStateOverlay {
                state_id: 1,
                deposits: vec![("oil".to_string(), 1, 3), ("steel".to_string(), 5, 6)],
            },
            ResourceStateOverlay {
                state_id: 2,
                deposits: vec![("coal".to_string(), 7, 2), ("unknown".to_string(), 8, 1)],
            },
        ];
        let plan = compose_resource_icon_render_plan(&overlays);
        assert_eq!(plan.resolved_identity_count, 3);
        assert_eq!(plan.invalid_frame_count, 1);
        assert_eq!(plan.draw_count, 0);
        assert_eq!(
            plan.placement_gap.code(),
            "ResourceIconPlacementUnavailable"
        );
    }
}
