use crate::*;
use serde::Serialize;
use std::collections::BTreeMap;
use std::ffi::CString;
use std::fs;
use std::mem;
use std::path::Path;
use std::ptr;

use crate::constants::{
    D3D11_BIND_INDEX_BUFFER, D3D11_BIND_RENDER_TARGET, D3D11_BIND_SHADER_RESOURCE,
    D3D11_BIND_VERTEX_BUFFER, D3D11_INPUT_PER_INSTANCE_DATA, D3D11_USAGE_DEFAULT,
    DXGI_FORMAT_B8G8R8A8_UNORM, DXGI_FORMAT_B8G8R8A8_UNORM_SRGB, DXGI_FORMAT_B8G8R8X8_UNORM,
    DXGI_FORMAT_R16G16B16A16_FLOAT, DXGI_FORMAT_R32G32B32A32_FLOAT, DXGI_FORMAT_R32G32B32_FLOAT,
    DXGI_FORMAT_R32G32_FLOAT, DXGI_FORMAT_R32_UINT, DXGI_FORMAT_R8G8B8A8_UNORM,
};
#[cfg(windows)]
use winapi::shared::dxgitype::DXGI_SAMPLE_DESC;
#[cfg(windows)]
use winapi::shared::minwindef::UINT;
#[cfg(windows)]
use winapi::shared::windef::RECT;
#[cfg(windows)]
use winapi::um::d3d11::*;
#[cfg(windows)]
use winapi::um::d3dcommon::*;
#[cfg(windows)]
use winapi::um::d3dcompiler::D3DCompile;
#[cfg(windows)]
use winapi::um::unknwnbase::IUnknown;

#[cfg(windows)]
const TERRAIN_COLOR_SNAPSHOT_HLSL: &str = r#"
Texture2D<float4> TerrainColor : register(t0);

struct VsOut {
    float4 pos : SV_Position;
};

VsOut vs_main(uint vertex_id : SV_VertexID) {
    VsOut output;
    float2 clip = float2(
        (vertex_id == 1) ? 3.0 : -1.0,
        (vertex_id == 2) ? -3.0 : 1.0);
    output.pos = float4(clip, 0.0, 1.0);
    return output;
}

float4 ps_main(VsOut input) : SV_Target {
    float3 terrain = TerrainColor.Load(int3(int2(input.pos.xy), 0)).rgb;
    // The game's WaterRefraction buffer is the scene re-rendered with heavy
    // underwater absorption: per-channel linear fit of the captured
    // refraction RT against the captured terrain RT of the same frame
    // (live_PAN2) gives refraction ~= (0.013, 0.023, 0.066) * terrain +
    // (0.0008, 0.0008, 0.0011) in linear space - almost black with a strong
    // blue remnant. Without this the replay's water refracts the raw lit
    // terrain and reads green instead of the game's deep blue.
    float3 refraction = terrain * float3(0.0128, 0.0225, 0.0655)
        + float3(0.00076, 0.00083, 0.00109);
    return float4(saturate(refraction), 1.0);
}
"#;

#[cfg(windows)]
pub(crate) struct D3dPtr<T>(*mut T);

#[cfg(windows)]
impl<T> D3dPtr<T> {
    pub(crate) unsafe fn from_raw(ptr: *mut T, role: &str) -> Result<Self> {
        if ptr.is_null() {
            Err(
                format!("production native D3D11 terrain replay blocked: {role} returned null")
                    .into(),
            )
        } else {
            Ok(Self(ptr))
        }
    }

    pub(crate) fn as_raw(&self) -> *mut T {
        self.0
    }
}

#[cfg(windows)]
impl<T> Drop for D3dPtr<T> {
    fn drop(&mut self) {
        unsafe {
            if !self.0.is_null() {
                (*(self.0 as *mut IUnknown)).Release();
            }
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct NativeD3d11UploadError {
    action: String,
    device_removed_reason: i32,
}

impl std::fmt::Display for NativeD3d11UploadError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            formatter,
            "production native D3D11 replay blocked: {} left the device unavailable (GetDeviceRemovedReason 0x{:08X}); uploaded state was not admitted",
            self.action,
            self.device_removed_reason as u32
        )
    }
}

impl std::error::Error for NativeD3d11UploadError {}

pub(crate) fn native_d3d11_validate_void_upload_result(
    device_removed_reason: i32,
    action: &str,
) -> Result<()> {
    if device_removed_reason < 0 {
        Err(Box::new(NativeD3d11UploadError {
            action: action.to_string(),
            device_removed_reason,
        }))
    } else {
        Ok(())
    }
}

#[cfg(windows)]
pub(crate) unsafe fn native_d3d11_check_void_upload_device(
    device: &D3dPtr<ID3D11Device>,
    action: &str,
) -> Result<()> {
    native_d3d11_validate_void_upload_result((*device.as_raw()).GetDeviceRemovedReason(), action)
}

#[cfg(test)]
mod void_upload_tests {
    use super::*;

    #[test]
    fn void_upload_result_is_admitted_only_while_device_is_available() {
        native_d3d11_validate_void_upload_result(0, "UpdateSubresource terrain cb0")
            .expect("S_OK device state admits the completed void upload");

        let error = native_d3d11_validate_void_upload_result(
            0x887A_0005u32 as i32,
            "UpdateSubresource terrain cb0",
        )
        .expect_err("negative device reason must reject the upload");
        assert!(error.downcast_ref::<NativeD3d11UploadError>().is_some());
        let message = error.to_string();
        assert!(
            message.contains("UpdateSubresource terrain cb0"),
            "{message}"
        );
        assert!(
            message.contains("uploaded state was not admitted"),
            "{message}"
        );
    }
}

#[cfg(windows)]
unsafe fn native_d3d11_check_update_subresource(
    context: &D3dPtr<ID3D11DeviceContext>,
    action: &str,
) -> Result<()> {
    let mut device = ptr::null_mut();
    (*context.as_raw()).GetDevice(&mut device);
    let device = D3dPtr::from_raw(device, "D3D11 device after UpdateSubresource")?;
    native_d3d11_check_void_upload_device(&device, action)
}

#[cfg(windows)]
pub(crate) struct NativeD3d11Device {
    pub(crate) device: D3dPtr<ID3D11Device>,
    pub(crate) context: D3dPtr<ID3D11DeviceContext>,
}

#[cfg(windows)]
pub(crate) struct NativeD3d11RenderTarget {
    pub(crate) texture: D3dPtr<ID3D11Texture2D>,
    pub(crate) view: D3dPtr<ID3D11RenderTargetView>,
    pub(crate) desc: MapFrameTexture2DDesc,
}

#[cfg(windows)]
pub(crate) struct NativeD3d11DepthStencil {
    pub(crate) texture: D3dPtr<ID3D11Texture2D>,
    pub(crate) view: D3dPtr<ID3D11DepthStencilView>,
    pub(crate) desc: MapFrameTexture2DDesc,
}

#[cfg(windows)]
pub(crate) struct NativeD3d11PreparedMapFrame {
    pub(crate) render_target: NativeD3d11RenderTarget,
    pub(crate) depth_target: NativeD3d11DepthStencil,
    pub(crate) pass: NativeD3d11PreparedMapPass,
}

#[cfg(windows)]
pub(crate) struct NativeD3d11PreparedTerrainChunkSynthesisFrame {
    pub(crate) render_target: NativeD3d11RenderTarget,
    pub(crate) depth_target: NativeD3d11DepthStencil,
    pub(crate) template_draw: NativeD3d11PreparedDraw,
    pub(crate) target_evidence: MapFrameOutputMergerTexture,
    pub(crate) depth_evidence: MapFrameOutputMergerTexture,
}

#[cfg(windows)]
pub(crate) struct NativeD3d11PreparedMapPass {
    pub(crate) pass_label: String,
    pub(crate) target_evidence: MapFrameOutputMergerTexture,
    pub(crate) depth_evidence: MapFrameOutputMergerTexture,
    pub(crate) reference_output: NativeD3d11ReferenceOutput,
    pub(crate) app_owned_terrain_color_srv_desc: Option<MapFrameShaderResourceViewDesc>,
    pub(crate) draws: Vec<NativeD3d11PreparedDraw>,
    pub(crate) summary: NativeD3d11ReplaySummary,
}

#[cfg(windows)]
pub(crate) struct NativeD3d11ReferenceOutput {
    pub(crate) width: u32,
    pub(crate) height: u32,
    pub(crate) dxgi_format: u32,
    pub(crate) bytes: Vec<u8>,
    pub(crate) fnv1a64: String,
    pub(crate) file: String,
}

#[derive(Clone, Debug, Serialize)]
pub(crate) struct NativeD3d11ParityReport {
    pub(crate) schema: String,
    pub(crate) pass_label: String,
    pub(crate) draw_count: usize,
    pub(crate) target: NativeD3d11ParityOutputTargetReport,
    pub(crate) depth: NativeD3d11ParityOutputTargetReport,
    pub(crate) initial_output: NativeD3d11ParityDump,
    pub(crate) initial_depth: Option<NativeD3d11ParityDump>,
    pub(crate) pre_draw_outputs: Vec<NativeD3d11ParityPreDrawReport>,
    pub(crate) pre_draw_depths: Vec<NativeD3d11ParityPostDrawReport>,
    pub(crate) post_draw_outputs: Vec<NativeD3d11ParityPostDrawReport>,
    pub(crate) post_draw_depths: Vec<NativeD3d11ParityPostDrawReport>,
    pub(crate) final_output: NativeD3d11ParityComparison,
    pub(crate) first_pre_draw_mismatch_draw_index: Option<u32>,
    pub(crate) first_pre_draw_mismatch_previous_draw_index: Option<u32>,
    pub(crate) draw_descs: Vec<NativeD3d11ParityDrawDescReport>,
}

#[derive(Clone, Debug, Serialize)]
pub(crate) struct NativeD3d11ParityOutputTargetReport {
    pub(crate) captured_texture_desc: MapFrameTexture2DDesc,
    pub(crate) captured_view_desc: Option<MapFrameOutputMergerViewDesc>,
    pub(crate) replay_texture_desc: MapFrameTexture2DDesc,
    pub(crate) replay_view_desc: Option<MapFrameOutputMergerViewDesc>,
}

#[derive(Clone, Debug, Serialize)]
pub(crate) struct NativeD3d11ParityDump {
    pub(crate) label: String,
    pub(crate) file: String,
    pub(crate) byte_len: usize,
    pub(crate) fnv1a64: String,
}

#[derive(Clone, Debug, Serialize)]
pub(crate) struct NativeD3d11ParityPreDrawReport {
    pub(crate) draw_index: u32,
    pub(crate) dump: NativeD3d11ParityDump,
    pub(crate) captured_pre_draw_comparison: Option<NativeD3d11ParityComparison>,
}

#[derive(Clone, Debug, Serialize)]
pub(crate) struct NativeD3d11ParityPostDrawReport {
    pub(crate) draw_index: u32,
    pub(crate) dump: NativeD3d11ParityDump,
    pub(crate) captured_final_comparison: Option<NativeD3d11ParityComparison>,
}

#[derive(Clone, Debug, Serialize)]
pub(crate) struct NativeD3d11ParityComparison {
    pub(crate) label: String,
    pub(crate) dimensions_match: bool,
    pub(crate) matches: bool,
    pub(crate) byte_len: usize,
    pub(crate) expected_fnv1a64: String,
    pub(crate) actual_fnv1a64: String,
    pub(crate) captured_staging_fnv1a64: String,
    pub(crate) expected_file: String,
    pub(crate) actual_file: Option<String>,
    pub(crate) differing_byte_count: Option<usize>,
    pub(crate) first_mismatch_byte: Option<usize>,
}

#[derive(Clone, Debug, Serialize)]
pub(crate) struct NativeD3d11ParityDrawDescReport {
    pub(crate) draw_index: u32,
    pub(crate) draw_call: String,
    pub(crate) draw_count: u32,
    pub(crate) draw_start: u32,
    pub(crate) base_vertex_location: i32,
    pub(crate) per_instance_count: Option<u32>,
    pub(crate) instance_count: Option<u32>,
    pub(crate) start_instance_location: Option<u32>,
    pub(crate) topology: u32,
    pub(crate) vertex_shader: Option<NativeD3d11ParityShaderReport>,
    pub(crate) pixel_shader: NativeD3d11ParityShaderReport,
    pub(crate) input_layout: MapFrameInputLayout,
    pub(crate) vertex_buffers: Vec<NativeD3d11ParityBufferReport>,
    pub(crate) index_buffer: Option<NativeD3d11ParityIndexBufferReport>,
    pub(crate) constant_buffers: Vec<NativeD3d11ParityBufferReport>,
    pub(crate) shader_resources: Vec<NativeD3d11ParityShaderResourceReport>,
    pub(crate) samplers: Vec<NativeD3d11ParitySamplerReport>,
    pub(crate) rasterizer: MapFrameRasterizerState,
    pub(crate) blend: MapFrameBlendState,
    pub(crate) depth_stencil: MapFrameDepthStencilState,
    pub(crate) render_target: Option<NativeD3d11ParityOutputTargetReport>,
    pub(crate) depth_target: Option<NativeD3d11ParityOutputTargetReport>,
    pub(crate) viewport_count: usize,
    pub(crate) scissor_rect_count: usize,
}

#[derive(Clone, Debug, Serialize)]
pub(crate) struct NativeD3d11ParityShaderReport {
    pub(crate) stage: String,
    pub(crate) bytecode_file: String,
    pub(crate) byte_len: usize,
    pub(crate) fnv1a64: String,
}

#[derive(Clone, Debug, Serialize)]
pub(crate) struct NativeD3d11ParityBufferDesc {
    pub(crate) byte_width: usize,
    pub(crate) usage: u32,
    pub(crate) bind_flags: u32,
    pub(crate) cpu_access_flags: u32,
    pub(crate) misc_flags: u32,
    pub(crate) structure_byte_stride: u32,
}

#[derive(Clone, Debug, Serialize)]
pub(crate) struct NativeD3d11ParityBufferReport {
    pub(crate) stage: String,
    pub(crate) slot: u32,
    pub(crate) declared_name: Option<String>,
    pub(crate) file: Option<String>,
    pub(crate) fnv1a64: Option<String>,
    pub(crate) stride: Option<u32>,
    pub(crate) offset: Option<u32>,
    pub(crate) dxgi_format: Option<u32>,
    pub(crate) captured_desc: NativeD3d11ParityBufferDesc,
    pub(crate) replay_desc: NativeD3d11ParityBufferDesc,
}

#[derive(Clone, Debug, Serialize)]
pub(crate) struct NativeD3d11ParityIndexBufferReport {
    pub(crate) file: Option<String>,
    pub(crate) fnv1a64: Option<String>,
    pub(crate) offset: u32,
    pub(crate) dxgi_format: u32,
    pub(crate) captured_desc: NativeD3d11ParityBufferDesc,
    pub(crate) replay_desc: NativeD3d11ParityBufferDesc,
}

#[derive(Clone, Debug, Serialize)]
pub(crate) struct NativeD3d11ParityShaderResourceReport {
    pub(crate) stage: String,
    pub(crate) slot: u32,
    pub(crate) declared_name: Option<String>,
    pub(crate) is_null: bool,
    pub(crate) file: Option<String>,
    pub(crate) fnv1a64: Option<String>,
    pub(crate) captured_resource_desc: MapFrameTexture2DDesc,
    pub(crate) captured_view_desc: Option<MapFrameShaderResourceViewDesc>,
    pub(crate) replay_texture_desc: MapFrameTexture2DDesc,
    pub(crate) replay_view_desc: Option<MapFrameShaderResourceViewDesc>,
}

#[derive(Clone, Debug, Serialize)]
pub(crate) struct NativeD3d11ParitySamplerReport {
    pub(crate) stage: String,
    pub(crate) slot: u32,
    pub(crate) captured_desc: MapFrameSamplerBinding,
    pub(crate) replay_desc: MapFrameSamplerBinding,
}

#[cfg(windows)]
pub(crate) struct NativeD3d11PreparedDraw {
    pub(crate) draw_index: u32,
    pub(crate) draw_call: String,
    pub(crate) draw_count: u32,
    pub(crate) draw_start: u32,
    pub(crate) base_vertex_location: i32,
    pub(crate) per_instance_count: Option<u32>,
    pub(crate) instance_count: Option<u32>,
    pub(crate) start_instance_location: Option<u32>,
    pub(crate) topology: u32,
    pub(crate) vertex_shader: D3dPtr<ID3D11VertexShader>,
    pub(crate) pixel_shader: D3dPtr<ID3D11PixelShader>,
    pub(crate) input_layout: D3dPtr<ID3D11InputLayout>,
    pub(crate) vertex_buffers: Vec<NativeD3d11PreparedVertexBuffer>,
    pub(crate) index_buffer: Option<NativeD3d11PreparedIndexBuffer>,
    pub(crate) constant_buffers: Vec<NativeD3d11PreparedConstantBuffer>,
    pub(crate) _textures: Vec<D3dPtr<ID3D11Texture2D>>,
    pub(crate) shader_resources: Vec<NativeD3d11PreparedShaderResource>,
    pub(crate) samplers: Vec<NativeD3d11PreparedSampler>,
    pub(crate) rasterizer: Option<D3dPtr<ID3D11RasterizerState>>,
    pub(crate) blend: Option<D3dPtr<ID3D11BlendState>>,
    pub(crate) blend_factor: [f32; 4],
    pub(crate) sample_mask: u32,
    pub(crate) depth_stencil: Option<D3dPtr<ID3D11DepthStencilState>>,
    pub(crate) stencil_ref: u32,
    pub(crate) viewports: Vec<D3D11_VIEWPORT>,
    pub(crate) scissor_rects: Vec<RECT>,
    pub(crate) pre_draw_render_target: Option<D3dPtr<ID3D11Texture2D>>,
    pub(crate) pre_draw_depth_stencil: Option<D3dPtr<ID3D11Texture2D>>,
    pub(crate) pre_draw_reference_output: Option<NativeD3d11ReferenceOutput>,
    pub(crate) post_draw_reference_output: Option<NativeD3d11ReferenceOutput>,
    pub(crate) parity_desc: NativeD3d11ParityDrawDescReport,
}

#[cfg(windows)]
pub(crate) struct NativeD3d11PreparedVertexBuffer {
    pub(crate) slot: u32,
    pub(crate) buffer: D3dPtr<ID3D11Buffer>,
    pub(crate) stride: u32,
    pub(crate) offset: u32,
}

#[cfg(windows)]
pub(crate) struct NativeD3d11PreparedIndexBuffer {
    pub(crate) buffer: D3dPtr<ID3D11Buffer>,
    pub(crate) dxgi_format: u32,
    pub(crate) offset: u32,
}

#[cfg(windows)]
pub(crate) struct NativeD3d11PreparedConstantBuffer {
    pub(crate) stage: String,
    pub(crate) slot: u32,
    pub(crate) buffer: D3dPtr<ID3D11Buffer>,
    pub(crate) camera_patch: Option<NativeD3d11PreparedCameraConstantBuffer>,
    pub(crate) terrain_draw_patch: Option<NativeD3d11PreparedTerrainDrawConstantBuffer>,
    pub(crate) tree_season_patch: Option<NativeD3d11PreparedTreeSeasonConstantBuffer>,
    pub(crate) map_clock_patch: Option<NativeD3d11PreparedMapClockConstantBuffer>,
}

#[cfg(windows)]
pub(crate) struct NativeD3d11PreparedCameraConstantBuffer {
    pub(crate) draw_index: u32,
    pub(crate) base_bytes: Vec<u8>,
    pub(crate) layout: ShaderConstantBufferLayout,
    pub(crate) update_method: TerrainSceneCb0CameraUpdateMethod,
}

#[cfg(windows)]
pub(crate) struct NativeD3d11PreparedTerrainDrawConstantBuffer {
    pub(crate) draw_index: u32,
    pub(crate) base_bytes: Vec<u8>,
}

#[cfg(windows)]
pub(crate) struct NativeD3d11PreparedTreeSeasonConstantBuffer {
    pub(crate) draw_index: u32,
    pub(crate) base_bytes: Vec<u8>,
}

#[cfg(windows)]
pub(crate) struct NativeD3d11PreparedMapClockConstantBuffer {
    pub(crate) draw_index: u32,
    pub(crate) base_bytes: Vec<u8>,
    pub(crate) kind: NativeD3d11MapClockConstantBufferKind,
}

#[cfg(windows)]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum NativeD3d11MapClockConstantBufferKind {
    WaterTimeHalfPixel,
    RiverTimeDirectionSeasonLerp,
}

#[cfg(windows)]
pub(crate) struct NativeD3d11PreparedShaderResource {
    pub(crate) stage: String,
    pub(crate) slot: u32,
    pub(crate) declared_name: Option<String>,
    pub(crate) source: NativeD3d11ShaderResourceSource,
    pub(crate) view: Option<D3dPtr<ID3D11ShaderResourceView>>,
}

#[cfg(windows)]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum NativeD3d11ShaderResourceSource {
    CapturedBundleTexture,
    AppOwnedTerrainColorSnapshot,
    UnderwaterPass,
}

#[cfg(windows)]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum NativeD3d11WaterRefractionSource {
    CapturedBundleTexture,
    AppOwnedTerrainColorSnapshot,
    UnderwaterPass,
}

#[cfg(windows)]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum NativeD3d11ReflectionCubeSource {
    MeanColorFlatten,
    ShippedFileCube,
}

#[cfg(windows)]
pub(crate) struct NativeD3d11PreparedSampler {
    pub(crate) stage: String,
    pub(crate) slot: u32,
    pub(crate) state: D3dPtr<ID3D11SamplerState>,
}

#[cfg(windows)]
pub(crate) struct NativeD3d11TerrainColorSnapshot {
    pub(crate) _texture: D3dPtr<ID3D11Texture2D>,
    pub(crate) view: D3dPtr<ID3D11ShaderResourceView>,
}

#[cfg(windows)]
pub(crate) struct NativeD3d11TerrainColorSnapshotPipeline {
    pub(crate) vertex_shader: D3dPtr<ID3D11VertexShader>,
    pub(crate) pixel_shader: D3dPtr<ID3D11PixelShader>,
}

#[cfg(windows)]
pub(crate) struct NativeD3d11AppOwnedShaderResources<'a> {
    pub(crate) terrain_color_snapshot: Option<&'a D3dPtr<ID3D11ShaderResourceView>>,
    pub(crate) underwater_refraction: Option<&'a D3dPtr<ID3D11ShaderResourceView>>,
}

#[cfg(windows)]
unsafe fn native_d3d11_preferred_adapter() -> Option<D3dPtr<winapi::shared::dxgi::IDXGIAdapter1>> {
    use winapi::shared::dxgi::{CreateDXGIFactory1, IDXGIAdapter1, IDXGIFactory1};
    use winapi::Interface;
    let mut factory: *mut IDXGIFactory1 = ptr::null_mut();
    if CreateDXGIFactory1(&IDXGIFactory1::uuidof(), &mut factory as *mut _ as *mut _) < 0
        || factory.is_null()
    {
        return None;
    }
    let factory = D3dPtr::from_raw(factory, "DXGI factory").ok()?;
    let mut best: Option<(usize, D3dPtr<IDXGIAdapter1>, [u16; 128])> = None;
    let mut index = 0u32;
    loop {
        let mut adapter: *mut IDXGIAdapter1 = ptr::null_mut();
        if (*factory.as_raw()).EnumAdapters1(index, &mut adapter) < 0 || adapter.is_null() {
            break;
        }
        index += 1;
        let Ok(adapter) = D3dPtr::from_raw(adapter, "DXGI adapter") else {
            continue;
        };
        let mut desc = std::mem::zeroed::<winapi::shared::dxgi::DXGI_ADAPTER_DESC1>();
        if (*adapter.as_raw()).GetDesc1(&mut desc) < 0 {
            continue;
        }
        const DXGI_ADAPTER_FLAG_SOFTWARE: u32 = 2;
        if desc.Flags & DXGI_ADAPTER_FLAG_SOFTWARE != 0 {
            continue;
        }
        let memory = desc.DedicatedVideoMemory;
        if best
            .as_ref()
            .map(|(best_memory, _, _)| memory > *best_memory)
            .unwrap_or(true)
        {
            best = Some((memory, adapter, desc.Description));
        }
    }
    best.map(|(memory, adapter, description)| {
        let name_len = description.iter().position(|&c| c == 0).unwrap_or(128);
        eprintln!(
            "native D3D11 device: using adapter \"{}\" ({} MiB dedicated)",
            String::from_utf16_lossy(&description[..name_len]),
            memory / (1024 * 1024)
        );
        adapter
    })
}

#[cfg(windows)]
pub(crate) unsafe fn native_d3d11_create_device() -> Result<NativeD3d11Device> {
    let mut device: *mut ID3D11Device = ptr::null_mut();
    let mut context: *mut ID3D11DeviceContext = ptr::null_mut();
    let feature_levels = [
        D3D_FEATURE_LEVEL_11_0,
        D3D_FEATURE_LEVEL_10_1,
        D3D_FEATURE_LEVEL_10_0,
    ];
    let mut feature_level = D3D_FEATURE_LEVEL_11_0;
    let adapter = native_d3d11_preferred_adapter();
    let (adapter_ptr, driver_type) = match adapter.as_ref() {
        Some(adapter) => (
            adapter.as_raw() as *mut winapi::shared::dxgi::IDXGIAdapter,
            winapi::um::d3dcommon::D3D_DRIVER_TYPE_UNKNOWN,
        ),
        None => (ptr::null_mut(), D3D_DRIVER_TYPE_HARDWARE),
    };
    let hr = D3D11CreateDevice(
        adapter_ptr,
        driver_type,
        ptr::null_mut(),
        D3D11_CREATE_DEVICE_BGRA_SUPPORT,
        feature_levels.as_ptr(),
        feature_levels.len() as UINT,
        D3D11_SDK_VERSION,
        &mut device,
        &mut feature_level,
        &mut context,
    );
    native_d3d11_hr(hr, "D3D11CreateDevice")?;
    Ok(NativeD3d11Device {
        device: D3dPtr::from_raw(device, "D3D11 device")?,
        context: D3dPtr::from_raw(context, "D3D11 immediate context")?,
    })
}

#[cfg(windows)]
pub(crate) unsafe fn native_d3d11_create_render_target(
    device: &NativeD3d11Device,
    target: &MapFrameOutputMergerTexture,
) -> Result<NativeD3d11RenderTarget> {
    let texture = native_d3d11_create_texture2d_from_desc(&device.device, &target.desc, None)?;
    let view_desc = native_d3d11_render_target_view_desc(target)?;
    let mut view: *mut ID3D11RenderTargetView = ptr::null_mut();
    let hr = (*device.device.as_raw()).CreateRenderTargetView(
        texture.as_raw() as *mut _,
        &view_desc,
        &mut view,
    );
    native_d3d11_hr(
        hr,
        "CreateRenderTargetView for captured terrain output target",
    )?;
    Ok(NativeD3d11RenderTarget {
        texture,
        view: D3dPtr::from_raw(view, "terrain render-target view")?,
        desc: target.desc.clone(),
    })
}

