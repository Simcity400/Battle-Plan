#![allow(dead_code)]

use std::collections::{BTreeMap, BTreeSet};
use std::ffi::CString;
use std::mem;
use std::ptr;

use map_viewer::{
    build_mapname_vertices, compute_controller_label_placements_with_text,
    mapname_measured_glyph_parameters, mapname_tracked_advance_px, mapname_tracking_px,
    AssignedStage, ControllerLabelLawInputs, CountryLabelPlacement, GameFileResolver,
    LabelTextMetrics, MapLabelGlyphLayoutQuad, PreparedMapLabelFrame, PreparedMapLabelSet,
    ProvinceKind, ReadyMapLabelDrawContract, Xmap17DrawReceipt, Xmap17ExecutionIdentity,
};
use replay_core::{
    ConstantBufferBinding, Leaf, MapnameD80State, StaticMapDraw, ViewStatus, WorldFrame,
};
use serde::Serialize;
use sha2::{Digest, Sha256};
use winapi::shared::dxgiformat::{
    DXGI_FORMAT_R32G32B32_FLOAT, DXGI_FORMAT_R32G32_FLOAT, DXGI_FORMAT_R8G8B8A8_UNORM,
};
use winapi::shared::dxgitype::DXGI_SAMPLE_DESC;
use winapi::shared::minwindef::UINT;
use winapi::um::d3d11::{
    ID3D11BlendState, ID3D11Buffer, ID3D11DepthStencilState, ID3D11InputLayout, ID3D11PixelShader,
    ID3D11SamplerState, ID3D11ShaderResourceView, ID3D11Texture2D, ID3D11VertexShader,
    D3D11_BIND_CONSTANT_BUFFER, D3D11_BIND_SHADER_RESOURCE, D3D11_BIND_VERTEX_BUFFER,
    D3D11_BLEND_DESC, D3D11_BLEND_INV_SRC_ALPHA, D3D11_BLEND_ONE, D3D11_BLEND_OP_ADD,
    D3D11_BLEND_SRC_ALPHA, D3D11_BUFFER_DESC, D3D11_COLOR_WRITE_ENABLE_BLUE,
    D3D11_COLOR_WRITE_ENABLE_GREEN, D3D11_COLOR_WRITE_ENABLE_RED, D3D11_COMPARISON_ALWAYS,
    D3D11_COMPARISON_LESS_EQUAL, D3D11_COMPARISON_NOT_EQUAL, D3D11_CPU_ACCESS_WRITE,
    D3D11_DEPTH_STENCIL_DESC, D3D11_DEPTH_WRITE_MASK_ALL, D3D11_FILTER_MIN_MAG_MIP_LINEAR,
    D3D11_INPUT_ELEMENT_DESC, D3D11_INPUT_PER_VERTEX_DATA, D3D11_MAPPED_SUBRESOURCE,
    D3D11_MAP_WRITE_DISCARD, D3D11_SAMPLER_DESC, D3D11_STENCIL_OP_KEEP, D3D11_SUBRESOURCE_DATA,
    D3D11_TEXTURE2D_DESC, D3D11_TEXTURE_ADDRESS_CLAMP, D3D11_USAGE_DYNAMIC, D3D11_USAGE_IMMUTABLE,
    D3D11_VIEWPORT,
};
use winapi::um::d3dcommon::D3D11_PRIMITIVE_TOPOLOGY_TRIANGLELIST;

use crate::d3d::{hr_ok, ComPtr, Gfx};
use crate::font::{
    load_game_font_for_language_from_resolver, load_game_font_preserving_color, Font, FontPage,
};
use crate::render::{compile_shader, create_immutable_texture};
use crate::text::TextRenderer;
use crate::Result;

pub const MAP_LABEL_FONT_NAME: &str = "tahoma_60";
pub const XMAP21_RESOURCE_TEXT_DRAW_SCHEMA: &str =
    "hoi4.replay.xmap17-xmap21-resource-text-draw.v1";
pub const XMAP21_VP_CAPITAL_TEXT_DRAW_SCHEMA: &str =
    "hoi4.replay.xmap17-xmap21-vp-capital-text-draw.v1";

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct Xmap21ResourceTextDrawReceipt {
    pub schema: String,
    pub resource_text_batch_sha256: String,
    pub main_scene_target_sha256: String,
    pub operation_count: usize,
    pub submitted_glyph_count: usize,
    pub submitted_page_draw_count: usize,
    pub deterministic_sha256: String,
}

impl Xmap21ResourceTextDrawReceipt {
    pub fn validate(&self) -> Result<()> {
        if self.schema != XMAP21_RESOURCE_TEXT_DRAW_SCHEMA
            || self.resource_text_batch_sha256.len() != 64
            || self.main_scene_target_sha256.len() != 64
            || !self
                .resource_text_batch_sha256
                .bytes()
                .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
            || !self
                .main_scene_target_sha256
                .bytes()
                .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
        {
            return Err("XMAP17 XMAP21 resource-text receipt body is invalid".into());
        }
        let mut unsigned = self.clone();
        unsigned.deterministic_sha256.clear();
        let expected = format!("{:x}", Sha256::digest(serde_json::to_vec(&unsigned)?));
        if self.deterministic_sha256 != expected {
            return Err("XMAP17 XMAP21 resource-text receipt digest mismatch".into());
        }
        Ok(())
    }
}

pub struct Xmap21ResourceTextRenderer {
    text: TextRenderer,
}

impl Xmap21ResourceTextRenderer {
    pub fn new(gfx: &Gfx, resolver: &GameFileResolver, language: &str) -> Result<Self> {
        let font = load_game_font_for_language_from_resolver(resolver, "hoi_16mbs", language)?;
        Ok(Self {
            text: TextRenderer::new(gfx, &font)?,
        })
    }

