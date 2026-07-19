//! CAP6 counter-quad GPU handoff for the captured production-map renderer.
//!
//! `map_viewer::CounterRenderBatch` owns FILE/CAPTURE/DERIVED CPU geometry.
//! This module owns only the D3D11 upload and draw seam. A batch is not enough
//! authority to draw: the retained CAP6 corpus proves the common GUI pipeline,
//! but runtime modulation, complete quad order, and the target-format receipt
//! remain frame-specific. Missing or mismatched admission therefore produces
//! zero GPU draws without affecting the authenticated portable map/UI route.

use std::collections::{BTreeMap, BTreeSet};
use std::ffi::CString;
use std::mem;
use std::path::{Path, PathBuf};
use std::ptr;

use map_viewer::{
    counter_texture_rgba_from_resolver, CounterHonesty, CounterQuad, CounterQuadLayer,
    CounterRenderBatch, GameFileResolver,
};
use sha2::{Digest, Sha256};
use winapi::shared::dxgiformat::{
    DXGI_FORMAT_R32G32B32A32_FLOAT, DXGI_FORMAT_R32G32_FLOAT, DXGI_FORMAT_R8G8B8A8_UNORM,
};
use winapi::shared::minwindef::UINT;
use winapi::shared::windef::RECT;
use winapi::um::d3d11::{
    ID3D11BlendState, ID3D11Buffer, ID3D11DepthStencilState, ID3D11InputLayout, ID3D11PixelShader,
    ID3D11RasterizerState, ID3D11SamplerState, ID3D11ShaderResourceView, ID3D11Texture2D,
    ID3D11VertexShader, D3D11_BIND_CONSTANT_BUFFER, D3D11_BIND_VERTEX_BUFFER, D3D11_BLEND_DESC,
    D3D11_BLEND_INV_SRC_ALPHA, D3D11_BLEND_OP_ADD, D3D11_BLEND_SRC_ALPHA, D3D11_BUFFER_DESC,
    D3D11_COLOR_WRITE_ENABLE_ALL, D3D11_COMPARISON_LESS, D3D11_COMPARISON_NEVER,
    D3D11_CPU_ACCESS_WRITE, D3D11_CULL_BACK, D3D11_DEPTH_STENCIL_DESC, D3D11_DEPTH_WRITE_MASK_ALL,
    D3D11_FILL_SOLID, D3D11_FILTER_MIN_MAG_LINEAR_MIP_POINT, D3D11_INPUT_ELEMENT_DESC,
    D3D11_INPUT_PER_VERTEX_DATA, D3D11_MAPPED_SUBRESOURCE, D3D11_MAP_WRITE_DISCARD,
    D3D11_RASTERIZER_DESC, D3D11_SAMPLER_DESC, D3D11_TEXTURE_ADDRESS_CLAMP,
    D3D11_TEXTURE_ADDRESS_WRAP, D3D11_USAGE_DYNAMIC, D3D11_VIEWPORT,
};
use winapi::um::d3dcommon::D3D11_PRIMITIVE_TOPOLOGY_TRIANGLESTRIP;

use crate::d3d::{hr_ok, ComPtr, Gfx};
use crate::render::{compile_shader, create_immutable_texture};
use crate::Result;

pub(crate) const CAP6_PIPELINE_RDC_SHA256: &str =
    "6ea38b10bd38f25e9499c9b349fdcd5d6591124e0919c855c5da69e0209d4fb2";
pub(crate) const CAP6_RESTORE_DRAW_CHUNK: u32 = 29_204;
pub(crate) const CAP6_COUNTER_BACKING_DRAW_CHUNK: u32 = 31_499;
pub(crate) const CAP6_TARGET_WIDTH: u32 = 1_920;
pub(crate) const CAP6_TARGET_HEIGHT: u32 = 1_080;
pub(crate) const CAP6_TARGET_FORMAT: u32 = DXGI_FORMAT_R8G8B8A8_UNORM;

const COUNTER_HLSL: &str = r#"
cbuffer ScreenCB : register(b0) {
    float2 clip_scale;
    float2 _pad;
};

Texture2D CounterTexture : register(t0);
SamplerState CounterSampler : register(s0);

struct VsIn {
    float2 pos : POSITION;
    float2 uv : TEXCOORD0;
    float4 modulation : COLOR0;
};

struct VsOut {
    float4 pos : SV_Position;
    float2 uv : TEXCOORD0;
    float4 modulation : COLOR0;
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
    return output;
}

float4 ps_main(VsOut input) : SV_Target {
    return CounterTexture.Sample(CounterSampler, input.uv) * input.modulation;
}
"#;