#[cfg(windows)]
pub(crate) unsafe fn native_d3d11_create_depth_stencil(
    device: &NativeD3d11Device,
    depth: &MapFrameOutputMergerTexture,
) -> Result<NativeD3d11DepthStencil> {
    let texture = native_d3d11_create_texture2d_from_desc(&device.device, &depth.desc, None)?;
    let view_desc = native_d3d11_depth_stencil_view_desc(depth)?;
    let mut view: *mut ID3D11DepthStencilView = ptr::null_mut();
    let hr = (*device.device.as_raw()).CreateDepthStencilView(
        texture.as_raw() as *mut _,
        &view_desc,
        &mut view,
    );
    native_d3d11_hr(
        hr,
        "CreateDepthStencilView for captured terrain depth target",
    )?;
    Ok(NativeD3d11DepthStencil {
        texture,
        view: D3dPtr::from_raw(view, "terrain depth-stencil view")?,
        desc: depth.desc.clone(),
    })
}

#[cfg(windows)]
pub(crate) unsafe fn native_d3d11_create_terrain_color_snapshot_pipeline(
    device: &NativeD3d11Device,
) -> Result<NativeD3d11TerrainColorSnapshotPipeline> {
    let vs_blob = native_d3d11_compile_shader(TERRAIN_COLOR_SNAPSHOT_HLSL, "vs_main", "vs_4_0")?;
    let ps_blob = native_d3d11_compile_shader(TERRAIN_COLOR_SNAPSHOT_HLSL, "ps_main", "ps_4_0")?;
    let mut vertex_shader: *mut ID3D11VertexShader = ptr::null_mut();
    native_d3d11_hr(
        (*device.device.as_raw()).CreateVertexShader(
            (*vs_blob.as_raw()).GetBufferPointer(),
            (*vs_blob.as_raw()).GetBufferSize(),
            ptr::null_mut(),
            &mut vertex_shader,
        ),
        "CreateVertexShader for terrain-color BGRA8 snapshot",
    )?;
    let mut pixel_shader: *mut ID3D11PixelShader = ptr::null_mut();
    native_d3d11_hr(
        (*device.device.as_raw()).CreatePixelShader(
            (*ps_blob.as_raw()).GetBufferPointer(),
            (*ps_blob.as_raw()).GetBufferSize(),
            ptr::null_mut(),
            &mut pixel_shader,
        ),
        "CreatePixelShader for terrain-color BGRA8 snapshot",
    )?;
    Ok(NativeD3d11TerrainColorSnapshotPipeline {
        vertex_shader: D3dPtr::from_raw(vertex_shader, "terrain-color snapshot vertex shader")?,
        pixel_shader: D3dPtr::from_raw(pixel_shader, "terrain-color snapshot pixel shader")?,
    })
}

#[cfg(windows)]
pub(crate) unsafe fn native_d3d11_compile_shader(
    source: &str,
    entry: &str,
    target: &str,
) -> Result<D3dPtr<ID3DBlob>> {
    let entry_c = CString::new(entry).map_err(|_| "shader entry point contains NUL")?;
    let target_c = CString::new(target).map_err(|_| "shader target contains NUL")?;
    let name_c = CString::new("terrain_color_snapshot.hlsl")
        .map_err(|_| "shader source name contains NUL")?;
    let mut code: *mut ID3DBlob = ptr::null_mut();
    let mut errors: *mut ID3DBlob = ptr::null_mut();
    let hr = D3DCompile(
        source.as_ptr() as *const _,
        source.len(),
        name_c.as_ptr(),
        ptr::null(),
        ptr::null_mut(),
        entry_c.as_ptr(),
        target_c.as_ptr(),
        0,
        0,
        &mut code,
        &mut errors,
    );
    let error_text = if errors.is_null() {
        String::new()
    } else {
        let blob = D3dPtr::from_raw(errors, "terrain-color snapshot shader error blob")?;
        let data = (*blob.as_raw()).GetBufferPointer() as *const u8;
        let len = (*blob.as_raw()).GetBufferSize();
        String::from_utf8_lossy(std::slice::from_raw_parts(data, len)).into_owned()
    };
    if hr < 0 {
        return Err(format!(
            "production native D3D11 map graph blocked: D3DCompile({entry}, {target}) failed with HRESULT 0x{:08X}: {}",
            hr as u32,
            error_text.trim()
        )
        .into());
    }
    D3dPtr::from_raw(code, "terrain-color snapshot shader blob")
}

#[cfg(windows)]
pub(crate) unsafe fn native_d3d11_create_terrain_color_snapshot_srv(
    device: &NativeD3d11Device,
    pipeline: &NativeD3d11TerrainColorSnapshotPipeline,
    render_target: &NativeD3d11RenderTarget,
    srv_view_desc: &MapFrameShaderResourceViewDesc,
) -> Result<NativeD3d11TerrainColorSnapshot> {
    if render_target.desc.mip_levels != 1 || render_target.desc.array_size != 1 {
        return Err(format!(
            "production native D3D11 map graph blocked: app-owned terrain color snapshot requires a single Texture2D mip/array target, got mip_levels {} array_size {}",
            render_target.desc.mip_levels, render_target.desc.array_size
        )
        .into());
    }
    if render_target.desc.sample_count == 0 {
        return Err(
            "production native D3D11 map graph blocked: terrain color target has zero sample count"
                .into(),
        );
    }
    if srv_view_desc.view_dimension != D3D11_SRV_DIMENSION_TEXTURE2D
        || srv_view_desc.most_detailed_mip != 0
        || srv_view_desc.mip_levels != 1
        || srv_view_desc.first_array_slice != 0
        || srv_view_desc.array_size > 1
        || srv_view_desc.num_cubes != 0
    {
        return Err(
            "production native D3D11 map graph blocked: WaterRefraction SRV descriptor is not compatible with the app-owned terrain color snapshot"
                .into(),
        );
    }
    let source_desc = MapFrameTexture2DDesc {
        mip_levels: 1,
        array_size: 1,
        sample_count: 1,
        sample_quality: 0,
        usage: D3D11_USAGE_DEFAULT,
        bind_flags: D3D11_BIND_SHADER_RESOURCE,
        cpu_access_flags: 0,
        misc_flags: 0,
        ..render_target.desc.clone()
    };
    let source_texture =
        native_d3d11_create_texture2d_from_desc(&device.device, &source_desc, None)?;
    (*device.context.as_raw()).OMSetRenderTargets(0, ptr::null_mut(), ptr::null_mut());
    if render_target.desc.sample_count > 1 {
        (*device.context.as_raw()).ResolveSubresource(
            source_texture.as_raw() as *mut _,
            0,
            render_target.texture.as_raw() as *mut _,
            0,
            render_target.desc.format as _,
        );
    } else {
        (*device.context.as_raw()).CopyResource(
            source_texture.as_raw() as *mut _,
            render_target.texture.as_raw() as *mut _,
        );
    }
    let texture = if srv_view_desc.format == render_target.desc.format {
        source_texture
    } else {
        let snapshot_desc = MapFrameTexture2DDesc {
            format: srv_view_desc.format,
            bind_flags: D3D11_BIND_RENDER_TARGET | D3D11_BIND_SHADER_RESOURCE,
            ..source_desc.clone()
        };
        let snapshot_texture =
            native_d3d11_create_texture2d_from_desc(&device.device, &snapshot_desc, None)?;
        native_d3d11_convert_terrain_color_snapshot(
            device,
            pipeline,
            &source_texture,
            &source_desc,
            &snapshot_texture,
            &snapshot_desc,
        )?;
        snapshot_texture
    };
    let view_resource = MapFrameTextureBinding {
        stage: "PS".to_string(),
        slot: 0,
        declared_name: Some(WATER_REFRACTION_RESOURCE_SYMBOL.to_string()),
        is_null: false,
        view_desc: Some(srv_view_desc.clone()),
        file: None,
        byte_len: 0,
        fnv1a64: None,
        dxgi_format: srv_view_desc.format,
        width: render_target.desc.width,
        height: render_target.desc.height,
        mip_count: 1,
        array_size: 1,
        sample_count: 1,
        sample_quality: 0,
        payload_format: None,
        subresources: Vec::new(),
        bind_flags: D3D11_BIND_SHADER_RESOURCE,
        usage: D3D11_USAGE_DEFAULT,
        cpu_access_flags: 0,
        misc_flags: 0,
    };
    let srv_desc = native_d3d11_shader_resource_view_desc(&view_resource)?.ok_or(
        "production native D3D11 map graph blocked: WaterRefraction has no SRV descriptor",
    )?;
    let mut view: *mut ID3D11ShaderResourceView = ptr::null_mut();
    native_d3d11_hr(
        (*device.device.as_raw()).CreateShaderResourceView(
            texture.as_raw() as *mut _,
            &srv_desc,
            &mut view,
        ),
        "CreateShaderResourceView for app-owned terrain color snapshot",
    )?;
    Ok(NativeD3d11TerrainColorSnapshot {
        _texture: texture,
        view: D3dPtr::from_raw(view, "app-owned terrain color WaterRefraction SRV")?,
    })
}

#[cfg(windows)]
pub(crate) unsafe fn native_d3d11_convert_terrain_color_snapshot(
    device: &NativeD3d11Device,
    pipeline: &NativeD3d11TerrainColorSnapshotPipeline,
    source_texture: &D3dPtr<ID3D11Texture2D>,
    source_desc: &MapFrameTexture2DDesc,
    snapshot_texture: &D3dPtr<ID3D11Texture2D>,
    snapshot_desc: &MapFrameTexture2DDesc,
) -> Result<()> {
    if snapshot_desc.format != DXGI_FORMAT_B8G8R8A8_UNORM
        && snapshot_desc.format != DXGI_FORMAT_R8G8B8A8_UNORM
    {
        return Err(format!(
            "production native D3D11 map graph blocked: WaterRefraction format {} needs an explicit terrain-color conversion path",
            snapshot_desc.format
        )
        .into());
    }
    let source_resource = MapFrameTextureBinding {
        stage: "PS".to_string(),
        slot: 0,
        declared_name: Some("TerrainColorSnapshotSource".to_string()),
        is_null: false,
        view_desc: Some(MapFrameShaderResourceViewDesc {
            format: source_desc.format,
            view_dimension: D3D11_SRV_DIMENSION_TEXTURE2D,
            most_detailed_mip: 0,
            mip_levels: 1,
            first_array_slice: 0,
            array_size: 1,
            num_cubes: 0,
        }),
        file: None,
        byte_len: 0,
        fnv1a64: None,
        dxgi_format: source_desc.format,
        width: source_desc.width,
        height: source_desc.height,
        mip_count: 1,
        array_size: 1,
        sample_count: 1,
        sample_quality: 0,
        payload_format: None,
        subresources: Vec::new(),
        bind_flags: D3D11_BIND_SHADER_RESOURCE,
        usage: D3D11_USAGE_DEFAULT,
        cpu_access_flags: 0,
        misc_flags: 0,
    };
    let source_srv_desc = native_d3d11_shader_resource_view_desc(&source_resource)?.ok_or(
        "production native D3D11 map graph blocked: terrain-color snapshot source has no SRV descriptor",
    )?;
    let mut source_srv: *mut ID3D11ShaderResourceView = ptr::null_mut();
    native_d3d11_hr(
        (*device.device.as_raw()).CreateShaderResourceView(
            source_texture.as_raw() as *mut _,
            &source_srv_desc,
            &mut source_srv,
        ),
        "CreateShaderResourceView for terrain-color snapshot source",
    )?;
    let source_srv = D3dPtr::from_raw(source_srv, "terrain-color snapshot source SRV")?;
    let mut rtv_desc: D3D11_RENDER_TARGET_VIEW_DESC = mem::zeroed();
    rtv_desc.Format = snapshot_desc.format as _;
    rtv_desc.ViewDimension = D3D11_RTV_DIMENSION_TEXTURE2D;
    *rtv_desc.u.Texture2D_mut() = D3D11_TEX2D_RTV { MipSlice: 0 };
    let mut snapshot_rtv: *mut ID3D11RenderTargetView = ptr::null_mut();
    native_d3d11_hr(
        (*device.device.as_raw()).CreateRenderTargetView(
            snapshot_texture.as_raw() as *mut _,
            &rtv_desc,
            &mut snapshot_rtv,
        ),
        "CreateRenderTargetView for terrain-color BGRA8 snapshot",
    )?;
    let snapshot_rtv = D3dPtr::from_raw(snapshot_rtv, "terrain-color BGRA8 snapshot RTV")?;
    let viewport = D3D11_VIEWPORT {
        TopLeftX: 0.0,
        TopLeftY: 0.0,
        Width: snapshot_desc.width as f32,
        Height: snapshot_desc.height as f32,
        MinDepth: 0.0,
        MaxDepth: 1.0,
    };
    let mut rtv = snapshot_rtv.as_raw();
    (*device.context.as_raw()).OMSetRenderTargets(1, &mut rtv, ptr::null_mut());
    (*device.context.as_raw()).RSSetViewports(1, &viewport);
    (*device.context.as_raw()).RSSetState(ptr::null_mut());
    let blend_factor = [0.0f32, 0.0, 0.0, 0.0];
    (*device.context.as_raw()).OMSetBlendState(ptr::null_mut(), &blend_factor, u32::MAX);
    (*device.context.as_raw()).OMSetDepthStencilState(ptr::null_mut(), 0);
    (*device.context.as_raw())
        .IASetPrimitiveTopology(winapi::um::d3dcommon::D3D11_PRIMITIVE_TOPOLOGY_TRIANGLELIST);
    (*device.context.as_raw()).IASetInputLayout(ptr::null_mut());
    (*device.context.as_raw()).VSSetShader(pipeline.vertex_shader.as_raw(), ptr::null_mut(), 0);
    (*device.context.as_raw()).PSSetShader(pipeline.pixel_shader.as_raw(), ptr::null_mut(), 0);
    let mut srv = source_srv.as_raw();
    (*device.context.as_raw()).PSSetShaderResources(0, 1, &mut srv);
    (*device.context.as_raw()).Draw(3, 0);
    let mut null_srv: *mut ID3D11ShaderResourceView = ptr::null_mut();
    (*device.context.as_raw()).PSSetShaderResources(0, 1, &mut null_srv);
    (*device.context.as_raw()).OMSetRenderTargets(0, ptr::null_mut(), ptr::null_mut());
    Ok(())
}

#[cfg(windows)]
pub(crate) fn native_d3d11_render_target_view_desc(
    target: &MapFrameOutputMergerTexture,
) -> Result<D3D11_RENDER_TARGET_VIEW_DESC> {
    let view = target.view_desc.as_ref().ok_or(
        "production native D3D11 terrain replay blocked: render target is missing captured D3D11 view descriptor",
    )?;
    let mut desc: D3D11_RENDER_TARGET_VIEW_DESC = unsafe { mem::zeroed() };
    desc.Format = view.format as _;
    desc.ViewDimension = view.view_dimension as _;
    unsafe {
        match view.view_dimension {
            D3D11_RTV_DIMENSION_TEXTURE2D => {
                *desc.u.Texture2D_mut() = D3D11_TEX2D_RTV {
                    MipSlice: view.mip_slice,
                };
            }
            D3D11_RTV_DIMENSION_TEXTURE2DARRAY => {
                *desc.u.Texture2DArray_mut() = D3D11_TEX2D_ARRAY_RTV {
                    MipSlice: view.mip_slice,
                    FirstArraySlice: view.first_array_slice,
                    ArraySize: view.array_size,
                };
            }
            D3D11_RTV_DIMENSION_TEXTURE2DMS => {
                *desc.u.Texture2DMS_mut() = D3D11_TEX2DMS_RTV {
                    UnusedField_NothingToDefine: 0,
                };
            }
            D3D11_RTV_DIMENSION_TEXTURE2DMSARRAY => {
                *desc.u.Texture2DMSArray_mut() = D3D11_TEX2DMS_ARRAY_RTV {
                    FirstArraySlice: view.first_array_slice,
                    ArraySize: view.array_size,
                };
            }
            other => {
                return Err(format!(
                    "production native D3D11 terrain replay blocked: unsupported render-target view dimension {other}"
                )
                .into())
            }
        }
    }
    Ok(desc)
}

#[cfg(windows)]
pub(crate) fn native_d3d11_depth_stencil_view_desc(
    depth: &MapFrameOutputMergerTexture,
) -> Result<D3D11_DEPTH_STENCIL_VIEW_DESC> {
    let view = depth.view_desc.as_ref().ok_or(
        "production native D3D11 terrain replay blocked: depth-stencil is missing captured D3D11 view descriptor",
    )?;
    let mut desc: D3D11_DEPTH_STENCIL_VIEW_DESC = unsafe { mem::zeroed() };
    desc.Format = view.format as _;
    desc.ViewDimension = view.view_dimension as _;
    desc.Flags = view.flags;
    unsafe {
        match view.view_dimension {
            D3D11_DSV_DIMENSION_TEXTURE2D => {
                *desc.u.Texture2D_mut() = D3D11_TEX2D_DSV {
                    MipSlice: view.mip_slice,
                };
            }
            D3D11_DSV_DIMENSION_TEXTURE2DARRAY => {
                *desc.u.Texture2DArray_mut() = D3D11_TEX2D_ARRAY_DSV {
                    MipSlice: view.mip_slice,
                    FirstArraySlice: view.first_array_slice,
                    ArraySize: view.array_size,
                };
            }
            D3D11_DSV_DIMENSION_TEXTURE2DMS => {
                *desc.u.Texture2DMS_mut() = D3D11_TEX2DMS_DSV {
                    UnusedField_NothingToDefine: 0,
                };
            }
            D3D11_DSV_DIMENSION_TEXTURE2DMSARRAY => {
                *desc.u.Texture2DMSArray_mut() = D3D11_TEX2DMS_ARRAY_DSV {
                    FirstArraySlice: view.first_array_slice,
                    ArraySize: view.array_size,
                };
            }
            other => {
                return Err(format!(
                    "production native D3D11 terrain replay blocked: unsupported depth-stencil view dimension {other}"
                )
                .into())
            }
        }
    }
    Ok(desc)
}

#[cfg(windows)]
pub(crate) unsafe fn native_d3d11_create_texture2d_from_desc(
    device: &D3dPtr<ID3D11Device>,
    desc: &MapFrameTexture2DDesc,
    initial: Option<&[D3D11_SUBRESOURCE_DATA]>,
) -> Result<D3dPtr<ID3D11Texture2D>> {
    let native_desc = D3D11_TEXTURE2D_DESC {
        Width: desc.width,
        Height: desc.height,
        MipLevels: desc.mip_levels,
        ArraySize: desc.array_size,
        Format: desc.format as _,
        SampleDesc: DXGI_SAMPLE_DESC {
            Count: desc.sample_count,
            Quality: desc.sample_quality,
        },
        Usage: desc.usage as _,
        BindFlags: desc.bind_flags,
        CPUAccessFlags: desc.cpu_access_flags,
        MiscFlags: desc.misc_flags,
    };
    let mut texture: *mut ID3D11Texture2D = ptr::null_mut();
    let initial_ptr = initial.map(|data| data.as_ptr()).unwrap_or(ptr::null());
    let hr = (*device.as_raw()).CreateTexture2D(&native_desc, initial_ptr, &mut texture);
    native_d3d11_hr(hr, "CreateTexture2D from captured descriptor")?;
    D3dPtr::from_raw(texture, "D3D11 texture2D")
}

#[cfg(windows)]
pub(crate) unsafe fn native_d3d11_create_b8g8r8x8_texture_srv(
    device: &D3dPtr<ID3D11Device>,
    width: u32,
    height: u32,
    bgra8: &[u8],
    role: &str,
) -> Result<(
    D3dPtr<ID3D11Texture2D>,
    D3dPtr<ID3D11ShaderResourceView>,
    MapFrameTexture2DDesc,
    MapFrameShaderResourceViewDesc,
)> {
    if width == 0 || height == 0 {
        return Err(format!("{role}: texture dimensions must be nonzero").into());
    }
    let row_pitch = width
        .checked_mul(4)
        .ok_or_else(|| format!("{role}: texture row pitch overflow"))?;
    let depth_pitch = row_pitch
        .checked_mul(height)
        .ok_or_else(|| format!("{role}: texture depth pitch overflow"))?;
    if bgra8.len() != usize::try_from(depth_pitch)? {
        return Err(format!(
            "{role}: B8G8R8X8 payload has {} bytes, expected {depth_pitch}",
            bgra8.len()
        )
        .into());
    }
    let texture_desc = MapFrameTexture2DDesc {
        width,
        height,
        mip_levels: 1,
        array_size: 1,
        format: DXGI_FORMAT_B8G8R8X8_UNORM,
        sample_count: 1,
        sample_quality: 0,
        usage: D3D11_USAGE_DEFAULT,
        bind_flags: D3D11_BIND_SHADER_RESOURCE,
        cpu_access_flags: 0,
        misc_flags: 0,
    };
    let initial = [D3D11_SUBRESOURCE_DATA {
        pSysMem: bgra8.as_ptr() as *const _,
        SysMemPitch: row_pitch,
        SysMemSlicePitch: depth_pitch,
    }];
    let texture = native_d3d11_create_texture2d_from_desc(device, &texture_desc, Some(&initial))?;
    let view_desc = MapFrameShaderResourceViewDesc {
        format: DXGI_FORMAT_B8G8R8X8_UNORM,
        view_dimension: D3D11_SRV_DIMENSION_TEXTURE2D,
        most_detailed_mip: 0,
        mip_levels: 1,
        first_array_slice: 0,
        array_size: 1,
        num_cubes: 0,
    };
    let mut native_view_desc: D3D11_SHADER_RESOURCE_VIEW_DESC = mem::zeroed();
    native_view_desc.Format = view_desc.format as _;
    native_view_desc.ViewDimension = view_desc.view_dimension as _;
    *native_view_desc.u.Texture2D_mut() = D3D11_TEX2D_SRV {
        MostDetailedMip: 0,
        MipLevels: 1,
    };
    let mut srv: *mut ID3D11ShaderResourceView = ptr::null_mut();
    native_d3d11_hr(
        (*device.as_raw()).CreateShaderResourceView(
            texture.as_raw() as *mut _,
            &native_view_desc,
            &mut srv,
        ),
        &format!("CreateShaderResourceView for {role}"),
    )?;
    Ok((
        texture,
        D3dPtr::from_raw(srv, role)?,
        texture_desc,
        view_desc,
    ))
}

#[cfg(windows)]
pub(crate) unsafe fn native_d3d11_clear_targets(
    context: &D3dPtr<ID3D11DeviceContext>,
    target: &MapFrameOutputMergerTexture,
    depth: &MapFrameOutputMergerTexture,
    render_target: &NativeD3d11RenderTarget,
    depth_target: &NativeD3d11DepthStencil,
) {
    if let Some(clear) = &target.last_render_target_clear {
        (*context.as_raw()).ClearRenderTargetView(render_target.view.as_raw(), &clear.color);
    } else {
        (*context.as_raw())
            .ClearRenderTargetView(render_target.view.as_raw(), &[0.0, 0.0, 0.0, 0.0]);
    }
    if let Some(clear) = &depth.last_depth_stencil_clear {
        (*context.as_raw()).ClearDepthStencilView(
            depth_target.view.as_raw(),
            clear.clear_flags,
            clear.depth,
            clear.stencil,
        );
    } else {
        (*context.as_raw()).ClearDepthStencilView(
            depth_target.view.as_raw(),
            crate::constants::D3D11_CLEAR_DEPTH | crate::constants::D3D11_CLEAR_STENCIL,
            1.0,
            0,
        );
    }
}

#[cfg(windows)]
pub(crate) unsafe fn native_d3d11_prepare_map_frame(
    device: &NativeD3d11Device,
    frame: &MapFrameRuntimeBundle,
    bundle_dir: &Path,
    app_camera: Option<ProductionMapCamera>,
) -> Result<NativeD3d11PreparedMapFrame> {
    let restore_pre_draw_outputs = frame.capture_moment != TERRAIN_CAPTURE_MOMENT;
    validate_native_d3d11_replay_payloads(frame, restore_pre_draw_outputs)?;
    let (target, depth) = native_d3d11_first_output_merger_targets(frame)?;

    let render_target = native_d3d11_create_render_target(device, target)?;
    let depth_target = native_d3d11_create_depth_stencil(device, depth)?;
    let pass = native_d3d11_prepare_map_pass(
        device,
        frame,
        bundle_dir,
        app_camera,
        restore_pre_draw_outputs,
        NativeD3d11WaterRefractionSource::CapturedBundleTexture,
        NativeD3d11ReflectionCubeSource::ShippedFileCube,
        None,
        false,
    )?;
    Ok(NativeD3d11PreparedMapFrame {
        render_target,
        depth_target,
        pass,
    })
}

#[cfg(windows)]
pub(crate) unsafe fn native_d3d11_prepare_terrain_chunk_library_frame(
    device: &NativeD3d11Device,
    frame: &MapFrameRuntimeBundle,
    bundle_dir: &Path,
    app_camera: ProductionMapCamera,
) -> Result<NativeD3d11PreparedMapFrame> {
    let (target, depth) = native_d3d11_first_output_merger_targets(frame)?;
    let render_target = native_d3d11_create_render_target(device, target)?;
    let depth_target = native_d3d11_create_depth_stencil(device, depth)?;
    let pass = native_d3d11_prepare_terrain_chunk_library_pass(
        device, frame, bundle_dir, app_camera, target, depth,
    )?;
    Ok(NativeD3d11PreparedMapFrame {
        render_target,
        depth_target,
        pass,
    })
}

#[cfg(windows)]
pub(crate) fn native_d3d11_first_output_merger_targets(
    frame: &MapFrameRuntimeBundle,
) -> Result<(&MapFrameOutputMergerTexture, &MapFrameOutputMergerTexture)> {
    let first_draw = frame
        .draws
        .first()
        .ok_or("production native D3D11 terrain replay blocked: map-frame bundle has no draws")?;
    let first_state = first_draw.draw_state.as_ref().ok_or(
        "production native D3D11 terrain replay blocked: first draw has no captured D3D11 state",
    )?;
    let output_merger = first_state.output_merger.as_ref().ok_or(
        "production native D3D11 terrain replay blocked: first draw has no output-merger capture",
    )?;
    let target = output_merger.render_targets.first().ok_or(
        "production native D3D11 terrain replay blocked: output-merger has no render target",
    )?;
    let depth = output_merger.depth_stencil.as_ref().ok_or(
        "production native D3D11 terrain replay blocked: output-merger has no depth-stencil target",
    )?;
    Ok((target, depth))
}