    pub unsafe fn draw_xmap21_resource_text(
        &self,
        gfx: &Gfx,
        batch: &map_viewer::Xmap21ResourceTextBatch,
        render_target_view: *mut winapi::um::d3d11::ID3D11RenderTargetView,
        target_width: u32,
        target_height: u32,
        main_scene_target_sha256: &str,
    ) -> Result<Xmap21ResourceTextDrawReceipt> {
        batch.validate()?;
        if render_target_view.is_null() || target_width == 0 || target_height == 0 {
            return Err("XMAP17 XMAP21 resource text has no authenticated MainScene target".into());
        }
        let mut submitted_glyph_count = 0usize;
        let mut submitted_page_draw_count = 0usize;
        for operation in &batch.operations {
            let (glyphs, pages) = self.draw_xmap21_resource_text_operation(
                gfx,
                operation,
                render_target_view,
                target_width,
                target_height,
            )?;
            submitted_glyph_count = submitted_glyph_count
                .checked_add(glyphs)
                .ok_or("XMAP17 XMAP21 submitted glyph count overflow")?;
            submitted_page_draw_count = submitted_page_draw_count
                .checked_add(pages)
                .ok_or("XMAP17 XMAP21 page draw count overflow")?;
        }
        self.seal_xmap21_resource_text_receipt(
            batch,
            main_scene_target_sha256,
            submitted_glyph_count,
            submitted_page_draw_count,
        )
    }

    pub(crate) unsafe fn draw_xmap21_resource_text_operation(
        &self,
        gfx: &Gfx,
        operation: &map_viewer::Xmap21ResourceTextOperation,
        render_target_view: *mut winapi::um::d3d11::ID3D11RenderTargetView,
        target_width: u32,
        target_height: u32,
    ) -> Result<(usize, usize)> {
        if render_target_view.is_null() || target_width == 0 || target_height == 0 {
            return Err("XMAP17 XMAP21 resource text has no authenticated MainScene target".into());
        }
        let [left, top, right, bottom] = operation.rect_px;
        if right - left != operation.max_width as f32 || bottom - top != operation.max_height as f32
        {
            return Err(format!(
                "XMAP17 XMAP21 resource text {} is outside its authenticated rectangle",
                operation.stable_id
            )
            .into());
        }
        let layout = self
            .text
            .font()
            .layout_text(left, top, &operation.resolved_text);
        if !layout.missing.is_empty()
            || layout.advance > operation.max_width as f32
            || self.text.font().line_height > operation.max_height
            || layout
                .quads
                .iter()
                .any(|quad| quad.x0 < left || quad.y0 < top || quad.x1 > right || quad.y1 > bottom)
        {
            return Err(format!(
                "XMAP17 XMAP21 resource text {} cannot be drawn exactly by the effective font",
                operation.stable_id
            )
            .into());
        }
        let (missing, pages, glyphs) = self.text.draw_text_to_target_with_page_receipts(
            gfx,
            render_target_view,
            target_width,
            target_height,
            left,
            top,
            &operation.resolved_text,
            self.text.font().default_color,
        )?;
        if !missing.is_empty() {
            return Err("XMAP17 XMAP21 font changed between preflight and draw".into());
        }
        Ok((glyphs, pages.len()))
    }