#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub(crate) struct CounterGpuQuadKey {
    pub(crate) unit_id: String,
    pub(crate) layer: CounterQuadLayer,
}

#[derive(Clone, Debug, PartialEq)]
pub(crate) struct CounterGpuQuadReceipt {
    pub(crate) key: CounterGpuQuadKey,
    pub(crate) texture_file: String,
    pub(crate) frame_index: u32,
    pub(crate) texture_width: usize,
    pub(crate) texture_height: usize,
    pub(crate) decoded_rgba_sha256: String,
    pub(crate) modulation_rgba: [f32; 4],
    pub(crate) modulation_honesty: CounterHonesty,
}

/// Event-scoped authority required in addition to CPU geometry. There is no
/// default constructor: the captured renderer must receive a mechanically
/// joined frame receipt, not infer one from stable replay IDs or draw layers.
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct CounterGpuFrameAdmission {
    pub(crate) captured_session_dir: PathBuf,
    pub(crate) source_rdc_sha256: String,
    pub(crate) restore_draw_chunk: u32,
    pub(crate) first_counter_draw_chunk: u32,
    pub(crate) target_width: u32,
    pub(crate) target_height: u32,
    pub(crate) target_format: u32,
    pub(crate) ordered_quads: Vec<CounterGpuQuadReceipt>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum CounterGpuOmissionReason {
    CpuBatchEmpty,
    FrameAdmissionUnavailable,
    PipelineEvidenceMismatch,
    ActiveCaptureSessionMismatch,
    TargetReceiptMismatch,
    DuplicateCpuQuadKey,
    OrderedReceiptCoverageMismatch,
    ReceiptTextureOrFrameMismatch,
    TextureEvidenceUnavailable,
    TextureReceiptMismatch,
    GeometryEvidenceUnavailable,
    ModulationEvidenceUnavailable,
}

impl CounterGpuOmissionReason {
    pub(crate) const fn code(&self) -> &'static str {
        match self {
            Self::CpuBatchEmpty => "CpuBatchEmpty",
            Self::FrameAdmissionUnavailable => "FrameAdmissionUnavailable",
            Self::PipelineEvidenceMismatch => "PipelineEvidenceMismatch",
            Self::ActiveCaptureSessionMismatch => "ActiveCaptureSessionMismatch",
            Self::TargetReceiptMismatch => "TargetReceiptMismatch",
            Self::DuplicateCpuQuadKey => "DuplicateCpuQuadKey",
            Self::OrderedReceiptCoverageMismatch => "OrderedReceiptCoverageMismatch",
            Self::ReceiptTextureOrFrameMismatch => "ReceiptTextureOrFrameMismatch",
            Self::TextureEvidenceUnavailable => "TextureEvidenceUnavailable",
            Self::TextureReceiptMismatch => "TextureReceiptMismatch",
            Self::GeometryEvidenceUnavailable => "GeometryEvidenceUnavailable",
            Self::ModulationEvidenceUnavailable => "ModulationEvidenceUnavailable",
        }
    }
}

#[derive(Clone, Copy)]
#[repr(C)]
struct CounterGpuVertex {
    position: [f32; 2],
    uv: [f32; 2],
    modulation: [f32; 4],
}

#[derive(Clone)]
struct CounterGpuDraw {
    key: CounterGpuQuadKey,
    texture_file: String,
    texture_width: usize,
    texture_height: usize,
    decoded_rgba_sha256: String,
    vertices: [CounterGpuVertex; 4],
}

#[derive(Clone, Default)]
struct CounterGpuDrawPlan {
    draws: Vec<CounterGpuDraw>,
    omission: Option<CounterGpuOmissionReason>,
}

impl CounterGpuDrawPlan {
    fn omitted(reason: CounterGpuOmissionReason) -> Self {
        Self {
            draws: Vec::new(),
            omission: Some(reason),
        }
    }
}

fn quad_key(unit_id: &str, quad: &CounterQuad) -> CounterGpuQuadKey {
    CounterGpuQuadKey {
        unit_id: unit_id.to_string(),
        layer: quad.layer,
    }
}

