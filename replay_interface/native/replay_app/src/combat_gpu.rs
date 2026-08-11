//! XMAP14 state-first combat-mapicon GPU consumer.
//!
//! This pass accepts only `PreparedXmap14Frame`.  Geometry, draw order,
//! texture identities, frame UVs, modulation, progress state and text boxes
//! were already derived by the admitted map-viewer adapter.  No captured
//! counter admission, selected-Present draw, recorder POV or prior-hour state
//! participates.

use std::collections::BTreeMap;
use std::ffi::CString;
use std::mem;
use std::ptr;

use map_viewer::{
    counter_texture_rgba_from_resolver, GameFileResolver, PreparedXmap14Draw,
    PreparedXmap14Frame, PreparedXmap14ProgressDraw, PreparedXmap14SpriteDraw,
};
use portable_content_manifest::UsedContentKey;
use winapi::shared::dxgiformat::{
    DXGI_FORMAT_R32G32B32A32_FLOAT, DXGI_FORMAT_R32G32_FLOAT,
    DXGI_FORMAT_R8G8B8A8_UNORM,
};
use winapi::shared::minwindef::UINT;
use winapi::shared::windef::RECT;
use winapi::um::d3d11::{
    ID3D11BlendState, ID3D11Buffer, ID3D11DepthStencilState, ID3D11InputLayout,
    ID3D11PixelShader, ID3D11RasterizerState, ID3D11SamplerState, ID3D11ShaderResourceView,
    ID3D11Texture2D, ID3D11VertexShader, D3D11_BIND_CONSTANT_BUFFER, D3D11_BIND_VERTEX_BUFFER,
    D3D11_BLEND_DESC, D3D11_BLEND_INV_SRC_ALPHA, D3D11_BLEND_OP_ADD, D3D11_BLEND_SRC_ALPHA,
    D3D11_BUFFER_DESC, D3D11_COLOR_WRITE_ENABLE_ALL, D3D11_COMPARISON_NEVER,
    D3D11_CPU_ACCESS_WRITE, D3D11_CULL_NONE, D3D11_DEPTH_STENCIL_DESC,
    D3D11_DEPTH_WRITE_MASK_ZERO, D3D11_FILL_SOLID, D3D11_FILTER_MIN_MAG_MIP_POINT,
    D3D11_INPUT_ELEMENT_DESC, D3D11_INPUT_PER_VERTEX_DATA, D3D11_MAPPED_SUBRESOURCE,
    D3D11_MAP_WRITE_DISCARD, D3D11_RASTERIZER_DESC, D3D11_SAMPLER_DESC,
    D3D11_TEXTURE_ADDRESS_WRAP, D3D11_USAGE_DYNAMIC, D3D11_VIEWPORT,
};
use winapi::um::d3dcommon::D3D11_PRIMITIVE_TOPOLOGY_TRIANGLESTRIP;

use crate::d3d::{hr_ok, ComPtr, Gfx};
use crate::font::load_game_font_for_language_from_resolver;
#[cfg(test)]
use crate::font::OVERLAY_FONT_NAME;
use crate::render::{compile_shader, create_immutable_texture};
use crate::text::TextRenderer;
use crate::Result;


const COMBAT_HLSL: &str = r#"
cbuffer ScreenCB : register(b0) {
    float2 clip_scale;
    float2 _pad;
};

Texture2D TextureOne : register(t0);
Texture2D TextureTwo : register(t1);
SamplerState GuiSampler : register(s0);

struct VsIn {
    float2 pos : POSITION;
    float2 uv : TEXCOORD0;
    float4 modulation : COLOR0;
    float split : TEXCOORD1;
};

struct VsOut {
    float4 pos : SV_Position;
    float2 uv : TEXCOORD0;
    float4 modulation : COLOR0;
    float split : TEXCOORD1;
};

VsOut vs_main(VsIn input) {
    VsOut output;
    output.pos = float4(
        input.pos.x * clip_scale.x - 1.0f,
        1.0f - input.pos.y * clip_scale.y,
        0.0f,
        1.0f);
    output.uv = input.uv;
    output.modulation = input.modulation;
    output.split = input.split;
    return output;
}