    pub(crate) fn seal_xmap21_resource_text_receipt(
        &self,
        batch: &map_viewer::Xmap21ResourceTextBatch,
        main_scene_target_sha256: &str,
        submitted_glyph_count: usize,
        submitted_page_draw_count: usize,
    ) -> Result<Xmap21ResourceTextDrawReceipt> {
        let mut receipt = Xmap21ResourceTextDrawReceipt {
            schema: XMAP21_RESOURCE_TEXT_DRAW_SCHEMA.into(),
            resource_text_batch_sha256: batch.deterministic_sha256.clone(),
            main_scene_target_sha256: main_scene_target_sha256.to_owned(),
            operation_count: batch.operations.len(),
            submitted_glyph_count,
            submitted_page_draw_count,
            deterministic_sha256: String::new(),
        };
        receipt.deterministic_sha256 =
            format!("{:x}", Sha256::digest(serde_json::to_vec(&receipt)?));
        receipt.validate()?;
        Ok(receipt)
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct Xmap21VpCapitalTextDrawReceipt {
    pub schema: String,
    pub vp_capital_text_batch_sha256: String,
    pub main_scene_target_sha256: String,
    pub operation_count: usize,
    pub submitted_glyph_count: usize,
    pub submitted_page_draw_count: usize,
    pub deterministic_sha256: String,
}

impl Xmap21VpCapitalTextDrawReceipt {
    pub fn validate(&self) -> Result<()> {
        if self.schema != XMAP21_VP_CAPITAL_TEXT_DRAW_SCHEMA
            || self.vp_capital_text_batch_sha256.len() != 64
            || self.main_scene_target_sha256.len() != 64
            || !self
                .vp_capital_text_batch_sha256
                .bytes()
                .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
            || !self
                .main_scene_target_sha256
                .bytes()
                .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
        {
            return Err("XMAP17 XMAP21 VP/capital-text receipt body is invalid".into());
        }
        let mut unsigned = self.clone();
        unsigned.deterministic_sha256.clear();
        let expected = format!("{:x}", Sha256::digest(serde_json::to_vec(&unsigned)?));
        if self.deterministic_sha256 != expected {
            return Err("XMAP17 XMAP21 VP/capital-text receipt digest mismatch".into());
        }
        Ok(())
    }
}

pub struct Xmap21VpCapitalTextRenderer {
    text: TextRenderer,
}

impl Xmap21VpCapitalTextRenderer {
    pub fn new(gfx: &Gfx, resolver: &GameFileResolver, language: &str) -> Result<Self> {
        let font = load_game_font_for_language_from_resolver(resolver, "hoi_20bs", language)?;
        Ok(Self {
            text: TextRenderer::new(gfx, &font)?,
        })
    }

    pub unsafe fn draw_xmap21_vp_capital_text(
        &self,
        gfx: &Gfx,
        batch: &map_viewer::Xmap21VpCapitalTextBatch,
        render_target_view: *mut winapi::um::d3d11::ID3D11RenderTargetView,
        target_width: u32,
        target_height: u32,
        main_scene_target_sha256: &str,
    ) -> Result<Xmap21VpCapitalTextDrawReceipt> {
        batch.validate()?;
        if render_target_view.is_null() || target_width == 0 || target_height == 0 {
            return Err(
                "XMAP17 XMAP21 VP/capital text has no authenticated MainScene target".into(),
            );
        }
        let mut submitted_glyph_count = 0usize;
        let mut submitted_page_draw_count = 0usize;
        for operation in &batch.operations {
            let (glyphs, pages) = self.draw_xmap21_vp_capital_text_operation(
                gfx,
                operation,
                render_target_view,
                target_width,
                target_height,
            )?;
            submitted_glyph_count = submitted_glyph_count
                .checked_add(glyphs)
                .ok_or("XMAP17 XMAP21 submitted VP/capital glyph count overflow")?;
            submitted_page_draw_count = submitted_page_draw_count
                .checked_add(pages)
                .ok_or("XMAP17 XMAP21 VP/capital page draw count overflow")?;
        }
        self.seal_xmap21_vp_capital_text_receipt(
            batch,
            main_scene_target_sha256,
            submitted_glyph_count,
            submitted_page_draw_count,
        )
    }

    pub(crate) unsafe fn draw_xmap21_vp_capital_text_operation(
        &self,
        gfx: &Gfx,
        operation: &map_viewer::Xmap21VpCapitalTextOperation,
        render_target_view: *mut winapi::um::d3d11::ID3D11RenderTargetView,
        target_width: u32,
        target_height: u32,
    ) -> Result<(usize, usize)> {
        if render_target_view.is_null() || target_width == 0 || target_height == 0 {
            return Err(
                "XMAP17 XMAP21 VP/capital text has no authenticated MainScene target".into(),
            );
        }
        let [left, top, right, bottom] = operation.rect_px;
        if right - left != operation.max_width as f32 || bottom - top != operation.max_height as f32
        {
            return Err(format!(
                "XMAP17 XMAP21 VP/capital text {} is outside its authenticated rectangle",
                operation.stable_id
            )
            .into());
        }
        let layout = self
            .text
            .font()
            .layout_text(left, top, &operation.resolved_text);
        if !layout.missing.is_empty()
            || layout.advance > operation.max_width as f32
            || self.text.font().line_height > operation.max_height
            || layout
                .quads
                .iter()
                .any(|quad| quad.x0 < left || quad.y0 < top || quad.x1 > right || quad.y1 > bottom)
        {
            return Err(format!(
                "XMAP17 XMAP21 VP/capital text {} cannot be drawn exactly by the effective font",
                operation.stable_id
            )
            .into());
        }
        let (missing, pages, glyphs) = self.text.draw_text_to_target_with_page_receipts(
            gfx,
            render_target_view,
            target_width,
            target_height,
            left,
            top,
            &operation.resolved_text,
            self.text.font().default_color,
        )?;
        if !missing.is_empty() {
            return Err("XMAP17 XMAP21 VP/capital font changed between preflight and draw".into());
        }
        Ok((glyphs, pages.len()))
    }

    pub(crate) fn seal_xmap21_vp_capital_text_receipt(
        &self,
        batch: &map_viewer::Xmap21VpCapitalTextBatch,
        main_scene_target_sha256: &str,
        submitted_glyph_count: usize,
        submitted_page_draw_count: usize,
    ) -> Result<Xmap21VpCapitalTextDrawReceipt> {
        let mut receipt = Xmap21VpCapitalTextDrawReceipt {
            schema: XMAP21_VP_CAPITAL_TEXT_DRAW_SCHEMA.into(),
            vp_capital_text_batch_sha256: batch.deterministic_sha256.clone(),
            main_scene_target_sha256: main_scene_target_sha256.to_owned(),
            operation_count: batch.operations.len(),
            submitted_glyph_count,
            submitted_page_draw_count,
            deterministic_sha256: String::new(),
        };
        receipt.deterministic_sha256 =
            format!("{:x}", Sha256::digest(serde_json::to_vec(&receipt)?));
        receipt.validate()?;
        Ok(receipt)
    }
}

pub const MAP_LABEL_TRANSP_OFFSET_SEMANTIC: &str = replay_core::MAPNAME_TRANSP_OFFSET_SEMANTIC;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AuthenticatedMapLabelRuntimeConstants {
    pub transp_offset_x_f32_bits: [u32; 4],
    pub trailing_f32_bits: [u32; 4],
    pub raw_sha256: String,
}

pub struct ReplayControllerLabelInputs<'a> {
    pub province_ids: &'a [u32],
    pub width: usize,
    pub height: usize,
    pub province_to_state: &'a BTreeMap<u32, u32>,
    pub impassable_states: &'a BTreeSet<u32>,
    pub province_importance: &'a BTreeMap<u32, u64>,
    pub province_type: &'a BTreeMap<u32, ProvinceKind>,
}

pub fn measured_control_by_province(frame: &WorldFrame) -> Result<BTreeMap<u32, String>> {
    measured_controller_rows(
        frame.control_by_province_status(),
        frame.control_by_province(),
    )
}

fn measured_controller_rows(
    status: ViewStatus,
    rows: &BTreeMap<u32, Leaf<String>>,
) -> Result<BTreeMap<u32, String>> {
    if status != ViewStatus::Present {
        return Err("map labels: control_by_province domain is not present".into());
    }
    let mut measured = BTreeMap::new();
    for (&province_id, controller) in rows {
        match controller {
            Leaf::Measured(tag) if !tag.trim().is_empty() => {
                measured.insert(province_id, tag.clone());
            }
            Leaf::Measured(_) => {
                return Err(format!(
                    "map labels: province {province_id} has an empty measured controller"
                )
                .into());
            }
            Leaf::Absent => {
                return Err(format!(
                    "map labels: province {province_id} controller row is explicitly absent"
                )
                .into());
            }
            Leaf::Unobtained => {
                return Err(format!(
                    "map labels: province {province_id} controller row is unobtained"
                )
                .into());
            }
        }
    }
    Ok(measured)
}

pub fn controller_label_placements_from_replay(
    frame: &WorldFrame,
    inputs: &ReplayControllerLabelInputs<'_>,
    text_metrics: &BTreeMap<String, LabelTextMetrics>,
) -> Result<Vec<CountryLabelPlacement>> {
    let controllers = measured_control_by_province(frame)?;
    Ok(compute_controller_label_placements_with_text(
        &ControllerLabelLawInputs {
            province_ids: inputs.province_ids,
            width: inputs.width,
            height: inputs.height,
            province_to_state: inputs.province_to_state,
            controllers: &controllers,
            impassable_states: inputs.impassable_states,
            province_importance: inputs.province_importance,
            province_type: inputs.province_type,
            province_adjacencies: &BTreeSet::new(),
        },
        text_metrics,
    ))
}

impl AuthenticatedMapLabelRuntimeConstants {
    pub fn from_static_map_draw(draw: &StaticMapDraw) -> Result<Option<Self>> {
        match draw.mapname_d80_state()? {
            MapnameD80State::Missing => Ok(None),
            MapnameD80State::LegacyUnstamped => Err(
                "map labels: legacy mapname draw lacks authenticated Transp_OffsetX semantic"
                    .into(),
            ),
            MapnameD80State::Authenticated => {
                Self::from_constant_buffers(&draw.ordered_constant_buffers).map(Some)
            }
        }
    }