fn compose_counter_gpu_plan(
    batch: &CounterRenderBatch,
    admission: Option<&CounterGpuFrameAdmission>,
    active_session_dir: &Path,
    target_width: u32,
    target_height: u32,
    target_format: u32,
) -> CounterGpuDrawPlan {
    let mut cpu_quads = BTreeMap::<CounterGpuQuadKey, &CounterQuad>::new();
    for instance in &batch.instances {
        if instance.stacking_honesty == CounterHonesty::Fit {
            return CounterGpuDrawPlan::omitted(
                CounterGpuOmissionReason::GeometryEvidenceUnavailable,
            );
        }
        for quad in &instance.quads {
            if quad.geometry_honesty == CounterHonesty::Fit
                || quad.selection_honesty == CounterHonesty::Fit
            {
                return CounterGpuDrawPlan::omitted(
                    CounterGpuOmissionReason::GeometryEvidenceUnavailable,
                );
            }
            if cpu_quads
                .insert(quad_key(&instance.unit_id, quad), quad)
                .is_some()
            {
                return CounterGpuDrawPlan::omitted(CounterGpuOmissionReason::DuplicateCpuQuadKey);
            }
        }
    }
    if cpu_quads.is_empty() {
        return CounterGpuDrawPlan::omitted(CounterGpuOmissionReason::CpuBatchEmpty);
    }
    let Some(admission) = admission else {
        return CounterGpuDrawPlan::omitted(CounterGpuOmissionReason::FrameAdmissionUnavailable);
    };
    if admission.source_rdc_sha256 != CAP6_PIPELINE_RDC_SHA256
        || admission.restore_draw_chunk != CAP6_RESTORE_DRAW_CHUNK
        || admission.first_counter_draw_chunk != CAP6_COUNTER_BACKING_DRAW_CHUNK
    {
        return CounterGpuDrawPlan::omitted(CounterGpuOmissionReason::PipelineEvidenceMismatch);
    }
    if admission.captured_session_dir.as_path() != active_session_dir {
        return CounterGpuDrawPlan::omitted(CounterGpuOmissionReason::ActiveCaptureSessionMismatch);
    }
    if admission.target_width != target_width
        || admission.target_height != target_height
        || admission.target_format != target_format
        || target_width != CAP6_TARGET_WIDTH
        || target_height != CAP6_TARGET_HEIGHT
        || target_format != CAP6_TARGET_FORMAT
    {
        return CounterGpuDrawPlan::omitted(CounterGpuOmissionReason::TargetReceiptMismatch);
    }
    if admission.ordered_quads.len() != cpu_quads.len() {
        return CounterGpuDrawPlan::omitted(
            CounterGpuOmissionReason::OrderedReceiptCoverageMismatch,
        );
    }
    let mut seen = BTreeSet::new();
    let mut texture_receipts = BTreeMap::<String, (usize, usize, String)>::new();
    let mut draws = Vec::with_capacity(admission.ordered_quads.len());
    for receipt in &admission.ordered_quads {
        if !seen.insert(receipt.key.clone()) {
            return CounterGpuDrawPlan::omitted(
                CounterGpuOmissionReason::OrderedReceiptCoverageMismatch,
            );
        }
        let Some(quad) = cpu_quads.get(&receipt.key).copied() else {
            return CounterGpuDrawPlan::omitted(
                CounterGpuOmissionReason::OrderedReceiptCoverageMismatch,
            );
        };
        if quad.texture_file != receipt.texture_file || quad.frame_index != receipt.frame_index {
            return CounterGpuDrawPlan::omitted(
                CounterGpuOmissionReason::ReceiptTextureOrFrameMismatch,
            );
        }
        if receipt.texture_width == 0
            || receipt.texture_height == 0
            || !is_sha256(&receipt.decoded_rgba_sha256)
        {
            return CounterGpuDrawPlan::omitted(CounterGpuOmissionReason::TextureReceiptMismatch);
        }
        let texture_receipt = (
            receipt.texture_width,
            receipt.texture_height,
            receipt.decoded_rgba_sha256.clone(),
        );
        if texture_receipts
            .insert(receipt.texture_file.clone(), texture_receipt.clone())
            .is_some_and(|previous| previous != texture_receipt)
        {
            return CounterGpuDrawPlan::omitted(CounterGpuOmissionReason::TextureReceiptMismatch);
        }
        if !receipt
            .modulation_rgba
            .iter()
            .all(|value| value.is_finite())
            || matches!(
                receipt.modulation_honesty,
                CounterHonesty::Derived | CounterHonesty::Fit
            )
        {
            return CounterGpuDrawPlan::omitted(
                CounterGpuOmissionReason::ModulationEvidenceUnavailable,
            );
        }
        // CAPTURE GUI IA order is TL, BL, TR, BR as a four-vertex strip. The
        // CPU quad stores clockwise TL, TR, BR, BL, hence this exact remap.
        let vertices = [0usize, 3, 1, 2].map(|index| CounterGpuVertex {
            position: [
                quad.vertices[index].position_px.x,
                quad.vertices[index].position_px.y,
            ],
            uv: quad.vertices[index].uv,
            modulation: receipt.modulation_rgba,
        });
        draws.push(CounterGpuDraw {
            key: receipt.key.clone(),
            texture_file: receipt.texture_file.clone(),
            texture_width: receipt.texture_width,
            texture_height: receipt.texture_height,
            decoded_rgba_sha256: receipt.decoded_rgba_sha256.clone(),
            vertices,
        });
    }
    CounterGpuDrawPlan {
        draws,
        omission: None,
    }
}