float4 ps_main(VsOut input) : SV_Target {
    float4 texel = input.uv.x <= input.split
        ? TextureOne.Sample(GuiSampler, input.uv)
        : TextureTwo.Sample(GuiSampler, input.uv);
    return texel * input.modulation;
}
"#;

#[derive(Clone, Copy)]
#[repr(C)]
struct CombatVertex {
    position: [f32; 2],
    uv: [f32; 2],
    modulation: [f32; 4],
    split: f32,
}

struct CombatTexture {
    _texture: ComPtr<ID3D11Texture2D>,
    srv: ComPtr<ID3D11ShaderResourceView>,
}

struct CombatQuadRuntime {
    vs: ComPtr<ID3D11VertexShader>,
    ps: ComPtr<ID3D11PixelShader>,
    input_layout: ComPtr<ID3D11InputLayout>,
    sampler: ComPtr<ID3D11SamplerState>,
    blend: ComPtr<ID3D11BlendState>,
    rasterizer: ComPtr<ID3D11RasterizerState>,
    depth: ComPtr<ID3D11DepthStencilState>,
    screen_cb: ComPtr<ID3D11Buffer>,
    vertex_buffer: ComPtr<ID3D11Buffer>,
    textures: BTreeMap<String, CombatTexture>,
}

impl CombatQuadRuntime {
    unsafe fn new(gfx: &Gfx) -> Result<Self> {
        let vs_blob = compile_shader(COMBAT_HLSL, "vs_main", "vs_4_0")?;
        let ps_blob = compile_shader(COMBAT_HLSL, "ps_main", "ps_4_0")?;
        let mut vs = ptr::null_mut();
        hr_ok(
            (*gfx.device.as_raw()).CreateVertexShader(
                (*vs_blob.as_raw()).GetBufferPointer(),
                (*vs_blob.as_raw()).GetBufferSize(),
                ptr::null_mut(),
                &mut vs,
            ),
            "CreateVertexShader for XMAP14 combat pass",
        )?;
        let vs = ComPtr::from_raw(vs, "XMAP14 combat vertex shader")?;
        let mut ps = ptr::null_mut();
        hr_ok(
            (*gfx.device.as_raw()).CreatePixelShader(
                (*ps_blob.as_raw()).GetBufferPointer(),
                (*ps_blob.as_raw()).GetBufferSize(),
                ptr::null_mut(),
                &mut ps,
            ),
            "CreatePixelShader for XMAP14 combat pass",
        )?;
        let ps = ComPtr::from_raw(ps, "XMAP14 combat pixel shader")?;

        let position = CString::new("POSITION").expect("static semantic");
        let texcoord = CString::new("TEXCOORD").expect("static semantic");
        let color = CString::new("COLOR").expect("static semantic");
        let elements = [
            input_element(position.as_ptr(), 0, DXGI_FORMAT_R32G32_FLOAT, 0),
            input_element(texcoord.as_ptr(), 0, DXGI_FORMAT_R32G32_FLOAT, 8),
            input_element(color.as_ptr(), 0, DXGI_FORMAT_R32G32B32A32_FLOAT, 16),
            input_element(texcoord.as_ptr(), 1, winapi::shared::dxgiformat::DXGI_FORMAT_R32_FLOAT, 32),
        ];
        let mut input_layout = ptr::null_mut();
        hr_ok(
            (*gfx.device.as_raw()).CreateInputLayout(
                elements.as_ptr(),
                elements.len() as UINT,
                (*vs_blob.as_raw()).GetBufferPointer(),
                (*vs_blob.as_raw()).GetBufferSize(),
                &mut input_layout,
            ),
            "CreateInputLayout for XMAP14 combat pass",
        )?;
        let input_layout = ComPtr::from_raw(input_layout, "XMAP14 combat input layout")?;

        let mut sampler_desc: D3D11_SAMPLER_DESC = mem::zeroed();
        sampler_desc.Filter = D3D11_FILTER_MIN_MAG_MIP_POINT;
        sampler_desc.AddressU = D3D11_TEXTURE_ADDRESS_WRAP;
        sampler_desc.AddressV = D3D11_TEXTURE_ADDRESS_WRAP;
        sampler_desc.AddressW = D3D11_TEXTURE_ADDRESS_WRAP;
        sampler_desc.ComparisonFunc = D3D11_COMPARISON_NEVER;
        sampler_desc.MinLOD = 0.0;
        sampler_desc.MaxLOD = 1024.0;
        let mut sampler = ptr::null_mut();
        hr_ok(
            (*gfx.device.as_raw()).CreateSamplerState(&sampler_desc, &mut sampler),
            "CreateSamplerState for XMAP14 combat pass",
        )?;
        let sampler = ComPtr::from_raw(sampler, "XMAP14 combat sampler")?;

        let mut blend_desc: D3D11_BLEND_DESC = mem::zeroed();
        blend_desc.RenderTarget[0].BlendEnable = 1;
        blend_desc.RenderTarget[0].SrcBlend = D3D11_BLEND_SRC_ALPHA;
        blend_desc.RenderTarget[0].DestBlend = D3D11_BLEND_INV_SRC_ALPHA;
        blend_desc.RenderTarget[0].BlendOp = D3D11_BLEND_OP_ADD;
        blend_desc.RenderTarget[0].SrcBlendAlpha = D3D11_BLEND_SRC_ALPHA;
        blend_desc.RenderTarget[0].DestBlendAlpha = D3D11_BLEND_INV_SRC_ALPHA;
        blend_desc.RenderTarget[0].BlendOpAlpha = D3D11_BLEND_OP_ADD;
        blend_desc.RenderTarget[0].RenderTargetWriteMask = D3D11_COLOR_WRITE_ENABLE_ALL as u8;
        let mut blend = ptr::null_mut();
        hr_ok(
            (*gfx.device.as_raw()).CreateBlendState(&blend_desc, &mut blend),
            "CreateBlendState for XMAP14 combat pass",
        )?;
        let blend = ComPtr::from_raw(blend, "XMAP14 combat blend")?;

        let raster_desc = D3D11_RASTERIZER_DESC {
            FillMode: D3D11_FILL_SOLID,
            CullMode: D3D11_CULL_NONE,
            FrontCounterClockwise: 0,
            DepthBias: 0,
            DepthBiasClamp: 0.0,
            SlopeScaledDepthBias: 0.0,
            DepthClipEnable: 1,
            ScissorEnable: 1,
            MultisampleEnable: 0,
            AntialiasedLineEnable: 0,
        };
        let mut rasterizer = ptr::null_mut();
        hr_ok(
            (*gfx.device.as_raw()).CreateRasterizerState(&raster_desc, &mut rasterizer),
            "CreateRasterizerState for XMAP14 combat pass",
        )?;
        let rasterizer = ComPtr::from_raw(rasterizer, "XMAP14 combat rasterizer")?;

        let mut depth_desc: D3D11_DEPTH_STENCIL_DESC = mem::zeroed();
        depth_desc.DepthEnable = 0;
        depth_desc.DepthWriteMask = D3D11_DEPTH_WRITE_MASK_ZERO;
        depth_desc.DepthFunc = D3D11_COMPARISON_NEVER;
        let mut depth = ptr::null_mut();
        hr_ok(
            (*gfx.device.as_raw()).CreateDepthStencilState(&depth_desc, &mut depth),
            "CreateDepthStencilState for XMAP14 combat pass",
        )?;
        let depth = ComPtr::from_raw(depth, "XMAP14 combat depth")?;

        Ok(Self {
            vs,
            ps,
            input_layout,
            sampler,
            blend,
            rasterizer,
            depth,
            screen_cb: create_dynamic_buffer(
                gfx,
                mem::size_of::<[f32; 4]>(),
                D3D11_BIND_CONSTANT_BUFFER,
                "XMAP14 combat screen constants",
            )?,
            vertex_buffer: create_dynamic_buffer(
                gfx,
                4 * mem::size_of::<CombatVertex>(),
                D3D11_BIND_VERTEX_BUFFER,
                "XMAP14 combat vertices",
            )?,
            textures: BTreeMap::new(),
        })
    }

