//! Flag-gated static building preview overlay.
//!
//! The data source is entirely resolver-backed through map_viewer. This is a
//! preview layer: it draws decoded shipped building meshes in map-pixel screen
//! space and keeps real slot visibility disabled unless the visual-gate
//! `--buildings-preview-all` override is set.

use std::collections::BTreeMap;
use std::ffi::CString;
use std::mem;
use std::ptr;

use map_viewer::{
    building_file_derived_layer_from_resolver, BuildingFileDerivedLayerStats,
    BuildingPreviewInstance, GameFileResolver, BUILDING_MESH_VERTEX_STRIDE_BYTES,
};
use winapi::shared::dxgiformat::{
    DXGI_FORMAT_R32G32B32A32_FLOAT, DXGI_FORMAT_R32G32B32_FLOAT, DXGI_FORMAT_R32G32_FLOAT,
    DXGI_FORMAT_R32_UINT,
};
use winapi::shared::minwindef::UINT;
use winapi::um::d3d11::{
    ID3D11Buffer, ID3D11InputLayout, ID3D11PixelShader, ID3D11RasterizerState, ID3D11VertexShader,
    D3D11_BIND_CONSTANT_BUFFER, D3D11_BIND_INDEX_BUFFER, D3D11_BIND_VERTEX_BUFFER,
    D3D11_BUFFER_DESC, D3D11_CPU_ACCESS_WRITE, D3D11_CULL_NONE, D3D11_FILL_SOLID,
    D3D11_INPUT_ELEMENT_DESC, D3D11_INPUT_PER_INSTANCE_DATA, D3D11_INPUT_PER_VERTEX_DATA,
    D3D11_MAPPED_SUBRESOURCE, D3D11_MAP_WRITE_DISCARD, D3D11_RASTERIZER_DESC,
    D3D11_SUBRESOURCE_DATA, D3D11_USAGE_DYNAMIC, D3D11_USAGE_IMMUTABLE, D3D11_VIEWPORT,
};
use winapi::um::d3dcommon::D3D11_PRIMITIVE_TOPOLOGY_TRIANGLELIST;

use crate::camera::Camera;
use crate::d3d::{hr_ok, ComPtr, Gfx};
use crate::render::compile_shader;
use crate::Result;

const BUILDING_PREVIEW_SHADER_HLSL: &str = r#"
cbuffer CameraCB : register(b0) {
    float2 screen_size;
    float2 center_map;
    float zoom;
    float map_w;
    float map_h;
    float _pad;
};

struct VsIn {
    float3 local_pos : POSITION;
    float3 normal : NORMAL;
    float4 tangent : TANGENT;
    float2 uv0 : TEXCOORD0;
    float4 instance_pos_rot : TEXCOORD1;
    float4 instance_color : TEXCOORD2;
};

struct VsOut {
    float4 pos : SV_Position;
    float3 normal : NORMAL;
    float4 color : COLOR0;
};

VsOut vs_main(VsIn input) {
    float c = cos(input.instance_pos_rot.w);
    float s = sin(input.instance_pos_rot.w);
    float2 local = input.local_pos.xz;
    float2 rotated = float2(local.x * c - local.y * s, local.x * s + local.y * c);
    float2 map_pos = input.instance_pos_rot.xz + rotated;
    float2 screen_pos = (map_pos - center_map) * zoom + screen_size * 0.5;
    screen_pos.y -= input.local_pos.y * zoom;

    VsOut output;
    output.pos = float4(
        screen_pos.x / screen_size.x * 2.0 - 1.0,
        1.0 - screen_pos.y / screen_size.y * 2.0,
        0.0,
        1.0);
    output.normal = normalize(input.normal);
    output.color = float4(input.instance_color.rgb, 1.0);
    return output;
}

float4 ps_main(VsOut input) : SV_Target {
    float light = saturate(dot(normalize(input.normal), normalize(float3(0.35, 0.75, -0.45))) * 0.5 + 0.5);
    return float4(input.color.rgb * lerp(0.55, 1.0, light), 1.0);
}
"#;

#[repr(C)]
#[derive(Clone, Copy)]
struct BuildingCameraConstants {
    screen_size: [f32; 2],
    center_map: [f32; 2],
    zoom: f32,
    map_w: f32,
    map_h: f32,
    _pad: f32,
}

struct BuildingGpuModel {
    vertex_buffer: ComPtr<ID3D11Buffer>,
    index_buffer: ComPtr<ID3D11Buffer>,
    instance_buffer: Option<ComPtr<ID3D11Buffer>>,
    index_count: u32,
    instance_count: u32,
}