fn is_sha256(value: &str) -> bool {
    value.len() == 64 && value.bytes().all(|byte| byte.is_ascii_hexdigit())
}

fn resolve_counter_textures(
    resolver: &GameFileResolver,
    plan: &CounterGpuDrawPlan,
) -> std::result::Result<BTreeMap<String, map_viewer::CounterTextureRgba>, CounterGpuOmissionReason>
{
    let mut decoded_by_path = BTreeMap::new();
    for draw in &plan.draws {
        if decoded_by_path.contains_key(&draw.texture_file) {
            continue;
        }
        let decoded = counter_texture_rgba_from_resolver(resolver, &draw.texture_file)
            .map_err(|_| CounterGpuOmissionReason::TextureEvidenceUnavailable)?;
        let rgba_sha256 = format!("{:x}", Sha256::digest(&decoded.rgba8));
        if decoded.width != draw.texture_width
            || decoded.height != draw.texture_height
            || rgba_sha256 != draw.decoded_rgba_sha256
        {
            return Err(CounterGpuOmissionReason::TextureReceiptMismatch);
        }
        decoded_by_path.insert(draw.texture_file.clone(), decoded);
    }
    Ok(decoded_by_path)
}

struct CounterGpuTexture {
    _texture: ComPtr<ID3D11Texture2D>,
    srv: ComPtr<ID3D11ShaderResourceView>,
    decoded_rgba_sha256: String,
}

struct CounterGpuRuntime {
    vs: ComPtr<ID3D11VertexShader>,
    ps: ComPtr<ID3D11PixelShader>,
    input_layout: ComPtr<ID3D11InputLayout>,
    sampler: ComPtr<ID3D11SamplerState>,
    blend: ComPtr<ID3D11BlendState>,
    rasterizer: ComPtr<ID3D11RasterizerState>,
    depth: ComPtr<ID3D11DepthStencilState>,
    screen_cb: ComPtr<ID3D11Buffer>,
    vertex_buffer: ComPtr<ID3D11Buffer>,
    vertex_capacity: usize,
    textures: BTreeMap<String, CounterGpuTexture>,
}

impl CounterGpuRuntime {
    unsafe fn new(gfx: &Gfx, vertex_capacity: usize) -> Result<Self> {
        let vs_blob = compile_shader(COUNTER_HLSL, "vs_main", "vs_4_0")?;
        let ps_blob = compile_shader(COUNTER_HLSL, "ps_main", "ps_4_0")?;
        let mut vs = ptr::null_mut();
        hr_ok(
            (*gfx.device.as_raw()).CreateVertexShader(
                (*vs_blob.as_raw()).GetBufferPointer(),
                (*vs_blob.as_raw()).GetBufferSize(),
                ptr::null_mut(),
                &mut vs,
            ),
            "CreateVertexShader for CAP6 counter pass",
        )?;
        let vs = ComPtr::from_raw(vs, "CAP6 counter vertex shader")?;
        let mut ps = ptr::null_mut();
        hr_ok(
            (*gfx.device.as_raw()).CreatePixelShader(
                (*ps_blob.as_raw()).GetBufferPointer(),
                (*ps_blob.as_raw()).GetBufferSize(),
                ptr::null_mut(),
                &mut ps,
            ),
            "CreatePixelShader for CAP6 counter pass",
        )?;
        let ps = ComPtr::from_raw(ps, "CAP6 counter pixel shader")?;

        let position = CString::new("POSITION").expect("static semantic");
        let texcoord = CString::new("TEXCOORD").expect("static semantic");
        let color = CString::new("COLOR").expect("static semantic");
        let elements = [
            input_element(position.as_ptr(), 0, DXGI_FORMAT_R32G32_FLOAT, 0),
            input_element(texcoord.as_ptr(), 0, DXGI_FORMAT_R32G32_FLOAT, 8),
            input_element(color.as_ptr(), 0, DXGI_FORMAT_R32G32B32A32_FLOAT, 16),
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
            "CreateInputLayout for CAP6 counter pass",
        )?;
        let input_layout = ComPtr::from_raw(input_layout, "CAP6 counter input layout")?;

        let mut sampler_desc: D3D11_SAMPLER_DESC = mem::zeroed();
        sampler_desc.Filter = D3D11_FILTER_MIN_MAG_LINEAR_MIP_POINT;
        sampler_desc.AddressU = D3D11_TEXTURE_ADDRESS_CLAMP;
        sampler_desc.AddressV = D3D11_TEXTURE_ADDRESS_CLAMP;
        sampler_desc.AddressW = D3D11_TEXTURE_ADDRESS_WRAP;
        sampler_desc.MipLODBias = -0.800_000_011_920_929;
        sampler_desc.ComparisonFunc = D3D11_COMPARISON_NEVER;
        sampler_desc.MinLOD = 0.0;
        sampler_desc.MaxLOD = 1024.0;
        let mut sampler = ptr::null_mut();
        hr_ok(
            (*gfx.device.as_raw()).CreateSamplerState(&sampler_desc, &mut sampler),
            "CreateSamplerState for CAP6 counter pass",
        )?;
        let sampler = ComPtr::from_raw(sampler, "CAP6 counter sampler")?;

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
            "CreateBlendState for CAP6 counter pass",
        )?;
        let blend = ComPtr::from_raw(blend, "CAP6 counter blend state")?;