    unsafe fn texture<'a>(
        &'a mut self,
        gfx: &Gfx,
        resolver: &GameFileResolver,
        path: &str,
    ) -> Result<&'a CombatTexture> {
        if !self.textures.contains_key(path) {
            let decoded = counter_texture_rgba_from_resolver(resolver, path)?;
            let (texture, srv) = create_immutable_texture(
                gfx,
                decoded.width,
                decoded.height,
                DXGI_FORMAT_R8G8B8A8_UNORM,
                &decoded.rgba8,
                &format!("XMAP14 combat texture {path}"),
            )?;
            self.textures.insert(
                path.to_string(),
                CombatTexture {
                    _texture: texture,
                    srv,
                },
            );
        }
        self.textures
            .get(path)
            .ok_or_else(|| format!("XMAP14 texture cache lost {path}").into())
    }

    unsafe fn draw(
        &mut self,
        gfx: &Gfx,
        resolver: &GameFileResolver,
        texture1: &str,
        texture2: &str,
        vertices: [CombatVertex; 4],
    ) -> Result<()> {
        let srv1 = self.texture(gfx, resolver, texture1)?.srv.as_raw();
        let srv2 = self.texture(gfx, resolver, texture2)?.srv.as_raw();
        write_dynamic_slice(gfx, &self.vertex_buffer, &vertices, "XMAP14 combat vertices")?;
        write_dynamic_slice(
            gfx,
            &self.screen_cb,
            &[2.0 / gfx.width as f32, 2.0 / gfx.height as f32, 0.0, 0.0],
            "XMAP14 combat screen constants",
        )?;

        let context = gfx.context.as_raw();
        (*context).OMSetRenderTargets(1, [gfx.rtv()?.as_raw()].as_ptr(), ptr::null_mut());
        (*context).OMSetDepthStencilState(self.depth.as_raw(), 0);
        let blend_factor = [0.0f32; 4];
        (*context).OMSetBlendState(self.blend.as_raw(), &blend_factor, u32::MAX);
        (*context).RSSetViewports(
            1,
            &D3D11_VIEWPORT {
                TopLeftX: 0.0,
                TopLeftY: 0.0,
                Width: gfx.width as f32,
                Height: gfx.height as f32,
                MinDepth: 0.0,
                MaxDepth: 1.0,
            },
        );
        (*context).RSSetScissorRects(
            1,
            &RECT {
                left: 0,
                top: 0,
                right: i32::try_from(gfx.width).map_err(|_| "XMAP14 width exceeds i32")?,
                bottom: i32::try_from(gfx.height).map_err(|_| "XMAP14 height exceeds i32")?,
            },
        );
        (*context).RSSetState(self.rasterizer.as_raw());
        (*context).IASetPrimitiveTopology(D3D11_PRIMITIVE_TOPOLOGY_TRIANGLESTRIP);
        (*context).IASetInputLayout(self.input_layout.as_raw());
        let stride = mem::size_of::<CombatVertex>() as UINT;
        let offset = 0u32;
        (*context).IASetVertexBuffers(
            0,
            1,
            [self.vertex_buffer.as_raw()].as_ptr(),
            &stride,
            &offset,
        );
        (*context).VSSetShader(self.vs.as_raw(), ptr::null(), 0);
        (*context).VSSetConstantBuffers(0, 1, [self.screen_cb.as_raw()].as_ptr());
        (*context).PSSetShader(self.ps.as_raw(), ptr::null(), 0);
        (*context).PSSetShaderResources(0, 2, [srv1, srv2].as_ptr());
        (*context).PSSetSamplers(0, 1, [self.sampler.as_raw()].as_ptr());
        (*context).Draw(4, 0);
        (*context).PSSetShaderResources(0, 2, [ptr::null_mut(), ptr::null_mut()].as_ptr());
        (*context).OMSetBlendState(ptr::null_mut(), &blend_factor, u32::MAX);
        (*context).OMSetDepthStencilState(ptr::null_mut(), 0);
        (*context).RSSetState(ptr::null_mut());
        Ok(())
    }
}