pub struct BuildingPreviewLayer {
    vs: ComPtr<ID3D11VertexShader>,
    ps: ComPtr<ID3D11PixelShader>,
    input_layout: ComPtr<ID3D11InputLayout>,
    camera_cb: ComPtr<ID3D11Buffer>,
    rasterizer: ComPtr<ID3D11RasterizerState>,
    models: Vec<BuildingGpuModel>,
    stats: BuildingFileDerivedLayerStats,
    map_w: f32,
    map_h: f32,
}

impl BuildingPreviewLayer {
    pub fn new(
        gfx: &Gfx,
        resolver: &GameFileResolver,
        preview_all: bool,
        map_w: usize,
        map_h: usize,
    ) -> Result<Self> {
        let layer = building_file_derived_layer_from_resolver(resolver, preview_all)?;
        unsafe {
            let vs_blob = compile_shader(BUILDING_PREVIEW_SHADER_HLSL, "vs_main", "vs_4_0")?;
            let ps_blob = compile_shader(BUILDING_PREVIEW_SHADER_HLSL, "ps_main", "ps_4_0")?;

            let mut vs: *mut ID3D11VertexShader = ptr::null_mut();
            let hr = (*gfx.device.as_raw()).CreateVertexShader(
                (*vs_blob.as_raw()).GetBufferPointer(),
                (*vs_blob.as_raw()).GetBufferSize(),
                ptr::null_mut(),
                &mut vs,
            );
            hr_ok(hr, "CreateVertexShader for building preview")?;
            let vs = ComPtr::from_raw(vs, "building preview vertex shader")?;

            let mut ps: *mut ID3D11PixelShader = ptr::null_mut();
            let hr = (*gfx.device.as_raw()).CreatePixelShader(
                (*ps_blob.as_raw()).GetBufferPointer(),
                (*ps_blob.as_raw()).GetBufferSize(),
                ptr::null_mut(),
                &mut ps,
            );
            hr_ok(hr, "CreatePixelShader for building preview")?;
            let ps = ComPtr::from_raw(ps, "building preview pixel shader")?;

            let input_layout = create_input_layout(gfx, &vs_blob)?;
            let camera_cb = create_camera_constant_buffer(gfx)?;
            let rasterizer = create_no_cull_rasterizer(gfx)?;

            let mut instances_by_model: BTreeMap<usize, Vec<&BuildingPreviewInstance>> =
                BTreeMap::new();
            for instance in &layer.instances {
                instances_by_model
                    .entry(instance.model_index)
                    .or_default()
                    .push(instance);
            }
            let mut models = Vec::with_capacity(layer.models.len());
            for (model_index, model) in layer.models.iter().enumerate() {
                let vertex_buffer = create_immutable_buffer(
                    gfx,
                    &model.vertices,
                    D3D11_BIND_VERTEX_BUFFER,
                    "building preview vertex buffer",
                )?;
                let index_bytes = u32_slice_bytes(&model.indices);
                let index_buffer = create_immutable_buffer(
                    gfx,
                    &index_bytes,
                    D3D11_BIND_INDEX_BUFFER,
                    "building preview index buffer",
                )?;
                let instances = instances_by_model
                    .get(&model_index)
                    .map(|items| instance_buffer_bytes(items))
                    .unwrap_or_default();
                let instance_buffer = if instances.is_empty() {
                    None
                } else {
                    Some(create_immutable_buffer(
                        gfx,
                        &instances,
                        D3D11_BIND_VERTEX_BUFFER,
                        "building preview instance buffer",
                    )?)
                };
                models.push(BuildingGpuModel {
                    vertex_buffer,
                    index_buffer,
                    instance_buffer,
                    index_count: u32::try_from(model.index_count)?,
                    instance_count: u32::try_from(
                        instances.len() / BUILDING_PREVIEW_INSTANCE_STRIDE_BYTES as usize,
                    )?,
                });
            }

            eprintln!(
                "replay_app: --buildings-from-files prepared {} models, {} instances (preview_all {}, coverage {}/{}, shader: {})",
                layer.stats.model_count,
                layer.stats.instance_count,
                if preview_all { "on" } else { "off" },
                layer.stats.resolved_type_count,
                layer.stats.show_on_map_type_count,
                layer.stats.shader.status
            );
            for gap in &layer.stats.gaps {
                eprintln!("replay_app: building preview gap: {gap}");
            }

            Ok(Self {
                vs,
                ps,
                input_layout,
                camera_cb,
                rasterizer,
                models,
                stats: layer.stats,
                map_w: map_w as f32,
                map_h: map_h as f32,
            })
        }
    }