        let raster_desc = D3D11_RASTERIZER_DESC {
            FillMode: D3D11_FILL_SOLID,
            CullMode: D3D11_CULL_BACK,
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
            "CreateRasterizerState for CAP6 counter pass",
        )?;
        let rasterizer = ComPtr::from_raw(rasterizer, "CAP6 counter rasterizer")?;

        let mut depth_desc: D3D11_DEPTH_STENCIL_DESC = mem::zeroed();
        depth_desc.DepthEnable = 0;
        depth_desc.DepthWriteMask = D3D11_DEPTH_WRITE_MASK_ALL;
        depth_desc.DepthFunc = D3D11_COMPARISON_LESS;
        let mut depth = ptr::null_mut();
        hr_ok(
            (*gfx.device.as_raw()).CreateDepthStencilState(&depth_desc, &mut depth),
            "CreateDepthStencilState for CAP6 counter pass",
        )?;
        let depth = ComPtr::from_raw(depth, "CAP6 counter depth state")?;

        let screen_cb = create_dynamic_buffer(
            gfx,
            mem::size_of::<[f32; 4]>(),
            D3D11_BIND_CONSTANT_BUFFER,
            "CAP6 counter screen constants",
        )?;
        let vertex_capacity = vertex_capacity.max(4);
        let vertex_buffer = create_dynamic_buffer(
            gfx,
            vertex_capacity
                .checked_mul(mem::size_of::<CounterGpuVertex>())
                .ok_or("CAP6 counter vertex capacity overflow")?,
            D3D11_BIND_VERTEX_BUFFER,
            "CAP6 counter vertex buffer",
        )?;