pub(crate) struct CombatGpuPass {
    resolver: GameFileResolver,
    prepared: Option<PreparedXmap14Frame>,
    validated_content: Option<Vec<UsedContentKey>>,
    quads: Option<CombatQuadRuntime>,
    text: BTreeMap<String, TextRenderer>,
    grid_counters: Option<crate::counter_gpu::Xmap14CounterGpuRuntime>,
    last_receipt: Option<map_viewer::Xmap14ExecutionReceipt>,
}

impl CombatGpuPass {
    pub(crate) fn new(resolver: GameFileResolver) -> Self {
        Self {
            resolver,
            prepared: None,
            validated_content: None,
            quads: None,
            text: BTreeMap::new(),
            grid_counters: None,
            last_receipt: None,
        }
    }

    pub(crate) fn update(&mut self, prepared: Option<PreparedXmap14Frame>) -> Result<()> {
        self.last_receipt = None;
        if let Some(frame) = &prepared {
            frame.validate()?;
            if self.validated_content.as_ref() != Some(&frame.ordered_used_content) {
                for content in &frame.ordered_used_content {
                    self.resolver.read_admitted_used_content(content)?;
                }
                self.validated_content = Some(frame.ordered_used_content.clone());
                self.quads = None;
                self.text.clear();
                self.grid_counters = None;
            }
        } else {
            self.validated_content = None;
            self.quads = None;
            self.text.clear();
            self.grid_counters = None;
        }
        self.prepared = prepared;
        Ok(())
    }