#[cfg(windows)]
pub(crate) fn native_d3d11_draw_output_merger_targets(
    draw: &MapFrameDraw,
) -> Result<(&MapFrameOutputMergerTexture, &MapFrameOutputMergerTexture)> {
    let state = draw.draw_state.as_ref().ok_or(
        "production native D3D11 terrain chunk synthesis blocked: template draw has no captured D3D11 state",
    )?;
    let output_merger = state.output_merger.as_ref().ok_or(
        "production native D3D11 terrain chunk synthesis blocked: template draw has no output-merger capture",
    )?;
    let target = output_merger.render_targets.first().ok_or(
        "production native D3D11 terrain chunk synthesis blocked: template output-merger has no render target",
    )?;
    let depth = output_merger.depth_stencil.as_ref().ok_or(
        "production native D3D11 terrain chunk synthesis blocked: template output-merger has no depth-stencil target",
    )?;
    Ok((target, depth))
}

#[cfg(windows)]
pub(crate) unsafe fn native_d3d11_prepare_terrain_chunk_synthesis_frame(
    device: &NativeD3d11Device,
    plan: &TerrainChunkSynthesisPlan,
    bundle_dir: &Path,
    app_camera: ProductionMapCamera,
    scene_cb0_override: Option<&[u8]>,
    synthetic_payloads: &BTreeMap<String, MapFrameSyntheticPayload>,
) -> Result<NativeD3d11PreparedTerrainChunkSynthesisFrame> {
    let (target, depth) = native_d3d11_draw_output_merger_targets(&plan.template_draw)?;
    let render_target = native_d3d11_create_render_target(device, target)?;
    let depth_target = native_d3d11_create_depth_stencil(device, depth)?;
    // the captured files). Under `--chunks-from-files` it carries the generated
    let template_draw = native_d3d11_prepare_draw(
        device,
        bundle_dir,
        synthetic_payloads,
        &plan.template_draw,
        target,
        Some(app_camera),
        false,
        NativeD3d11WaterRefractionSource::CapturedBundleTexture,
        NativeD3d11ReflectionCubeSource::ShippedFileCube,
        None,
        "terrain chunk synthesis",
        true,
        scene_cb0_override,
        false,
    )?;
    Ok(NativeD3d11PreparedTerrainChunkSynthesisFrame {
        render_target,
        depth_target,
        template_draw,
        target_evidence: target.clone(),
        depth_evidence: depth.clone(),
    })
}

#[cfg(windows)]
pub(crate) unsafe fn native_d3d11_prepare_map_pass(
    device: &NativeD3d11Device,
    frame: &MapFrameRuntimeBundle,
    bundle_dir: &Path,
    app_camera: Option<ProductionMapCamera>,
    restore_pre_draw_outputs: bool,
    water_refraction_source: NativeD3d11WaterRefractionSource,
    reflection_cube_source: NativeD3d11ReflectionCubeSource,
    reflection_resolver: Option<&GameFileResolver>,
    patch_tree_season_cb1: bool,
) -> Result<NativeD3d11PreparedMapPass> {
    validate_native_d3d11_replay_payloads(frame, restore_pre_draw_outputs)?;
    let (target, depth) = native_d3d11_first_output_merger_targets(frame)?;
    let app_owned_terrain_color_srv_desc =
        water_refraction_app_owned_terrain_color_srv_desc(frame, &target.desc)?;
    let reference_output = native_d3d11_load_post_pass_reference_output(frame, bundle_dir, target)?;
    let pass_label = ProductionMapPassKind::from_capture_moment(&frame.capture_moment)
        .map(ProductionMapPassKind::pass_label)
        .unwrap_or("unknown map");
    let draws = frame
        .draws
        .iter()
        .map(|draw| {
            native_d3d11_prepare_draw(
                device,
                bundle_dir,
                &frame.synthetic_payloads,
                draw,
                target,
                app_camera,
                restore_pre_draw_outputs,
                water_refraction_source,
                reflection_cube_source,
                reflection_resolver,
                pass_label,
                false,
                None,
                patch_tree_season_cb1,
            )
        })
        .collect::<Result<Vec<_>>>()?;
    let summary = NativeD3d11ReplaySummary {
        draw_count: frame.draws.len(),
        vertex_buffer_count: frame
            .draws
            .iter()
            .map(|draw| draw.vertex_buffers.len())
            .sum(),
        index_buffer_count: frame
            .draws
            .iter()
            .filter(|draw| draw.index_buffer.is_some())
            .count(),
        constant_buffer_count: frame
            .draws
            .iter()
            .map(|draw| draw.constant_buffers.len())
            .sum(),
        shader_resource_count: frame
            .draws
            .iter()
            .map(|draw| draw.shader_resources.len())
            .sum(),
        sampler_count: frame.draws.iter().map(|draw| draw.samplers.len()).sum(),
        target_width: target.desc.width,
        target_height: target.desc.height,
        target_format: target.desc.format,
    };
    Ok(NativeD3d11PreparedMapPass {
        pass_label: pass_label.to_string(),
        target_evidence: target.clone(),
        depth_evidence: depth.clone(),
        reference_output,
        app_owned_terrain_color_srv_desc,
        draws,
        summary,
    })
}

#[cfg(windows)]
unsafe fn native_d3d11_prepare_terrain_chunk_library_pass(
    device: &NativeD3d11Device,
    frame: &MapFrameRuntimeBundle,
    bundle_dir: &Path,
    app_camera: ProductionMapCamera,
    target: &MapFrameOutputMergerTexture,
    depth: &MapFrameOutputMergerTexture,
) -> Result<NativeD3d11PreparedMapPass> {
    let draws = frame
        .draws
        .iter()
        .map(|draw| {
            native_d3d11_prepare_draw(
                device,
                bundle_dir,
                &frame.synthetic_payloads,
                draw,
                target,
                Some(app_camera),
                false,
                NativeD3d11WaterRefractionSource::CapturedBundleTexture,
                NativeD3d11ReflectionCubeSource::ShippedFileCube,
                None,
                "terrain",
                false,
                None,
                false,
            )
        })
        .collect::<Result<Vec<_>>>()?;
    let summary = NativeD3d11ReplaySummary {
        draw_count: frame.draws.len(),
        vertex_buffer_count: frame
            .draws
            .iter()
            .map(|draw| draw.vertex_buffers.len())
            .sum(),
        index_buffer_count: frame
            .draws
            .iter()
            .filter(|draw| draw.index_buffer.is_some())
            .count(),
        constant_buffer_count: frame
            .draws
            .iter()
            .map(|draw| draw.constant_buffers.len())
            .sum(),
        shader_resource_count: frame
            .draws
            .iter()
            .map(|draw| draw.shader_resources.len())
            .sum(),
        sampler_count: frame.draws.iter().map(|draw| draw.samplers.len()).sum(),
        target_width: target.desc.width,
        target_height: target.desc.height,
        target_format: target.desc.format,
    };
    Ok(NativeD3d11PreparedMapPass {
        pass_label: "terrain".to_string(),
        target_evidence: target.clone(),
        depth_evidence: depth.clone(),
        reference_output: NativeD3d11ReferenceOutput {
            width: target.desc.width,
            height: target.desc.height,
            dxgi_format: target.desc.format,
            bytes: Vec::new(),
            fnv1a64: String::new(),
            file: "terrain_chunk_library_no_frame_reference".to_string(),
        },
        app_owned_terrain_color_srv_desc: None,
        draws,
        summary,
    })
}

#[cfg(windows)]
pub(crate) fn native_d3d11_load_post_pass_reference_output(
    frame: &MapFrameRuntimeBundle,
    bundle_dir: &Path,
    replay_target: &MapFrameOutputMergerTexture,
) -> Result<NativeD3d11ReferenceOutput> {
    let output_merger = frame.post_pass_output_merger.as_ref().ok_or(
        "production native D3D11 terrain replay blocked: map-frame bundle has no post-pass output-merger reference",
    )?;
    let pass_label = ProductionMapPassKind::from_capture_moment(&frame.capture_moment)
        .map(ProductionMapPassKind::pass_label)
        .unwrap_or("unknown map");
    let label = format!("post-pass {pass_label} render target");
    native_d3d11_load_output_merger_reference_output(
        output_merger,
        bundle_dir,
        replay_target,
        &label,
    )
}

#[cfg(windows)]
pub(crate) fn native_d3d11_load_output_merger_reference_output(
    output_merger: &MapFrameOutputMergerState,
    bundle_dir: &Path,
    replay_target: &MapFrameOutputMergerTexture,
    label: &str,
) -> Result<NativeD3d11ReferenceOutput> {
    let reference_target = output_merger.render_targets.first().ok_or_else(|| {
        format!(
            "production native D3D11 terrain replay blocked: captured output-merger has no {label} reference"
        )
    })?;
    if reference_target.desc.width != replay_target.desc.width
        || reference_target.desc.height != replay_target.desc.height
        || reference_target.desc.format != replay_target.desc.format
    {
        return Err(format!(
            "production native D3D11 terrain replay blocked: {label} reference target {}x{} format {} does not match replay target {}x{} format {}",
            reference_target.desc.width,
            reference_target.desc.height,
            reference_target.desc.format,
            replay_target.desc.width,
            replay_target.desc.height,
            replay_target.desc.format
        )
        .into());
    }
    let payload = reference_target.post_pass_payload.as_ref().ok_or_else(|| {
        format!("production native D3D11 terrain replay blocked: captured {label} has no payload")
    })?;
    native_d3d11_load_output_merger_texture_reference_output(
        reference_target,
        payload,
        bundle_dir,
        replay_target,
        label,
    )
}

#[cfg(windows)]
pub(crate) fn native_d3d11_load_output_merger_texture_reference_output(
    reference_target: &MapFrameOutputMergerTexture,
    payload: &MapFrameOutputMergerTexturePayload,
    bundle_dir: &Path,
    replay_target: &MapFrameOutputMergerTexture,
    label: &str,
) -> Result<NativeD3d11ReferenceOutput> {
    if reference_target.desc.width != replay_target.desc.width
        || reference_target.desc.height != replay_target.desc.height
        || reference_target.desc.format != replay_target.desc.format
    {
        return Err(format!(
            "production native D3D11 terrain replay blocked: {label} reference target {}x{} format {} does not match replay target {}x{} format {}",
            reference_target.desc.width,
            reference_target.desc.height,
            reference_target.desc.format,
            replay_target.desc.width,
            replay_target.desc.height,
            replay_target.desc.format
        )
        .into());
    }
    let raw = read_verified_map_frame_file(
        bundle_dir,
        &payload.file,
        payload.byte_len,
        &payload.fnv1a64,
        label,
    )?;
    let subresource = payload.subresources.first().ok_or_else(|| {
        format!(
            "production native D3D11 terrain replay blocked: captured {label} has no subresource layout"
        )
    })?;
    let row_bytes = native_d3d11_render_target_row_bytes(
        reference_target.desc.format,
        reference_target.desc.width,
    )?;
    if subresource.row_pitch < row_bytes {
        return Err(format!(
            "production native D3D11 terrain replay blocked: {label} row_pitch {} is below row byte width {}",
            subresource.row_pitch, row_bytes
        )
        .into());
    }
    let start = subresource.file_offset;
    let depth_pitch = usize::try_from(subresource.depth_pitch)?;
    let end = start.checked_add(depth_pitch).ok_or(
        "production native D3D11 terrain replay blocked: captured target byte range overflow",
    )?;
    if end > raw.len() {
        return Err(format!(
            "production native D3D11 terrain replay blocked: {label} subresource byte range {start}..{end} exceeds file byte_len {}",
            raw.len()
        )
        .into());
    }
    let row_pitch = usize::try_from(subresource.row_pitch)?;
    let row_bytes = usize::try_from(row_bytes)?;
    let mut bytes = Vec::with_capacity(
        row_bytes
            .checked_mul(usize::try_from(reference_target.desc.height)?)
            .ok_or("production native D3D11 terrain replay blocked: captured reference byte_len overflow")?,
    );
    for row in 0..usize::try_from(reference_target.desc.height)? {
        let row_start = start
            .checked_add(row.checked_mul(row_pitch).ok_or(
                "production native D3D11 terrain replay blocked: captured reference row offset overflow",
            )?)
            .ok_or("production native D3D11 terrain replay blocked: captured reference row start overflow")?;
        let row_end = row_start.checked_add(row_bytes).ok_or(
            "production native D3D11 terrain replay blocked: captured reference row end overflow",
        )?;
        if row_end > end {
            return Err(format!(
                "production native D3D11 terrain replay blocked: {label} reference row {row} extends beyond captured subresource"
            )
            .into());
        }
        bytes.extend_from_slice(&raw[row_start..row_end]);
    }
    Ok(NativeD3d11ReferenceOutput {
        width: reference_target.desc.width,
        height: reference_target.desc.height,
        dxgi_format: reference_target.desc.format,
        bytes,
        fnv1a64: payload.fnv1a64.clone(),
        file: payload.file.clone(),
    })
}

#[cfg(windows)]
pub(crate) unsafe fn native_d3d11_prepare_pre_draw_output_targets(
    device: &D3dPtr<ID3D11Device>,
    bundle_dir: &Path,
    draw: &MapFrameDraw,
    replay_target: &MapFrameOutputMergerTexture,
) -> Result<(
    Option<D3dPtr<ID3D11Texture2D>>,
    Option<D3dPtr<ID3D11Texture2D>>,
)> {
    let state = draw.draw_state.as_ref().ok_or_else(|| {
        format!(
            "production native D3D11 terrain replay blocked: draw {} has no captured draw state",
            draw.draw_index
        )
    })?;
    let output_merger = state.output_merger.as_ref().ok_or_else(|| {
        format!(
            "production native D3D11 terrain replay blocked: draw {} has no pre-draw output-merger state",
            draw.draw_index
        )
    })?;
    let target = output_merger.render_targets.first().ok_or_else(|| {
        format!(
            "production native D3D11 terrain replay blocked: draw {} pre-draw output-merger has no render target",
            draw.draw_index
        )
    })?;
    let target_payload = target.pre_draw_payload.as_ref().ok_or_else(|| {
        format!(
            "production native D3D11 terrain replay blocked: draw {} render target slot {:?} has no pre-draw payload",
            draw.draw_index, target.slot
        )
    })?;
    let render_target = Some(unsafe {
        native_d3d11_create_texture_from_output_payload(
            device,
            bundle_dir,
            target,
            replay_target,
            target_payload,
            &format!("pre-draw {} render target", draw.draw_index),
        )
    }?);
    let depth = output_merger.depth_stencil.as_ref().ok_or_else(|| {
        format!(
            "production native D3D11 terrain replay blocked: draw {} pre-draw output-merger has no depth-stencil target",
            draw.draw_index
        )
    })?;
    let depth_payload = depth.pre_draw_payload.as_ref().ok_or_else(|| {
        format!(
            "production native D3D11 terrain replay blocked: draw {} depth-stencil target has no pre-draw payload",
            draw.draw_index
        )
    })?;
    let depth_stencil = Some(unsafe {
        native_d3d11_create_texture_from_output_payload(
            device,
            bundle_dir,
            depth,
            depth,
            depth_payload,
            &format!("pre-draw {} depth-stencil", draw.draw_index),
        )
    }?);
    Ok((render_target, depth_stencil))
}

#[cfg(windows)]
pub(crate) unsafe fn native_d3d11_create_texture_from_output_payload(
    device: &D3dPtr<ID3D11Device>,
    bundle_dir: &Path,
    captured_target: &MapFrameOutputMergerTexture,
    replay_target: &MapFrameOutputMergerTexture,
    payload: &MapFrameOutputMergerTexturePayload,
    label: &str,
) -> Result<D3dPtr<ID3D11Texture2D>> {
    if payload.payload_format != NATIVE_D3D11_TEXTURE_PAYLOAD_FORMAT {
        return Err(format!(
            "production native D3D11 terrain replay blocked: {label} payload_format is {}, expected {NATIVE_D3D11_TEXTURE_PAYLOAD_FORMAT}",
            payload.payload_format
        )
        .into());
    }
    if captured_target.desc.width != replay_target.desc.width
        || captured_target.desc.height != replay_target.desc.height
        || captured_target.desc.format != replay_target.desc.format
    {
        return Err(format!(
            "production native D3D11 terrain replay blocked: {label} {}x{} format {} does not match replay target {}x{} format {}",
            captured_target.desc.width,
            captured_target.desc.height,
            captured_target.desc.format,
            replay_target.desc.width,
            replay_target.desc.height,
            replay_target.desc.format
        )
        .into());
    }
    if captured_target.desc.sample_count != 1 {
        return Err(format!(
            "production native D3D11 terrain replay blocked: {label} uses sample_count {}; pre-draw payload upload currently requires single-sample textures",
            captured_target.desc.sample_count
        )
        .into());
    }
    let raw = read_verified_map_frame_file(
        bundle_dir,
        &payload.file,
        payload.byte_len,
        &payload.fnv1a64,
        label,
    )?;
    let subresource = payload.subresources.first().ok_or_else(|| {
        format!("production native D3D11 terrain replay blocked: {label} has no subresource layout")
    })?;
    if payload.subresources.len() != 1
        || subresource.mip_slice != 0
        || subresource.array_slice != 0
        || subresource.width != captured_target.desc.width
        || subresource.height != captured_target.desc.height
    {
        return Err(format!(
            "production native D3D11 terrain replay blocked: {label} subresource layout does not match the captured Texture2D descriptor"
        )
        .into());
    }
    let end = subresource
        .file_offset
        .checked_add(usize::try_from(subresource.depth_pitch)?)
        .ok_or(
            "production native D3D11 terrain replay blocked: pre-draw payload byte range overflow",
        )?;
    if end > raw.len() {
        return Err(format!(
            "production native D3D11 terrain replay blocked: {label} subresource byte range {}..{} exceeds file length {}",
            subresource.file_offset,
            end,
            raw.len()
        )
        .into());
    }
    let initial = [D3D11_SUBRESOURCE_DATA {
        pSysMem: raw.as_ptr().add(subresource.file_offset) as *const _,
        SysMemPitch: subresource.row_pitch,
        SysMemSlicePitch: subresource.depth_pitch,
    }];
    let source_desc = MapFrameTexture2DDesc {
        usage: 0,
        bind_flags: 0,
        cpu_access_flags: 0,
        misc_flags: 0,
        ..captured_target.desc.clone()
    };
    native_d3d11_create_texture2d_from_desc(device, &source_desc, Some(&initial))
}

#[cfg(windows)]
pub(crate) fn native_d3d11_render_prepared_map_frame(
    device: &NativeD3d11Device,
    prepared: &NativeD3d11PreparedMapFrame,
    clear_targets: bool,
    output: Option<&Path>,
    return_readback: bool,
    compare_each_draw: bool,
    compare_final_reference: bool,
    parity_report_dir: Option<&Path>,
) -> Result<NativeD3d11ReplayResult> {
    native_d3d11_render_prepared_map_pass(
        device,
        &prepared.pass,
        &prepared.render_target,
        &prepared.depth_target,
        None,
        None,
        None,
        clear_targets,
        output,
        return_readback,
        compare_each_draw,
        compare_final_reference,
        parity_report_dir,
    )
}

#[cfg(windows)]
pub(crate) fn native_d3d11_render_prepared_terrain_chunk_synthesis(
    device: &NativeD3d11Device,
    prepared: &NativeD3d11PreparedTerrainChunkSynthesisFrame,
    plan: &TerrainChunkSynthesisPlan,
    clear_targets: bool,
    output: Option<&Path>,
    return_readback: bool,
    cull_world_rect: Option<[f32; 4]>,
) -> Result<NativeD3d11ReplayResult> {
    unsafe {
        let mut rtv_ptr = prepared.render_target.view.as_raw();
        let dsv_ptr = prepared.depth_target.view.as_raw();
        (*device.context.as_raw()).OMSetRenderTargets(1, &mut rtv_ptr, dsv_ptr);
        if clear_targets {
            native_d3d11_clear_targets(
                &device.context,
                &prepared.target_evidence,
                &prepared.depth_evidence,
                &prepared.render_target,
                &prepared.depth_target,
            );
        }

        let tile_span = plan.target_tile_size() as f32;
        let mut drawn = 0usize;
        for tile in plan.tiles() {
            if let Some(rect) = cull_world_rect {
                let tile_min_x = tile.quad_offset_scale_is_detail[0];
                let tile_min_z = tile.quad_offset_scale_is_detail[1];
                if !terrain_chunk_tile_intersects_world_rect(
                    tile_min_x, tile_min_z, tile_span, rect,
                ) {
                    continue;
                }
            }
            drawn += 1;
            native_d3d11_update_terrain_chunk_cb1(
                &device.context,
                &prepared.template_draw,
                tile.quad_offset_scale_is_detail,
                tile.detail_edge_flags,
            )?;
            native_d3d11_replay_prepared_draw(
                &device.context,
                &prepared.template_draw,
                &prepared.render_target,
                &prepared.depth_target,
                None,
                Some(&[(1, tile.slot1_vertex_offset)]),
            )?;
        }

        let readback = if output.is_some() || return_readback {
            Some(native_d3d11_read_render_target(
                device,
                &device.context,
                &prepared.render_target,
            )?)
        } else {
            None
        };
        if let Some(output) = output {
            let readback = readback.as_ref().ok_or(
                "production native D3D11 terrain replay blocked: output requested without readback",
            )?;
            fs::write(output, &readback.bytes)?;
        }
        let summary = NativeD3d11ReplaySummary {
            draw_count: drawn,
            vertex_buffer_count: prepared.template_draw.vertex_buffers.len(),
            index_buffer_count: usize::from(prepared.template_draw.index_buffer.is_some()),
            constant_buffer_count: prepared.template_draw.constant_buffers.len(),
            shader_resource_count: prepared.template_draw.shader_resources.len(),
            sampler_count: prepared.template_draw.samplers.len(),
            target_width: prepared.render_target.desc.width,
            target_height: prepared.render_target.desc.height,
            target_format: prepared.render_target.desc.format,
        };
        Ok(NativeD3d11ReplayResult {
            summary,
            readback: return_readback.then(|| readback).flatten(),
            reference_comparison: None,
        })
    }
}

#[cfg(windows)]
fn terrain_chunk_tile_intersects_world_rect(
    tile_min_x: f32,
    tile_min_z: f32,
    tile_span: f32,
    rect: [f32; 4],
) -> bool {
    const MAP_WIDTH: f32 = 5632.0;
    let [min_x, min_z, max_x, max_z] = rect;
    if tile_min_z + tile_span < min_z || tile_min_z > max_z {
        return false;
    }
    for offset in [-MAP_WIDTH, 0.0, MAP_WIDTH] {
        let x0 = tile_min_x + offset;
        if x0 + tile_span >= min_x && x0 <= max_x {
            return true;
        }
    }
    false
}