        Ok(Self {
            vs,
            ps,
            input_layout,
            sampler,
            blend,
            rasterizer,
            depth,
            screen_cb,
            vertex_buffer,
            vertex_capacity,
            textures: BTreeMap::new(),
        })
    }

    unsafe fn ensure_vertex_capacity(&mut self, gfx: &Gfx, needed: usize) -> Result<()> {
        if needed <= self.vertex_capacity {
            return Ok(());
        }
        self.vertex_buffer = create_dynamic_buffer(
            gfx,
            needed
                .checked_mul(mem::size_of::<CounterGpuVertex>())
                .ok_or("CAP6 counter vertex capacity overflow")?,
            D3D11_BIND_VERTEX_BUFFER,
            "CAP6 counter resized vertex buffer",
        )?;
        self.vertex_capacity = needed;
        Ok(())
    }

    unsafe fn ensure_textures(
        &mut self,
        gfx: &Gfx,
        decoded_by_path: &BTreeMap<String, map_viewer::CounterTextureRgba>,
        draws: &[CounterGpuDraw],
    ) -> Result<()> {
        for texture_file in draws
            .iter()
            .map(|draw| draw.texture_file.as_str())
            .collect::<BTreeSet<_>>()
        {
            let draw = draws
                .iter()
                .find(|draw| draw.texture_file == texture_file)
                .ok_or("CAP6 texture draw disappeared during upload")?;
            if self
                .textures
                .get(texture_file)
                .is_some_and(|texture| texture.decoded_rgba_sha256 == draw.decoded_rgba_sha256)
            {
                continue;
            }
            let decoded = decoded_by_path
                .get(texture_file)
                .ok_or("CAP6 decoded texture receipt disappeared during upload")?;
            let (texture, srv) = create_immutable_texture(
                gfx,
                decoded.width,
                decoded.height,
                DXGI_FORMAT_R8G8B8A8_UNORM,
                &decoded.rgba8,
                &format!("CAP6 counter texture {}", decoded.texture_file),
            )?;
            self.textures.insert(
                texture_file.to_string(),
                CounterGpuTexture {
                    _texture: texture,
                    srv,
                    decoded_rgba_sha256: draw.decoded_rgba_sha256.clone(),
                },
            );
        }
        Ok(())
    }

    unsafe fn draw(
        &mut self,
        gfx: &Gfx,
        decoded_by_path: &BTreeMap<String, map_viewer::CounterTextureRgba>,
        plan: &CounterGpuDrawPlan,
    ) -> Result<usize> {
        let vertex_count = plan
            .draws
            .len()
            .checked_mul(4)
            .ok_or("CAP6 counter vertex count overflow")?;
        self.ensure_vertex_capacity(gfx, vertex_count)?;
        self.ensure_textures(gfx, decoded_by_path, &plan.draws)?;
        let vertices = plan
            .draws
            .iter()
            .flat_map(|draw| draw.vertices)
            .collect::<Vec<_>>();
        write_dynamic_slice(gfx, &self.vertex_buffer, &vertices, "CAP6 counter vertices")?;
        let screen = [2.0 / gfx.width as f32, 2.0 / gfx.height as f32, 0.0, 0.0];
        write_dynamic_slice(
            gfx,
            &self.screen_cb,
            &screen,
            "CAP6 counter screen constants",
        )?;

        let context = gfx.context.as_raw();
        let rtv = [gfx.rtv()?.as_raw()];
        (*context).OMSetRenderTargets(1, rtv.as_ptr(), ptr::null_mut());
        (*context).OMSetDepthStencilState(self.depth.as_raw(), 0);
        let blend_factor = [0.0f32; 4];
        (*context).OMSetBlendState(self.blend.as_raw(), &blend_factor, u32::MAX);
        let viewport = D3D11_VIEWPORT {
            TopLeftX: 0.0,
            TopLeftY: 0.0,
            Width: gfx.width as f32,
            Height: gfx.height as f32,
            MinDepth: 0.0,
            MaxDepth: 1.0,
        };
        (*context).RSSetViewports(1, &viewport);
        let scissor = RECT {
            left: 0,
            top: 0,
            right: i32::try_from(gfx.width).map_err(|_| "CAP6 target width exceeds i32")?,
            bottom: i32::try_from(gfx.height).map_err(|_| "CAP6 target height exceeds i32")?,
        };
        (*context).RSSetScissorRects(1, &scissor);
        (*context).RSSetState(self.rasterizer.as_raw());
        (*context).IASetPrimitiveTopology(D3D11_PRIMITIVE_TOPOLOGY_TRIANGLESTRIP);
        (*context).IASetInputLayout(self.input_layout.as_raw());
        let stride = mem::size_of::<CounterGpuVertex>() as UINT;
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
        (*context).PSSetSamplers(0, 1, [self.sampler.as_raw()].as_ptr());
        for (index, draw) in plan.draws.iter().enumerate() {
            let texture = self
                .textures
                .get(&draw.texture_file)
                .ok_or("CAP6 counter texture cache lost a planned texture")?;
            (*context).PSSetShaderResources(0, 1, [texture.srv.as_raw()].as_ptr());
            (*context).Draw(
                4,
                u32::try_from(index * 4).map_err(|_| "CAP6 vertex offset overflow")?,
            );
        }
        let null_srv = [ptr::null_mut()];
        (*context).PSSetShaderResources(0, 1, null_srv.as_ptr());
        (*context).OMSetBlendState(ptr::null_mut(), &blend_factor, u32::MAX);
        (*context).OMSetDepthStencilState(ptr::null_mut(), 0);
        (*context).RSSetState(ptr::null_mut());
        Ok(plan.draws.len())
    }
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

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub(crate) struct CounterGpuOutcome {
    pub(crate) draw_count: usize,
    pub(crate) omission: Option<CounterGpuOmissionReason>,
}

pub(crate) struct CounterGpuPass {
    resolver: GameFileResolver,
    active_session_dir: PathBuf,
    batch: CounterRenderBatch,
    admission: Option<CounterGpuFrameAdmission>,
    runtime: Option<CounterGpuRuntime>,
    last_outcome: CounterGpuOutcome,
}

impl CounterGpuPass {
    pub(crate) fn new(resolver: GameFileResolver, active_session_dir: PathBuf) -> Self {
        Self {
            resolver,
            active_session_dir,
            batch: CounterRenderBatch::default(),
            admission: None,
            runtime: None,
            last_outcome: CounterGpuOutcome::default(),
        }
    }

    pub(crate) fn update(
        &mut self,
        batch: CounterRenderBatch,
        admission: Option<CounterGpuFrameAdmission>,
    ) {
        self.batch = batch;
        self.admission = admission;
    }

