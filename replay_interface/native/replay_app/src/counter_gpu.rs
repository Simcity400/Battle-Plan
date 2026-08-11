//! Native the game counter-sprite GPU pipeline and legacy CAP6 admission handoff.
//!
//! The shared native runtime preserves the established engine counter law:
//! f32 modulation, linear/linear/point sampling with -0.8 mip bias, captured
//! clamp/clamp/wrap addressing, and straight-alpha blending. XMAP13 supplies
//! complete effective-file-authenticated primitives. CAP6 remains only the
//! admission oracle for the legacy `CounterRenderBatch` comparison route.

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
use crate::text::TextRenderer;
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

/// Shared native counter-sprite pipeline. Its state is the established the game
/// counter law (float modulation, linear/linear/point sampling, -0.8 mip bias,
/// straight-alpha blending); CAP6 only controls admission to the legacy pass.
struct CounterSpriteGpuRuntime {
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

impl CounterSpriteGpuRuntime {
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
            "CreateVertexShader for counter-sprite pass",
        )?;
        let vs = ComPtr::from_raw(vs, "counter-sprite vertex shader")?;
        let mut ps = ptr::null_mut();
        hr_ok(
            (*gfx.device.as_raw()).CreatePixelShader(
                (*ps_blob.as_raw()).GetBufferPointer(),
                (*ps_blob.as_raw()).GetBufferSize(),
                ptr::null_mut(),
                &mut ps,
            ),
            "CreatePixelShader for counter-sprite pass",
        )?;
        let ps = ComPtr::from_raw(ps, "counter-sprite pixel shader")?;

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
            "CreateInputLayout for counter-sprite pass",
        )?;
        let input_layout = ComPtr::from_raw(input_layout, "counter-sprite input layout")?;

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
            "CreateSamplerState for counter-sprite pass",
        )?;
        let sampler = ComPtr::from_raw(sampler, "counter-sprite sampler")?;

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
            "CreateBlendState for counter-sprite pass",
        )?;
        let blend = ComPtr::from_raw(blend, "counter-sprite blend state")?;

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
            "CreateRasterizerState for counter-sprite pass",
        )?;
        let rasterizer = ComPtr::from_raw(rasterizer, "counter-sprite rasterizer")?;

        let mut depth_desc: D3D11_DEPTH_STENCIL_DESC = mem::zeroed();
        depth_desc.DepthEnable = 0;
        depth_desc.DepthWriteMask = D3D11_DEPTH_WRITE_MASK_ALL;
        depth_desc.DepthFunc = D3D11_COMPARISON_LESS;
        let mut depth = ptr::null_mut();
        hr_ok(
            (*gfx.device.as_raw()).CreateDepthStencilState(&depth_desc, &mut depth),
            "CreateDepthStencilState for counter-sprite pass",
        )?;
        let depth = ComPtr::from_raw(depth, "counter-sprite depth state")?;

        let screen_cb = create_dynamic_buffer(
            gfx,
            mem::size_of::<[f32; 4]>(),
            D3D11_BIND_CONSTANT_BUFFER,
            "counter-sprite screen constants",
        )?;
        let vertex_capacity = vertex_capacity.max(4);
        let vertex_buffer = create_dynamic_buffer(
            gfx,
            vertex_capacity
                .checked_mul(mem::size_of::<CounterGpuVertex>())
                .ok_or("counter-sprite vertex capacity overflow")?,
            D3D11_BIND_VERTEX_BUFFER,
            "counter-sprite vertex buffer",
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
                .ok_or("counter-sprite vertex capacity overflow")?,
            D3D11_BIND_VERTEX_BUFFER,
            "counter-sprite resized vertex buffer",
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
                .ok_or("counter-sprite texture draw disappeared during upload")?;
            if self
                .textures
                .get(texture_file)
                .is_some_and(|texture| texture.decoded_rgba_sha256 == draw.decoded_rgba_sha256)
            {
                continue;
            }
            let decoded = decoded_by_path
                .get(texture_file)
                .ok_or("counter-sprite decoded texture receipt disappeared during upload")?;
            let (texture, srv) = create_immutable_texture(
                gfx,
                decoded.width,
                decoded.height,
                DXGI_FORMAT_R8G8B8A8_UNORM,
                &decoded.rgba8,
                &format!("counter-sprite texture {}", decoded.texture_file),
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
            .ok_or("counter-sprite vertex count overflow")?;
        self.ensure_vertex_capacity(gfx, vertex_count)?;
        self.ensure_textures(gfx, decoded_by_path, &plan.draws)?;
        let vertices = plan
            .draws
            .iter()
            .flat_map(|draw| draw.vertices)
            .collect::<Vec<_>>();
        write_dynamic_slice(gfx, &self.vertex_buffer, &vertices, "counter-sprite vertices")?;
        let screen = [2.0 / gfx.width as f32, 2.0 / gfx.height as f32, 0.0, 0.0];
        write_dynamic_slice(
            gfx,
            &self.screen_cb,
            &screen,
            "counter-sprite screen constants",
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
            right: i32::try_from(gfx.width).map_err(|_| "counter-sprite target width exceeds i32")?,
            bottom: i32::try_from(gfx.height).map_err(|_| "counter-sprite target height exceeds i32")?,
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
                .ok_or("counter-sprite texture cache lost a planned texture")?;
            (*context).PSSetShaderResources(0, 1, [texture.srv.as_raw()].as_ptr());
            (*context).Draw(
                4,
                u32::try_from(index * 4).map_err(|_| "counter-sprite vertex offset overflow")?,
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
    runtime: Option<CounterSpriteGpuRuntime>,
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
            self.runtime = Some(unsafe { CounterSpriteGpuRuntime::new(gfx, needed)? });
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

/// Production XMAP13 counter compositor. Unlike `CounterGpuPass`, this path
/// consumes the complete viewer-independent XMAP13 presentation alternatives,
/// group/item census, and viewer-owned camera. CAP6 remains a comparison
/// oracle only.
pub(crate) struct Xmap13CounterGpuPass {
    resolver: GameFileResolver,
    runtime: Option<CounterSpriteGpuRuntime>,
    text: BTreeMap<String, TextRenderer>,
    last_receipt: Option<map_viewer::Xmap13NativeComponentReceipt>,
}

impl Xmap13CounterGpuPass {
    pub(crate) fn new(resolver: GameFileResolver) -> Self {
        Self {
            resolver,
            runtime: None,
            text: BTreeMap::new(),
            last_receipt: None,
        }
    }

    pub(crate) fn last_receipt(&self) -> Option<&map_viewer::Xmap13NativeComponentReceipt> {
        self.last_receipt.as_ref()
    }

    pub(crate) fn execute(
        &mut self,
        gfx: &Gfx,
        prepared: &map_viewer::PreparedXmap13Frame,
        viewer: &map_viewer::ViewerOwnedMapState,
        unit_receipt: &map_viewer::Xmap13NativeComponentReceipt,
        counter_target_sha256: &str,
    ) -> Result<&map_viewer::Xmap13NativeComponentReceipt> {
        prepared.validate()?;
        viewer.validate()?;
        unit_receipt.validate_for(&unit_receipt.target_execution_identity, prepared)?;
        if unit_receipt.component != map_viewer::Xmap13NativeComponent::UnitEntities
            || unit_receipt
                .target_execution_identity
                .counter_target_sha256
                .as_deref()
                != Some(counter_target_sha256)
        {
            return Err("XMAP13 counter compositor target differs from unit execution".into());
        }
        let (commands, decoded) = xmap13_counter_plan(&self.resolver, prepared, viewer)?;
        let draw = if commands.is_empty() {
            if !prepared.displayed_counter_groups.is_empty() {
                return Err("XMAP13 non-empty counter census produced no compositor draws".into());
            }
            map_viewer::Xmap13NativeDrawClosure::AuthenticatedEmpty {
                enumerator_witness_sha256: prepared.identity_provider().census_sha256().into(),
            }
        } else {
            let mut ordered_receipts = Vec::with_capacity(commands.len());
            for command in commands {
                match command {
                    Xmap13CounterCommand::Sprite { draw, receipt } => {
                        if self.runtime.is_none() {
                            self.runtime = Some(unsafe { CounterSpriteGpuRuntime::new(gfx, 4)? });
                        }
                        let issued = unsafe {
                            self.runtime
                                .as_mut()
                                .expect("XMAP13 counter-sprite runtime created")
                                .draw(
                                    gfx,
                                    &decoded,
                                    &CounterGpuDrawPlan {
                                        draws: vec![draw],
                                        omission: None,
                                    },
                                )?
                        };
                        if issued != 1 {
                            return Err("XMAP13 counter sprite did not issue exactly one draw".into());
                        }
                        ordered_receipts.push(receipt);
                    }
                    Xmap13CounterCommand::Text {
                        font_name,
                        font_language: _,
                        value,
                        control_rect,
                        horizontal_alignment,
                        vertical_alignment,
                        color_argb,
                        ordered_font_content,
                        receipt,
                    } => {
                        for content in &ordered_font_content {
                            self.resolver.read_admitted_used_content(content)?;
                        }
                        if !self.text.contains_key(&font_name) {
                            let font = crate::font::load_game_font_from_resolver(
                                &self.resolver,
                                &font_name,
                            )?;
                            self.text
                                .insert(font_name.clone(), TextRenderer::new(gfx, &font)?);
                        }
                        let renderer = self
                            .text
                            .get(&font_name)
                            .ok_or("XMAP13 runtime font disappeared")?;
                        let (width, height) = renderer.measure(&value);
                        let [x, y] = xmap13_aligned_text_origin(
                            control_rect,
                            width as f32,
                            height as f32,
                            horizontal_alignment,
                            vertical_alignment,
                        );
                        let missing = renderer.draw_text(gfx, x, y, &value, color_argb)?;
                        if !missing.is_empty() {
                            return Err(format!(
                                "XMAP13 font {font_name} is missing runtime glyphs {missing:?}"
                            )
                            .into());
                        }
                        ordered_receipts.push(receipt);
                    }
                }
            }
            let actual_draw_count = u32::try_from(ordered_receipts.len())?;
            map_viewer::Xmap13NativeDrawClosure::Present {
                executor_target_sha256: counter_target_sha256.into(),
                actual_draw_count,
                ordered_draw_receipt_sha256s: ordered_receipts,
            }
        };
        let assignment = &prepared.counter_quad_assignment;
        let receipt = map_viewer::Xmap13NativeComponentReceipt {
            schema: map_viewer::XMAP13_NATIVE_RECEIPT_SCHEMA.into(),
            component: map_viewer::Xmap13NativeComponent::CounterQuads,
            target_execution_identity: unit_receipt.target_execution_identity.clone(),
            assigned_topology_sha256: assignment.topology_sha256.clone(),
            assigned_stage_id: assignment.stage_name.clone(),
            assigned_operation_id: assignment.operation_name.clone(),
            assigned_ordinal: assignment.assigned_ordinal,
            assigned_assignment_sha256: assignment.assignment_sha256.clone(),
            prepared_frame_sha256: prepared.deterministic_sha256.clone(),
            draw,
            packet_draw_receipt_sha256: String::new(),
        }
        .seal()?;
        receipt.validate_for(&receipt.target_execution_identity, prepared)?;
        self.last_receipt = Some(receipt);
        Ok(self.last_receipt.as_ref().expect("XMAP13 receipt retained"))
    }
}

pub(crate) struct Xmap14CounterGpuRuntime {
    resolver: GameFileResolver,
    runtime: Option<CounterSpriteGpuRuntime>,
    text: BTreeMap<String, TextRenderer>,
}

impl Xmap14CounterGpuRuntime {
    pub(crate) fn new(resolver: GameFileResolver) -> Self {
        Self {
            resolver,
            runtime: None,
            text: BTreeMap::new(),
        }
    }

    pub(crate) fn execute(
        &mut self,
        gfx: &Gfx,
        draw: &map_viewer::PreparedXmap14CounterDraw,
        frame_content: &[replay_core::UsedContentKey],
    ) -> Result<Vec<String>> {
        let origin = draw.origin_f32_bits.map(f32::from_bits);
        let item_key = format!(
            "xmap14:{}:{}",
            draw.grid_control_name,
            draw.ordered_member_identities
                .iter()
                .map(map_viewer::Xmap13UnitIdentity::canonical_id)
                .collect::<Vec<_>>()
                .join("+")
        );
        let mut commands = Vec::new();
        let mut decoded = BTreeMap::<String, map_viewer::CounterTextureRgba>::new();
        for primitive in &draw.ordered_primitives {
            match primitive {
                map_viewer::Xmap13CounterPrimitive::Sprite {
                    stable_layer_id,
                    texture_content,
                    source_rect_px,
                    destination_rect_f32_bits,
                    rotation_f32_bits,
                    visible_fraction_f32_bits,
                    modulation_rgba_f32_bits,
                    ..
                } => xmap13_push_runtime_sprite(
                    &self.resolver,
                    &mut decoded,
                    &mut commands,
                    &item_key,
                    stable_layer_id,
                    texture_content,
                    *source_rect_px,
                    destination_rect_f32_bits.map(f32::from_bits),
                    f32::from_bits(*rotation_f32_bits),
                    visible_fraction_f32_bits.map(f32::from_bits),
                    modulation_rgba_f32_bits.map(f32::from_bits),
                    origin[0],
                    origin[1],
                )?,
                map_viewer::Xmap13CounterPrimitive::Text {
                    stable_layer_id,
                    font_name,
                    ordered_font_content,
                    value,
                    control_rect_f32_bits,
                    horizontal_alignment,
                    vertical_alignment,
                    color_argb,
                    ..
                } => {
                    xmap13_validate_font_content(
                        &self.resolver,
                        font_name,
                        ordered_font_content,
                    )?;
                    let selected_font_content = xmap14_selected_font_content(
                        &self.resolver,
                        font_name,
                        &draw.font_language,
                        frame_content,
                    )?;
                    let value = match value {
                        map_viewer::Xmap13CounterTextValue::ItemMemberCount => {
                            draw.member_count.to_string()
                        }
                        map_viewer::Xmap13CounterTextValue::Literal { value } => value.clone(),
                    };
                    let mut rect = control_rect_f32_bits.map(f32::from_bits);
                    rect[0] += origin[0];
                    rect[1] += origin[1];
                    let receipt = format!(
                        "{:x}",
                        Sha256::digest(serde_json::to_vec(&(
                            item_key.as_str(),
                            stable_layer_id,
                            font_name,
                            &draw.font_language,
                            &value,
                            rect.map(f32::to_bits),
                            horizontal_alignment,
                            vertical_alignment,
                            color_argb,
                            ordered_font_content,
                            &selected_font_content,
                        ))?)
                    );
                    commands.push(Xmap13CounterCommand::Text {
                        font_name: font_name.clone(),
                        font_language: Some(draw.font_language.clone()),
                        value,
                        control_rect: rect,
                        horizontal_alignment: *horizontal_alignment,
                        vertical_alignment: *vertical_alignment,
                        color_argb: *color_argb,
                        ordered_font_content: selected_font_content,
                        receipt,
                    });
                }
                map_viewer::Xmap13CounterPrimitive::Hidden { .. } => {}
            }
        }
        if commands.is_empty() {
            return Err("XMAP14 naval grid presentation has no drawable primitives".into());
        }
        let mut receipts = Vec::new();
        for command in commands {
            match command {
                Xmap13CounterCommand::Sprite { draw, receipt } => {
                    if self.runtime.is_none() {
                        self.runtime = Some(unsafe { CounterSpriteGpuRuntime::new(gfx, 4)? });
                    }
                    let issued = unsafe {
                        self.runtime
                            .as_mut()
                            .expect("XMAP14 counter runtime created")
                            .draw(
                                gfx,
                                &decoded,
                                &CounterGpuDrawPlan {
                                    draws: vec![draw],
                                    omission: None,
                                },
                            )?
                    };
                    if issued != 1 {
                        return Err("XMAP14 naval grid sprite did not issue one draw".into());
                    }
                    receipts.push(receipt);
                }
                Xmap13CounterCommand::Text {
                    font_name,
                    font_language,
                    value,
                    control_rect,
                    horizontal_alignment,
                    vertical_alignment,
                    color_argb,
                    ordered_font_content,
                    receipt,
                } => {
                    for content in &ordered_font_content {
                        self.resolver.read_admitted_used_content(content)?;
                    }
                    let font_key = format!(
                        "{}\0{}",
                        font_name,
                        font_language.as_deref().unwrap_or("")
                    );
                    if !self.text.contains_key(&font_key) {
                        let font = match font_language.as_deref() {
                            Some(language) => crate::font::load_game_font_for_language_from_resolver(
                                &self.resolver,
                                &font_name,
                                language,
                            )?,
                            None => crate::font::load_game_font_from_resolver(
                                &self.resolver,
                                &font_name,
                            )?,
                        };
                        self.text
                            .insert(font_key.clone(), TextRenderer::new(gfx, &font)?);
                    }
                    let renderer = self
                        .text
                        .get(&font_key)
                        .ok_or("XMAP14 naval-grid font disappeared")?;
                    let (width, height) = renderer.measure(&value);
                    let [x, y] = xmap13_aligned_text_origin(
                        control_rect,
                        width as f32,
                        height as f32,
                        horizontal_alignment,
                        vertical_alignment,
                    );
                    let (missing, submitted_pages) = renderer
                        .draw_text_with_page_receipts(gfx, x, y, &value, color_argb)?;
                    if !missing.is_empty() {
                        return Err(format!(
                            "XMAP14 naval-grid font {font_name} is missing glyphs {missing:?}"
                        )
                        .into());
                    }
                    if submitted_pages.is_empty() {
                        return Err("XMAP14 naval-grid text issued no atlas-page draw".into());
                    }
                    for page in submitted_pages {
                        receipts.push(format!(
                            "{:x}",
                            Sha256::digest(serde_json::to_vec(&(
                                receipt.as_str(),
                                font_language.as_deref(),
                                page,
                                &ordered_font_content,
                            ))?)
                        ));
                    }
                }
            }
        }
        Ok(receipts)
    }
}

fn xmap13_aligned_text_origin(
    control_rect: [f32; 4],
    width: f32,
    height: f32,
    horizontal: map_viewer::Xmap13CounterHorizontalAlignment,
    vertical: map_viewer::Xmap13CounterVerticalAlignment,
) -> [f32; 2] {
    let x = match horizontal {
        map_viewer::Xmap13CounterHorizontalAlignment::Left => control_rect[0],
        map_viewer::Xmap13CounterHorizontalAlignment::Center => {
            control_rect[0] + (control_rect[2] - width) * 0.5
        }
        map_viewer::Xmap13CounterHorizontalAlignment::Right => {
            control_rect[0] + control_rect[2] - width
        }
    };
    let y = match vertical {
        map_viewer::Xmap13CounterVerticalAlignment::Top => control_rect[1],
        map_viewer::Xmap13CounterVerticalAlignment::Center => {
            control_rect[1] + (control_rect[3] - height) * 0.5
        }
        map_viewer::Xmap13CounterVerticalAlignment::Bottom => {
            control_rect[1] + control_rect[3] - height
        }
    };
    [x, y]
}

enum Xmap13CounterCommand {
    Sprite {
        draw: CounterGpuDraw,
        receipt: String,
    },
    Text {
        font_name: String,
        font_language: Option<String>,
        value: String,
        control_rect: [f32; 4],
        horizontal_alignment: map_viewer::Xmap13CounterHorizontalAlignment,
        vertical_alignment: map_viewer::Xmap13CounterVerticalAlignment,
        color_argb: u32,
        ordered_font_content: Vec<replay_core::UsedContentKey>,
        receipt: String,
    },
}

fn xmap13_counter_plan(
    resolver: &GameFileResolver,
    prepared: &map_viewer::PreparedXmap13Frame,
    viewer: &map_viewer::ViewerOwnedMapState,
) -> Result<(
    Vec<Xmap13CounterCommand>,
    BTreeMap<String, map_viewer::CounterTextureRgba>,
)> {
    let camera = viewer
        .camera
        .as_ref()
        .ok_or("XMAP13 counter compositor requires the viewer-owned camera")?
        .production_camera()?;
    let by_identity = prepared
        .visible_units
        .iter()
        .map(|unit| (&unit.identity, unit))
        .collect::<BTreeMap<_, _>>();
    let mut commands = Vec::new();
    let mut decoded = BTreeMap::<String, map_viewer::CounterTextureRgba>::new();
    for group in &prepared.displayed_counter_groups {
        let world = group.group_world_anchor_f32_bits.map(f32::from_bits);
        let (screen_x, screen_y) = camera
            .world_to_screen(
                world,
                viewer.viewport_width as f32,
                viewer.viewport_height as f32,
            )
            .ok_or("XMAP13 counter group projects behind the viewer camera")?;
        let mut stack_y = 0.0f32;
        for item in &group.ordered_items {
            let representative = item
                .ordered_member_identities
                .first()
                .and_then(|identity| by_identity.get(identity))
                .ok_or("XMAP13 counter item lost its representative member")?;
            let representative_counter = representative
                .counter
                .as_ref()
                .ok_or("XMAP13 counter item representative has no counter semantics")?;
            let item_selected = item
                .ordered_member_identities
                .iter()
                .any(|identity| by_identity.get(identity).is_some_and(|unit| unit.selected));
            let presentation = representative_counter
                .ordered_presentations
                .iter()
                .find(|presentation| {
                    presentation.grouping_tier == group.grouping_tier
                        && presentation.counter_item_id == item.stable_item_id
                        && presentation.allegiance == group.allegiance
                        && presentation.selected == item_selected
                })
                .ok_or("XMAP13 counter item has no exact live viewer presentation")?;
            for identity in &item.ordered_member_identities {
                let member_counter = by_identity
                    .get(identity)
                    .and_then(|row| row.counter.as_ref())
                    .ok_or("XMAP13 counter item member lost its counter semantics")?;
                let member_presentation = member_counter
                    .ordered_presentations
                    .iter()
                    .find(|candidate| {
                        candidate.grouping_tier == group.grouping_tier
                            && candidate.counter_item_id == item.stable_item_id
                            && candidate.allegiance == group.allegiance
                            && candidate.selected == item_selected
                    })
                    .ok_or("XMAP13 grouped member lacks the live viewer presentation")?;
                if member_presentation != presentation {
                    return Err(
                        "XMAP13 grouped counter members disagree on runtime presentation".into(),
                    );
                }
            }
            let offset = presentation.anchor_offset_px_f32_bits.map(f32::from_bits);
            let origin_x = (screen_x + offset[0]).round();
            let origin_y = (screen_y + offset[1] + stack_y).round();
            let item_key = format!("{}:{}", group.stable_group_id, item.stable_item_id);
            for primitive in &presentation.ordered_primitives {
                match primitive {
                    map_viewer::Xmap13CounterPrimitive::Sprite {
                        stable_layer_id,
                        texture_content,
                        source_rect_px,
                        destination_rect_f32_bits,
                        rotation_f32_bits,
                        visible_fraction_f32_bits,
                        modulation_rgba_f32_bits,
                        ..
                    } => xmap13_push_runtime_sprite(
                        resolver,
                        &mut decoded,
                        &mut commands,
                        &item_key,
                        stable_layer_id,
                        texture_content,
                        *source_rect_px,
                        destination_rect_f32_bits.map(f32::from_bits),
                        f32::from_bits(*rotation_f32_bits),
                        visible_fraction_f32_bits.map(f32::from_bits),
                        modulation_rgba_f32_bits.map(f32::from_bits),
                        origin_x,
                        origin_y,
                    )?,
                    map_viewer::Xmap13CounterPrimitive::Text {
                        stable_layer_id,
                        font_name,
                        ordered_font_content,
                        value,
                        control_rect_f32_bits,
                        horizontal_alignment,
                        vertical_alignment,
                        color_argb,
                        ..
                    } => {
                        xmap13_validate_font_content(
                            resolver,
                            font_name,
                            ordered_font_content,
                        )?;
                        let value = match value {
                            map_viewer::Xmap13CounterTextValue::ItemMemberCount => {
                                item.ordered_member_identities.len().to_string()
                            }
                            map_viewer::Xmap13CounterTextValue::Literal { value } => value.clone(),
                        };
                        let mut rect = control_rect_f32_bits.map(f32::from_bits);
                        rect[0] += origin_x;
                        rect[1] += origin_y;
                        let receipt = format!(
                            "{:x}",
                            Sha256::digest(serde_json::to_vec(&(
                                item_key.as_str(),
                                stable_layer_id,
                                font_name,
                                &value,
                                rect.map(f32::to_bits),
                                horizontal_alignment,
                                vertical_alignment,
                                color_argb,
                                ordered_font_content,
                            ))?)
                        );
                        commands.push(Xmap13CounterCommand::Text {
                            font_name: font_name.clone(),
                            font_language: None,
                            value,
                            control_rect: rect,
                            horizontal_alignment: *horizontal_alignment,
                            vertical_alignment: *vertical_alignment,
                            color_argb: *color_argb,
                            ordered_font_content: ordered_font_content.clone(),
                            receipt,
                        });
                    }
                    map_viewer::Xmap13CounterPrimitive::Hidden { .. } => {}
                }
            }
            stack_y += f32::from_bits(presentation.stack_advance_px_f32_bits);
        }
    }
    Ok((commands, decoded))
}

fn xmap13_validate_font_content(
    resolver: &GameFileResolver,
    font_name: &str,
    ordered_font_content: &[replay_core::UsedContentKey],
) -> Result<()> {
    let expected = crate::font::game_font_dependency_paths(resolver, font_name)?
        .into_iter()
        .map(|path| path.replace('\\', "/").trim_start_matches('/').to_ascii_lowercase())
        .collect::<Vec<_>>();
    let actual = ordered_font_content
        .iter()
        .map(|content| {
            content
                .normalized_game_relative_path
                .replace('\\', "/")
                .trim_start_matches('/')
                .to_ascii_lowercase()
        })
        .collect::<Vec<_>>();
    if actual != expected {
        return Err("XMAP13 font closure differs from every descriptor/page opened by the product loader".into());
    }
    for content in ordered_font_content {
        resolver.read_admitted_used_content(content)?;
    }
    Ok(())
}

fn xmap14_selected_font_content(
    resolver: &GameFileResolver,
    font_name: &str,
    language: &str,
    frame_content: &[replay_core::UsedContentKey],
) -> Result<Vec<replay_core::UsedContentKey>> {
    let expected = crate::font::game_font_dependency_paths_for_language(
        resolver,
        font_name,
        Some(language),
    )?
    .into_iter()
    .map(|path| {
        path.replace('\\', "/")
            .trim_start_matches('/')
            .to_ascii_lowercase()
    })
    .collect::<Vec<_>>();
    let mut selected = Vec::with_capacity(expected.len());
    for expected_path in expected {
        let mut matches = frame_content.iter().filter(|content| {
            content
                .normalized_game_relative_path
                .replace('\\', "/")
                .trim_start_matches('/')
                .eq_ignore_ascii_case(&expected_path)
        });
        let content = matches.next().ok_or_else(|| {
            format!(
                "XMAP14 prepared closure omitted selected {language} font dependency {expected_path}"
            )
        })?;
        if matches.next().is_some() {
            return Err(format!(
                "XMAP14 prepared closure duplicated selected font dependency {expected_path}"
            )
            .into());
        }
        resolver.read_admitted_used_content(content)?;
        selected.push(content.clone());
    }
    if selected.is_empty() {
        return Err("XMAP14 selected counter font closure is empty".into());
    }
    Ok(selected)
}

#[allow(clippy::too_many_arguments)]
fn xmap13_push_runtime_sprite(
    resolver: &GameFileResolver,
    decoded: &mut BTreeMap<String, map_viewer::CounterTextureRgba>,
    commands: &mut Vec<Xmap13CounterCommand>,
    item_key: &str,
    stable_layer_id: &str,
    texture_content: &replay_core::UsedContentKey,
    source_rect_px: [u32; 4],
    destination_rect: [f32; 4],
    rotation: f32,
    visible_fraction: Option<f32>,
    modulation: [f32; 4],
    origin_x: f32,
    origin_y: f32,
) -> Result<()> {
    resolver.read_admitted_used_content(texture_content)?;
    let texture = xmap13_decoded_texture(
        resolver,
        decoded,
        &texture_content.normalized_game_relative_path,
    )?;
    let [source_x, source_y, source_width, source_height] = source_rect_px;
    let texture_width = u32::try_from(texture.width)?;
    let texture_height = u32::try_from(texture.height)?;
    if source_x
        .checked_add(source_width)
        .is_none_or(|end| end > texture_width)
        || source_y
            .checked_add(source_height)
            .is_none_or(|end| end > texture_height)
    {
        return Err("XMAP13 runtime sprite rectangle exceeds its admitted texture".into());
    }
    let x = origin_x + destination_rect[0];
    let y = origin_y + destination_rect[1];
    let width = destination_rect[2];
    let height = destination_rect[3];
    let u0 = source_x as f32 / texture.width as f32;
    let v0 = source_y as f32 / texture.height as f32;
    let u1 = (source_x + source_width) as f32 / texture.width as f32;
    let v1 = (source_y + source_height) as f32 / texture.height as f32;
    let corners = xmap13_sprite_corners(
        x,
        y,
        width,
        height,
        [u0, v0, u1, v1],
        rotation,
        visible_fraction,
    );
    let vertices = corners.map(|(position, uv)| CounterGpuVertex {
        position,
        uv,
        modulation,
    });
    let receipt = format!(
        "{:x}",
        Sha256::digest(serde_json::to_vec(&(
            item_key,
            stable_layer_id,
            texture_content,
            source_rect_px,
            destination_rect.map(f32::to_bits),
            rotation.to_bits(),
            visible_fraction.map(f32::to_bits),
            modulation.map(f32::to_bits),
            [origin_x.to_bits(), origin_y.to_bits()],
        ))?)
    );
    commands.push(Xmap13CounterCommand::Sprite {
        draw: CounterGpuDraw {
            key: CounterGpuQuadKey {
                unit_id: format!("{item_key}:{stable_layer_id}"),
                layer: CounterQuadLayer::Frame,
            },
            texture_file: texture_content.normalized_game_relative_path.clone(),
            texture_width: texture.width,
            texture_height: texture.height,
            decoded_rgba_sha256: format!("{:x}", Sha256::digest(&texture.rgba8)),
            vertices,
        },
        receipt,
    });
    Ok(())
}

fn xmap13_sprite_corners(
    x: f32,
    y: f32,
    width: f32,
    height: f32,
    uv: [f32; 4],
    rotation: f32,
    visible_fraction: Option<f32>,
) -> [([f32; 2], [f32; 2]); 4] {
    let [u0, v0, u1, v1] = uv;
    if let Some(fraction) = visible_fraction {
        let visible_width = width * fraction;
        let cropped_v = v1 - (v1 - v0) * fraction;
        [
            ([x, y], [u0, v1]),
            ([x, y + height], [u1, v1]),
            ([x + visible_width, y], [u0, cropped_v]),
            ([x + visible_width, y + height], [u1, cropped_v]),
        ]
    } else {
        let (sin, cos) = rotation.sin_cos();
        let rotate = |local_x: f32, local_y: f32| {
            [x + local_x * cos - local_y * sin, y + local_x * sin + local_y * cos]
        };
        [
            (rotate(0.0, 0.0), [u0, v0]),
            (rotate(0.0, height), [u0, v1]),
            (rotate(width, 0.0), [u1, v0]),
            (rotate(width, height), [u1, v1]),
        ]
    }
}

fn xmap13_decoded_texture<'a>(
    resolver: &GameFileResolver,
    decoded: &'a mut BTreeMap<String, map_viewer::CounterTextureRgba>,
    path: &str,
) -> Result<&'a map_viewer::CounterTextureRgba> {
    if !decoded.contains_key(path) {
        decoded.insert(
            path.into(),
            counter_texture_rgba_from_resolver(resolver, path)?,
        );
    }
    decoded
        .get(path)
        .ok_or_else(|| "XMAP13 decoded texture cache lost an entry".into())
}

#[cfg(test)]
mod tests {
    use super::*;
    use map_viewer::{CounterQuadVertex, CounterRenderInstance, ScreenPointPx};

    #[test]
    fn xmap13_text_alignment_uses_the_authored_control_rectangle() {
        let rect = [10.0, 20.0, 30.0, 40.0];
        assert_eq!(
            xmap13_aligned_text_origin(
                rect,
                8.0,
                12.0,
                map_viewer::Xmap13CounterHorizontalAlignment::Left,
                map_viewer::Xmap13CounterVerticalAlignment::Top,
            ),
            [10.0, 20.0]
        );
        assert_eq!(
            xmap13_aligned_text_origin(
                rect,
                8.0,
                12.0,
                map_viewer::Xmap13CounterHorizontalAlignment::Center,
                map_viewer::Xmap13CounterVerticalAlignment::Center,
            ),
            [21.0, 34.0]
        );
        assert_eq!(
            xmap13_aligned_text_origin(
                rect,
                8.0,
                12.0,
                map_viewer::Xmap13CounterHorizontalAlignment::Right,
                map_viewer::Xmap13CounterVerticalAlignment::Bottom,
            ),
            [32.0, 48.0]
        );
    }

    #[test]
    fn xmap13_fractional_bar_matches_the_proven_rotated_clip_law() {
        let corners = xmap13_sprite_corners(
            14.0,
            8.0,
            17.0,
            3.0,
            [0.0, 0.0, 1.0, 1.0],
            1.5708,
            Some(0.25),
        );
        assert_eq!(
            corners,
            [
                ([14.0, 8.0], [0.0, 1.0]),
                ([14.0, 11.0], [1.0, 1.0]),
                ([18.25, 8.0], [0.0, 0.75]),
                ([18.25, 11.0], [1.0, 0.75]),
            ]
        );
    }

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