    pub(crate) fn execute(
        &mut self,
        gfx: &Gfx,
        executor_target_sha256: &str,
    ) -> Result<Option<&map_viewer::Xmap14ExecutionReceipt>> {
        let Some(prepared) = self.prepared.as_ref() else {
            self.last_receipt = None;
            return Ok(None);
        };
        prepared.validate()?;
        let mut ordered_draw_receipts = Vec::new();
        for indicator in &prepared.indicators {
            for (draw_index, draw) in indicator.ordered_draws.iter().enumerate() {
                match draw {
                    PreparedXmap14Draw::Sprite(draw) => {
                        if self.quads.is_none() {
                            self.quads = Some(unsafe { CombatQuadRuntime::new(gfx)? });
                        }
                        let vertices = sprite_vertices(draw);
                        unsafe {
                            self.quads
                                .as_mut()
                                .ok_or("XMAP14 quad runtime disappeared")?
                                .draw(
                                    gfx,
                                    &self.resolver,
                                    &draw.texture_file,
                                    &draw.texture_file,
                                    vertices,
                                )?;
                        }
                        ordered_draw_receipts.push(draw_receipt_sha256(
                            executor_target_sha256,
                            &prepared.combat_indicator_assignment.assignment_sha256,
                            &indicator.identity,
                            draw_index,
                            None,
                            &PreparedXmap14Draw::Sprite(draw.clone()),
                            None,
                        )?);
                    }
                    PreparedXmap14Draw::Progress(draw) => {
                        let shader_key = prepared
                            .ordered_used_content
                            .iter()
                            .find(|content| {
                                content
                                    .normalized_game_relative_path
                                    .eq_ignore_ascii_case(&draw.shader_file)
                            })
                            .ok_or("XMAP14 progress shader is outside prepared content")?;
                        let shader = self.resolver.read_admitted_used_content(shader_key)?;
                        let actual_shader_sha256 = snapshot::sha256::Sha256::hex(&shader);
                        if actual_shader_sha256 != draw.shader_sha256 {
                            return Err("XMAP14 progress shader differs from prepared admitted program".into());
                        }
                        if self.quads.is_none() {
                            self.quads = Some(unsafe { CombatQuadRuntime::new(gfx)? });
                        }
                        let vertices = progress_vertices(draw);
                        unsafe {
                            self.quads
                                .as_mut()
                                .ok_or("XMAP14 quad runtime disappeared")?
                                .draw(
                                    gfx,
                                    &self.resolver,
                                    &draw.texture_file1,
                                    &draw.texture_file2,
                                    vertices,
                                )?;
                        }
                        ordered_draw_receipts.push(draw_receipt_sha256(
                            executor_target_sha256,
                            &prepared.combat_indicator_assignment.assignment_sha256,
                            &indicator.identity,
                            draw_index,
                            None,
                            &PreparedXmap14Draw::Progress(draw.clone()),
                            None,
                        )?);
                    }
                    PreparedXmap14Draw::Text(draw) => {
                        let font_key = format!("{}\0{}", draw.font_name, draw.font_language);
                        if !self.text.contains_key(&font_key) {
                            let font = load_game_font_for_language_from_resolver(
                                &self.resolver,
                                &draw.font_name,
                                &draw.font_language,
                            )?;
                            self.text.insert(font_key.clone(), TextRenderer::new(gfx, &font)?);
                        }
                        let text = self
                            .text
                            .get(&font_key)
                            .ok_or("XMAP14 text runtime disappeared")?;
                        let min = draw.box_min_f32_bits.map(f32::from_bits);
                        let size = draw.box_size_f32_bits.map(f32::from_bits);
                        let (width, height) = text.measure(&draw.text);
                        let x = min[0] + (size[0] - width as f32) * 0.5;
                        let y = min[1] + (size[1] - height as f32) * 0.5;
                        let (missing, submitted_pages) = text.draw_text_with_page_receipts(
                            gfx,
                            x,
                            y,
                            &draw.text,
                            text.font().default_color,
                        )?;
                        if !missing.is_empty() {
                            return Err(format!(
                                "XMAP14 font {} is missing text glyphs {:?}",
                                draw.font_name, missing
                            )
                            .into());
                        }
                        if submitted_pages.is_empty() {
                            return Err("XMAP14 non-empty text issued no atlas-page draw".into());
                        }
                        for page in submitted_pages {
                            ordered_draw_receipts.push(draw_receipt_sha256(
                                executor_target_sha256,
                                &prepared.combat_indicator_assignment.assignment_sha256,
                                &indicator.identity,
                                draw_index,
                                Some(page),
                                &PreparedXmap14Draw::Text(draw.clone()),
                                None,
                            )?);
                        }
                    }
                    PreparedXmap14Draw::Counter(draw) => {
                        if self.grid_counters.is_none() {
                            self.grid_counters = Some(
                                crate::counter_gpu::Xmap14CounterGpuRuntime::new(
                                    self.resolver.clone(),
                                ),
                            );
                        }
                        let receipts = self
                            .grid_counters
                            .as_mut()
                            .ok_or("XMAP14 naval-grid runtime disappeared")?
                            .execute(gfx, draw, &prepared.ordered_used_content)?;
                        if receipts.is_empty() {
                            return Err("XMAP14 naval-grid draw issued no GPU primitives".into());
                        }
                        for (primitive_index, native_receipt) in receipts.into_iter().enumerate() {
                            ordered_draw_receipts.push(draw_receipt_sha256(
                                executor_target_sha256,
                                &prepared.combat_indicator_assignment.assignment_sha256,
                                &indicator.identity,
                                draw_index,
                                Some(primitive_index),
                                &PreparedXmap14Draw::Counter(draw.clone()),
                                Some(&native_receipt),
                            )?);
                        }
                    }
                }
            }
        }
        let draw = if prepared.indicators.is_empty() {
            if !ordered_draw_receipts.is_empty() {
                return Err("XMAP14 empty prepared census issued native draws".into());
            }
            map_viewer::Xmap14NativeDrawClosure::AuthenticatedEmpty {
                enumerator_witness_sha256: prepared.enumerator_witness_sha256.clone(),
            }
        } else {
            let expected_semantic_draws: usize = prepared
                .indicators
                .iter()
                .map(|indicator| indicator.ordered_draws.len())
                .sum();
            if expected_semantic_draws == 0 {
                if !ordered_draw_receipts.is_empty() {
                    return Err("XMAP14 fully culled frame issued native draws".into());
                }
                map_viewer::Xmap14NativeDrawClosure::AuthenticatedCulled {
                    enumerator_witness_sha256: prepared.enumerator_witness_sha256.clone(),
                    indicator_count: u32::try_from(prepared.indicators.len())?,
                    ordered_identity_sha256: prepared.ordered_identity_sha256(),
                }
            } else {
                if ordered_draw_receipts.len() < expected_semantic_draws {
                return Err("XMAP14 non-empty prepared census did not execute every child draw".into());
                }
                map_viewer::Xmap14NativeDrawClosure::Present {
                    executor_target_sha256: executor_target_sha256.into(),
                    actual_draw_count: u32::try_from(ordered_draw_receipts.len())?,
                    ordered_draw_receipt_sha256s: ordered_draw_receipts,
                }
            }
        };
        let assignment = &prepared.combat_indicator_assignment;
        let receipt = map_viewer::Xmap14ExecutionReceipt {
            schema: map_viewer::XMAP14_NATIVE_RECEIPT_SCHEMA.into(),
            hour_index: prepared.hour_index,
            executor_target_sha256: executor_target_sha256.into(),
            assigned_topology_sha256: assignment.topology_sha256.clone(),
            assigned_stage_id: assignment.stage_name.clone(),
            assigned_operation_id: assignment.operation_name.clone(),
            assigned_ordinal: assignment.assigned_ordinal,
            assigned_assignment_sha256: assignment.assignment_sha256.clone(),
            prepared_frame_sha256: prepared.deterministic_sha256.clone(),
            hit_targets_sha256: snapshot::sha256::Sha256::hex(&serde_json::to_vec(
                &prepared.hit_targets,
            )?),
            draw,
            deterministic_sha256: String::new(),
        }
        .seal()?;
        receipt.validate_for(prepared, executor_target_sha256)?;
        self.last_receipt = Some(receipt);
        Ok(self.last_receipt.as_ref())
    }
}