#[cfg(windows)]
pub(crate) fn native_d3d11_render_prepared_map_pass(
    device: &NativeD3d11Device,
    prepared: &NativeD3d11PreparedMapPass,
    render_target: &NativeD3d11RenderTarget,
    depth_target: &NativeD3d11DepthStencil,
    app_owned_resources: Option<&NativeD3d11AppOwnedShaderResources<'_>>,
    tree_season_cb: Option<(f32, f32)>,
    map_clock_time: Option<f32>,
    clear_targets: bool,
    output: Option<&Path>,
    return_readback: bool,
    compare_each_draw: bool,
    compare_final_reference: bool,
    parity_report_dir: Option<&Path>,
) -> Result<NativeD3d11ReplayResult> {
    unsafe {
        if let Some(report_dir) = parity_report_dir {
            fs::create_dir_all(report_dir)?;
        }
        let mut rtv_ptr = render_target.view.as_raw();
        let dsv_ptr = depth_target.view.as_raw();
        (*device.context.as_raw()).OMSetRenderTargets(1, &mut rtv_ptr, dsv_ptr);
        if clear_targets {
            native_d3d11_clear_targets(
                &device.context,
                &prepared.target_evidence,
                &prepared.depth_evidence,
                render_target,
                depth_target,
            );
        }

        let mut parity_initial_output = None;
        let mut parity_initial_depth = None;
        let mut parity_pre_draw_outputs = Vec::new();
        let mut parity_pre_draw_depths = Vec::new();
        let mut parity_post_draw_outputs = Vec::new();
        let mut parity_post_draw_depths = Vec::new();
        if let Some(report_dir) = parity_report_dir {
            let initial_readback =
                native_d3d11_read_render_target(device, &device.context, render_target)?;
            parity_initial_output = Some(native_d3d11_write_parity_dump(
                report_dir,
                "initial_output.raw",
                "initial output after captured clears",
                &initial_readback,
            )?);
            let initial_depth =
                native_d3d11_read_depth_stencil(device, &device.context, depth_target)?;
            parity_initial_depth = Some(native_d3d11_write_parity_dump(
                report_dir,
                "initial_depth.raw",
                "initial depth after captured clears",
                &initial_depth,
            )?);
        }

        let draw_count = prepared.draws.len();
        for draw in &prepared.draws {
            native_d3d11_restore_pre_draw_output_targets(
                &device.context,
                draw,
                render_target,
                depth_target,
            );
            if let Some(report_dir) = parity_report_dir {
                let pre_draw_readback =
                    native_d3d11_read_render_target(device, &device.context, render_target)?;
                let pre_draw_file = format!(
                    "pre_draw_{:03}_draw{:03}.raw",
                    draw.draw_index, draw.draw_index
                );
                let pre_draw_dump = native_d3d11_write_parity_dump(
                    report_dir,
                    &pre_draw_file,
                    &format!("pre-draw {} replay output", draw.draw_index),
                    &pre_draw_readback,
                )?;
                let captured_pre_draw_comparison =
                    draw.pre_draw_reference_output.as_ref().map(|reference| {
                        native_d3d11_parity_comparison(
                            &format!("pre-draw {} replay output", draw.draw_index),
                            reference,
                            &pre_draw_readback,
                            Some(pre_draw_dump.file.clone()),
                        )
                    });
                parity_pre_draw_outputs.push(NativeD3d11ParityPreDrawReport {
                    draw_index: draw.draw_index,
                    dump: pre_draw_dump,
                    captured_pre_draw_comparison,
                });
                let pre_draw_depth =
                    native_d3d11_read_depth_stencil(device, &device.context, depth_target)?;
                let pre_draw_depth_file = format!(
                    "pre_draw_depth_{:03}_draw{:03}.raw",
                    draw.draw_index, draw.draw_index
                );
                let pre_draw_depth_dump = native_d3d11_write_parity_dump(
                    report_dir,
                    &pre_draw_depth_file,
                    &format!("pre-draw {} replay depth", draw.draw_index),
                    &pre_draw_depth,
                )?;
                parity_pre_draw_depths.push(NativeD3d11ParityPostDrawReport {
                    draw_index: draw.draw_index,
                    dump: pre_draw_depth_dump,
                    captured_final_comparison: None,
                });
            }
            if let Some(tree_season_cb) = tree_season_cb {
                let updated =
                    native_d3d11_update_tree_season_cb1(&device.context, draw, tree_season_cb)?;
                if prepared.pass_label == "trees" && updated == 0 {
                    return Err(format!(
                        "production native D3D11 tree pass blocked: draw {} has no prepared tree-season CB1 override buffer",
                        draw.draw_index
                    )
                    .into());
                }
            }
            if let Some(map_clock_time) = map_clock_time {
                native_d3d11_update_map_clock_constant_buffers(
                    &device.context,
                    draw,
                    map_clock_time,
                )?;
            }
            native_d3d11_replay_prepared_draw(
                &device.context,
                draw,
                render_target,
                depth_target,
                app_owned_resources,
                None,
            )?;
            if let Some(report_dir) = parity_report_dir {
                let post_draw_readback =
                    native_d3d11_read_render_target(device, &device.context, render_target)?;
                let post_draw_file = format!(
                    "post_draw_{:03}_draw{:03}.raw",
                    draw.draw_index, draw.draw_index
                );
                let post_draw_dump = native_d3d11_write_parity_dump(
                    report_dir,
                    &post_draw_file,
                    &format!("post-draw {} replay output", draw.draw_index),
                    &post_draw_readback,
                )?;
                parity_post_draw_outputs.push(NativeD3d11ParityPostDrawReport {
                    draw_index: draw.draw_index,
                    captured_final_comparison: Some(native_d3d11_parity_comparison(
                        &format!("post-draw {} replay output", draw.draw_index),
                        &prepared.reference_output,
                        &post_draw_readback,
                        Some(post_draw_dump.file.clone()),
                    )),
                    dump: post_draw_dump,
                });
                let post_draw_depth =
                    native_d3d11_read_depth_stencil(device, &device.context, depth_target)?;
                let post_draw_depth_file = format!(
                    "post_draw_depth_{:03}_draw{:03}.raw",
                    draw.draw_index, draw.draw_index
                );
                let post_draw_depth_dump = native_d3d11_write_parity_dump(
                    report_dir,
                    &post_draw_depth_file,
                    &format!("post-draw {} replay depth", draw.draw_index),
                    &post_draw_depth,
                )?;
                parity_post_draw_depths.push(NativeD3d11ParityPostDrawReport {
                    draw_index: draw.draw_index,
                    dump: post_draw_depth_dump,
                    captured_final_comparison: None,
                });
            }
            if compare_each_draw {
                let reference = draw.post_draw_reference_output.as_ref().ok_or_else(|| {
                    format!(
                        "production native D3D11 terrain replay blocked: draw {} has no captured post-draw render target reference",
                        draw.draw_index
                    )
                })?;
                let readback =
                    native_d3d11_read_render_target(device, &device.context, render_target)?;
                native_d3d11_compare_reference_output_with_label(
                    &format!("post-draw {} replay output", draw.draw_index),
                    reference,
                    &readback,
                )?;
            }
        }

        let needs_final_readback = output.is_some()
            || return_readback
            || compare_final_reference
            || parity_report_dir.is_some();
        let readback = if needs_final_readback {
            Some(native_d3d11_read_render_target(
                device,
                &device.context,
                render_target,
            )?)
        } else {
            None
        };
        if let Some(output) = output {
            let readback = readback.as_ref().ok_or(
                "production native D3D11 terrain replay blocked: output requested without readback",
            )?;
            fs::write(output, &readback.bytes)?;
        }
        if let (Some(report_dir), Some(initial_output)) = (parity_report_dir, parity_initial_output)
        {
            let readback = readback
                .as_ref()
                .ok_or("production native D3D11 terrain replay blocked: parity report requested without readback")?;
            let final_dump = native_d3d11_write_parity_dump(
                report_dir,
                "final_output.raw",
                "final replay output",
                readback,
            )?;
            let final_output = native_d3d11_parity_comparison(
                "final replay output",
                &prepared.reference_output,
                readback,
                Some(final_dump.file),
            );
            let first_pre_draw_mismatch_draw_index =
                parity_pre_draw_outputs.iter().find_map(|entry| {
                    match &entry.captured_pre_draw_comparison {
                        Some(comparison) if !comparison.matches => Some(entry.draw_index),
                        _ => None,
                    }
                });
            let first_pre_draw_mismatch_previous_draw_index = first_pre_draw_mismatch_draw_index
                .and_then(|draw_index| {
                    prepared
                        .draws
                        .iter()
                        .position(|draw| draw.draw_index == draw_index)
                        .and_then(|position| position.checked_sub(1))
                        .map(|position| prepared.draws[position].draw_index)
                });
            let report = NativeD3d11ParityReport {
                schema: "battle-plan-terrain-d3d11-parity-report-v1".to_string(),
                pass_label: prepared.pass_label.clone(),
                draw_count,
                target: native_d3d11_parity_output_target_report(&prepared.target_evidence),
                depth: native_d3d11_parity_output_target_report(&prepared.depth_evidence),
                initial_output,
                initial_depth: parity_initial_depth,
                pre_draw_outputs: parity_pre_draw_outputs,
                pre_draw_depths: parity_pre_draw_depths,
                post_draw_outputs: parity_post_draw_outputs,
                post_draw_depths: parity_post_draw_depths,
                final_output,
                first_pre_draw_mismatch_draw_index,
                first_pre_draw_mismatch_previous_draw_index,
                draw_descs: prepared
                    .draws
                    .iter()
                    .map(|draw| draw.parity_desc.clone())
                    .collect(),
            };
            write_json(
                &report_dir.join(format!("{}_d3d11_parity_report.json", prepared.pass_label)),
                &report,
            )?;
        }
        let reference_comparison = if compare_final_reference {
            let readback = readback
                .as_ref()
                .ok_or("production native D3D11 terrain replay blocked: final reference comparison requested without readback")?;
            Some(native_d3d11_compare_reference_output(
                &prepared.reference_output,
                readback,
            )?)
        } else {
            None
        };
        let readback = return_readback.then(|| readback).flatten();

        Ok(NativeD3d11ReplayResult {
            summary: prepared.summary.clone(),
            readback,
            reference_comparison,
        })
    }
}

#[cfg(windows)]
pub(crate) unsafe fn native_d3d11_update_prepared_map_pass_scene(
    context: &D3dPtr<ID3D11DeviceContext>,
    prepared: &NativeD3d11PreparedMapPass,
    camera: ProductionMapCamera,
    day_night_hour: Option<f32>,
    map_clock_time: Option<f32>,
    political: Option<PoliticalSceneCb0>,
    pass_name: &str,
) -> Result<usize> {
    camera.validate()?;
    let mut updated = 0usize;
    for draw in &prepared.draws {
        for constant_buffer in &draw.constant_buffers {
            let Some(camera_patch) = constant_buffer.camera_patch.as_ref() else {
                continue;
            };
            let bytes = terrain_scene_cb0_bytes_with_app_scene_layout(
                &camera_patch.layout,
                camera_patch.base_bytes.clone(),
                camera,
                day_night_hour,
                map_clock_time,
                political,
            )
            .map_err(|err| {
                format!(
                    "production native D3D11 map viewer blocked: failed to update {pass_name} draw {} {TERRAIN_SCENE_CB0_SYMBOL} {} constant buffer slot {}: {err}",
                    camera_patch.draw_index, constant_buffer.stage, constant_buffer.slot
                )
            })?;
            match camera_patch.update_method {
                TerrainSceneCb0CameraUpdateMethod::DefaultUpdateSubresource => {
                    (*context.as_raw()).UpdateSubresource(
                        constant_buffer.buffer.as_raw() as *mut _,
                        0,
                        ptr::null(),
                        bytes.as_ptr() as *const _,
                        0,
                        0,
                    );
                    native_d3d11_check_update_subresource(
                        context,
                        &format!(
                            "UpdateSubresource for {pass_name} draw {} {TERRAIN_SCENE_CB0_SYMBOL} {} constant buffer slot {}",
                            camera_patch.draw_index, constant_buffer.stage, constant_buffer.slot
                        ),
                    )?;
                }
                TerrainSceneCb0CameraUpdateMethod::DynamicMapWriteDiscard => {
                    let mut mapped: D3D11_MAPPED_SUBRESOURCE = mem::zeroed();
                    native_d3d11_hr(
                        (*context.as_raw()).Map(
                            constant_buffer.buffer.as_raw() as *mut _,
                            0,
                            D3D11_MAP_WRITE_DISCARD,
                            0,
                            &mut mapped,
                        ),
                        &format!(
                            "Map dynamic {pass_name} draw {} {TERRAIN_SCENE_CB0_SYMBOL} {} constant buffer slot {} for WRITE_DISCARD",
                            camera_patch.draw_index, constant_buffer.stage, constant_buffer.slot
                        ),
                    )?;
                    if mapped.pData.is_null() {
                        (*context.as_raw()).Unmap(constant_buffer.buffer.as_raw() as *mut _, 0);
                        return Err(format!(
                            "production native D3D11 map viewer blocked: Map(WRITE_DISCARD) for {pass_name} draw {} {TERRAIN_SCENE_CB0_SYMBOL} {} constant buffer slot {} returned null data",
                            camera_patch.draw_index, constant_buffer.stage, constant_buffer.slot
                        )
                        .into());
                    }
                    ptr::copy_nonoverlapping(bytes.as_ptr(), mapped.pData as *mut u8, bytes.len());
                    (*context.as_raw()).Unmap(constant_buffer.buffer.as_raw() as *mut _, 0);
                }
            }
            updated += 1;
        }
    }
    if updated == 0 {
        return Err(format!(
            "production native D3D11 map viewer blocked: prepared {pass_name} pass has no camera-updatable {TERRAIN_SCENE_CB0_SYMBOL} constant buffers"
        )
        .into());
    }
    Ok(updated)
}

#[cfg(windows)]
pub(crate) unsafe fn native_d3d11_update_prepared_draw_scene(
    context: &D3dPtr<ID3D11DeviceContext>,
    draw: &NativeD3d11PreparedDraw,
    camera: ProductionMapCamera,
    day_night_hour: Option<f32>,
    map_clock_time: Option<f32>,
    political: Option<PoliticalSceneCb0>,
    pass_name: &str,
) -> Result<usize> {
    camera.validate()?;
    let mut updated = 0usize;
    for constant_buffer in &draw.constant_buffers {
        let Some(camera_patch) = constant_buffer.camera_patch.as_ref() else {
            continue;
        };
        let bytes = terrain_scene_cb0_bytes_with_app_scene_layout(
            &camera_patch.layout,
            camera_patch.base_bytes.clone(),
            camera,
            day_night_hour,
            map_clock_time,
            political,
        )
        .map_err(|err| {
            format!(
                "production native D3D11 map viewer blocked: failed to update {pass_name} draw {} {TERRAIN_SCENE_CB0_SYMBOL} {} constant buffer slot {}: {err}",
                camera_patch.draw_index, constant_buffer.stage, constant_buffer.slot
            )
        })?;
        match camera_patch.update_method {
            TerrainSceneCb0CameraUpdateMethod::DefaultUpdateSubresource => {
                (*context.as_raw()).UpdateSubresource(
                    constant_buffer.buffer.as_raw() as *mut _,
                    0,
                    ptr::null(),
                    bytes.as_ptr() as *const _,
                    0,
                    0,
                );
                native_d3d11_check_update_subresource(
                    context,
                    &format!(
                        "UpdateSubresource for {pass_name} draw {} {TERRAIN_SCENE_CB0_SYMBOL} {} constant buffer slot {}",
                        camera_patch.draw_index, constant_buffer.stage, constant_buffer.slot
                    ),
                )?;
            }
            TerrainSceneCb0CameraUpdateMethod::DynamicMapWriteDiscard => {
                native_d3d11_write_discard_constant_buffer(
                    context,
                    constant_buffer,
                    &bytes,
                    &format!(
                        "Map dynamic {pass_name} draw {} {TERRAIN_SCENE_CB0_SYMBOL} {} constant buffer slot {} for WRITE_DISCARD",
                        camera_patch.draw_index, constant_buffer.stage, constant_buffer.slot
                    ),
                )?;
            }
        }
        updated += 1;
    }
    if updated == 0 {
        return Err(format!(
            "production native D3D11 map viewer blocked: prepared {pass_name} draw has no camera-updatable {TERRAIN_SCENE_CB0_SYMBOL} constant buffers"
        )
        .into());
    }
    Ok(updated)
}

#[cfg(windows)]
pub(crate) unsafe fn native_d3d11_update_terrain_chunk_cb1(
    context: &D3dPtr<ID3D11DeviceContext>,
    draw: &NativeD3d11PreparedDraw,
    quad_offset_scale_is_detail: [f32; 4],
    detail_edge_flags: [f32; 4],
) -> Result<()> {
    let mut updated = 0;
    for constant_buffer in &draw.constant_buffers {
        let Some(patch) = constant_buffer.terrain_draw_patch.as_ref() else {
            continue;
        };
        let mut bytes = patch.base_bytes.clone();
        write_terrain_chunk_cb1_quad_offset_scale(&mut bytes, quad_offset_scale_is_detail)?;
        write_terrain_chunk_cb1_detail_edge_flags(&mut bytes, detail_edge_flags)?;
        native_d3d11_write_discard_constant_buffer(
            context,
            constant_buffer,
            &bytes,
            &format!(
                "Map dynamic terrain chunk synthesis draw {} {} CB1 QuadOffset_Scale_IsDetail for WRITE_DISCARD",
                patch.draw_index, constant_buffer.stage
            ),
        )?;
        updated += 1;
    }
    if updated == 0 {
        return Err(format!(
            "production native D3D11 terrain chunk synthesis blocked: prepared draw {} has no dynamic CB1 QuadOffset_Scale_IsDetail patch buffers",
            draw.draw_index
        )
        .into());
    }
    Ok(())
}

#[cfg(windows)]
pub(crate) unsafe fn native_d3d11_update_tree_season_cb1(
    context: &D3dPtr<ID3D11DeviceContext>,
    draw: &NativeD3d11PreparedDraw,
    tree_season_cb: (f32, f32),
) -> Result<usize> {
    let mut updated = 0usize;
    for constant_buffer in &draw.constant_buffers {
        let Some(patch) = constant_buffer.tree_season_patch.as_ref() else {
            continue;
        };
        let mut bytes = patch.base_bytes.clone();
        if !apply_tree_season_cb_override(&mut bytes, Some(tree_season_cb)) {
            return Err(format!(
                "production native D3D11 tree pass blocked: draw {} {} CB1 is too small for tree-season bytes 64..71",
                patch.draw_index, constant_buffer.stage
            )
            .into());
        }
        native_d3d11_write_discard_constant_buffer(
            context,
            constant_buffer,
            &bytes,
            &format!(
                "Map dynamic tree draw {} {} CB1 vSeasonLerp/vSeasonColumn for WRITE_DISCARD",
                patch.draw_index, constant_buffer.stage
            ),
        )?;
        updated += 1;
    }
    Ok(updated)
}

#[cfg(windows)]
pub(crate) unsafe fn native_d3d11_update_map_clock_constant_buffers(
    context: &D3dPtr<ID3D11DeviceContext>,
    draw: &NativeD3d11PreparedDraw,
    map_clock_time: f32,
) -> Result<usize> {
    let mut updated = 0usize;
    for constant_buffer in &draw.constant_buffers {
        let Some(patch) = constant_buffer.map_clock_patch.as_ref() else {
            continue;
        };
        let mut bytes = patch.base_bytes.clone();
        apply_map_clock_constant_buffer_override(&mut bytes, patch.kind, map_clock_time)?;
        native_d3d11_write_discard_constant_buffer(
            context,
            constant_buffer,
            &bytes,
            &format!(
                "Map dynamic draw {} {} slot {} map-clock override for WRITE_DISCARD",
                patch.draw_index, constant_buffer.stage, constant_buffer.slot
            ),
        )?;
        updated += 1;
    }
    Ok(updated)
}

#[cfg(windows)]
pub(crate) fn apply_map_clock_constant_buffer_override(
    bytes: &mut [u8],
    kind: NativeD3d11MapClockConstantBufferKind,
    map_clock_time: f32,
) -> Result<()> {
    if !map_clock_time.is_finite() || map_clock_time < 0.0 {
        return Err(format!(
            "production native D3D11 map viewer blocked: map clock time must be finite non-negative seconds, got {map_clock_time}"
        )
        .into());
    }
    let offset = match kind {
        NativeD3d11MapClockConstantBufferKind::WaterTimeHalfPixel => 0,
        NativeD3d11MapClockConstantBufferKind::RiverTimeDirectionSeasonLerp => 64,
    };
    write_f32_at(bytes, offset, map_clock_time)
}

#[cfg(windows)]
fn native_d3d11_map_clock_patch_for_binding(
    pass_label: &str,
    draw_index: u32,
    binding: &MapFrameBufferBinding,
    base_bytes: &[u8],
) -> Result<Option<NativeD3d11PreparedMapClockConstantBuffer>> {
    if pass_label == "water" && binding.slot == 3 && base_bytes.len() >= 12 {
        for offset in [0usize, 4, 8] {
            read_f32_at(base_bytes, offset)?;
        }
        return Ok(Some(NativeD3d11PreparedMapClockConstantBuffer {
            draw_index,
            base_bytes: base_bytes.to_vec(),
            kind: NativeD3d11MapClockConstantBufferKind::WaterTimeHalfPixel,
        }));
    }
    if pass_label == "rivers"
        && binding.slot == TERRAIN_DRAW_CB1_BIND_POINT
        && native_d3d11_looks_like_river_time_direction_season_lerp(base_bytes)?
    {
        return Ok(Some(NativeD3d11PreparedMapClockConstantBuffer {
            draw_index,
            base_bytes: base_bytes.to_vec(),
            kind: NativeD3d11MapClockConstantBufferKind::RiverTimeDirectionSeasonLerp,
        }));
    }
    Ok(None)
}

#[cfg(windows)]
fn native_d3d11_looks_like_river_time_direction_season_lerp(bytes: &[u8]) -> Result<bool> {
    if bytes.len() < 76 {
        return Ok(false);
    }
    let time = read_f32_at(bytes, 64)?;
    let direction = read_f32_at(bytes, 68)?;
    let season_lerp = read_f32_at(bytes, 72)?;
    Ok(time.is_finite()
        && direction.is_finite()
        && season_lerp.is_finite()
        && (direction.abs() - 1.0).abs() <= 0.001
        && (0.0..=1.0).contains(&season_lerp))
}

#[cfg(windows)]
unsafe fn native_d3d11_write_discard_constant_buffer(
    context: &D3dPtr<ID3D11DeviceContext>,
    constant_buffer: &NativeD3d11PreparedConstantBuffer,
    bytes: &[u8],
    label: &str,
) -> Result<()> {
    let mut mapped: D3D11_MAPPED_SUBRESOURCE = mem::zeroed();
    native_d3d11_hr(
        (*context.as_raw()).Map(
            constant_buffer.buffer.as_raw() as *mut _,
            0,
            D3D11_MAP_WRITE_DISCARD,
            0,
            &mut mapped,
        ),
        label,
    )?;
    if mapped.pData.is_null() {
        (*context.as_raw()).Unmap(constant_buffer.buffer.as_raw() as *mut _, 0);
        return Err(format!(
            "production native D3D11 map viewer blocked: {label} returned null data"
        )
        .into());
    }
    ptr::copy_nonoverlapping(bytes.as_ptr(), mapped.pData as *mut u8, bytes.len());
    (*context.as_raw()).Unmap(constant_buffer.buffer.as_raw() as *mut _, 0);
    Ok(())
}

#[cfg(windows)]
pub(crate) unsafe fn native_d3d11_restore_pre_draw_output_targets(
    context: &D3dPtr<ID3D11DeviceContext>,
    draw: &NativeD3d11PreparedDraw,
    render_target: &NativeD3d11RenderTarget,
    depth_target: &NativeD3d11DepthStencil,
) {
    if let Some(source) = &draw.pre_draw_render_target {
        (*context.as_raw()).CopyResource(
            render_target.texture.as_raw() as *mut _,
            source.as_raw() as *mut _,
        );
    }
    if let Some(source) = &draw.pre_draw_depth_stencil {
        (*context.as_raw()).CopyResource(
            depth_target.texture.as_raw() as *mut _,
            source.as_raw() as *mut _,
        );
    }
}

#[cfg(windows)]
pub(crate) unsafe fn native_d3d11_prepare_draw(
    device: &NativeD3d11Device,
    bundle_dir: &Path,
    synthetic_payloads: &BTreeMap<String, MapFrameSyntheticPayload>,
    draw: &MapFrameDraw,
    target: &MapFrameOutputMergerTexture,
    app_camera: Option<ProductionMapCamera>,
    restore_pre_draw_outputs: bool,
    water_refraction_source: NativeD3d11WaterRefractionSource,
    reflection_cube_source: NativeD3d11ReflectionCubeSource,
    reflection_resolver: Option<&GameFileResolver>,
    pass_label: &str,
    patch_terrain_draw_cb1: bool,
    scene_cb0_override: Option<&[u8]>,
    patch_tree_season_cb1: bool,
) -> Result<NativeD3d11PreparedDraw> {
    let vertex_shader = draw.vertex_shader.as_ref().ok_or_else(|| {
        format!(
            "production native D3D11 terrain replay blocked: draw {} missing VS bytecode",
            draw.draw_index
        )
    })?;
    let vs_bytes = native_d3d11_read_shader_bytecode(bundle_dir, vertex_shader)?;
    let ps_bytes = native_d3d11_read_shader_bytecode(bundle_dir, &draw.shader)?;

    let mut vs: *mut ID3D11VertexShader = ptr::null_mut();
    native_d3d11_hr(
        (*device.device.as_raw()).CreateVertexShader(
            vs_bytes.as_ptr() as *const _,
            vs_bytes.len(),
            ptr::null_mut(),
            &mut vs,
        ),
        "CreateVertexShader from captured the game bytecode",
    )?;

    let mut ps: *mut ID3D11PixelShader = ptr::null_mut();
    native_d3d11_hr(
        (*device.device.as_raw()).CreatePixelShader(
            ps_bytes.as_ptr() as *const _,
            ps_bytes.len(),
            ptr::null_mut(),
            &mut ps,
        ),
        "CreatePixelShader from captured the game bytecode",
    )?;

    let input_layout = native_d3d11_create_input_layout(&device.device, draw, &vs_bytes)?;
    let vertex_buffers =
        native_d3d11_prepare_vertex_buffers(&device.device, bundle_dir, synthetic_payloads, draw)?;
    let index_buffer =
        native_d3d11_prepare_index_buffer(&device.device, bundle_dir, synthetic_payloads, draw)?;
    let constant_buffers = native_d3d11_prepare_constant_buffers(
        &device.device,
        bundle_dir,
        synthetic_payloads,
        draw,
        pass_label,
        app_camera,
        patch_terrain_draw_cb1,
        scene_cb0_override,
        patch_tree_season_cb1,
    )?;
    let (_textures, shader_resources) = native_d3d11_prepare_shader_resources(
        &device.device,
        bundle_dir,
        synthetic_payloads,
        draw,
        water_refraction_source,
        reflection_cube_source,
        reflection_resolver,
    )?;
    let samplers = native_d3d11_prepare_samplers(&device.device, draw)?;
    let rasterizer = native_d3d11_prepare_rasterizer_state(&device.device, draw)?;
    let (blend, blend_factor, sample_mask) =
        native_d3d11_prepare_blend_state(&device.device, draw)?;
    let (depth_stencil, stencil_ref) =
        native_d3d11_prepare_depth_stencil_state(&device.device, draw)?;
    let (viewports, scissor_rects) = native_d3d11_prepare_viewport_scissor(draw)?;
    let (pre_draw_render_target, pre_draw_depth_stencil) = if restore_pre_draw_outputs {
        native_d3d11_prepare_pre_draw_output_targets(&device.device, bundle_dir, draw, target)?
    } else {
        (None, None)
    };
    let pre_draw_reference_output = native_d3d11_load_pre_draw_reference_output(
        draw,
        bundle_dir,
        target,
        &format!("pre-draw {} {pass_label} render target", draw.draw_index),
    )?;
    let post_draw_reference_output = draw
        .post_draw_output_merger
        .as_ref()
        .map(|output_merger| {
            native_d3d11_load_output_merger_reference_output(
                output_merger,
                bundle_dir,
                target,
                &format!("post-draw {} {pass_label} render target", draw.draw_index),
            )
        })
        .transpose()?;
    let parity_desc = native_d3d11_parity_draw_desc_report(draw)?;

    Ok(NativeD3d11PreparedDraw {
        draw_index: draw.draw_index,
        draw_call: draw.draw_call.clone(),
        draw_count: draw.draw_count,
        draw_start: draw.draw_start,
        base_vertex_location: draw.base_vertex_location,
        per_instance_count: draw.per_instance_count,
        instance_count: draw.instance_count,
        start_instance_location: draw.start_instance_location,
        topology: draw.topology.ok_or_else(|| {
            format!(
                "production native D3D11 terrain replay blocked: draw {} has no captured IA primitive topology",
                draw.draw_index
            )
        })?,
        vertex_shader: D3dPtr::from_raw(vs, "captured terrain vertex shader")?,
        pixel_shader: D3dPtr::from_raw(ps, "captured pdxmap pixel shader")?,
        input_layout,
        vertex_buffers,
        index_buffer,
        constant_buffers,
        _textures,
        shader_resources,
        samplers,
        rasterizer,
        blend,
        blend_factor,
        sample_mask,
        depth_stencil,
        stencil_ref,
        viewports,
        scissor_rects,
        pre_draw_render_target,
        pre_draw_depth_stencil,
        pre_draw_reference_output,
        post_draw_reference_output,
        parity_desc,
    })
}

#[cfg(windows)]
pub(crate) fn native_d3d11_load_pre_draw_reference_output(
    draw: &MapFrameDraw,
    bundle_dir: &Path,
    replay_target: &MapFrameOutputMergerTexture,
    label: &str,
) -> Result<Option<NativeD3d11ReferenceOutput>> {
    let Some(state) = draw.draw_state.as_ref() else {
        return Ok(None);
    };
    let Some(output_merger) = state.output_merger.as_ref() else {
        return Ok(None);
    };
    let Some(reference_target) = output_merger.render_targets.first() else {
        return Ok(None);
    };
    let Some(payload) = reference_target.pre_draw_payload.as_ref() else {
        return Ok(None);
    };
    native_d3d11_load_output_merger_texture_reference_output(
        reference_target,
        payload,
        bundle_dir,
        replay_target,
        label,
    )
    .map(Some)
}