    pub(crate) fn last_outcome(&self) -> &CounterGpuOutcome {
        &self.last_outcome
    }

    pub(crate) fn draw(&mut self, gfx: &Gfx) -> Result<()> {
        // CAP6 screen geometry and frame admission are event-scoped. Consume
        // both exactly once so a camera/hour transition cannot redraw stale
        // screen-space quads if the producer misses an update.
        let batch = mem::take(&mut self.batch);
        let admission = self.admission.take();
        let plan = compose_counter_gpu_plan(
            &batch,
            admission.as_ref(),
            &self.active_session_dir,
            gfx.width,
            gfx.height,
            gfx.target_format(),
        );
        if let Some(reason) = plan.omission.clone() {
            self.last_outcome = CounterGpuOutcome {
                draw_count: 0,
                omission: Some(reason),
            };
            return Ok(());
        }
        let decoded_by_path = match resolve_counter_textures(&self.resolver, &plan) {
            Ok(decoded) => decoded,
            Err(reason) => {
                self.last_outcome = CounterGpuOutcome {
                    draw_count: 0,
                    omission: Some(reason),
                };
                return Ok(());
            }
        };
        let needed = plan.draws.len().saturating_mul(4);
        if self.runtime.is_none() {
            self.runtime = Some(unsafe { CounterGpuRuntime::new(gfx, needed)? });
        }
        let draw_count = unsafe {
            self.runtime
                .as_mut()
                .expect("counter runtime created")
                .draw(gfx, &decoded_by_path, &plan)?
        };
        self.last_outcome = CounterGpuOutcome {
            draw_count,
            omission: None,
        };
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use map_viewer::{CounterQuadVertex, CounterRenderInstance, ScreenPointPx};

    fn quad(layer: CounterQuadLayer, texture: &str) -> CounterQuad {
        CounterQuad {
            layer,
            texture_file: texture.into(),
            source_file: "interface/mapicons.gfx".into(),
            source_line: 79,
            frame_index: 0,
            selection_honesty: CounterHonesty::Capture,
            vertices: [
                vertex(10.0, 20.0, 0.0, 0.0),
                vertex(30.0, 20.0, 1.0, 0.0),
                vertex(30.0, 40.0, 1.0, 1.0),
                vertex(10.0, 40.0, 0.0, 1.0),
            ],
            geometry_honesty: CounterHonesty::Derived,
        }
    }

    fn vertex(x: f32, y: f32, u: f32, v: f32) -> CounterQuadVertex {
        CounterQuadVertex {
            position_px: ScreenPointPx { x, y },
            uv: [u, v],
        }
    }

    fn batch() -> CounterRenderBatch {
        CounterRenderBatch {
            instances: vec![CounterRenderInstance {
                unit_id: "unit:7".into(),
                province_id: 1,
                stack_index: 0,
                stack_count: 1,
                origin_px: ScreenPointPx { x: 10.0, y: 20.0 },
                stack_offset_gui: [0.0, 0.0],
                stacking_honesty: CounterHonesty::Capture,
                quads: vec![
                    quad(
                        CounterQuadLayer::Frame,
                        "gfx/interface/onmap_unit_counter.dds",
                    ),
                    quad(
                        CounterQuadLayer::StrengthBar,
                        "gfx/interface/mapicons/mapicon_bar_str_sprite.dds",
                    ),
                ],
            }],
            omissions: Vec::new(),
        }
    }

    fn receipt() -> CounterGpuFrameAdmission {
        CounterGpuFrameAdmission {
            captured_session_dir: PathBuf::from("capture/session"),
            source_rdc_sha256: CAP6_PIPELINE_RDC_SHA256.into(),
            restore_draw_chunk: CAP6_RESTORE_DRAW_CHUNK,
            first_counter_draw_chunk: CAP6_COUNTER_BACKING_DRAW_CHUNK,
            target_width: CAP6_TARGET_WIDTH,
            target_height: CAP6_TARGET_HEIGHT,
            target_format: CAP6_TARGET_FORMAT,
            ordered_quads: vec![
                CounterGpuQuadReceipt {
                    key: CounterGpuQuadKey {
                        unit_id: "unit:7".into(),
                        layer: CounterQuadLayer::StrengthBar,
                    },
                    texture_file: "gfx/interface/mapicons/mapicon_bar_str_sprite.dds".into(),
                    frame_index: 0,
                    texture_width: 3,
                    texture_height: 17,
                    decoded_rgba_sha256: "11".repeat(32),
                    modulation_rgba: [1.0; 4],
                    modulation_honesty: CounterHonesty::Capture,
                },
                CounterGpuQuadReceipt {
                    key: CounterGpuQuadKey {
                        unit_id: "unit:7".into(),
                        layer: CounterQuadLayer::Frame,
                    },
                    texture_file: "gfx/interface/onmap_unit_counter.dds".into(),
                    frame_index: 0,
                    texture_width: 198,
                    texture_height: 27,
                    decoded_rgba_sha256:
                        "273c7fc23b12a199d46f3a5a591c9fc5004b4a254d18169019ee54cc092e913d".into(),
                    modulation_rgba: [0.32, 0.71, 0.39, 1.0],
                    modulation_honesty: CounterHonesty::Capture,
                },
            ],
        }
    }

    #[test]
    fn gpu_plan_omits_nonempty_cpu_batch_without_frame_receipt() {
        let plan = compose_counter_gpu_plan(
            &batch(),
            None,
            Path::new("capture/session"),
            CAP6_TARGET_WIDTH,
            CAP6_TARGET_HEIGHT,
            CAP6_TARGET_FORMAT,
        );
        assert!(plan.draws.is_empty());
        assert_eq!(
            plan.omission,
            Some(CounterGpuOmissionReason::FrameAdmissionUnavailable)
        );
    }

    #[test]
    fn gpu_plan_rejects_app_bgra_target_not_proven_by_cap6() {
        let plan = compose_counter_gpu_plan(
            &batch(),
            Some(&receipt()),
            Path::new("capture/session"),
            CAP6_TARGET_WIDTH,
            CAP6_TARGET_HEIGHT,
            winapi::shared::dxgiformat::DXGI_FORMAT_B8G8R8A8_UNORM,
        );
        assert!(plan.draws.is_empty());
        assert_eq!(
            plan.omission,
            Some(CounterGpuOmissionReason::TargetReceiptMismatch)
        );
    }

    #[test]
    fn exact_receipt_controls_order_modulation_and_triangle_strip_remap() {
        let plan = compose_counter_gpu_plan(
            &batch(),
            Some(&receipt()),
            Path::new("capture/session"),
            CAP6_TARGET_WIDTH,
            CAP6_TARGET_HEIGHT,
            CAP6_TARGET_FORMAT,
        );
        assert_eq!(plan.omission, None);
        assert_eq!(plan.draws.len(), 2);
        assert_eq!(plan.draws[0].key.layer, CounterQuadLayer::StrengthBar);
        assert_eq!(plan.draws[1].key.layer, CounterQuadLayer::Frame);
        assert_eq!(
            plan.draws[1]
                .vertices
                .iter()
                .map(|vertex| vertex.position)
                .collect::<Vec<_>>(),
            vec![[10.0, 20.0], [10.0, 40.0], [30.0, 20.0], [30.0, 40.0]],
        );
        assert!(plan.draws[1]
            .vertices
            .iter()
            .all(|vertex| vertex.modulation == [0.32, 0.71, 0.39, 1.0]));
    }

    #[test]
    fn incomplete_receipt_never_draws_a_partial_batch() {
        let mut admission = receipt();
        admission.ordered_quads.pop();
        let plan = compose_counter_gpu_plan(
            &batch(),
            Some(&admission),
            Path::new("capture/session"),
            CAP6_TARGET_WIDTH,
            CAP6_TARGET_HEIGHT,
            CAP6_TARGET_FORMAT,
        );
        assert!(plan.draws.is_empty());
        assert_eq!(
            plan.omission,
            Some(CounterGpuOmissionReason::OrderedReceiptCoverageMismatch)
        );
    }

    #[test]
    fn receipt_for_another_capture_session_is_rejected() {
        let plan = compose_counter_gpu_plan(
            &batch(),
            Some(&receipt()),
            Path::new("capture/another-session"),
            CAP6_TARGET_WIDTH,
            CAP6_TARGET_HEIGHT,
            CAP6_TARGET_FORMAT,
        );
        assert!(plan.draws.is_empty());
        assert_eq!(
            plan.omission,
            Some(CounterGpuOmissionReason::ActiveCaptureSessionMismatch)
        );
    }

    #[test]
    fn missing_texture_identity_rejects_the_whole_batch() {
        let mut admission = receipt();
        admission.ordered_quads[0].decoded_rgba_sha256.clear();
        let plan = compose_counter_gpu_plan(
            &batch(),
            Some(&admission),
            Path::new("capture/session"),
            CAP6_TARGET_WIDTH,
            CAP6_TARGET_HEIGHT,
            CAP6_TARGET_FORMAT,
        );
        assert!(plan.draws.is_empty());
        assert_eq!(
            plan.omission,
            Some(CounterGpuOmissionReason::TextureReceiptMismatch)
        );
    }
}