fn draw_receipt_sha256(
    target_sha256: &str,
    assignment_sha256: &str,
    identity: &map_viewer::Xmap14IndicatorIdentity,
    draw_index: usize,
    atlas_page: Option<usize>,
    draw: &PreparedXmap14Draw,
    native_subdraw_receipt: Option<&str>,
) -> Result<String> {
    Ok(snapshot::sha256::Sha256::hex(&serde_json::to_vec(&(
        target_sha256,
        assignment_sha256,
        identity,
        draw_index,
        atlas_page,
        draw,
        native_subdraw_receipt,
    ))?))
}

fn sprite_vertices(draw: &PreparedXmap14SpriteDraw) -> [CombatVertex; 4] {
    std::array::from_fn(|index| CombatVertex {
        position: draw.positions_f32_bits[index].map(f32::from_bits),
        uv: draw.uv_f32_bits[index].map(f32::from_bits),
        modulation: draw.modulation_rgba_f32_bits.map(f32::from_bits),
        split: 1.0,
    })
}

fn progress_vertices(draw: &PreparedXmap14ProgressDraw) -> [CombatVertex; 4] {
    std::array::from_fn(|index| CombatVertex {
        position: draw.positions_f32_bits[index].map(f32::from_bits),
        uv: draw.uv_f32_bits[index].map(f32::from_bits),
        modulation: [1.0; 4],
        split: f32::from_bits(draw.current_state_f32_bits),
    })
}