#[cfg(windows)]
pub(crate) fn native_d3d11_parity_draw_desc_report(
    draw: &MapFrameDraw,
) -> Result<NativeD3d11ParityDrawDescReport> {
    let state = draw.draw_state.as_ref().ok_or_else(|| {
        format!(
            "production native D3D11 terrain replay blocked: draw {} has no captured state for parity report",
            draw.draw_index
        )
    })?;
    let output_merger = state.output_merger.as_ref();
    let render_target = output_merger
        .and_then(|om| om.render_targets.first())
        .map(native_d3d11_parity_output_target_report);
    let depth_target = output_merger
        .and_then(|om| om.depth_stencil.as_ref())
        .map(native_d3d11_parity_output_target_report);
    let shader_resources = draw
        .shader_resources
        .iter()
        .map(native_d3d11_parity_shader_resource_report)
        .collect::<Vec<_>>();
    let samplers = draw
        .samplers
        .iter()
        .cloned()
        .map(|sampler| NativeD3d11ParitySamplerReport {
            stage: sampler.stage.clone(),
            slot: sampler.slot,
            captured_desc: sampler.clone(),
            replay_desc: sampler,
        })
        .collect::<Vec<_>>();
    Ok(NativeD3d11ParityDrawDescReport {
        draw_index: draw.draw_index,
        draw_call: draw.draw_call.clone(),
        draw_count: draw.draw_count,
        draw_start: draw.draw_start,
        base_vertex_location: draw.base_vertex_location,
        per_instance_count: draw.per_instance_count,
        instance_count: draw.instance_count,
        start_instance_location: draw.start_instance_location,
        topology: draw.topology.unwrap_or(0),
        vertex_shader: draw
            .vertex_shader
            .as_ref()
            .map(native_d3d11_parity_shader_report),
        pixel_shader: native_d3d11_parity_shader_report(&draw.shader),
        input_layout: draw.input_layout.clone(),
        vertex_buffers: draw
            .vertex_buffers
            .values()
            .map(native_d3d11_parity_buffer_report)
            .collect(),
        index_buffer: draw
            .index_buffer
            .as_ref()
            .map(native_d3d11_parity_index_buffer_report),
        constant_buffers: draw
            .constant_buffers
            .iter()
            .map(native_d3d11_parity_buffer_report)
            .collect(),
        shader_resources,
        samplers,
        rasterizer: state.rasterizer.clone(),
        blend: state.blend.clone(),
        depth_stencil: state.depth_stencil.clone(),
        render_target,
        depth_target,
        viewport_count: state.viewports.len(),
        scissor_rect_count: state.scissor_rects.len(),
    })
}

pub(crate) fn native_d3d11_parity_shader_report(
    shader: &MapFrameShader,
) -> NativeD3d11ParityShaderReport {
    NativeD3d11ParityShaderReport {
        stage: shader.stage.clone(),
        bytecode_file: shader.bytecode_file.clone(),
        byte_len: shader.byte_len,
        fnv1a64: shader.fnv1a64.clone(),
    }
}

pub(crate) fn native_d3d11_parity_buffer_report(
    binding: &MapFrameBufferBinding,
) -> NativeD3d11ParityBufferReport {
    let desc = NativeD3d11ParityBufferDesc {
        byte_width: binding.byte_len,
        usage: binding.usage,
        bind_flags: binding.bind_flags,
        cpu_access_flags: binding.cpu_access_flags,
        misc_flags: 0,
        structure_byte_stride: 0,
    };
    NativeD3d11ParityBufferReport {
        stage: binding.stage.clone(),
        slot: binding.slot,
        declared_name: binding.declared_name.clone(),
        file: binding.file.clone(),
        fnv1a64: binding.fnv1a64.clone(),
        stride: binding.stride,
        offset: binding.offset,
        dxgi_format: binding.dxgi_format,
        captured_desc: desc.clone(),
        replay_desc: desc,
    }
}

pub(crate) fn native_d3d11_parity_index_buffer_report(
    binding: &MapFrameIndexBufferBinding,
) -> NativeD3d11ParityIndexBufferReport {
    let desc = NativeD3d11ParityBufferDesc {
        byte_width: binding.byte_len,
        usage: binding.usage,
        bind_flags: binding.bind_flags,
        cpu_access_flags: binding.cpu_access_flags,
        misc_flags: 0,
        structure_byte_stride: 0,
    };
    NativeD3d11ParityIndexBufferReport {
        file: binding.file.clone(),
        fnv1a64: binding.fnv1a64.clone(),
        offset: binding.offset,
        dxgi_format: binding.dxgi_format,
        captured_desc: desc.clone(),
        replay_desc: desc,
    }
}

pub(crate) fn native_d3d11_parity_output_target_report(
    target: &MapFrameOutputMergerTexture,
) -> NativeD3d11ParityOutputTargetReport {
    NativeD3d11ParityOutputTargetReport {
        captured_texture_desc: target.desc.clone(),
        captured_view_desc: target.view_desc.clone(),
        replay_texture_desc: target.desc.clone(),
        replay_view_desc: target.view_desc.clone(),
    }
}

pub(crate) fn native_d3d11_parity_shader_resource_report(
    resource: &MapFrameTextureBinding,
) -> NativeD3d11ParityShaderResourceReport {
    let desc = MapFrameTexture2DDesc {
        width: resource.width,
        height: resource.height,
        mip_levels: resource.mip_count,
        array_size: resource.array_size,
        format: resource.dxgi_format,
        sample_count: resource.sample_count,
        sample_quality: resource.sample_quality,
        usage: resource.usage,
        bind_flags: resource.bind_flags,
        cpu_access_flags: resource.cpu_access_flags,
        misc_flags: resource.misc_flags,
    };
    NativeD3d11ParityShaderResourceReport {
        stage: resource.stage.clone(),
        slot: resource.slot,
        declared_name: resource.declared_name.clone(),
        is_null: resource.is_null,
        file: resource.file.clone(),
        fnv1a64: resource.fnv1a64.clone(),
        captured_resource_desc: desc.clone(),
        captured_view_desc: resource.view_desc.clone(),
        replay_texture_desc: desc,
        replay_view_desc: resource.view_desc.clone(),
    }
}

#[cfg(windows)]
pub(crate) unsafe fn native_d3d11_replay_prepared_draw(
    context: &D3dPtr<ID3D11DeviceContext>,
    draw: &NativeD3d11PreparedDraw,
    render_target: &NativeD3d11RenderTarget,
    depth_target: &NativeD3d11DepthStencil,
    app_owned_resources: Option<&NativeD3d11AppOwnedShaderResources<'_>>,
    vertex_offset_overrides: Option<&[(u32, u32)]>,
) -> Result<()> {
    (*context.as_raw()).VSSetShader(draw.vertex_shader.as_raw(), ptr::null_mut(), 0);
    (*context.as_raw()).PSSetShader(draw.pixel_shader.as_raw(), ptr::null_mut(), 0);
    (*context.as_raw()).IASetInputLayout(draw.input_layout.as_raw());
    (*context.as_raw()).IASetPrimitiveTopology(draw.topology as _);

    for vertex_buffer in &draw.vertex_buffers {
        let mut raw = vertex_buffer.buffer.as_raw();
        let mut stride = vertex_buffer.stride;
        let mut offset = vertex_offset_overrides
            .and_then(|overrides| {
                overrides
                    .iter()
                    .find(|(slot, _)| *slot == vertex_buffer.slot)
                    .map(|(_, offset)| *offset)
            })
            .unwrap_or(vertex_buffer.offset);
        (*context.as_raw()).IASetVertexBuffers(
            vertex_buffer.slot,
            1,
            &mut raw,
            &mut stride,
            &mut offset,
        );
    }
    if let Some(index_buffer) = &draw.index_buffer {
        (*context.as_raw()).IASetIndexBuffer(
            index_buffer.buffer.as_raw(),
            index_buffer.dxgi_format as _,
            index_buffer.offset,
        );
    }
    for constant_buffer in &draw.constant_buffers {
        let mut raw = constant_buffer.buffer.as_raw();
        match constant_buffer.stage.as_str() {
            "VS" => (*context.as_raw()).VSSetConstantBuffers(constant_buffer.slot, 1, &mut raw),
            "PS" => (*context.as_raw()).PSSetConstantBuffers(constant_buffer.slot, 1, &mut raw),
            stage => {
                return Err(format!(
                    "production native D3D11 terrain replay blocked: draw {} constant buffer stage {stage} is unsupported",
                    draw.draw_index
                )
                .into())
            }
        }
    }
    for resource in &draw.shader_resources {
        let mut raw = match resource.source {
            NativeD3d11ShaderResourceSource::CapturedBundleTexture => resource
                .view
                .as_ref()
                .map(|view| view.as_raw())
                .unwrap_or(ptr::null_mut()),
            NativeD3d11ShaderResourceSource::AppOwnedTerrainColorSnapshot => {
                let resources = app_owned_resources.ok_or_else(|| {
                    format!(
                        "production native D3D11 map graph blocked: draw {} needs app-owned terrain color for PS SRV slot {}, but no graph resources were provided",
                        draw.draw_index, resource.slot
                    )
                })?;
                resources
                    .terrain_color_snapshot
                    .ok_or_else(|| {
                        format!(
                            "production native D3D11 map graph blocked: draw {} needs app-owned terrain color for PS SRV slot {}, but terrain color snapshot was not created",
                            draw.draw_index, resource.slot
                        )
                    })?
                    .as_raw()
            }
            NativeD3d11ShaderResourceSource::UnderwaterPass => {
                let resources = app_owned_resources.ok_or_else(|| {
                    format!(
                        "--water-full underwater pass blocked: draw {} needs the underwater WaterRefraction SRV at PS slot {}, but no graph resources were provided",
                        draw.draw_index, resource.slot
                    )
                })?;
                resources
                    .underwater_refraction
                    .ok_or_else(|| {
                        format!(
                            "--water-full underwater pass blocked: draw {} needs the underwater WaterRefraction SRV at PS slot {}, but the underwater pass was not rendered",
                            draw.draw_index, resource.slot
                        )
                    })?
                    .as_raw()
            }
        };
        match resource.stage.as_str() {
            "PS" => (*context.as_raw()).PSSetShaderResources(resource.slot, 1, &mut raw),
            stage => {
                return Err(format!(
                    "production native D3D11 terrain replay blocked: draw {} shader resource stage {stage} is unsupported",
                    draw.draw_index
                )
                .into())
            }
        }
    }
    for sampler in &draw.samplers {
        let mut raw = sampler.state.as_raw();
        match sampler.stage.as_str() {
            "PS" => (*context.as_raw()).PSSetSamplers(sampler.slot, 1, &mut raw),
            stage => {
                return Err(format!(
                    "production native D3D11 terrain replay blocked: draw {} sampler stage {stage} is unsupported",
                    draw.draw_index
                )
                .into())
            }
        }
    }
    (*context.as_raw()).RSSetState(
        draw.rasterizer
            .as_ref()
            .map(|state| state.as_raw())
            .unwrap_or(ptr::null_mut()),
    );
    (*context.as_raw()).OMSetBlendState(
        draw.blend
            .as_ref()
            .map(|state| state.as_raw())
            .unwrap_or(ptr::null_mut()),
        &draw.blend_factor,
        draw.sample_mask,
    );
    (*context.as_raw()).OMSetDepthStencilState(
        draw.depth_stencil
            .as_ref()
            .map(|state| state.as_raw())
            .unwrap_or(ptr::null_mut()),
        draw.stencil_ref,
    );
    (*context.as_raw()).RSSetViewports(draw.viewports.len() as UINT, draw.viewports.as_ptr());
    if !draw.scissor_rects.is_empty() {
        (*context.as_raw()).RSSetScissorRects(
            draw.scissor_rects.len() as UINT,
            draw.scissor_rects.as_ptr(),
        );
    } else {
        (*context.as_raw()).RSSetScissorRects(0, ptr::null());
    }

    let mut rtv_ptr = render_target.view.as_raw();
    let dsv_ptr = depth_target.view.as_raw();
    (*context.as_raw()).OMSetRenderTargets(1, &mut rtv_ptr, dsv_ptr);

    match draw.draw_call.as_str() {
        "DrawIndexed" => (*context.as_raw()).DrawIndexed(
            draw.draw_count,
            draw.draw_start,
            draw.base_vertex_location,
        ),
        "Draw" => (*context.as_raw()).Draw(draw.draw_count, draw.draw_start),
        "DrawIndexedInstanced" => (*context.as_raw()).DrawIndexedInstanced(
            draw.per_instance_count.ok_or_else(|| {
                format!(
                    "production native D3D11 terrain replay blocked: draw {} is DrawIndexedInstanced but has no captured per-instance index count",
                    draw.draw_index
                )
            })?,
            draw.instance_count.ok_or_else(|| {
                format!(
                    "production native D3D11 terrain replay blocked: draw {} is DrawIndexedInstanced but has no captured instance count",
                    draw.draw_index
                )
            })?,
            draw.draw_start,
            draw.base_vertex_location,
            draw.start_instance_location.ok_or_else(|| {
                format!(
                    "production native D3D11 terrain replay blocked: draw {} is DrawIndexedInstanced but has no captured start instance location",
                    draw.draw_index
                )
            })?,
        ),
        "DrawInstanced" => (*context.as_raw()).DrawInstanced(
            draw.per_instance_count.ok_or_else(|| {
                format!(
                    "production native D3D11 terrain replay blocked: draw {} is DrawInstanced but has no captured per-instance vertex count",
                    draw.draw_index
                )
            })?,
            draw.instance_count.ok_or_else(|| {
                format!(
                    "production native D3D11 terrain replay blocked: draw {} is DrawInstanced but has no captured instance count",
                    draw.draw_index
                )
            })?,
            draw.draw_start,
            draw.start_instance_location.ok_or_else(|| {
                format!(
                    "production native D3D11 terrain replay blocked: draw {} is DrawInstanced but has no captured start instance location",
                    draw.draw_index
                )
            })?,
        ),
        other => {
            return Err(format!(
                "production native D3D11 terrain replay blocked: draw {} uses unsupported draw call {other}",
                draw.draw_index
            )
            .into())
        }
    }
    Ok(())
}

#[cfg(windows)]
pub(crate) unsafe fn native_d3d11_prepare_vertex_buffers(
    device: &D3dPtr<ID3D11Device>,
    bundle_dir: &Path,
    synthetic_payloads: &BTreeMap<String, MapFrameSyntheticPayload>,
    draw: &MapFrameDraw,
) -> Result<Vec<NativeD3d11PreparedVertexBuffer>> {
    let mut buffers = Vec::new();
    for binding in draw.vertex_buffers.values() {
        let mut bytes = native_d3d11_read_buffer_file_or_payload(
            bundle_dir,
            synthetic_payloads,
            binding,
            "vertex buffer",
        )?;
        const VERTEX_FETCH_PAD_BYTES: usize = 16;
        bytes.extend_from_slice(&[0u8; VERTEX_FETCH_PAD_BYTES]);
        let buffer = native_d3d11_create_buffer(device, binding, &bytes, "vertex buffer")?;
        let stride = binding.stride.ok_or_else(|| {
            format!(
                "production native D3D11 terrain replay blocked: draw {} vertex buffer slot {} has no stride",
                draw.draw_index, binding.slot
            )
        })?;
        buffers.push(NativeD3d11PreparedVertexBuffer {
            slot: binding.slot,
            buffer,
            stride,
            offset: binding.offset.unwrap_or(0),
        });
    }
    Ok(buffers)
}

#[cfg(windows)]
pub(crate) unsafe fn native_d3d11_prepare_index_buffer(
    device: &D3dPtr<ID3D11Device>,
    bundle_dir: &Path,
    synthetic_payloads: &BTreeMap<String, MapFrameSyntheticPayload>,
    draw: &MapFrameDraw,
) -> Result<Option<NativeD3d11PreparedIndexBuffer>> {
    let Some(binding) = draw.index_buffer.as_ref() else {
        return Ok(None);
    };
    let file = binding.file.as_ref().ok_or_else(|| {
        format!(
            "production native D3D11 terrain replay blocked: draw {} index buffer has no raw file",
            draw.draw_index
        )
    })?;
    let hash = binding.fnv1a64.as_ref().ok_or_else(|| {
        format!(
            "production native D3D11 terrain replay blocked: draw {} index buffer has no hash",
            draw.draw_index
        )
    })?;
    let bytes = read_map_frame_resource_payload(
        bundle_dir,
        synthetic_payloads,
        file,
        binding.byte_len,
        hash,
        "index buffer",
    )?;
    let desc = MapFrameBufferBinding {
        stage: "IA".to_string(),
        slot: 0,
        declared_name: None,
        file: binding.file.clone(),
        byte_len: binding.byte_len,
        fnv1a64: binding.fnv1a64.clone(),
        stride: None,
        offset: Some(binding.offset),
        dxgi_format: Some(binding.dxgi_format),
        bind_flags: binding.bind_flags,
        usage: binding.usage,
        cpu_access_flags: binding.cpu_access_flags,
    };
    let buffer = native_d3d11_create_buffer(device, &desc, &bytes, "index buffer")?;
    Ok(Some(NativeD3d11PreparedIndexBuffer {
        buffer,
        dxgi_format: binding.dxgi_format,
        offset: binding.offset,
    }))
}

#[cfg(windows)]
pub(crate) unsafe fn native_d3d11_create_generated_vertex_buffer(
    device: &D3dPtr<ID3D11Device>,
    slot: u32,
    stride: u32,
    bytes: &[u8],
    role: &str,
) -> Result<NativeD3d11PreparedVertexBuffer> {
    let binding = MapFrameBufferBinding {
        stage: "IA".to_string(),
        slot,
        declared_name: None,
        file: None,
        byte_len: bytes.len(),
        fnv1a64: None,
        stride: Some(stride),
        offset: Some(0),
        dxgi_format: None,
        bind_flags: D3D11_BIND_VERTEX_BUFFER,
        usage: D3D11_USAGE_DEFAULT,
        cpu_access_flags: 0,
    };
    let buffer = native_d3d11_create_buffer(device, &binding, bytes, role)?;
    Ok(NativeD3d11PreparedVertexBuffer {
        slot,
        buffer,
        stride,
        offset: 0,
    })
}

#[cfg(windows)]
pub(crate) unsafe fn native_d3d11_create_generated_index_buffer_u32(
    device: &D3dPtr<ID3D11Device>,
    indices: &[u32],
    role: &str,
) -> Result<NativeD3d11PreparedIndexBuffer> {
    let mut bytes = Vec::with_capacity(indices.len() * 4);
    for index in indices {
        bytes.extend_from_slice(&index.to_le_bytes());
    }
    let binding = MapFrameBufferBinding {
        stage: "IA".to_string(),
        slot: 0,
        declared_name: None,
        file: None,
        byte_len: bytes.len(),
        fnv1a64: None,
        stride: None,
        offset: Some(0),
        dxgi_format: Some(DXGI_FORMAT_R32_UINT),
        bind_flags: D3D11_BIND_INDEX_BUFFER,
        usage: D3D11_USAGE_DEFAULT,
        cpu_access_flags: 0,
    };
    let buffer = native_d3d11_create_buffer(device, &binding, &bytes, role)?;
    Ok(NativeD3d11PreparedIndexBuffer {
        buffer,
        dxgi_format: DXGI_FORMAT_R32_UINT,
        offset: 0,
    })
}

#[cfg(windows)]
pub(crate) unsafe fn native_d3d11_prepare_constant_buffers(
    device: &D3dPtr<ID3D11Device>,
    bundle_dir: &Path,
    synthetic_payloads: &BTreeMap<String, MapFrameSyntheticPayload>,
    draw: &MapFrameDraw,
    pass_label: &str,
    app_camera: Option<ProductionMapCamera>,
    patch_terrain_draw_cb1: bool,
    scene_cb0_override: Option<&[u8]>,
    patch_tree_season_cb1: bool,
) -> Result<Vec<NativeD3d11PreparedConstantBuffer>> {
    let mut buffers = Vec::new();
    for binding in &draw.constant_buffers {
        let captured_bytes = native_d3d11_read_buffer_file_or_payload(
            bundle_dir,
            synthetic_payloads,
            binding,
            "constant buffer",
        )?;
        let base_bytes = match scene_cb0_override {
            Some(scene) if binding.slot == 0 => {
                if scene.len() != captured_bytes.len() {
                    return Err(format!(
                        "production native D3D11 map viewer blocked: scene CB0 override is {} bytes but draw {} {} slot 0 constant buffer is {} bytes",
                        scene.len(),
                        draw.draw_index,
                        binding.stage,
                        captured_bytes.len()
                    )
                    .into());
                }
                scene.to_vec()
            }
            _ => captured_bytes,
        };
        let terrain_draw_patch =
            if patch_terrain_draw_cb1 && binding.slot == TERRAIN_DRAW_CB1_BIND_POINT {
                terrain_chunk_cb1_quad_offset_scale_from_bytes(&base_bytes)?;
                Some(NativeD3d11PreparedTerrainDrawConstantBuffer {
                    draw_index: draw.draw_index,
                    base_bytes: base_bytes.clone(),
                })
            } else {
                None
            };
        let tree_season_patch = if patch_tree_season_cb1
            && binding.slot == TERRAIN_DRAW_CB1_BIND_POINT
        {
            if base_bytes.len() < 72 {
                return Err(format!(
                        "production native D3D11 tree pass blocked: draw {} {} CB1 has {} bytes, expected at least 72 for vSeasonLerp/vSeasonColumn",
                        draw.draw_index,
                        binding.stage,
                        base_bytes.len()
                    )
                    .into());
            }
            Some(NativeD3d11PreparedTreeSeasonConstantBuffer {
                draw_index: draw.draw_index,
                base_bytes: base_bytes.clone(),
            })
        } else {
            None
        };
        let map_clock_patch = native_d3d11_map_clock_patch_for_binding(
            pass_label,
            draw.draw_index,
            binding,
            &base_bytes,
        )?;
        let layout = terrain_scene_cb0_camera_layout_for_binding(
            bundle_dir,
            draw,
            binding,
            base_bytes.len(),
        )?;
        let camera_patch = layout
            .map(|layout| {
                let update_method = camera_constant_buffer_update_method(
                    binding,
                    base_bytes.len(),
                    draw.draw_index,
                )?;
                Ok::<_, Box<dyn std::error::Error>>(NativeD3d11PreparedCameraConstantBuffer {
                    draw_index: draw.draw_index,
                    base_bytes: base_bytes.clone(),
                    layout,
                    update_method,
                })
            })
            .transpose()?;
        let bytes = if let (Some(camera), Some(camera_patch)) = (app_camera, camera_patch.as_ref())
        {
            terrain_scene_cb0_bytes_with_app_camera_layout(
                &camera_patch.layout,
                base_bytes.clone(),
                camera,
            )?
        } else {
            base_bytes
        };
        let buffer = if let Some(camera_patch) = camera_patch.as_ref() {
            native_d3d11_create_camera_constant_buffer(
                device,
                binding,
                &bytes,
                camera_patch.update_method,
            )?
        } else if terrain_draw_patch.is_some() {
            native_d3d11_create_dynamic_constant_buffer(
                device,
                binding,
                &bytes,
                "terrain chunk synthesis CB1 constant buffer",
            )?
        } else if tree_season_patch.is_some() {
            native_d3d11_create_dynamic_constant_buffer(
                device,
                binding,
                &bytes,
                "tree-season override CB1 constant buffer",
            )?
        } else if map_clock_patch.is_some() {
            native_d3d11_create_dynamic_constant_buffer(
                device,
                binding,
                &bytes,
                "map-clock override constant buffer",
            )?
        } else {
            native_d3d11_create_buffer(device, binding, &bytes, "constant buffer")?
        };
        buffers.push(NativeD3d11PreparedConstantBuffer {
            stage: binding.stage.clone(),
            slot: binding.slot,
            buffer,
            camera_patch,
            terrain_draw_patch,
            tree_season_patch,
            map_clock_patch,
        });
    }
    Ok(buffers)
}