    pub fn stats(&self) -> &BuildingFileDerivedLayerStats {
        &self.stats
    }

    pub fn draw(&self, gfx: &Gfx, camera: &Camera) -> Result<()> {
        if self.stats.instance_count == 0 || gfx.width == 0 || gfx.height == 0 {
            return Ok(());
        }
        unsafe {
            let constants = BuildingCameraConstants {
                screen_size: [gfx.width as f32, gfx.height as f32],
                center_map: [camera.center_x as f32, camera.center_y as f32],
                zoom: camera.zoom as f32,
                map_w: self.map_w,
                map_h: self.map_h,
                _pad: 0.0,
            };
            let mut mapped: D3D11_MAPPED_SUBRESOURCE = mem::zeroed();
            let hr = (*gfx.context.as_raw()).Map(
                self.camera_cb.as_raw() as *mut _,
                0,
                D3D11_MAP_WRITE_DISCARD,
                0,
                &mut mapped,
            );
            hr_ok(hr, "Map building preview camera constants")?;
            ptr::copy_nonoverlapping(
                &constants as *const BuildingCameraConstants as *const u8,
                mapped.pData as *mut u8,
                mem::size_of::<BuildingCameraConstants>(),
            );
            (*gfx.context.as_raw()).Unmap(self.camera_cb.as_raw() as *mut _, 0);

            let context = gfx.context.as_raw();
            let rtvs = [gfx.rtv()?.as_raw()];
            (*context).OMSetRenderTargets(1, rtvs.as_ptr(), ptr::null_mut());
            (*context).OMSetDepthStencilState(ptr::null_mut(), 0);
            let blend_factor = [0.0f32; 4];
            (*context).OMSetBlendState(ptr::null_mut(), &blend_factor, 0xFFFF_FFFF);
            (*context).RSSetState(self.rasterizer.as_raw());
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
            (*context).VSSetShader(self.vs.as_raw(), ptr::null(), 0);
            let constant_buffers = [self.camera_cb.as_raw()];
            (*context).VSSetConstantBuffers(0, 1, constant_buffers.as_ptr());
            (*context).PSSetShader(self.ps.as_raw(), ptr::null(), 0);

            for model in &self.models {
                if model.instance_count == 0 {
                    continue;
                }
                let Some(instance_buffer) = model.instance_buffer.as_ref() else {
                    continue;
                };
                let buffers = [model.vertex_buffer.as_raw(), instance_buffer.as_raw()];
                let strides = [
                    BUILDING_MESH_VERTEX_STRIDE_BYTES,
                    BUILDING_PREVIEW_INSTANCE_STRIDE_BYTES,
                ];
                let offsets = [0u32, 0u32];
                (*context).IASetVertexBuffers(
                    0,
                    2,
                    buffers.as_ptr(),
                    strides.as_ptr(),
                    offsets.as_ptr(),
                );
                (*context).IASetIndexBuffer(model.index_buffer.as_raw(), DXGI_FORMAT_R32_UINT, 0);
                (*context).DrawIndexedInstanced(model.index_count, model.instance_count, 0, 0, 0);
            }
            (*context).RSSetState(ptr::null_mut());
        }
        Ok(())
    }
}

const BUILDING_PREVIEW_INSTANCE_STRIDE_BYTES: u32 = 32;

fn instance_buffer_bytes(instances: &[&BuildingPreviewInstance]) -> Vec<u8> {
    let mut bytes =
        Vec::with_capacity(instances.len() * BUILDING_PREVIEW_INSTANCE_STRIDE_BYTES as usize);
    for instance in instances {
        push_f32s(
            &mut bytes,
            &[
                instance.pos[0],
                instance.pos[1],
                instance.pos[2],
                instance.rotation,
            ],
        );
        push_f32s(
            &mut bytes,
            &[
                instance.color[0],
                instance.color[1],
                instance.color[2],
                instance.color[3],
            ],
        );
    }
    bytes
}

fn u32_slice_bytes(values: &[u32]) -> Vec<u8> {
    let mut bytes = Vec::with_capacity(values.len() * 4);
    for value in values {
        bytes.extend_from_slice(&value.to_le_bytes());
    }
    bytes
}

fn push_f32s<const N: usize>(bytes: &mut Vec<u8>, values: &[f32; N]) {
    for value in values {
        bytes.extend_from_slice(&value.to_le_bytes());
    }
}