    pub fn from_constant_buffers(bindings: &[ConstantBufferBinding]) -> Result<Self> {
        const MAPNAME_STAGES: u8 = 0b0000_0011;
        const CB_BYTES: usize = 32;

        let mut covered_stages = 0u8;
        let mut admitted: Option<([u8; CB_BYTES], String)> = None;
        for binding in bindings.iter().filter(|binding| {
            binding.slot == 1 && binding.semantic_layout == MAP_LABEL_TRANSP_OFFSET_SEMANTIC
        }) {
            if binding.stage_mask_u8 == 0 || binding.stage_mask_u8 & !MAPNAME_STAGES != 0 {
                return Err("map labels: Transp_OffsetX CB has non-mapname shader stages".into());
            }
            if binding.byte_offset != 0
                || binding.byte_count as usize != CB_BYTES
                || binding.raw_bytes.len() != CB_BYTES
            {
                return Err("map labels: Transp_OffsetX CB is not the exact 32-byte range".into());
            }
            let observed_sha256 = format!("{:x}", Sha256::digest(&binding.raw_bytes));
            if observed_sha256 != binding.sha256 {
                return Err("map labels: Transp_OffsetX CB SHA-256 mismatch".into());
            }
            let bytes: [u8; CB_BYTES] = binding
                .raw_bytes
                .as_slice()
                .try_into()
                .map_err(|_| "map labels: Transp_OffsetX CB byte conversion failed")?;
            if let Some((prior, _)) = &admitted {
                if prior != &bytes {
                    return Err("map labels: VS/PS Transp_OffsetX observations disagree".into());
                }
            } else {
                admitted = Some((bytes, observed_sha256));
            }
            covered_stages |= binding.stage_mask_u8;
        }

        if covered_stages != MAPNAME_STAGES {
            return Err(
                "map labels: authenticated Transp_OffsetX CB is missing VS or PS coverage".into(),
            );
        }
        let (bytes, raw_sha256) = admitted
            .ok_or_else(|| "map labels: authenticated Transp_OffsetX CB is absent".to_string())?;
        let mut words = [0u32; 8];
        for (index, chunk) in bytes.chunks_exact(4).enumerate() {
            words[index] = u32::from_le_bytes(chunk.try_into().expect("four-byte chunk"));
        }
        let alpha = f32::from_bits(words[0]);
        let wrap = f32::from_bits(words[1]);
        if !alpha.is_finite() || !(0.0..=1.0).contains(&alpha) {
            return Err("map labels: authenticated Transp_OffsetX.x is outside [0,1]".into());
        }
        if !wrap.is_finite() {
            return Err("map labels: authenticated Transp_OffsetX.y is non-finite".into());
        }
        Ok(Self {
            transp_offset_x_f32_bits: words[0..4].try_into().expect("four words"),
            trailing_f32_bits: words[4..8].try_into().expect("four words"),
            raw_sha256,
        })
    }
}

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
    depth_stencil: ComPtr<ID3D11DepthStencilState>,
    scene_cb: ComPtr<ID3D11Buffer>,
    transp_cb: ComPtr<ID3D11Buffer>,
    vertex_buffer: ComPtr<ID3D11Buffer>,
    pages: Vec<(ComPtr<ID3D11Texture2D>, ComPtr<ID3D11ShaderResourceView>)>,
}