#[cfg(windows)]
pub(crate) unsafe fn native_d3d11_prepare_shader_resources(
    device: &D3dPtr<ID3D11Device>,
    bundle_dir: &Path,
    synthetic_payloads: &BTreeMap<String, MapFrameSyntheticPayload>,
    draw: &MapFrameDraw,
    water_refraction_source: NativeD3d11WaterRefractionSource,
    reflection_cube_source: NativeD3d11ReflectionCubeSource,
    reflection_resolver: Option<&GameFileResolver>,
) -> Result<(
    Vec<D3dPtr<ID3D11Texture2D>>,
    Vec<NativeD3d11PreparedShaderResource>,
)> {
    let mut textures = Vec::new();
    let mut views = Vec::new();
    for resource in &draw.shader_resources {
        if resource.is_null {
            views.push(NativeD3d11PreparedShaderResource {
                stage: resource.stage.clone(),
                slot: resource.slot,
                declared_name: resource.declared_name.clone(),
                source: NativeD3d11ShaderResourceSource::CapturedBundleTexture,
                view: None,
            });
            continue;
        }
        if resource.declared_name.as_deref() == Some(WATER_REFRACTION_RESOURCE_SYMBOL)
            && water_refraction_source
                == NativeD3d11WaterRefractionSource::AppOwnedTerrainColorSnapshot
        {
            views.push(NativeD3d11PreparedShaderResource {
                stage: resource.stage.clone(),
                slot: resource.slot,
                declared_name: resource.declared_name.clone(),
                source: NativeD3d11ShaderResourceSource::AppOwnedTerrainColorSnapshot,
                view: None,
            });
            continue;
        }
        if resource.declared_name.as_deref() == Some(WATER_REFRACTION_RESOURCE_SYMBOL)
            && water_refraction_source == NativeD3d11WaterRefractionSource::UnderwaterPass
        {
            views.push(NativeD3d11PreparedShaderResource {
                stage: resource.stage.clone(),
                slot: resource.slot,
                declared_name: resource.declared_name.clone(),
                source: NativeD3d11ShaderResourceSource::UnderwaterPass,
                view: None,
            });
            continue;
        }
        if resource.declared_name.as_deref() == Some("ReflectionCubeMap")
            && water_refraction_source
                == NativeD3d11WaterRefractionSource::AppOwnedTerrainColorSnapshot
            && reflection_cube_source == NativeD3d11ReflectionCubeSource::MeanColorFlatten
        {
            let file = resource.file.as_ref().ok_or_else(|| {
                format!(
                    "production native D3D11 terrain replay blocked: draw {} ReflectionCubeMap slot {} has no raw texture payload file",
                    draw.draw_index, resource.slot
                )
            })?;
            let hash = resource.fnv1a64.as_ref().ok_or_else(|| {
                format!(
                    "production native D3D11 terrain replay blocked: draw {} ReflectionCubeMap slot {} has no hash",
                    draw.draw_index, resource.slot
                )
            })?;
            let raw = read_verified_map_frame_file(
                bundle_dir,
                file,
                resource.byte_len,
                hash,
                "reflection cube payload",
            )?;
            let (texture, view) = native_d3d11_uniform_reflection_cube(device, resource, &raw)?;
            views.push(NativeD3d11PreparedShaderResource {
                stage: resource.stage.clone(),
                slot: resource.slot,
                declared_name: resource.declared_name.clone(),
                source: NativeD3d11ShaderResourceSource::CapturedBundleTexture,
                view: Some(view),
            });
            textures.push(texture);
            continue;
        }
        let is_reflection_cube = resource.declared_name.as_deref() == Some("ReflectionCubeMap");
        if is_reflection_cube
            && reflection_cube_source == NativeD3d11ReflectionCubeSource::ShippedFileCube
        {
            native_d3d11_validate_shipped_reflection_cube_contract(draw.draw_index, resource)?;
        }
        let raw = if is_reflection_cube {
            if let Some(resolver) = reflection_resolver {
                let decoded = decode_reflection_cube_payload(resolver, resource)?;
                let file = resource.file.as_ref().ok_or_else(|| {
                    format!(
                        "production native D3D11 terrain replay blocked: draw {} ReflectionCubeMap \
                         slot {} has a reflection resolver but no captured payload file to \
                         byte-verify against (provenance verify requires the captured payload)",
                        draw.draw_index, resource.slot
                    )
                })?;
                let hash = resource.fnv1a64.as_ref().ok_or_else(|| {
                    format!(
                        "production native D3D11 terrain replay blocked: draw {} ReflectionCubeMap \
                         slot {} has a reflection resolver but no captured payload hash to \
                         byte-verify against (provenance verify requires the captured payload)",
                        draw.draw_index, resource.slot
                    )
                })?;
                let captured = read_map_frame_resource_payload(
                    bundle_dir,
                    synthetic_payloads,
                    file,
                    resource.byte_len,
                    hash,
                    "reflection cube captured payload",
                )?;
                if decoded != captured {
                    return Err(format!(
                        "production native D3D11 terrain replay blocked: draw {} \
                         ReflectionCubeMap slot {} resolver-decoded map/terrain/reflection.dds \
                         ({} B) != captured payload ({} B) — provenance byte-verify failed \
                         (see water_law.rs)",
                        draw.draw_index,
                        resource.slot,
                        decoded.len(),
                        captured.len()
                    )
                    .into());
                }
                decoded
            } else {
                let file = resource.file.as_ref().ok_or_else(|| {
                    format!(
                        "production native D3D11 terrain replay blocked: draw {} shader resource {:?} slot {} has no raw texture payload file",
                        draw.draw_index, resource.declared_name, resource.slot
                    )
                })?;
                let hash = resource.fnv1a64.as_ref().ok_or_else(|| {
                    format!(
                        "production native D3D11 terrain replay blocked: draw {} shader resource {:?} slot {} has no hash",
                        draw.draw_index, resource.declared_name, resource.slot
                    )
                })?;
                read_map_frame_resource_payload(
                    bundle_dir,
                    synthetic_payloads,
                    file,
                    resource.byte_len,
                    hash,
                    "shader resource texture",
                )?
            }
        } else {
            let file = resource.file.as_ref().ok_or_else(|| {
                format!(
                    "production native D3D11 terrain replay blocked: draw {} shader resource {:?} slot {} has no raw texture payload file",
                    draw.draw_index, resource.declared_name, resource.slot
                )
            })?;
            let hash = resource.fnv1a64.as_ref().ok_or_else(|| {
                format!(
                    "production native D3D11 terrain replay blocked: draw {} shader resource {:?} slot {} has no hash",
                    draw.draw_index, resource.declared_name, resource.slot
                )
            })?;
            read_map_frame_resource_payload(
                bundle_dir,
                synthetic_payloads,
                file,
                resource.byte_len,
                hash,
                "shader resource texture",
            )?
        };
        let upload = native_d3d11_texture_upload(resource, &raw)?;
        let texture_desc = MapFrameTexture2DDesc {
            width: resource.width,
            height: resource.height,
            mip_levels: resource.mip_count,
            array_size: native_d3d11_texture_array_size(resource)?,
            format: resource.dxgi_format,
            sample_count: resource.sample_count,
            sample_quality: resource.sample_quality,
            usage: resource.usage,
            bind_flags: resource.bind_flags,
            cpu_access_flags: resource.cpu_access_flags,
            misc_flags: resource.misc_flags,
        };
        let texture =
            native_d3d11_create_texture2d_from_desc(device, &texture_desc, Some(&upload.data))?;
        let srv_desc = native_d3d11_shader_resource_view_desc(resource)?;
        let srv_desc_ptr = srv_desc
            .as_ref()
            .map(|desc| desc as *const D3D11_SHADER_RESOURCE_VIEW_DESC)
            .unwrap_or(ptr::null());
        let mut srv: *mut ID3D11ShaderResourceView = ptr::null_mut();
        native_d3d11_hr(
            (*device.as_raw()).CreateShaderResourceView(
                texture.as_raw() as *mut _,
                srv_desc_ptr,
                &mut srv,
            ),
            "CreateShaderResourceView from captured texture",
        )?;
        views.push(NativeD3d11PreparedShaderResource {
            stage: resource.stage.clone(),
            slot: resource.slot,
            declared_name: resource.declared_name.clone(),
            source: NativeD3d11ShaderResourceSource::CapturedBundleTexture,
            view: Some(D3dPtr::from_raw(srv, "captured shader resource view")?),
        });
        textures.push(texture);
    }
    Ok((textures, views))
}

#[cfg(windows)]
unsafe fn native_d3d11_uniform_reflection_cube(
    device: &D3dPtr<ID3D11Device>,
    resource: &MapFrameTextureBinding,
    raw: &[u8],
) -> Result<(D3dPtr<ID3D11Texture2D>, D3dPtr<ID3D11ShaderResourceView>)> {
    if resource.dxgi_format != 87 && resource.dxgi_format != 91 {
        return Err(format!(
            "production native D3D11 terrain replay blocked: ReflectionCubeMap payload format {} has no mean-color path",
            resource.dxgi_format
        )
        .into());
    }
    if raw.len() < 4 || raw.len() % 4 != 0 {
        return Err(format!(
            "production native D3D11 terrain replay blocked: ReflectionCubeMap payload has {} bytes, expected a non-empty multiple of 4",
            raw.len()
        )
        .into());
    }
    let mut sums = [0u64; 4];
    for texel in raw.chunks_exact(4) {
        for (sum, byte) in sums.iter_mut().zip(texel) {
            *sum += u64::from(*byte);
        }
    }
    let count = (raw.len() / 4) as u64;
    let mean: Vec<u8> = sums
        .iter()
        .map(|sum| ((sum + count / 2) / count) as u8)
        .collect();
    let face_data: Vec<D3D11_SUBRESOURCE_DATA> = (0..6)
        .map(|_| D3D11_SUBRESOURCE_DATA {
            pSysMem: mean.as_ptr() as *const _,
            SysMemPitch: 4,
            SysMemSlicePitch: 4,
        })
        .collect();
    let desc = D3D11_TEXTURE2D_DESC {
        Width: 1,
        Height: 1,
        MipLevels: 1,
        ArraySize: 6,
        Format: resource.dxgi_format,
        SampleDesc: DXGI_SAMPLE_DESC {
            Count: 1,
            Quality: 0,
        },
        Usage: D3D11_USAGE_IMMUTABLE,
        BindFlags: D3D11_BIND_SHADER_RESOURCE,
        CPUAccessFlags: 0,
        MiscFlags: winapi::um::d3d11::D3D11_RESOURCE_MISC_TEXTURECUBE,
    };
    let mut texture: *mut ID3D11Texture2D = ptr::null_mut();
    native_d3d11_hr(
        (*device.as_raw()).CreateTexture2D(&desc, face_data.as_ptr(), &mut texture),
        "CreateTexture2D for uniform reflection cube",
    )?;
    let texture = D3dPtr::from_raw(texture, "uniform reflection cube texture")?;
    let mut srv: *mut ID3D11ShaderResourceView = ptr::null_mut();
    native_d3d11_hr(
        (*device.as_raw()).CreateShaderResourceView(
            texture.as_raw() as *mut _,
            ptr::null(),
            &mut srv,
        ),
        "CreateShaderResourceView for uniform reflection cube",
    )?;
    let srv = D3dPtr::from_raw(srv, "uniform reflection cube SRV")?;
    Ok((texture, srv))
}

#[cfg(windows)]
pub(crate) fn native_d3d11_validate_shipped_reflection_cube_contract(
    draw_index: u32,
    resource: &MapFrameTextureBinding,
) -> Result<()> {
    const EXPECTED_SIZE: u32 = 256;
    const EXPECTED_MIPS: u32 = 9;
    const EXPECTED_FACES: u32 = 6;
    let mut problems = Vec::new();
    if resource.width != EXPECTED_SIZE || resource.height != EXPECTED_SIZE {
        problems.push(format!(
            "dims {}x{} (expected {EXPECTED_SIZE}x{EXPECTED_SIZE})",
            resource.width, resource.height
        ));
    }
    if resource.mip_count != EXPECTED_MIPS {
        problems.push(format!(
            "mip_count {} (expected {EXPECTED_MIPS})",
            resource.mip_count
        ));
    }
    if resource.array_size != EXPECTED_FACES {
        problems.push(format!(
            "array_size {} (expected {EXPECTED_FACES} cube faces)",
            resource.array_size
        ));
    }
    if resource.dxgi_format != DXGI_FORMAT_B8G8R8A8_UNORM_SRGB {
        problems.push(format!(
            "dxgi_format {} (expected {DXGI_FORMAT_B8G8R8A8_UNORM_SRGB} = B8G8R8A8_UNORM_SRGB)",
            resource.dxgi_format
        ));
    }
    if resource.misc_flags & winapi::um::d3d11::D3D11_RESOURCE_MISC_TEXTURECUBE == 0 {
        problems.push(format!(
            "misc_flags {:#x} missing D3D11_RESOURCE_MISC_TEXTURECUBE",
            resource.misc_flags
        ));
    }
    if problems.is_empty() {
        Ok(())
    } else {
        Err(format!(
            "production native D3D11 terrain replay blocked: --water-full draw {draw_index} ReflectionCubeMap slot {} does not match the shipped map/terrain/reflection.dds cube contract: {}",
            resource.slot,
            problems.join("; ")
        )
        .into())
    }
}

#[cfg(windows)]
pub(crate) unsafe fn native_d3d11_prepare_samplers(
    device: &D3dPtr<ID3D11Device>,
    draw: &MapFrameDraw,
) -> Result<Vec<NativeD3d11PreparedSampler>> {
    let mut samplers = Vec::new();
    for sampler in &draw.samplers {
        let desc = D3D11_SAMPLER_DESC {
            Filter: sampler.filter as _,
            AddressU: sampler.address_u as _,
            AddressV: sampler.address_v as _,
            AddressW: sampler.address_w as _,
            MipLODBias: sampler.mip_lod_bias,
            MaxAnisotropy: sampler.max_anisotropy,
            ComparisonFunc: sampler.comparison_func as _,
            BorderColor: sampler.border_color,
            MinLOD: sampler.min_lod,
            MaxLOD: sampler.max_lod,
        };
        let mut state: *mut ID3D11SamplerState = ptr::null_mut();
        native_d3d11_hr(
            (*device.as_raw()).CreateSamplerState(&desc, &mut state),
            "CreateSamplerState from captured sampler descriptor",
        )?;
        samplers.push(NativeD3d11PreparedSampler {
            stage: sampler.stage.clone(),
            slot: sampler.slot,
            state: D3dPtr::from_raw(state, "captured sampler state")?,
        });
    }
    Ok(samplers)
}

#[cfg(windows)]
pub(crate) unsafe fn native_d3d11_prepare_rasterizer_state(
    device: &D3dPtr<ID3D11Device>,
    draw: &MapFrameDraw,
) -> Result<Option<D3dPtr<ID3D11RasterizerState>>> {
    let state = draw.draw_state.as_ref().ok_or_else(|| {
        format!(
            "production native D3D11 terrain replay blocked: draw {} has no rasterizer state",
            draw.draw_index
        )
    })?;
    if state.rasterizer.default_state {
        return Ok(None);
    }
    let desc = state.rasterizer.desc.as_ref().ok_or_else(|| {
        format!(
            "production native D3D11 terrain replay blocked: draw {} rasterizer descriptor missing",
            draw.draw_index
        )
    })?;
    let native_desc = D3D11_RASTERIZER_DESC {
        FillMode: desc.fill_mode as _,
        CullMode: desc.cull_mode as _,
        FrontCounterClockwise: native_d3d11_bool(desc.front_counter_clockwise),
        DepthBias: desc.depth_bias,
        DepthBiasClamp: desc.depth_bias_clamp,
        SlopeScaledDepthBias: desc.slope_scaled_depth_bias,
        DepthClipEnable: native_d3d11_bool(desc.depth_clip_enable),
        ScissorEnable: native_d3d11_bool(desc.scissor_enable),
        MultisampleEnable: native_d3d11_bool(desc.multisample_enable),
        AntialiasedLineEnable: native_d3d11_bool(desc.antialiased_line_enable),
    };
    let mut rasterizer: *mut ID3D11RasterizerState = ptr::null_mut();
    native_d3d11_hr(
        (*device.as_raw()).CreateRasterizerState(&native_desc, &mut rasterizer),
        "CreateRasterizerState from captured descriptor",
    )?;
    D3dPtr::from_raw(rasterizer, "captured rasterizer state").map(Some)
}

#[cfg(windows)]
pub(crate) unsafe fn native_d3d11_prepare_blend_state(
    device: &D3dPtr<ID3D11Device>,
    draw: &MapFrameDraw,
) -> Result<(Option<D3dPtr<ID3D11BlendState>>, [f32; 4], u32)> {
    let state = draw.draw_state.as_ref().ok_or_else(|| {
        format!(
            "production native D3D11 terrain replay blocked: draw {} has no blend state",
            draw.draw_index
        )
    })?;
    if state.blend.default_state {
        return Ok((None, state.blend.blend_factor, state.blend.sample_mask));
    }
    let desc = state.blend.desc.as_ref().ok_or_else(|| {
        format!(
            "production native D3D11 terrain replay blocked: draw {} blend descriptor missing",
            draw.draw_index
        )
    })?;
    let mut native_desc: D3D11_BLEND_DESC = mem::zeroed();
    native_desc.AlphaToCoverageEnable = native_d3d11_bool(desc.alpha_to_coverage_enable);
    native_desc.IndependentBlendEnable = native_d3d11_bool(desc.independent_blend_enable);
    for (index, target) in desc.render_targets.iter().take(8).enumerate() {
        native_desc.RenderTarget[index] = D3D11_RENDER_TARGET_BLEND_DESC {
            BlendEnable: native_d3d11_bool(target.blend_enable),
            SrcBlend: target.src_blend as _,
            DestBlend: target.dest_blend as _,
            BlendOp: target.blend_op as _,
            SrcBlendAlpha: target.src_blend_alpha as _,
            DestBlendAlpha: target.dest_blend_alpha as _,
            BlendOpAlpha: target.blend_op_alpha as _,
            RenderTargetWriteMask: target.render_target_write_mask,
        };
    }
    let mut blend: *mut ID3D11BlendState = ptr::null_mut();
    native_d3d11_hr(
        (*device.as_raw()).CreateBlendState(&native_desc, &mut blend),
        "CreateBlendState from captured descriptor",
    )?;
    Ok((
        Some(D3dPtr::from_raw(blend, "captured blend state")?),
        state.blend.blend_factor,
        state.blend.sample_mask,
    ))
}

#[cfg(windows)]
pub(crate) unsafe fn native_d3d11_prepare_depth_stencil_state(
    device: &D3dPtr<ID3D11Device>,
    draw: &MapFrameDraw,
) -> Result<(Option<D3dPtr<ID3D11DepthStencilState>>, u32)> {
    let state = draw.draw_state.as_ref().ok_or_else(|| {
        format!(
            "production native D3D11 terrain replay blocked: draw {} has no depth-stencil state",
            draw.draw_index
        )
    })?;
    if state.depth_stencil.default_state {
        return Ok((None, state.depth_stencil.stencil_ref));
    }
    let desc = state.depth_stencil.desc.as_ref().ok_or_else(|| {
        format!(
            "production native D3D11 terrain replay blocked: draw {} depth-stencil descriptor missing",
            draw.draw_index
        )
    })?;
    let native_desc = D3D11_DEPTH_STENCIL_DESC {
        DepthEnable: native_d3d11_bool(desc.depth_enable),
        DepthWriteMask: desc.depth_write_mask as _,
        DepthFunc: desc.depth_func as _,
        StencilEnable: native_d3d11_bool(desc.stencil_enable),
        StencilReadMask: desc.stencil_read_mask,
        StencilWriteMask: desc.stencil_write_mask,
        FrontFace: native_d3d11_stencil_op_desc(&desc.front_face),
        BackFace: native_d3d11_stencil_op_desc(&desc.back_face),
    };
    let mut depth_state: *mut ID3D11DepthStencilState = ptr::null_mut();
    native_d3d11_hr(
        (*device.as_raw()).CreateDepthStencilState(&native_desc, &mut depth_state),
        "CreateDepthStencilState from captured descriptor",
    )?;
    Ok((
        Some(D3dPtr::from_raw(
            depth_state,
            "captured depth-stencil state",
        )?),
        state.depth_stencil.stencil_ref,
    ))
}

#[cfg(windows)]
pub(crate) fn native_d3d11_prepare_viewport_scissor(
    draw: &MapFrameDraw,
) -> Result<(Vec<D3D11_VIEWPORT>, Vec<RECT>)> {
    let state = draw.draw_state.as_ref().ok_or_else(|| {
        format!(
            "production native D3D11 terrain replay blocked: draw {} has no viewport state",
            draw.draw_index
        )
    })?;
    let viewports = state
        .viewports
        .iter()
        .map(|viewport| D3D11_VIEWPORT {
            TopLeftX: viewport.top_left_x,
            TopLeftY: viewport.top_left_y,
            Width: viewport.width,
            Height: viewport.height,
            MinDepth: viewport.min_depth,
            MaxDepth: viewport.max_depth,
        })
        .collect::<Vec<_>>();
    let scissor_rects = state
        .scissor_rects
        .iter()
        .map(|rect| RECT {
            left: rect.left,
            top: rect.top,
            right: rect.right,
            bottom: rect.bottom,
        })
        .collect::<Vec<_>>();
    Ok((viewports, scissor_rects))
}

#[cfg(windows)]
#[allow(dead_code)]
pub(crate) unsafe fn native_d3d11_replay_draw(
    device: &NativeD3d11Device,
    context: &D3dPtr<ID3D11DeviceContext>,
    bundle_dir: &Path,
    draw: &MapFrameDraw,
    render_target: &NativeD3d11RenderTarget,
    depth_target: &NativeD3d11DepthStencil,
    summary: &mut NativeD3d11ReplaySummary,
) -> Result<()> {
    let vertex_shader = draw.vertex_shader.as_ref().ok_or_else(|| {
        format!(
            "production native D3D11 terrain replay blocked: draw {} missing VS bytecode",
            draw.draw_index
        )
    })?;
    let vs_bytes = native_d3d11_read_shader_bytecode(bundle_dir, vertex_shader)?;
    let ps_bytes = native_d3d11_read_shader_bytecode(bundle_dir, &draw.shader)?;

    let mut vs: *mut ID3D11VertexShader = ptr::null_mut();
    native_d3d11_hr(
        (*device.device.as_raw()).CreateVertexShader(
            vs_bytes.as_ptr() as *const _,
            vs_bytes.len(),
            ptr::null_mut(),
            &mut vs,
        ),
        "CreateVertexShader from captured the game bytecode",
    )?;
    let vs = D3dPtr::from_raw(vs, "captured terrain vertex shader")?;

    let mut ps: *mut ID3D11PixelShader = ptr::null_mut();
    native_d3d11_hr(
        (*device.device.as_raw()).CreatePixelShader(
            ps_bytes.as_ptr() as *const _,
            ps_bytes.len(),
            ptr::null_mut(),
            &mut ps,
        ),
        "CreatePixelShader from captured the game bytecode",
    )?;
    let ps = D3dPtr::from_raw(ps, "captured pdxmap pixel shader")?;

    let input_layout = native_d3d11_create_input_layout(&device.device, draw, &vs_bytes)?;
    (*context.as_raw()).VSSetShader(vs.as_raw(), ptr::null_mut(), 0);
    (*context.as_raw()).PSSetShader(ps.as_raw(), ptr::null_mut(), 0);
    (*context.as_raw()).IASetInputLayout(input_layout.as_raw());
    let topology = draw.topology.ok_or_else(|| {
        format!(
            "production native D3D11 terrain replay blocked: draw {} has no captured IA primitive topology",
            draw.draw_index
        )
    })?;
    (*context.as_raw()).IASetPrimitiveTopology(topology as _);

    let vertex_buffers =
        native_d3d11_bind_vertex_buffers(&device.device, context, bundle_dir, draw)?;
    let index_buffer = native_d3d11_bind_index_buffer(&device.device, context, bundle_dir, draw)?;
    let constant_buffers =
        native_d3d11_bind_constant_buffers(&device.device, context, bundle_dir, draw)?;
    let synthetic_payloads = BTreeMap::new();
    let (_textures, srvs) = native_d3d11_bind_shader_resources(
        &device.device,
        context,
        bundle_dir,
        &synthetic_payloads,
        draw,
    )?;
    let samplers = native_d3d11_bind_samplers(&device.device, context, draw)?;
    let rasterizer = native_d3d11_bind_rasterizer_state(&device.device, context, draw)?;
    let blend = native_d3d11_bind_blend_state(&device.device, context, draw)?;
    let depth = native_d3d11_bind_depth_stencil_state(&device.device, context, draw)?;
    native_d3d11_bind_viewport_scissor(context, draw)?;

    let mut rtv_ptr = render_target.view.as_raw();
    let dsv_ptr = depth_target.view.as_raw();
    (*context.as_raw()).OMSetRenderTargets(1, &mut rtv_ptr, dsv_ptr);

    match draw.draw_call.as_str() {
        "DrawIndexed" => (*context.as_raw()).DrawIndexed(
            draw.draw_count,
            draw.draw_start,
            draw.base_vertex_location,
        ),
        "Draw" => (*context.as_raw()).Draw(draw.draw_count, draw.draw_start),
        "DrawIndexedInstanced" => (*context.as_raw()).DrawIndexedInstanced(
            draw.per_instance_count.ok_or_else(|| {
                format!(
                    "production native D3D11 terrain replay blocked: draw {} is DrawIndexedInstanced but has no captured per-instance index count",
                    draw.draw_index
                )
            })?,
            draw.instance_count.ok_or_else(|| {
                format!(
                    "production native D3D11 terrain replay blocked: draw {} is DrawIndexedInstanced but has no captured instance count",
                    draw.draw_index
                )
            })?,
            draw.draw_start,
            draw.base_vertex_location,
            draw.start_instance_location.ok_or_else(|| {
                format!(
                    "production native D3D11 terrain replay blocked: draw {} is DrawIndexedInstanced but has no captured start instance location",
                    draw.draw_index
                )
            })?,
        ),
        "DrawInstanced" => (*context.as_raw()).DrawInstanced(
            draw.per_instance_count.ok_or_else(|| {
                format!(
                    "production native D3D11 terrain replay blocked: draw {} is DrawInstanced but has no captured per-instance vertex count",
                    draw.draw_index
                )
            })?,
            draw.instance_count.ok_or_else(|| {
                format!(
                    "production native D3D11 terrain replay blocked: draw {} is DrawInstanced but has no captured instance count",
                    draw.draw_index
                )
            })?,
            draw.draw_start,
            draw.start_instance_location.ok_or_else(|| {
                format!(
                    "production native D3D11 terrain replay blocked: draw {} is DrawInstanced but has no captured start instance location",
                    draw.draw_index
                )
            })?,
        ),
        other => {
            return Err(format!(
                "production native D3D11 terrain replay blocked: draw {} uses unsupported draw call {other}",
                draw.draw_index
            )
            .into())
        }
    }

    let _keep_alive = (
        vs,
        ps,
        input_layout,
        vertex_buffers,
        index_buffer,
        constant_buffers,
        srvs,
        samplers,
        rasterizer,
        blend,
        depth,
    );
    summary.draw_count += 1;
    summary.vertex_buffer_count += draw.vertex_buffers.len();
    summary.index_buffer_count += usize::from(draw.index_buffer.is_some());
    summary.constant_buffer_count += draw.constant_buffers.len();
    summary.shader_resource_count += draw.shader_resources.len();
    summary.sampler_count += draw.samplers.len();
    Ok(())
}

#[cfg(windows)]
pub(crate) unsafe fn native_d3d11_create_input_layout(
    device: &D3dPtr<ID3D11Device>,
    draw: &MapFrameDraw,
    vs_bytes: &[u8],
) -> Result<D3dPtr<ID3D11InputLayout>> {
    let elements = native_d3d11_effective_input_layout_elements(draw);
    native_d3d11_create_input_layout_from_elements(device, &elements, vs_bytes)
}

#[cfg(windows)]
pub(crate) unsafe fn native_d3d11_create_input_layout_from_elements(
    device: &D3dPtr<ID3D11Device>,
    elements: &[RuntimeInputElementMeta],
    vs_bytes: &[u8],
) -> Result<D3dPtr<ID3D11InputLayout>> {
    let semantic_names = elements
        .iter()
        .map(|element| CString::new(element.semantic_name.as_str()))
        .collect::<std::result::Result<Vec<_>, _>>()?;
    let descriptors = elements
        .iter()
        .zip(semantic_names.iter())
        .map(|(element, semantic)| D3D11_INPUT_ELEMENT_DESC {
            SemanticName: semantic.as_ptr(),
            SemanticIndex: element.semantic_index,
            Format: element.format as _,
            InputSlot: element.input_slot,
            AlignedByteOffset: element.aligned_byte_offset,
            InputSlotClass: element.input_slot_class as _,
            InstanceDataStepRate: element.instance_data_step_rate,
        })
        .collect::<Vec<_>>();
    let mut input_layout: *mut ID3D11InputLayout = ptr::null_mut();
    native_d3d11_hr(
        (*device.as_raw()).CreateInputLayout(
            descriptors.as_ptr(),
            descriptors.len() as UINT,
            vs_bytes.as_ptr() as *const _,
            vs_bytes.len(),
            &mut input_layout,
        ),
        "CreateInputLayout from captured IA descriptor and VS bytecode",
    )?;
    D3dPtr::from_raw(input_layout, "captured terrain input layout")
}

#[cfg(windows)]
pub(crate) fn native_d3d11_effective_input_layout_elements(
    draw: &MapFrameDraw,
) -> Vec<RuntimeInputElementMeta> {
    if native_d3d11_needs_river_input_layout_repair(draw) {
        return native_d3d11_river_input_layout_elements();
    }
    if native_d3d11_needs_tree_input_layout_repair(draw) {
        return native_d3d11_tree_input_layout_elements();
    }
    draw.input_layout.elements.clone()
}

#[cfg(windows)]
fn native_d3d11_needs_river_input_layout_repair(draw: &MapFrameDraw) -> bool {
    draw.draw_call == "DrawIndexed"
        && draw.vertex_shader.as_ref().is_some_and(|shader| {
            shader
                .source_evidence
                .iter()
                .any(|source| source == "gfx/FX/river.shader")
        })
        && draw
            .vertex_buffers
            .get("0")
            .and_then(|buffer| buffer.stride)
            == Some(RIVER_VERTEX_STRIDE_BYTES)
}