fn input_element(
    semantic: *const i8,
    semantic_index: u32,
    format: u32,
    offset: u32,
) -> D3D11_INPUT_ELEMENT_DESC {
    D3D11_INPUT_ELEMENT_DESC {
        SemanticName: semantic,
        SemanticIndex: semantic_index,
        Format: format,
        InputSlot: 0,
        AlignedByteOffset: offset,
        InputSlotClass: D3D11_INPUT_PER_VERTEX_DATA,
        InstanceDataStepRate: 0,
    }
}

unsafe fn create_dynamic_buffer(
    gfx: &Gfx,
    byte_width: usize,
    bind_flags: u32,
    role: &str,
) -> Result<ComPtr<ID3D11Buffer>> {
    let desc = D3D11_BUFFER_DESC {
        ByteWidth: u32::try_from(byte_width).map_err(|_| format!("{role} exceeds D3D11 limits"))?,
        Usage: D3D11_USAGE_DYNAMIC,
        BindFlags: bind_flags,
        CPUAccessFlags: D3D11_CPU_ACCESS_WRITE,
        MiscFlags: 0,
        StructureByteStride: 0,
    };
    let mut raw = ptr::null_mut();
    hr_ok(
        (*gfx.device.as_raw()).CreateBuffer(&desc, ptr::null(), &mut raw),
        role,
    )?;
    ComPtr::from_raw(raw, role)
}