unsafe fn create_input_layout(
    gfx: &Gfx,
    vs_blob: &ComPtr<winapi::um::d3dcommon::ID3DBlob>,
) -> Result<ComPtr<ID3D11InputLayout>> {
    let position = CString::new("POSITION").expect("static semantic");
    let normal = CString::new("NORMAL").expect("static semantic");
    let tangent = CString::new("TANGENT").expect("static semantic");
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
            SemanticName: normal.as_ptr(),
            SemanticIndex: 0,
            Format: DXGI_FORMAT_R32G32B32_FLOAT,
            InputSlot: 0,
            AlignedByteOffset: 12,
            InputSlotClass: D3D11_INPUT_PER_VERTEX_DATA,
            InstanceDataStepRate: 0,
        },
        D3D11_INPUT_ELEMENT_DESC {
            SemanticName: tangent.as_ptr(),
            SemanticIndex: 0,
            Format: DXGI_FORMAT_R32G32B32A32_FLOAT,
            InputSlot: 0,
            AlignedByteOffset: 24,
            InputSlotClass: D3D11_INPUT_PER_VERTEX_DATA,
            InstanceDataStepRate: 0,
        },
        D3D11_INPUT_ELEMENT_DESC {
            SemanticName: texcoord.as_ptr(),
            SemanticIndex: 0,
            Format: DXGI_FORMAT_R32G32_FLOAT,
            InputSlot: 0,
            AlignedByteOffset: 40,
            InputSlotClass: D3D11_INPUT_PER_VERTEX_DATA,
            InstanceDataStepRate: 0,
        },
        D3D11_INPUT_ELEMENT_DESC {
            SemanticName: texcoord.as_ptr(),
            SemanticIndex: 1,
            Format: DXGI_FORMAT_R32G32B32A32_FLOAT,
            InputSlot: 1,
            AlignedByteOffset: 0,
            InputSlotClass: D3D11_INPUT_PER_INSTANCE_DATA,
            InstanceDataStepRate: 1,
        },
        D3D11_INPUT_ELEMENT_DESC {
            SemanticName: texcoord.as_ptr(),
            SemanticIndex: 2,
            Format: DXGI_FORMAT_R32G32B32A32_FLOAT,
            InputSlot: 1,
            AlignedByteOffset: 16,
            InputSlotClass: D3D11_INPUT_PER_INSTANCE_DATA,
            InstanceDataStepRate: 1,
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
    hr_ok(hr, "CreateInputLayout for building preview")?;
    ComPtr::from_raw(input_layout, "building preview input layout")
}

unsafe fn create_immutable_buffer(
    gfx: &Gfx,
    bytes: &[u8],
    bind_flags: u32,
    role: &str,
) -> Result<ComPtr<ID3D11Buffer>> {
    if bytes.is_empty() {
        return Err(format!("{role} cannot be empty").into());
    }
    let desc = D3D11_BUFFER_DESC {
        ByteWidth: u32::try_from(bytes.len())?,
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
    let hr = (*gfx.device.as_raw()).CreateBuffer(&desc, &init, &mut buffer);
    hr_ok(hr, &format!("CreateBuffer for {role}"))?;
    ComPtr::from_raw(buffer, role)
}

unsafe fn create_camera_constant_buffer(gfx: &Gfx) -> Result<ComPtr<ID3D11Buffer>> {
    let desc = D3D11_BUFFER_DESC {
        ByteWidth: mem::size_of::<BuildingCameraConstants>() as UINT,
        Usage: D3D11_USAGE_DYNAMIC,
        BindFlags: D3D11_BIND_CONSTANT_BUFFER,
        CPUAccessFlags: D3D11_CPU_ACCESS_WRITE,
        MiscFlags: 0,
        StructureByteStride: 0,
    };
    let mut buffer: *mut ID3D11Buffer = ptr::null_mut();
    let hr = (*gfx.device.as_raw()).CreateBuffer(&desc, ptr::null(), &mut buffer);
    hr_ok(hr, "CreateBuffer for building preview camera constants")?;
    ComPtr::from_raw(buffer, "building preview camera constant buffer")
}

unsafe fn create_no_cull_rasterizer(gfx: &Gfx) -> Result<ComPtr<ID3D11RasterizerState>> {
    let mut desc: D3D11_RASTERIZER_DESC = mem::zeroed();
    desc.FillMode = D3D11_FILL_SOLID;
    desc.CullMode = D3D11_CULL_NONE;
    desc.DepthClipEnable = 1;
    let mut rasterizer: *mut ID3D11RasterizerState = ptr::null_mut();
    let hr = (*gfx.device.as_raw()).CreateRasterizerState(&desc, &mut rasterizer);
    hr_ok(hr, "CreateRasterizerState for building preview")?;
    ComPtr::from_raw(rasterizer, "building preview rasterizer state")
}