#[cfg(windows)]
fn native_d3d11_needs_tree_input_layout_repair(draw: &MapFrameDraw) -> bool {
    draw.draw_call == "DrawIndexedInstanced"
        && draw.vertex_shader.as_ref().is_some_and(|shader| {
            shader
                .source_evidence
                .iter()
                .any(|source| source == "gfx/FX/tree.shader")
                && shader
                    .input_parameters
                    .iter()
                    .any(|param| param.semantic_name == "TEXCOORD" && param.semantic_index == 5)
        })
        && draw
            .vertex_buffers
            .get("0")
            .and_then(|buffer| buffer.stride)
            == Some(56)
        && draw.input_layout.elements.len() < 7
}

#[cfg(windows)]
pub(crate) fn native_d3d11_river_input_layout_elements() -> Vec<RuntimeInputElementMeta> {
    let per_vertex = crate::constants::D3D11_INPUT_PER_VERTEX_DATA;
    vec![
        RuntimeInputElementMeta {
            semantic_name: "POSITION".to_string(),
            semantic_index: 0,
            format: DXGI_FORMAT_R32G32B32A32_FLOAT,
            input_slot: 0,
            aligned_byte_offset: 0,
            input_slot_class: per_vertex,
            instance_data_step_rate: 0,
        },
        RuntimeInputElementMeta {
            semantic_name: "TEXCOORD".to_string(),
            semantic_index: 0,
            format: DXGI_FORMAT_R32G32B32A32_FLOAT,
            input_slot: 0,
            aligned_byte_offset: 16,
            input_slot_class: per_vertex,
            instance_data_step_rate: 0,
        },
    ]
}

#[cfg(windows)]
pub(crate) fn native_d3d11_tree_input_layout_elements() -> Vec<RuntimeInputElementMeta> {
    let per_vertex = crate::constants::D3D11_INPUT_PER_VERTEX_DATA;
    let per_instance = D3D11_INPUT_PER_INSTANCE_DATA;
    vec![
        RuntimeInputElementMeta {
            semantic_name: "POSITION".to_string(),
            semantic_index: 0,
            format: DXGI_FORMAT_R32G32B32_FLOAT,
            input_slot: 0,
            aligned_byte_offset: 0,
            input_slot_class: per_vertex,
            instance_data_step_rate: 0,
        },
        RuntimeInputElementMeta {
            semantic_name: "TEXCOORD".to_string(),
            semantic_index: 0,
            format: DXGI_FORMAT_R32G32B32_FLOAT,
            input_slot: 0,
            aligned_byte_offset: 12,
            input_slot_class: per_vertex,
            instance_data_step_rate: 0,
        },
        RuntimeInputElementMeta {
            semantic_name: "TEXCOORD".to_string(),
            semantic_index: 1,
            format: DXGI_FORMAT_R32G32B32A32_FLOAT,
            input_slot: 0,
            aligned_byte_offset: 24,
            input_slot_class: per_vertex,
            instance_data_step_rate: 0,
        },
        RuntimeInputElementMeta {
            semantic_name: "TEXCOORD".to_string(),
            semantic_index: 2,
            format: DXGI_FORMAT_R32G32_FLOAT,
            input_slot: 0,
            aligned_byte_offset: 40,
            input_slot_class: per_vertex,
            instance_data_step_rate: 0,
        },
        RuntimeInputElementMeta {
            semantic_name: "TEXCOORD".to_string(),
            semantic_index: 3,
            format: DXGI_FORMAT_R32G32_FLOAT,
            input_slot: 0,
            aligned_byte_offset: 48,
            input_slot_class: per_vertex,
            instance_data_step_rate: 0,
        },
        RuntimeInputElementMeta {
            semantic_name: "TEXCOORD".to_string(),
            semantic_index: 4,
            format: DXGI_FORMAT_R32G32B32A32_FLOAT,
            input_slot: 1,
            aligned_byte_offset: 0,
            input_slot_class: per_instance,
            instance_data_step_rate: 1,
        },
        RuntimeInputElementMeta {
            semantic_name: "TEXCOORD".to_string(),
            semantic_index: 5,
            format: DXGI_FORMAT_R32G32B32_FLOAT,
            input_slot: 1,
            aligned_byte_offset: 16,
            input_slot_class: per_instance,
            instance_data_step_rate: 1,
        },
    ]
}

#[cfg(windows)]
pub(crate) unsafe fn native_d3d11_bind_vertex_buffers(
    device: &D3dPtr<ID3D11Device>,
    context: &D3dPtr<ID3D11DeviceContext>,
    bundle_dir: &Path,
    draw: &MapFrameDraw,
) -> Result<Vec<D3dPtr<ID3D11Buffer>>> {
    let mut buffers = Vec::new();
    for binding in draw.vertex_buffers.values() {
        let bytes = native_d3d11_read_buffer_file(bundle_dir, binding, "vertex buffer")?;
        let buffer = native_d3d11_create_buffer(device, binding, &bytes, "vertex buffer")?;
        let mut raw = buffer.as_raw();
        let mut stride = binding.stride.ok_or_else(|| {
            format!(
                "production native D3D11 terrain replay blocked: draw {} vertex buffer slot {} has no stride",
                draw.draw_index, binding.slot
            )
        })?;
        let mut offset = binding.offset.unwrap_or(0);
        (*context.as_raw()).IASetVertexBuffers(binding.slot, 1, &mut raw, &mut stride, &mut offset);
        buffers.push(buffer);
    }
    Ok(buffers)
}

#[cfg(windows)]
pub(crate) unsafe fn native_d3d11_bind_index_buffer(
    device: &D3dPtr<ID3D11Device>,
    context: &D3dPtr<ID3D11DeviceContext>,
    bundle_dir: &Path,
    draw: &MapFrameDraw,
) -> Result<Option<D3dPtr<ID3D11Buffer>>> {
    let Some(binding) = draw.index_buffer.as_ref() else {
        return Ok(None);
    };
    let file = binding.file.as_ref().ok_or_else(|| {
        format!(
            "production native D3D11 terrain replay blocked: draw {} index buffer has no raw file",
            draw.draw_index
        )
    })?;
    let hash = binding.fnv1a64.as_ref().ok_or_else(|| {
        format!(
            "production native D3D11 terrain replay blocked: draw {} index buffer has no hash",
            draw.draw_index
        )
    })?;
    let bytes =
        read_verified_map_frame_file(bundle_dir, file, binding.byte_len, hash, "index buffer")?;
    let desc = MapFrameBufferBinding {
        stage: "IA".to_string(),
        slot: 0,
        declared_name: None,
        file: binding.file.clone(),
        byte_len: binding.byte_len,
        fnv1a64: binding.fnv1a64.clone(),
        stride: None,
        offset: Some(binding.offset),
        dxgi_format: Some(binding.dxgi_format),
        bind_flags: binding.bind_flags,
        usage: binding.usage,
        cpu_access_flags: binding.cpu_access_flags,
    };
    let buffer = native_d3d11_create_buffer(device, &desc, &bytes, "index buffer")?;
    (*context.as_raw()).IASetIndexBuffer(buffer.as_raw(), binding.dxgi_format as _, binding.offset);
    Ok(Some(buffer))
}

#[cfg(windows)]
pub(crate) unsafe fn native_d3d11_bind_constant_buffers(
    device: &D3dPtr<ID3D11Device>,
    context: &D3dPtr<ID3D11DeviceContext>,
    bundle_dir: &Path,
    draw: &MapFrameDraw,
) -> Result<Vec<D3dPtr<ID3D11Buffer>>> {
    let mut buffers = Vec::new();
    for binding in &draw.constant_buffers {
        let bytes = native_d3d11_read_buffer_file(bundle_dir, binding, "constant buffer")?;
        let buffer = native_d3d11_create_buffer(device, binding, &bytes, "constant buffer")?;
        let mut raw = buffer.as_raw();
        match binding.stage.as_str() {
            "VS" => (*context.as_raw()).VSSetConstantBuffers(binding.slot, 1, &mut raw),
            "PS" => (*context.as_raw()).PSSetConstantBuffers(binding.slot, 1, &mut raw),
            stage => {
                return Err(format!(
                    "production native D3D11 terrain replay blocked: draw {} constant buffer stage {stage} is unsupported",
                    draw.draw_index
                )
                .into())
            }
        }
        buffers.push(buffer);
    }
    Ok(buffers)
}

#[cfg(windows)]
pub(crate) unsafe fn native_d3d11_bind_shader_resources(
    device: &D3dPtr<ID3D11Device>,
    context: &D3dPtr<ID3D11DeviceContext>,
    bundle_dir: &Path,
    synthetic_payloads: &BTreeMap<String, MapFrameSyntheticPayload>,
    draw: &MapFrameDraw,
) -> Result<(
    Vec<D3dPtr<ID3D11Texture2D>>,
    Vec<Option<D3dPtr<ID3D11ShaderResourceView>>>,
)> {
    let mut textures = Vec::new();
    let mut views = Vec::new();
    for resource in &draw.shader_resources {
        if resource.is_null {
            let mut null_srv: *mut ID3D11ShaderResourceView = ptr::null_mut();
            (*context.as_raw()).PSSetShaderResources(resource.slot, 1, &mut null_srv);
            views.push(None);
            continue;
        }
        let file = resource.file.as_ref().ok_or_else(|| {
            format!(
                "production native D3D11 terrain replay blocked: draw {} shader resource {:?} slot {} has no raw texture payload file",
                draw.draw_index, resource.declared_name, resource.slot
            )
        })?;
        let hash = resource.fnv1a64.as_ref().ok_or_else(|| {
            format!(
                "production native D3D11 terrain replay blocked: draw {} shader resource {:?} slot {} has no hash",
                draw.draw_index, resource.declared_name, resource.slot
            )
        })?;
        let raw = read_map_frame_resource_payload(
            bundle_dir,
            synthetic_payloads,
            file,
            resource.byte_len,
            hash,
            "shader resource texture",
        )?;
        let upload = native_d3d11_texture_upload(resource, &raw)?;
        let texture_desc = MapFrameTexture2DDesc {
            width: resource.width,
            height: resource.height,
            mip_levels: resource.mip_count,
            array_size: native_d3d11_texture_array_size(resource)?,
            format: resource.dxgi_format,
            sample_count: resource.sample_count,
            sample_quality: resource.sample_quality,
            usage: resource.usage,
            bind_flags: resource.bind_flags,
            cpu_access_flags: resource.cpu_access_flags,
            misc_flags: resource.misc_flags,
        };
        let texture =
            native_d3d11_create_texture2d_from_desc(device, &texture_desc, Some(&upload.data))?;
        let srv_desc = native_d3d11_shader_resource_view_desc(resource)?;
        let srv_desc_ptr = srv_desc
            .as_ref()
            .map(|desc| desc as *const D3D11_SHADER_RESOURCE_VIEW_DESC)
            .unwrap_or(ptr::null());
        let mut srv: *mut ID3D11ShaderResourceView = ptr::null_mut();
        native_d3d11_hr(
            (*device.as_raw()).CreateShaderResourceView(
                texture.as_raw() as *mut _,
                srv_desc_ptr,
                &mut srv,
            ),
            "CreateShaderResourceView from captured texture",
        )?;
        let srv = D3dPtr::from_raw(srv, "captured shader resource view")?;
        let mut raw_srv = srv.as_raw();
        (*context.as_raw()).PSSetShaderResources(resource.slot, 1, &mut raw_srv);
        textures.push(texture);
        views.push(Some(srv));
    }
    Ok((textures, views))
}

#[cfg(windows)]
pub(crate) struct NativeTextureUpload {
    #[allow(dead_code)]
    pub(crate) storage: Vec<u8>,
    pub(crate) data: Vec<D3D11_SUBRESOURCE_DATA>,
}

#[cfg(windows)]
pub(crate) fn native_d3d11_texture_upload(
    resource: &MapFrameTextureBinding,
    raw: &[u8],
) -> Result<NativeTextureUpload> {
    native_d3d11_validate_texture_payload_shape(resource)?;
    if resource.payload_format.as_deref() == Some(NATIVE_D3D11_TEXTURE_PAYLOAD_FORMAT) {
        let storage = raw.to_vec();
        let data = resource
            .subresources
            .iter()
            .map(|subresource| {
                let end = subresource
                    .file_offset
                    .checked_add(subresource.byte_len)
                    .ok_or("subresource file range overflow")?;
                if end > storage.len() {
                    return Err(format!(
                        "production native D3D11 terrain replay blocked: texture subresource range {}..{} exceeds raw file len {}",
                        subresource.file_offset,
                        end,
                        storage.len()
                    )
                    .into());
                }
                Ok(D3D11_SUBRESOURCE_DATA {
                    pSysMem: unsafe { storage.as_ptr().add(subresource.file_offset) } as *const _,
                    SysMemPitch: subresource.row_pitch,
                    SysMemSlicePitch: subresource.depth_pitch,
                })
            })
            .collect::<Result<Vec<_>>>()?;
        return Ok(NativeTextureUpload { storage, data });
    }
    let (storage, row_pitch) = match resource.dxgi_format {
        DXGI_FORMAT_A8_UNORM => (
            raw.to_vec(),
            resource
                .width
                .checked_mul(1)
                .ok_or("texture row pitch overflow")?,
        ),
        DXGI_FORMAT_R8G8B8A8_UNORM => (
            raw.to_vec(),
            resource
                .width
                .checked_mul(4)
                .ok_or("texture row pitch overflow")?,
        ),
        DXGI_FORMAT_B8G8R8A8_UNORM | DXGI_FORMAT_B8G8R8A8_UNORM_SRGB => {
            let mut bgra = raw.to_vec();
            for pixel in bgra.chunks_exact_mut(4) {
                pixel.swap(0, 2);
            }
            (
                bgra,
                resource
                    .width
                    .checked_mul(4)
                    .ok_or("texture row pitch overflow")?,
            )
        }
        DXGI_FORMAT_R32G32B32A32_FLOAT => (
            raw.to_vec(),
            resource.width.checked_mul(16).ok_or("texture row pitch overflow")?,
        ),
        other => {
            return Err(format!(
                "production native D3D11 terrain replay blocked: unsupported texture upload DXGI format {other}"
            )
            .into())
        }
    };
    let depth_pitch = row_pitch
        .checked_mul(resource.height)
        .ok_or("texture slice pitch overflow")?;
    if storage.len() != usize::try_from(depth_pitch)? {
        return Err(format!(
            "production native D3D11 terrain replay blocked: flat texture payload has {} bytes, expected {}",
            storage.len(),
            depth_pitch
        )
        .into());
    }
    let data = vec![D3D11_SUBRESOURCE_DATA {
        pSysMem: storage.as_ptr() as *const _,
        SysMemPitch: row_pitch,
        SysMemSlicePitch: depth_pitch,
    }];
    Ok(NativeTextureUpload { storage, data })
}

#[cfg(windows)]
pub(crate) fn native_d3d11_shader_resource_view_desc(
    resource: &MapFrameTextureBinding,
) -> Result<Option<D3D11_SHADER_RESOURCE_VIEW_DESC>> {
    let Some(view) = resource.view_desc.as_ref() else {
        return Ok(None);
    };
    let mut desc: D3D11_SHADER_RESOURCE_VIEW_DESC = unsafe { mem::zeroed() };
    desc.Format = view.format as _;
    desc.ViewDimension = view.view_dimension as _;
    unsafe {
        match view.view_dimension {
            D3D11_SRV_DIMENSION_TEXTURE2D => {
                *desc.u.Texture2D_mut() = D3D11_TEX2D_SRV {
                    MostDetailedMip: view.most_detailed_mip,
                    MipLevels: view.mip_levels,
                };
            }
            D3D11_SRV_DIMENSION_TEXTURE2DARRAY => {
                *desc.u.Texture2DArray_mut() = D3D11_TEX2D_ARRAY_SRV {
                    MostDetailedMip: view.most_detailed_mip,
                    MipLevels: view.mip_levels,
                    FirstArraySlice: view.first_array_slice,
                    ArraySize: view.array_size,
                };
            }
            D3D11_SRV_DIMENSION_TEXTURECUBE => {
                *desc.u.TextureCube_mut() = D3D11_TEXCUBE_SRV {
                    MostDetailedMip: view.most_detailed_mip,
                    MipLevels: view.mip_levels,
                };
            }
            D3D11_SRV_DIMENSION_TEXTURECUBEARRAY => {
                *desc.u.TextureCubeArray_mut() = D3D11_TEXCUBE_ARRAY_SRV {
                    MostDetailedMip: view.most_detailed_mip,
                    MipLevels: view.mip_levels,
                    First2DArrayFace: view.first_array_slice,
                    NumCubes: view.num_cubes,
                };
            }
            other => {
                return Err(format!(
                    "production native D3D11 terrain replay blocked: shader resource {:?} slot {} uses unsupported SRV view dimension {other}",
                    resource.declared_name, resource.slot
                )
                .into())
            }
        }
    }
    Ok(Some(desc))
}

#[cfg(windows)]
pub(crate) unsafe fn native_d3d11_bind_samplers(
    device: &D3dPtr<ID3D11Device>,
    context: &D3dPtr<ID3D11DeviceContext>,
    draw: &MapFrameDraw,
) -> Result<Vec<D3dPtr<ID3D11SamplerState>>> {
    let mut samplers = Vec::new();
    for sampler in &draw.samplers {
        let desc = D3D11_SAMPLER_DESC {
            Filter: sampler.filter as _,
            AddressU: sampler.address_u as _,
            AddressV: sampler.address_v as _,
            AddressW: sampler.address_w as _,
            MipLODBias: sampler.mip_lod_bias,
            MaxAnisotropy: sampler.max_anisotropy,
            ComparisonFunc: sampler.comparison_func as _,
            BorderColor: sampler.border_color,
            MinLOD: sampler.min_lod,
            MaxLOD: sampler.max_lod,
        };
        let mut state: *mut ID3D11SamplerState = ptr::null_mut();
        native_d3d11_hr(
            (*device.as_raw()).CreateSamplerState(&desc, &mut state),
            "CreateSamplerState from captured sampler descriptor",
        )?;
        let state = D3dPtr::from_raw(state, "captured sampler state")?;
        let mut raw = state.as_raw();
        match sampler.stage.as_str() {
            "PS" => (*context.as_raw()).PSSetSamplers(sampler.slot, 1, &mut raw),
            stage => {
                return Err(format!(
                    "production native D3D11 terrain replay blocked: draw {} sampler stage {stage} is unsupported",
                    draw.draw_index
                )
                .into())
            }
        }
        samplers.push(state);
    }
    Ok(samplers)
}

#[cfg(windows)]
pub(crate) unsafe fn native_d3d11_bind_rasterizer_state(
    device: &D3dPtr<ID3D11Device>,
    context: &D3dPtr<ID3D11DeviceContext>,
    draw: &MapFrameDraw,
) -> Result<Option<D3dPtr<ID3D11RasterizerState>>> {
    let state = draw.draw_state.as_ref().ok_or_else(|| {
        format!(
            "production native D3D11 terrain replay blocked: draw {} has no rasterizer state",
            draw.draw_index
        )
    })?;
    if state.rasterizer.default_state {
        (*context.as_raw()).RSSetState(ptr::null_mut());
        return Ok(None);
    }
    let desc = state.rasterizer.desc.as_ref().ok_or_else(|| {
        format!(
            "production native D3D11 terrain replay blocked: draw {} rasterizer descriptor missing",
            draw.draw_index
        )
    })?;
    let native_desc = D3D11_RASTERIZER_DESC {
        FillMode: desc.fill_mode as _,
        CullMode: desc.cull_mode as _,
        FrontCounterClockwise: native_d3d11_bool(desc.front_counter_clockwise),
        DepthBias: desc.depth_bias,
        DepthBiasClamp: desc.depth_bias_clamp,
        SlopeScaledDepthBias: desc.slope_scaled_depth_bias,
        DepthClipEnable: native_d3d11_bool(desc.depth_clip_enable),
        ScissorEnable: native_d3d11_bool(desc.scissor_enable),
        MultisampleEnable: native_d3d11_bool(desc.multisample_enable),
        AntialiasedLineEnable: native_d3d11_bool(desc.antialiased_line_enable),
    };
    let mut rasterizer: *mut ID3D11RasterizerState = ptr::null_mut();
    native_d3d11_hr(
        (*device.as_raw()).CreateRasterizerState(&native_desc, &mut rasterizer),
        "CreateRasterizerState from captured descriptor",
    )?;
    let rasterizer = D3dPtr::from_raw(rasterizer, "captured rasterizer state")?;
    (*context.as_raw()).RSSetState(rasterizer.as_raw());
    Ok(Some(rasterizer))
}

#[cfg(windows)]
pub(crate) unsafe fn native_d3d11_bind_blend_state(
    device: &D3dPtr<ID3D11Device>,
    context: &D3dPtr<ID3D11DeviceContext>,
    draw: &MapFrameDraw,
) -> Result<Option<D3dPtr<ID3D11BlendState>>> {
    let state = draw.draw_state.as_ref().ok_or_else(|| {
        format!(
            "production native D3D11 terrain replay blocked: draw {} has no blend state",
            draw.draw_index
        )
    })?;
    if state.blend.default_state {
        (*context.as_raw()).OMSetBlendState(
            ptr::null_mut(),
            &state.blend.blend_factor,
            state.blend.sample_mask,
        );
        return Ok(None);
    }
    let desc = state.blend.desc.as_ref().ok_or_else(|| {
        format!(
            "production native D3D11 terrain replay blocked: draw {} blend descriptor missing",
            draw.draw_index
        )
    })?;
    let mut native_desc: D3D11_BLEND_DESC = mem::zeroed();
    native_desc.AlphaToCoverageEnable = native_d3d11_bool(desc.alpha_to_coverage_enable);
    native_desc.IndependentBlendEnable = native_d3d11_bool(desc.independent_blend_enable);
    for (index, target) in desc.render_targets.iter().take(8).enumerate() {
        native_desc.RenderTarget[index] = D3D11_RENDER_TARGET_BLEND_DESC {
            BlendEnable: native_d3d11_bool(target.blend_enable),
            SrcBlend: target.src_blend as _,
            DestBlend: target.dest_blend as _,
            BlendOp: target.blend_op as _,
            SrcBlendAlpha: target.src_blend_alpha as _,
            DestBlendAlpha: target.dest_blend_alpha as _,
            BlendOpAlpha: target.blend_op_alpha as _,
            RenderTargetWriteMask: target.render_target_write_mask,
        };
    }
    let mut blend: *mut ID3D11BlendState = ptr::null_mut();
    native_d3d11_hr(
        (*device.as_raw()).CreateBlendState(&native_desc, &mut blend),
        "CreateBlendState from captured descriptor",
    )?;
    let blend = D3dPtr::from_raw(blend, "captured blend state")?;
    (*context.as_raw()).OMSetBlendState(
        blend.as_raw(),
        &state.blend.blend_factor,
        state.blend.sample_mask,
    );
    Ok(Some(blend))
}

#[cfg(windows)]
pub(crate) unsafe fn native_d3d11_bind_depth_stencil_state(
    device: &D3dPtr<ID3D11Device>,
    context: &D3dPtr<ID3D11DeviceContext>,
    draw: &MapFrameDraw,
) -> Result<Option<D3dPtr<ID3D11DepthStencilState>>> {
    let state = draw.draw_state.as_ref().ok_or_else(|| {
        format!(
            "production native D3D11 terrain replay blocked: draw {} has no depth-stencil state",
            draw.draw_index
        )
    })?;
    if state.depth_stencil.default_state {
        (*context.as_raw())
            .OMSetDepthStencilState(ptr::null_mut(), state.depth_stencil.stencil_ref);
        return Ok(None);
    }
    let desc = state.depth_stencil.desc.as_ref().ok_or_else(|| {
        format!(
            "production native D3D11 terrain replay blocked: draw {} depth-stencil descriptor missing",
            draw.draw_index
        )
    })?;
    let native_desc = D3D11_DEPTH_STENCIL_DESC {
        DepthEnable: native_d3d11_bool(desc.depth_enable),
        DepthWriteMask: desc.depth_write_mask as _,
        DepthFunc: desc.depth_func as _,
        StencilEnable: native_d3d11_bool(desc.stencil_enable),
        StencilReadMask: desc.stencil_read_mask,
        StencilWriteMask: desc.stencil_write_mask,
        FrontFace: native_d3d11_stencil_op_desc(&desc.front_face),
        BackFace: native_d3d11_stencil_op_desc(&desc.back_face),
    };
    let mut depth_state: *mut ID3D11DepthStencilState = ptr::null_mut();
    native_d3d11_hr(
        (*device.as_raw()).CreateDepthStencilState(&native_desc, &mut depth_state),
        "CreateDepthStencilState from captured descriptor",
    )?;
    let depth_state = D3dPtr::from_raw(depth_state, "captured depth-stencil state")?;
    (*context.as_raw())
        .OMSetDepthStencilState(depth_state.as_raw(), state.depth_stencil.stencil_ref);
    Ok(Some(depth_state))
}

#[cfg(windows)]
pub(crate) fn native_d3d11_stencil_op_desc(
    desc: &MapFrameDepthStencilOpDesc,
) -> D3D11_DEPTH_STENCILOP_DESC {
    D3D11_DEPTH_STENCILOP_DESC {
        StencilFailOp: desc.stencil_fail_op as _,
        StencilDepthFailOp: desc.stencil_depth_fail_op as _,
        StencilPassOp: desc.stencil_pass_op as _,
        StencilFunc: desc.stencil_func as _,
    }
}

#[cfg(windows)]
pub(crate) unsafe fn native_d3d11_bind_viewport_scissor(
    context: &D3dPtr<ID3D11DeviceContext>,
    draw: &MapFrameDraw,
) -> Result<()> {
    let state = draw.draw_state.as_ref().ok_or_else(|| {
        format!(
            "production native D3D11 terrain replay blocked: draw {} has no viewport state",
            draw.draw_index
        )
    })?;
    let viewports = state
        .viewports
        .iter()
        .map(|viewport| D3D11_VIEWPORT {
            TopLeftX: viewport.top_left_x,
            TopLeftY: viewport.top_left_y,
            Width: viewport.width,
            Height: viewport.height,
            MinDepth: viewport.min_depth,
            MaxDepth: viewport.max_depth,
        })
        .collect::<Vec<_>>();
    (*context.as_raw()).RSSetViewports(viewports.len() as UINT, viewports.as_ptr());
    if !state.scissor_rects.is_empty() {
        let rects = state
            .scissor_rects
            .iter()
            .map(|rect| RECT {
                left: rect.left,
                top: rect.top,
                right: rect.right,
                bottom: rect.bottom,
            })
            .collect::<Vec<_>>();
        (*context.as_raw()).RSSetScissorRects(rects.len() as UINT, rects.as_ptr());
    }
    Ok(())
}