unsafe fn write_dynamic_slice<T: Copy>(
    gfx: &Gfx,
    buffer: &ComPtr<ID3D11Buffer>,
    values: &[T],
    role: &str,
) -> Result<()> {
    let mut mapped: D3D11_MAPPED_SUBRESOURCE = mem::zeroed();
    hr_ok(
        (*gfx.context.as_raw()).Map(
            buffer.as_raw() as *mut _,
            0,
            D3D11_MAP_WRITE_DISCARD,
            0,
            &mut mapped,
        ),
        role,
    )?;
    ptr::copy_nonoverlapping(
        values.as_ptr() as *const u8,
        mapped.pData as *mut u8,
        values.len() * mem::size_of::<T>(),
    );
    (*gfx.context.as_raw()).Unmap(buffer.as_raw() as *mut _, 0);
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn prepared_sprite_bits_reach_gui_triangle_strip_unchanged() {
        let draw = PreparedXmap14SpriteDraw {
            texture_file: "gfx/test.dds".into(),
            frame_index: 0,
            positions_f32_bits: [
                [1.0f32.to_bits(), 2.0f32.to_bits()],
                [1.0f32.to_bits(), 6.0f32.to_bits()],
                [5.0f32.to_bits(), 2.0f32.to_bits()],
                [5.0f32.to_bits(), 6.0f32.to_bits()],
            ],
            uv_f32_bits: [
                [0.0f32.to_bits(), 0.0f32.to_bits()],
                [0.0f32.to_bits(), 1.0f32.to_bits()],
                [1.0f32.to_bits(), 0.0f32.to_bits()],
                [1.0f32.to_bits(), 1.0f32.to_bits()],
            ],
            modulation_rgba_f32_bits: [1.0f32.to_bits(); 4],
        };
        let vertices = sprite_vertices(&draw);
        assert_eq!(vertices[0].position, [1.0, 2.0]);
        assert_eq!(vertices[3].position, [5.0, 6.0]);
        assert_eq!(vertices[3].uv, [1.0, 1.0]);
        assert_eq!(vertices[0].split, 1.0);
    }

    #[test]
    fn progress_current_state_reaches_every_vertex() {
        let draw = PreparedXmap14ProgressDraw {
            texture_file1: "a.dds".into(),
            texture_file2: "b.dds".into(),
            shader_file: "gfx/fx/progress.shader".into(),
            shader_sha256: "a".repeat(64),
            positions_f32_bits: [[0.0f32.to_bits(); 2]; 4],
            uv_f32_bits: [[0.0f32.to_bits(); 2]; 4],
            current_state_f32_bits: 0.625f32.to_bits(),
        };
        assert!(progress_vertices(&draw)
            .iter()
            .all(|vertex| vertex.split == 0.625));
    }

    #[test]
    fn combat_font_is_not_the_debug_overlay_font() {
        assert_ne!("hoi_16mbs", OVERLAY_FONT_NAME);
    }
}