impl MapLabelRenderer {
    pub fn new(gfx: &Gfx, font: &Font) -> Result<Self> {
        let (vs_blob, ps_blob) = unsafe {
            (
                compile_shader(MAP_LABEL_SHADER_HLSL, "vs_main", "vs_4_0")?,
                compile_shader(MAP_LABEL_SHADER_HLSL, "ps_main", "ps_4_0")?,
            )
        };
        let vertex = unsafe {
            std::slice::from_raw_parts(
                (*vs_blob.as_raw()).GetBufferPointer() as *const u8,
                (*vs_blob.as_raw()).GetBufferSize(),
            )
        };
        let pixel = unsafe {
            std::slice::from_raw_parts(
                (*ps_blob.as_raw()).GetBufferPointer() as *const u8,
                (*ps_blob.as_raw()).GetBufferSize(),
            )
        };
        Self::new_from_shader_bytecode(gfx, &font.name, &font.pages, vertex, pixel)
    }

    pub fn new_product(
        gfx: &Gfx,
        resolver: &GameFileResolver,
        frame: &PreparedMapLabelFrame,
    ) -> Result<Self> {
        frame.validate()?;
        let atlas_bytes = resolver.read_admitted_used_content(&frame.program.font_atlas)?;
        let mut levels = game_ui::dds::decode_mip_chain_bytes(&atlas_bytes)
            .map_err(|error| format!("XMAP17 admitted font atlas mip chain: {error}"))?;
        if levels.is_empty() {
            return Err("XMAP17 admitted font atlas decoded to no mip levels".into());
        }
        let base = levels.remove(0);
        let page = FontPage {
            width: u32::try_from(base.width)?,
            height: u32::try_from(base.height)?,
            rgba: base.pixels,
            mips: levels
                .into_iter()
                .map(|level| {
                    Ok((
                        u32::try_from(level.width)?,
                        u32::try_from(level.height)?,
                        level.pixels,
                    ))
                })
                .collect::<Result<Vec<_>>>()?,
        };
        Self::new_from_shader_bytecode(
            gfx,
            MAP_LABEL_FONT_NAME,
            &[page],
            &frame.program.vertex_bytecode,
            &frame.program.pixel_bytecode,
        )
    }