#[cfg(windows)]
pub(crate) unsafe fn native_d3d11_create_camera_constant_buffer(
    device: &D3dPtr<ID3D11Device>,
    binding: &MapFrameBufferBinding,
    bytes: &[u8],
    update_method: TerrainSceneCb0CameraUpdateMethod,
) -> Result<D3dPtr<ID3D11Buffer>> {
    let (usage, cpu_access_flags) = match update_method {
        TerrainSceneCb0CameraUpdateMethod::DefaultUpdateSubresource => (D3D11_USAGE_DEFAULT, 0),
        TerrainSceneCb0CameraUpdateMethod::DynamicMapWriteDiscard => {
            (D3D11_USAGE_DYNAMIC_CAPTURED, D3D11_CPU_ACCESS_WRITE)
        }
    };
    let desc = D3D11_BUFFER_DESC {
        ByteWidth: u32::try_from(bytes.len())?,
        Usage: usage as _,
        BindFlags: binding.bind_flags,
        CPUAccessFlags: cpu_access_flags,
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
        (*device.as_raw()).CreateBuffer(&desc, &init, &mut buffer),
        "CreateBuffer for camera-updatable TerrainSceneCB0 constant buffer",
    )?;
    D3dPtr::from_raw(buffer, "camera-updatable TerrainSceneCB0 constant buffer")
}

#[cfg(windows)]
pub(crate) unsafe fn native_d3d11_create_dynamic_constant_buffer(
    device: &D3dPtr<ID3D11Device>,
    binding: &MapFrameBufferBinding,
    bytes: &[u8],
    role: &str,
) -> Result<D3dPtr<ID3D11Buffer>> {
    let desc = D3D11_BUFFER_DESC {
        ByteWidth: u32::try_from(bytes.len())?,
        Usage: D3D11_USAGE_DYNAMIC_CAPTURED as _,
        BindFlags: binding.bind_flags,
        CPUAccessFlags: D3D11_CPU_ACCESS_WRITE,
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
        (*device.as_raw()).CreateBuffer(&desc, &init, &mut buffer),
        &format!("CreateBuffer for {role}"),
    )?;
    D3dPtr::from_raw(buffer, role)
}

#[cfg(windows)]
pub(crate) unsafe fn native_d3d11_create_exact_constant_buffer(
    device: &D3dPtr<ID3D11Device>,
    bytes: &[u8],
    role: &str,
) -> Result<D3dPtr<ID3D11Buffer>> {
    if bytes.is_empty() || bytes.len() % 16 != 0 {
        return Err(format!(
            "production native D3D11 terrain replay blocked: {role} is {} bytes; constant buffers must be non-empty and 16-byte aligned",
            bytes.len()
        )
        .into());
    }
    let desc = D3D11_BUFFER_DESC {
        ByteWidth: u32::try_from(bytes.len())?,
        Usage: D3D11_USAGE_DEFAULT as _,
        BindFlags: winapi::um::d3d11::D3D11_BIND_CONSTANT_BUFFER,
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
        (*device.as_raw()).CreateBuffer(&desc, &init, &mut buffer),
        &format!("CreateBuffer for {role}"),
    )?;
    D3dPtr::from_raw(buffer, role)
}

#[cfg(windows)]
pub(crate) unsafe fn native_d3d11_create_buffer(
    device: &D3dPtr<ID3D11Device>,
    binding: &MapFrameBufferBinding,
    bytes: &[u8],
    role: &str,
) -> Result<D3dPtr<ID3D11Buffer>> {
    let desc = D3D11_BUFFER_DESC {
        ByteWidth: u32::try_from(bytes.len())?,
        Usage: binding.usage as _,
        BindFlags: binding.bind_flags,
        CPUAccessFlags: binding.cpu_access_flags,
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
        (*device.as_raw()).CreateBuffer(&desc, &init, &mut buffer),
        &format!("CreateBuffer for captured {role}"),
    )?;
    D3dPtr::from_raw(buffer, role)
}

#[cfg(windows)]
pub(crate) fn native_d3d11_read_shader_bytecode(
    bundle_dir: &Path,
    shader: &MapFrameShader,
) -> Result<Vec<u8>> {
    read_verified_map_frame_file(
        bundle_dir,
        &shader.bytecode_file,
        shader.byte_len,
        &shader.fnv1a64,
        &format!("{} shader bytecode", shader.stage),
    )
}

#[cfg(windows)]
pub(crate) fn native_d3d11_read_buffer_file(
    bundle_dir: &Path,
    binding: &MapFrameBufferBinding,
    role: &str,
) -> Result<Vec<u8>> {
    let file = binding.file.as_ref().ok_or_else(|| {
        format!(
            "production native D3D11 terrain replay blocked: {role} slot {} has no raw file",
            binding.slot
        )
    })?;
    let hash = binding.fnv1a64.as_ref().ok_or_else(|| {
        format!(
            "production native D3D11 terrain replay blocked: {role} slot {} has no hash",
            binding.slot
        )
    })?;
    read_verified_map_frame_file(bundle_dir, file, binding.byte_len, hash, role)
}

/// generated terrain-chunk slot0/index/slot1-pool/CB1 under `--chunks-from-files`),
pub(crate) fn native_d3d11_read_buffer_file_or_payload(
    bundle_dir: &Path,
    synthetic_payloads: &BTreeMap<String, MapFrameSyntheticPayload>,
    binding: &MapFrameBufferBinding,
    role: &str,
) -> Result<Vec<u8>> {
    let file = binding.file.as_ref().ok_or_else(|| {
        format!(
            "production native D3D11 terrain replay blocked: {role} slot {} has no raw file",
            binding.slot
        )
    })?;
    let hash = binding.fnv1a64.as_ref().ok_or_else(|| {
        format!(
            "production native D3D11 terrain replay blocked: {role} slot {} has no hash",
            binding.slot
        )
    })?;
    read_map_frame_resource_payload(
        bundle_dir,
        synthetic_payloads,
        file,
        binding.byte_len,
        hash,
        role,
    )
}

pub(crate) fn read_verified_map_frame_file(
    bundle_dir: &Path,
    file: &str,
    expected_len: usize,
    expected_hash: &str,
    role: &str,
) -> Result<Vec<u8>> {
    let path = bundle_dir.join(file);
    let bytes = fs::read(&path).map_err(|error| {
        format!(
            "production native D3D11 terrain replay blocked: {role} {} is not readable: {error}",
            path.display()
        )
    })?;
    if bytes.len() != expected_len {
        return Err(format!(
            "production native D3D11 terrain replay blocked: {role} {} has {} bytes, metadata expects {}",
            path.display(),
            bytes.len(),
            expected_len
        )
        .into());
    }
    let actual_hash = fnv1a64_file(&path)?;
    if actual_hash != expected_hash {
        return Err(format!(
            "production native D3D11 terrain replay blocked: {role} {} hash changed from {} to {}",
            path.display(),
            expected_hash,
            actual_hash
        )
        .into());
    }
    Ok(bytes)
}

pub(crate) fn read_map_frame_resource_payload(
    bundle_dir: &Path,
    synthetic_payloads: &BTreeMap<String, MapFrameSyntheticPayload>,
    file: &str,
    expected_len: usize,
    expected_hash: &str,
    role: &str,
) -> Result<Vec<u8>> {
    if let Some(payload) = synthetic_payloads.get(file) {
        if payload.bytes.len() != expected_len {
            return Err(format!(
                "production native D3D11 terrain replay blocked: {role} synthetic payload {file} has {} bytes, metadata expects {}",
                payload.bytes.len(),
                expected_len
            )
            .into());
        }
        if payload.fnv1a64 != expected_hash {
            return Err(format!(
                "production native D3D11 terrain replay blocked: {role} synthetic payload {file} hash changed from {} to {} ({})",
                expected_hash,
                payload.fnv1a64,
                payload.provenance
            )
            .into());
        }
        return Ok(payload.bytes.clone());
    }
    if is_synthetic_map_frame_file(file) {
        return Err(format!(
            "production native D3D11 terrain replay blocked: {role} synthetic payload {file} is missing from the loaded frame sidecar"
        )
        .into());
    }
    read_verified_map_frame_file(bundle_dir, file, expected_len, expected_hash, role)
}

#[cfg(windows)]
pub(crate) unsafe fn native_d3d11_read_render_target(
    device: &NativeD3d11Device,
    context: &D3dPtr<ID3D11DeviceContext>,
    render_target: &NativeD3d11RenderTarget,
) -> Result<NativeD3d11ReplayReadback> {
    let source = if render_target.desc.sample_count > 1 {
        let resolve_desc = MapFrameTexture2DDesc {
            sample_count: 1,
            sample_quality: 0,
            bind_flags: 0,
            cpu_access_flags: 0,
            misc_flags: 0,
            ..render_target.desc.clone()
        };
        let resolve = native_d3d11_create_texture2d_from_desc(&device.device, &resolve_desc, None)?;
        (*context.as_raw()).ResolveSubresource(
            resolve.as_raw() as *mut _,
            0,
            render_target.texture.as_raw() as *mut _,
            0,
            render_target.desc.format as _,
        );
        Some(resolve)
    } else {
        None
    };
    let source_texture = source
        .as_ref()
        .map(|texture| texture.as_raw())
        .unwrap_or_else(|| render_target.texture.as_raw());
    let staging_desc = MapFrameTexture2DDesc {
        sample_count: 1,
        sample_quality: 0,
        usage: D3D11_USAGE_STAGING as u32,
        bind_flags: 0,
        cpu_access_flags: D3D11_CPU_ACCESS_READ,
        misc_flags: 0,
        ..render_target.desc.clone()
    };
    let staging = native_d3d11_create_texture2d_from_desc(&device.device, &staging_desc, None)?;
    (*context.as_raw()).CopyResource(staging.as_raw() as *mut _, source_texture as *mut _);
    let mut mapped: D3D11_MAPPED_SUBRESOURCE = mem::zeroed();
    native_d3d11_hr(
        (*context.as_raw()).Map(
            staging.as_raw() as *mut _,
            0,
            D3D11_MAP_READ,
            0,
            &mut mapped,
        ),
        "Map staging terrain render target",
    )?;
    let row_bytes =
        native_d3d11_render_target_row_bytes(render_target.desc.format, render_target.desc.width)?;
    let mut output_bytes = Vec::with_capacity(
        usize::try_from(row_bytes)?
            .checked_mul(usize::try_from(render_target.desc.height)?)
            .ok_or("render target readback size overflow")?,
    );
    for row in 0..render_target.desc.height {
        let row_ptr = (mapped.pData as *const u8)
            .add(usize::try_from(row)? * usize::try_from(mapped.RowPitch)?);
        let row_slice = std::slice::from_raw_parts(row_ptr, usize::try_from(row_bytes)?);
        output_bytes.extend_from_slice(row_slice);
    }
    (*context.as_raw()).Unmap(staging.as_raw() as *mut _, 0);
    Ok(NativeD3d11ReplayReadback {
        width: render_target.desc.width,
        height: render_target.desc.height,
        dxgi_format: render_target.desc.format,
        row_pitch: row_bytes,
        bytes: output_bytes,
    })
}

#[cfg(windows)]
pub(crate) unsafe fn native_d3d11_read_depth_stencil(
    device: &NativeD3d11Device,
    context: &D3dPtr<ID3D11DeviceContext>,
    depth_target: &NativeD3d11DepthStencil,
) -> Result<NativeD3d11ReplayReadback> {
    let staging_desc = MapFrameTexture2DDesc {
        sample_count: 1,
        sample_quality: 0,
        usage: D3D11_USAGE_STAGING as u32,
        bind_flags: 0,
        cpu_access_flags: D3D11_CPU_ACCESS_READ,
        misc_flags: 0,
        ..depth_target.desc.clone()
    };
    let staging = native_d3d11_create_texture2d_from_desc(&device.device, &staging_desc, None)?;
    (*context.as_raw()).CopyResource(
        staging.as_raw() as *mut _,
        depth_target.texture.as_raw() as *mut _,
    );
    let mut mapped: D3D11_MAPPED_SUBRESOURCE = mem::zeroed();
    native_d3d11_hr(
        (*context.as_raw()).Map(
            staging.as_raw() as *mut _,
            0,
            D3D11_MAP_READ,
            0,
            &mut mapped,
        ),
        "Map staging terrain depth-stencil",
    )?;
    let row_bytes =
        native_d3d11_depth_stencil_row_bytes(depth_target.desc.format, depth_target.desc.width)?;
    let mut output_bytes = Vec::with_capacity(
        usize::try_from(row_bytes)?
            .checked_mul(usize::try_from(depth_target.desc.height)?)
            .ok_or("depth-stencil readback size overflow")?,
    );
    for row in 0..depth_target.desc.height {
        let row_ptr = (mapped.pData as *const u8)
            .add(usize::try_from(row)? * usize::try_from(mapped.RowPitch)?);
        let row_slice = std::slice::from_raw_parts(row_ptr, usize::try_from(row_bytes)?);
        output_bytes.extend_from_slice(row_slice);
    }
    (*context.as_raw()).Unmap(staging.as_raw() as *mut _, 0);
    Ok(NativeD3d11ReplayReadback {
        width: depth_target.desc.width,
        height: depth_target.desc.height,
        dxgi_format: depth_target.desc.format,
        row_pitch: row_bytes,
        bytes: output_bytes,
    })
}

pub(crate) fn native_d3d11_depth_stencil_row_bytes(format: u32, width: u32) -> Result<u32> {
    let bytes_per_pixel = match format {
        45 => 4,
        other => {
            return Err(format!(
                "production native D3D11 terrain replay blocked: depth-stencil format {other} has no readback writer"
            )
            .into())
        }
    };
    width
        .checked_mul(bytes_per_pixel)
        .ok_or_else(|| "depth-stencil row byte width overflow".into())
}

#[cfg(windows)]
pub(crate) fn native_d3d11_compare_reference_output(
    reference: &NativeD3d11ReferenceOutput,
    actual: &NativeD3d11ReplayReadback,
) -> Result<NativeD3d11ReferenceComparison> {
    native_d3d11_compare_reference_output_with_label("final replay output", reference, actual)
}

pub(crate) fn native_d3d11_write_parity_dump(
    report_dir: &Path,
    file_name: &str,
    label: &str,
    readback: &NativeD3d11ReplayReadback,
) -> Result<NativeD3d11ParityDump> {
    let path = report_dir.join(file_name);
    fs::write(&path, &readback.bytes)?;
    Ok(NativeD3d11ParityDump {
        label: label.to_string(),
        file: file_name.to_string(),
        byte_len: readback.bytes.len(),
        fnv1a64: fnv1a64_bytes(&readback.bytes),
    })
}

pub(crate) fn native_d3d11_parity_comparison(
    label: &str,
    reference: &NativeD3d11ReferenceOutput,
    actual: &NativeD3d11ReplayReadback,
    actual_file: Option<String>,
) -> NativeD3d11ParityComparison {
    let dimensions_match = reference.width == actual.width
        && reference.height == actual.height
        && reference.dxgi_format == actual.dxgi_format;
    let actual_fnv1a64 = fnv1a64_bytes(&actual.bytes);
    let expected_fnv1a64 = fnv1a64_bytes(&reference.bytes);
    let (differing_byte_count, first_mismatch_byte) = if dimensions_match {
        let mut count = 0usize;
        let mut first = None;
        for (index, (expected, actual)) in
            reference.bytes.iter().zip(actual.bytes.iter()).enumerate()
        {
            if expected != actual {
                count += 1;
                if first.is_none() {
                    first = Some(index);
                }
            }
        }
        if reference.bytes.len() != actual.bytes.len() {
            count += reference.bytes.len().abs_diff(actual.bytes.len());
            if first.is_none() {
                first = Some(reference.bytes.len().min(actual.bytes.len()));
            }
        }
        (Some(count), first)
    } else {
        (None, None)
    };
    NativeD3d11ParityComparison {
        label: label.to_string(),
        dimensions_match,
        matches: dimensions_match && reference.bytes == actual.bytes,
        byte_len: actual.bytes.len(),
        expected_fnv1a64,
        actual_fnv1a64,
        captured_staging_fnv1a64: reference.fnv1a64.clone(),
        expected_file: reference.file.clone(),
        actual_file,
        differing_byte_count,
        first_mismatch_byte,
    }
}

pub(crate) fn native_d3d11_compare_reference_output_with_label(
    label: &str,
    reference: &NativeD3d11ReferenceOutput,
    actual: &NativeD3d11ReplayReadback,
) -> Result<NativeD3d11ReferenceComparison> {
    if reference.width != actual.width
        || reference.height != actual.height
        || reference.dxgi_format != actual.dxgi_format
    {
        return Err(format!(
            "production native D3D11 terrain replay blocked: {label} {}x{} format {} does not match the game reference {}x{} format {}",
            actual.width,
            actual.height,
            actual.dxgi_format,
            reference.width,
            reference.height,
            reference.dxgi_format
        )
        .into());
    }
    let actual_fnv1a64 = fnv1a64_bytes(&actual.bytes);
    let expected_fnv1a64 = fnv1a64_bytes(&reference.bytes);
    if actual.bytes != reference.bytes {
        return Err(format!(
            "production native D3D11 terrain replay blocked: {label} differs from captured the game terrain render target reference; expected tight-row fnv1a64 {expected_fnv1a64}, actual {actual_fnv1a64}, captured staging fnv1a64 {}",
            reference.fnv1a64
        )
        .into());
    }
    Ok(NativeD3d11ReferenceComparison {
        byte_len: actual.bytes.len(),
        expected_fnv1a64,
        actual_fnv1a64,
    })
}

pub(crate) fn native_d3d11_render_target_row_bytes(format: u32, width: u32) -> Result<u32> {
    let bytes_per_pixel = match format {
        DXGI_FORMAT_R16G16B16A16_FLOAT => 8,
        DXGI_FORMAT_R8G8B8A8_UNORM
        | DXGI_FORMAT_B8G8R8A8_UNORM
        | DXGI_FORMAT_B8G8R8A8_UNORM_SRGB => 4,
        other => {
            return Err(format!(
                "production native D3D11 terrain replay blocked: render target format {other} has no readback writer"
            )
            .into())
        }
    };
    width
        .checked_mul(bytes_per_pixel)
        .ok_or_else(|| "render target row byte width overflow".into())
}

#[cfg(windows)]
pub(crate) fn native_d3d11_bool(value: bool) -> i32 {
    if value {
        1
    } else {
        0
    }
}

#[cfg(windows)]
pub(crate) fn native_d3d11_hr(hr: i32, action: &str) -> Result<()> {
    if hr < 0 {
        Err(format!(
            "production native D3D11 terrain replay blocked: {action} failed with HRESULT 0x{:08X}",
            hr as u32
        )
        .into())
    } else {
        Ok(())
    }
}

#[cfg(all(test, windows))]
mod tests {
    use super::*;
    use crate::test_support::*;
    use std::fs;

    fn tree_shader_input_parameters() -> Vec<MapFrameShaderInputParameter> {
        let mut inputs = vec![MapFrameShaderInputParameter {
            semantic_name: "POSITION".to_string(),
            semantic_index: 0,
            register: 0,
            component_type: D3D_REGISTER_COMPONENT_FLOAT32,
            mask: 0x07,
            read_write_mask: 0x07,
        }];
        for (semantic_index, register, mask) in [
            (0, 1, 0x07),
            (1, 2, 0x0f),
            (2, 3, 0x03),
            (3, 4, 0x03),
            (4, 5, 0x0f),
            (5, 6, 0x07),
        ] {
            inputs.push(MapFrameShaderInputParameter {
                semantic_name: "TEXCOORD".to_string(),
                semantic_index,
                register,
                component_type: D3D_REGISTER_COMPONENT_FLOAT32,
                mask,
                read_write_mask: mask,
            });
        }
        inputs
    }

    fn truncated_tree_draw(root_name: &str) -> MapFrameDraw {
        let root = temp_test_dir(root_name);
        fs::create_dir_all(&root).expect("create root");
        let path = write_valid_map_frame_bundle(&root, |frame| frame);
        let frame: MapFrameRuntimeBundle =
            serde_json::from_slice(&fs::read(path).expect("read bundle")).expect("parse bundle");
        let mut draw = frame.draws[0].clone();
        draw.draw_call = "DrawIndexedInstanced".to_string();
        draw.vertex_buffers
            .get_mut("0")
            .expect("slot 0 vertex buffer")
            .stride = Some(56);
        draw.vertex_shader
            .as_mut()
            .expect("vertex shader")
            .source_evidence = vec!["gfx/FX/tree.shader".to_string()];
        draw.vertex_shader
            .as_mut()
            .expect("vertex shader")
            .input_parameters = tree_shader_input_parameters();
        draw.input_layout.elements = vec![
            RuntimeInputElementMeta {
                semantic_name: "POSITION".to_string(),
                semantic_index: 0,
                format: DXGI_FORMAT_R32G32B32_FLOAT,
                input_slot: 0,
                aligned_byte_offset: 0,
                input_slot_class: crate::constants::D3D11_INPUT_PER_VERTEX_DATA,
                instance_data_step_rate: 0,
            },
            RuntimeInputElementMeta {
                semantic_name: "TEXCOORD".to_string(),
                semantic_index: 0,
                format: DXGI_FORMAT_R16G16_SINT,
                input_slot: 0,
                aligned_byte_offset: 12,
                input_slot_class: crate::constants::D3D11_INPUT_PER_VERTEX_DATA,
                instance_data_step_rate: 0,
            },
        ];
        draw
    }

    #[test]
    fn tree_input_layout_repair_expands_captured_truncated_layout() {
        let draw = truncated_tree_draw("native_d3d11_tree_input_layout_repair_expands");

        let elements = native_d3d11_effective_input_layout_elements(&draw);

        assert_eq!(elements.len(), 7);
        assert_eq!(elements[0].semantic_name, "POSITION");
        assert_eq!(elements[0].format, DXGI_FORMAT_R32G32B32_FLOAT);
        assert_eq!(elements[0].aligned_byte_offset, 0);
        assert_eq!(elements[1].semantic_name, "TEXCOORD");
        assert_eq!(elements[1].semantic_index, 0);
        assert_eq!(elements[1].format, DXGI_FORMAT_R32G32B32_FLOAT);
        assert_eq!(elements[1].aligned_byte_offset, 12);
        assert_eq!(elements[2].semantic_index, 1);
        assert_eq!(elements[2].format, DXGI_FORMAT_R32G32B32A32_FLOAT);
        assert_eq!(elements[2].aligned_byte_offset, 24);
        assert_eq!(elements[3].semantic_index, 2);
        assert_eq!(elements[3].format, DXGI_FORMAT_R32G32_FLOAT);
        assert_eq!(elements[3].aligned_byte_offset, 40);
        assert_eq!(elements[4].semantic_index, 3);
        assert_eq!(elements[4].format, DXGI_FORMAT_R32G32_FLOAT);
        assert_eq!(elements[4].aligned_byte_offset, 48);
        assert_eq!(elements[5].semantic_index, 4);
        assert_eq!(elements[5].format, DXGI_FORMAT_R32G32B32A32_FLOAT);
        assert_eq!(elements[5].input_slot, 1);
        assert_eq!(elements[5].aligned_byte_offset, 0);
        assert_eq!(
            elements[5].input_slot_class,
            crate::constants::D3D11_INPUT_PER_INSTANCE_DATA
        );
        assert_eq!(elements[5].instance_data_step_rate, 1);
        assert_eq!(elements[6].semantic_index, 5);
        assert_eq!(elements[6].format, DXGI_FORMAT_R32G32B32_FLOAT);
        assert_eq!(elements[6].input_slot, 1);
        assert_eq!(elements[6].aligned_byte_offset, 16);
        assert_eq!(
            elements[6].input_slot_class,
            crate::constants::D3D11_INPUT_PER_INSTANCE_DATA
        );
        assert_eq!(elements[6].instance_data_step_rate, 1);
    }

    #[test]
    fn non_tree_input_layout_uses_captured_elements() {
        let mut draw = truncated_tree_draw("native_d3d11_tree_input_layout_repair_non_tree");
        draw.vertex_shader
            .as_mut()
            .expect("vertex shader")
            .source_evidence = vec!["gfx/FX/pdxmap.shader".to_string()];
        let captured = draw.input_layout.elements.clone();

        let elements = native_d3d11_effective_input_layout_elements(&draw);

        assert_eq!(elements, captured);
    }

    #[test]
    fn map_clock_water_patch_updates_only_time_lane() {
        let mut bytes = vec![0u8; 16];
        write_f32_at(&mut bytes, 0, 3.809_123).expect("write captured water time");
        write_f32_at(&mut bytes, 4, 0.000_260).expect("write half-pixel x");
        write_f32_at(&mut bytes, 8, 0.000_463).expect("write half-pixel y");
        let original = bytes.clone();

        apply_map_clock_constant_buffer_override(
            &mut bytes,
            NativeD3d11MapClockConstantBufferKind::WaterTimeHalfPixel,
            10.5,
        )
        .expect("patch water clock");

        assert_close(read_f32_at(&bytes, 0).expect("read water time"), 10.5);
        assert_eq!(&bytes[4..], &original[4..]);
    }

    #[test]
    fn map_clock_river_patch_updates_only_time_lane() {
        let mut bytes = vec![0u8; 80];
        write_f32_at(&mut bytes, 64, 8.960_006).expect("write captured river time");
        write_f32_at(&mut bytes, 68, -1.0).expect("write river direction");
        write_f32_at(&mut bytes, 72, 0.0).expect("write river season lerp");
        let original = bytes.clone();

        apply_map_clock_constant_buffer_override(
            &mut bytes,
            NativeD3d11MapClockConstantBufferKind::RiverTimeDirectionSeasonLerp,
            11.25,
        )
        .expect("patch river clock");

        assert_eq!(&bytes[..64], &original[..64]);
        assert_close(read_f32_at(&bytes, 64).expect("read river time"), 11.25);
        assert_eq!(&bytes[68..], &original[68..]);
    }

    #[test]
    fn map_clock_river_shape_guard_rejects_dimension_like_tuple() {
        let mut bytes = vec![0u8; 80];
        write_f32_at(&mut bytes, 64, 2048.0).expect("write width-like value");
        write_f32_at(&mut bytes, 68, 1024.0).expect("write height-like value");
        write_f32_at(&mut bytes, 72, 8.0).expect("write count-like value");

        assert!(
            !native_d3d11_looks_like_river_time_direction_season_lerp(&bytes)
                .expect("evaluate river shape")
        );
    }

    #[test]
    fn map_clock_river_shape_guard_accepts_live_tuple() {
        let mut bytes = vec![0u8; 80];
        write_f32_at(&mut bytes, 64, 5.289_814).expect("write river time");
        write_f32_at(&mut bytes, 68, -1.0).expect("write river direction");
        write_f32_at(&mut bytes, 72, 0.0).expect("write river season lerp");

        assert!(
            native_d3d11_looks_like_river_time_direction_season_lerp(&bytes)
                .expect("evaluate river shape")
        );
    }
}