    fn new_from_shader_bytecode(
        gfx: &Gfx,
        font_name: &str,
        font_pages: &[FontPage],
        vertex_bytecode: &[u8],
        pixel_bytecode: &[u8],
    ) -> Result<Self> {
        unsafe {
            let mut vs: *mut ID3D11VertexShader = ptr::null_mut();
            let hr = (*gfx.device.as_raw()).CreateVertexShader(
                vertex_bytecode.as_ptr() as *const _,
                vertex_bytecode.len(),
                ptr::null_mut(),
                &mut vs,
            );
            hr_ok(hr, "CreateVertexShader for map labels")?;
            let vs = ComPtr::from_raw(vs, "map-label vertex shader")?;

            let mut ps: *mut ID3D11PixelShader = ptr::null_mut();
            let hr = (*gfx.device.as_raw()).CreatePixelShader(
                pixel_bytecode.as_ptr() as *const _,
                pixel_bytecode.len(),
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
                vertex_bytecode.as_ptr() as *const _,
                vertex_bytecode.len(),
                &mut input_layout,
            );
            hr_ok(hr, "CreateInputLayout for map labels")?;
            let input_layout = ComPtr::from_raw(input_layout, "map-label input layout")?;

            let sampler = create_map_label_sampler(gfx)?;
            let blend = create_map_label_blend(gfx)?;
            let depth_stencil = create_map_label_depth_stencil(gfx)?;
            let scene_cb = create_dynamic_constant_buffer::<MapLabelSceneConstants>(
                gfx,
                "map-label scene constants",
            )?;
            let transp_cb = create_dynamic_constant_buffer::<MapLabelTranspOffset>(
                gfx,
                "map-label Transp_OffsetX constants",
            )?;
            let vertex_buffer = create_dynamic_vertex_buffer(gfx)?;

            let mut pages = Vec::with_capacity(font_pages.len());
            for (index, page) in font_pages.iter().enumerate() {
                let label = format!("map-label font {font_name} page {index} UNORM atlas");
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
                depth_stencil,
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
        runtime: &AuthenticatedMapLabelRuntimeConstants,
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
                    transp_offset_x: runtime.transp_offset_x_f32_bits.map(f32::from_bits),
                    _pad: runtime.trailing_f32_bits.map(f32::from_bits),
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
            (*context).OMSetDepthStencilState(self.depth_stencil.as_raw(), 4);
            (*context).RSSetState(ptr::null_mut());

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
            (*context).OMSetDepthStencilState(ptr::null_mut(), 0);
        }
        Ok(())
    }

    pub unsafe fn draw_prepared_product(
        &self,
        gfx: &Gfx,
        frame: &PreparedMapLabelFrame,
        ready: &ReadyMapLabelDrawContract,
        scene_cb0: &map_viewer::NativeXmap10MapCopySceneCb0View<'_>,
        main_scene: &map_viewer::NativeProductionMainSceneTarget,
        identity: Xmap17ExecutionIdentity,
        assigned_stage: AssignedStage,
    ) -> Result<Xmap17DrawReceipt> {
        let expected_totals = frame.preflight_execution(ready, &identity, &assigned_stage)?;
        frame.validate()?;
        ready.validate()?;
        if ready.label_frame_sha256 != frame.deterministic_sha256 {
            return Err("XMAP17 product renderer/frame identity mismatch".into());
        }
        if scene_cb0.raw_buffer().is_null()
            || main_scene.color_rtv().is_null()
            || main_scene.depth_dsv().is_null()
            || scene_cb0.device_identity() != gfx.device.as_raw() as usize
            || main_scene.device_identity() != gfx.device.as_raw() as usize
        {
            return Err("XMAP17 native providers are null or belong to another device".into());
        }
        if scene_cb0.main_scene_resource_identity_sha256() != main_scene.resource_identity_sha256()
            || ready.scene_cb0_sha256 != scene_cb0.payload_sha256()
            || ready.final_image_target_sha256 != main_scene.resource_identity_sha256()
            || identity.main_scene_target_sha256 != main_scene.resource_identity_sha256()
            || identity.map_copy_receipt_set_sha256 != scene_cb0.map_copy_receipt_set_sha256()
            || frame.map_copy_receipt_set_sha256 != scene_cb0.map_copy_receipt_set_sha256()
            || scene_cb0.hour_index() != frame.hour_index
            || scene_cb0.map_width() != frame.map_width
            || scene_cb0.map_height() != frame.map_height
            || scene_cb0.camera_frame_sha256() != frame.viewer_camera_sha256
        {
            return Err(
                "XMAP17 XMAP10 scene/target identities do not match the prepared frame".into(),
            );
        }
        let near = frame.selected_label_set(ready.near_selector)?;
        let far = frame.selected_label_set(ready.far_selector)?;
        let Some((_, atlas)) = self.pages.first() else {
            return Err("XMAP17 product renderer has no uploaded map-font atlas".into());
        };
        let near_rows = near
            .map(prepared_label_set_rows)
            .transpose()?
            .unwrap_or_default();
        let far_rows = far
            .map(prepared_label_set_rows)
            .transpose()?
            .unwrap_or_default();
        let copies = u64::try_from(frame.wrap_offset_f32_bits.len())?;
        let preflight_draw_count = u32::try_from(
            (near_rows.len() + far_rows.len())
                .checked_mul(frame.wrap_offset_f32_bits.len())
                .ok_or("XMAP17 preflight draw-count overflow")?,
        )?;
        let preflight_vertex_count = near_rows
            .iter()
            .chain(&far_rows)
            .map(|vertices| u64::try_from(vertices.len()))
            .collect::<std::result::Result<Vec<_>, _>>()?
            .into_iter()
            .try_fold(0u64, |total, count| total.checked_add(count))
            .ok_or("XMAP17 preflight vertex-count overflow")?
            .checked_mul(copies)
            .ok_or("XMAP17 preflight submitted-vertex overflow")?;
        if expected_totals != (preflight_draw_count, preflight_vertex_count) {
            return Err("XMAP17 native preflight totals disagree with prepared label bytes".into());
        }
        let mut draw_count = 0u32;
        let mut submitted_vertex_count = 0u64;
        unsafe {
            let context = gfx.context.as_raw();
            let rtvs = [main_scene.color_rtv()];
            (*context).OMSetRenderTargets(1, rtvs.as_ptr(), main_scene.depth_dsv());
            let viewport = D3D11_VIEWPORT {
                TopLeftX: 0.0,
                TopLeftY: 0.0,
                Width: main_scene.desc().color_width as f32,
                Height: main_scene.desc().color_height as f32,
                MinDepth: 0.0,
                MaxDepth: 1.0,
            };
            (*context).RSSetViewports(1, &viewport);
            (*context).IASetPrimitiveTopology(D3D11_PRIMITIVE_TOPOLOGY_TRIANGLELIST);
            (*context).IASetInputLayout(self.input_layout.as_raw());
            let stride = mem::size_of::<MapLabelVertex>() as UINT;
            let offset = 0u32;
            (*context).IASetVertexBuffers(
                0,
                1,
                [self.vertex_buffer.as_raw()].as_ptr(),
                &stride,
                &offset,
            );
            (*context).VSSetShader(self.vs.as_raw(), ptr::null(), 0);
            (*context).PSSetShader(self.ps.as_raw(), ptr::null(), 0);
            (*context).VSSetConstantBuffers(0, 1, [scene_cb0.raw_buffer()].as_ptr());
            (*context).PSSetConstantBuffers(0, 1, [scene_cb0.raw_buffer()].as_ptr());
            (*context).VSSetConstantBuffers(1, 1, [self.transp_cb.as_raw()].as_ptr());
            (*context).PSSetConstantBuffers(1, 1, [self.transp_cb.as_raw()].as_ptr());
            (*context).PSSetSamplers(0, 1, [self.sampler.as_raw()].as_ptr());
            (*context).PSSetShaderResources(0, 1, [atlas.as_raw()].as_ptr());
            let blend_factor = [0.0f32; 4];
            (*context).OMSetBlendState(self.blend.as_raw(), &blend_factor, u32::MAX);
            (*context).OMSetDepthStencilState(self.depth_stencil.as_raw(), 4);
            (*context).RSSetState(ptr::null_mut());

            for (rows, alpha_bits) in [
                (&near_rows, ready.near_alpha_f32_bits),
                (&far_rows, ready.far_alpha_f32_bits),
            ] {
                for wrap_bits in &frame.wrap_offset_f32_bits {
                    write_dynamic_buffer(
                        context,
                        &self.transp_cb,
                        &MapLabelTranspOffset {
                            transp_offset_x: [
                                f32::from_bits(alpha_bits),
                                f32::from_bits(*wrap_bits),
                                0.0,
                                0.0,
                            ],
                            _pad: [0.0; 4],
                        },
                        "XMAP17 product Transp_OffsetX constants",
                    )?;
                    for vertices in rows.iter() {
                        write_dynamic_vertex_chunk(context, &self.vertex_buffer, vertices)?;
                        (*context).Draw(u32::try_from(vertices.len())?, 0);
                        draw_count = draw_count
                            .checked_add(1)
                            .ok_or("XMAP17 native draw-count overflow")?;
                        submitted_vertex_count = submitted_vertex_count
                            .checked_add(u64::try_from(vertices.len())?)
                            .ok_or("XMAP17 native vertex-count overflow")?;
                    }
                }
            }
            (*context).PSSetShaderResources(
                0,
                1,
                [ptr::null_mut::<ID3D11ShaderResourceView>()].as_ptr(),
            );
            (*context).OMSetBlendState(ptr::null_mut(), &blend_factor, u32::MAX);
            (*context).OMSetDepthStencilState(ptr::null_mut(), 0);
        }
        frame.seal_execution_receipt(
            ready,
            identity,
            assigned_stage,
            draw_count,
            submitted_vertex_count,
        )
    }
}

fn prepared_label_set_rows(set: &PreparedMapLabelSet) -> Result<Vec<Vec<MapLabelVertex>>> {
    set.rows
        .iter()
        .map(|row| {
            let vertices = prepared_label_row_vertices(set, row)?;
            if vertices.len() > VERTEX_CAPACITY {
                return Err(format!(
                    "XMAP17 label row {} has {} vertices, exceeding the exact one-draw capacity {}",
                    row.key,
                    vertices.len(),
                    VERTEX_CAPACITY
                )
                .into());
            }
            Ok(vertices)
        })
        .collect()
}

fn prepared_label_row_vertices(
    set: &PreparedMapLabelSet,
    row: &map_viewer::PreparedMapLabelRow,
) -> Result<Vec<MapLabelVertex>> {
    let first = usize::try_from(row.first_vertex)?
        .checked_mul(map_viewer::MAPNAME_VERTEX_STRIDE)
        .ok_or("XMAP17 row byte offset overflow")?;
    let byte_len = usize::try_from(row.vertex_count)?
        .checked_mul(map_viewer::MAPNAME_VERTEX_STRIDE)
        .ok_or("XMAP17 row byte length overflow")?;
    let bytes = set
        .vertex_bytes
        .get(first..first + byte_len)
        .ok_or("XMAP17 row points outside the prepared vertex payload")?;
    let mut vertices = Vec::with_capacity(usize::try_from(row.vertex_count)?);
    for vertex in bytes.chunks_exact(map_viewer::MAPNAME_VERTEX_STRIDE) {
        let word = |offset: usize| -> f32 {
            f32::from_le_bytes(
                vertex[offset..offset + 4]
                    .try_into()
                    .expect("four-byte word"),
            )
        };
        let candidate = MapLabelVertex {
            position: [word(0), word(4), word(8)],
            uv: [word(12), word(16)],
        };
        if candidate
            .position
            .into_iter()
            .chain(candidate.uv)
            .any(|value| !value.is_finite())
        {
            return Err("XMAP17 prepared vertex payload contains a non-finite value".into());
        }
        vertices.push(candidate);
    }
    Ok(vertices)
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
    desc.RenderTarget[0].RenderTargetWriteMask = (D3D11_COLOR_WRITE_ENABLE_RED
        | D3D11_COLOR_WRITE_ENABLE_GREEN
        | D3D11_COLOR_WRITE_ENABLE_BLUE) as u8;
    let mut blend: *mut ID3D11BlendState = ptr::null_mut();
    let hr = (*gfx.device.as_raw()).CreateBlendState(&desc, &mut blend);
    hr_ok(hr, "CreateBlendState for map labels")?;
    ComPtr::from_raw(blend, "map-label blend state")
}

unsafe fn create_map_label_depth_stencil(gfx: &Gfx) -> Result<ComPtr<ID3D11DepthStencilState>> {
    let mut desc: D3D11_DEPTH_STENCIL_DESC = mem::zeroed();
    desc.DepthEnable = 0;
    desc.DepthWriteMask = D3D11_DEPTH_WRITE_MASK_ALL;
    desc.DepthFunc = D3D11_COMPARISON_LESS_EQUAL;
    desc.StencilEnable = 1;
    desc.StencilReadMask = 4;
    desc.StencilWriteMask = u8::MAX;
    desc.FrontFace.StencilFailOp = D3D11_STENCIL_OP_KEEP;
    desc.FrontFace.StencilDepthFailOp = D3D11_STENCIL_OP_KEEP;
    desc.FrontFace.StencilPassOp = D3D11_STENCIL_OP_KEEP;
    desc.FrontFace.StencilFunc = D3D11_COMPARISON_NOT_EQUAL;
    desc.BackFace.StencilFailOp = D3D11_STENCIL_OP_KEEP;
    desc.BackFace.StencilDepthFailOp = D3D11_STENCIL_OP_KEEP;
    desc.BackFace.StencilPassOp = D3D11_STENCIL_OP_KEEP;
    desc.BackFace.StencilFunc = D3D11_COMPARISON_ALWAYS;
    let mut state = ptr::null_mut();
    let hr = (*gfx.device.as_raw()).CreateDepthStencilState(&desc, &mut state);
    hr_ok(hr, "CreateDepthStencilState for map labels")?;
    ComPtr::from_raw(state, "map-label depth-stencil state")
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

    fn transp_offset_binding(stage_mask_u8: u8, words: [u32; 8]) -> ConstantBufferBinding {
        let raw_bytes = words
            .into_iter()
            .flat_map(u32::to_le_bytes)
            .collect::<Vec<_>>();
        ConstantBufferBinding {
            stage_mask_u8,
            slot: 1,
            byte_offset: 0,
            byte_count: raw_bytes.len() as u32,
            sha256: format!("{:x}", Sha256::digest(&raw_bytes)),
            raw_bytes,
            semantic_layout: MAP_LABEL_TRANSP_OFFSET_SEMANTIC.into(),
        }
    }

    #[test]
    fn b10_admits_hash_verified_stage_complete_transp_offset_bits() {
        let words = [
            0x3f22_1719,
            (-5632.0f32).to_bits(),
            0,
            0,
            0x7fc0_0001,
            0x8000_0000,
            0,
            0,
        ];
        let runtime = AuthenticatedMapLabelRuntimeConstants::from_constant_buffers(&[
            transp_offset_binding(0b01, words),
            transp_offset_binding(0b10, words),
        ])
        .expect("authenticated mapname CB1");
        assert_eq!(runtime.transp_offset_x_f32_bits, words[0..4]);
        assert_eq!(runtime.trailing_f32_bits, words[4..8]);
    }

    #[test]
    fn b10_rejects_slot_only_or_conflicting_transp_offset_observations() {
        let words = [1.0f32.to_bits(), 0, 0, 0, 0, 0, 0, 0];
        let mut slot_only = transp_offset_binding(0b11, words);
        slot_only.semantic_layout = "unproven_slot_1".into();
        assert!(
            AuthenticatedMapLabelRuntimeConstants::from_constant_buffers(&[slot_only]).is_err()
        );

        let mut conflicting = words;
        conflicting[0] = 0.5f32.to_bits();
        assert!(
            AuthenticatedMapLabelRuntimeConstants::from_constant_buffers(&[
                transp_offset_binding(0b01, words),
                transp_offset_binding(0b10, conflicting),
            ])
            .is_err()
        );
    }

    #[test]
    fn b10_materializes_only_measured_controller_rows() {
        let rows = BTreeMap::from([
            (10, Leaf::Measured("GER".to_string())),
            (11, Leaf::Measured("POL".to_string())),
        ]);
        assert_eq!(
            measured_controller_rows(ViewStatus::Present, &rows).expect("measured rows"),
            BTreeMap::from([(10, "GER".to_string()), (11, "POL".to_string())])
        );

        let unobtained = BTreeMap::from([(10, Leaf::Unobtained)]);
        assert!(measured_controller_rows(ViewStatus::Present, &unobtained).is_err());
        assert!(measured_controller_rows(ViewStatus::DomainAbsent, &BTreeMap::new()).is_err());
    }

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
        let oracle_path = replay_test_gates::require_existing_file(
            std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
                .join("../map_viewer/tests/fixtures/mapname/engine_label_oracle.json"),
        );
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

    #[test]
    fn product_draw_binds_the_typed_xmap10_main_scene_not_the_swapchain() {
        let source = crate::source_scan::map_labels_production();
        let start = source
            .find("pub unsafe fn draw_prepared_product(")
            .expect("product label executor");
        let product = &source[start
            ..source
                .find("fn prepared_label_row_vertices(")
                .expect("end of product label executor")];
        assert!(product.contains("&map_viewer::NativeXmap10MapCopySceneCb0View<'_>"));
        assert!(product.contains("&map_viewer::NativeProductionMainSceneTarget"));
        assert!(
            product.contains("let rtvs = [main_scene.color_rtv()]")
                && product.contains("main_scene.depth_dsv()")
        );
        assert!(product.contains("OMSetDepthStencilState(self.depth_stencil.as_raw(), 4)"));
        assert!(product.contains("RSSetState(ptr::null_mut())"));
        assert!(product.contains(
            "frame.map_copy_receipt_set_sha256 != scene_cb0.map_copy_receipt_set_sha256()"
        ));
        assert!(!product.contains("gfx.rtv()?"));
        assert!(!product.contains("OMSetRenderTargets(1, rtvs.as_ptr(), ptr::null_mut())"));
        let preflight = product
            .find("frame.preflight_execution(ready, &identity, &assigned_stage)")
            .expect("complete XMAP17 contract preflight");
        let row_preflight = product
            .find(".map(prepared_label_set_rows)")
            .expect("prepared label-byte preflight");
        let native_context = product
            .find("let context = gfx.context.as_raw()")
            .expect("native D3D execution boundary");
        assert!(preflight < row_preflight && row_preflight < native_context);

        let blend_start = source
            .find("unsafe fn create_map_label_blend(")
            .expect("mapname blend state");
        let blend = &source[blend_start
            ..source
                .find("unsafe fn create_map_label_depth_stencil(")
                .expect("end of mapname blend state")];
        assert!(
            blend.contains("D3D11_COLOR_WRITE_ENABLE_RED")
                && blend.contains("D3D11_COLOR_WRITE_ENABLE_GREEN")
                && blend.contains("D3D11_COLOR_WRITE_ENABLE_BLUE")
        );
        assert!(!blend.contains("D3D11_COLOR_WRITE_ENABLE_ALPHA"));
    }

    #[test]
    fn xmap21_resource_text_receipt_binds_batch_and_main_scene() {
        let mut receipt = Xmap21ResourceTextDrawReceipt {
            schema: XMAP21_RESOURCE_TEXT_DRAW_SCHEMA.into(),
            resource_text_batch_sha256: "a".repeat(64),
            main_scene_target_sha256: "b".repeat(64),
            operation_count: 2,
            submitted_glyph_count: 8,
            submitted_page_draw_count: 2,
            deterministic_sha256: String::new(),
        };
        receipt.deterministic_sha256 = format!(
            "{:x}",
            Sha256::digest(serde_json::to_vec(&receipt).unwrap())
        );
        receipt.validate().unwrap();

        let mut forged = receipt.clone();
        forged.resource_text_batch_sha256 = "c".repeat(64);
        assert!(forged.validate().is_err());
    }
}
