
use crate::{
    PreparedPoliticalFrame, Result, TerrainDependentResourceBinding, Xmap05GeneratedTexture,
};
use snapshot::sha256::Sha256;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Xmap05D3dHandoffReceipt {
    pub political_frame_sha256: String,
    pub uploaded_resources: u32,
    pub ordered_resource_sha256: Vec<String>,
    pub ordered_descriptor_sha256: Vec<String>,
    pub terrain_dependency_bindings: Vec<TerrainDependentResourceBinding>,
    pub deterministic_sha256: String,
}

#[cfg(windows)]
pub struct NativeXmap05PoliticalHandoff {
    resources: Vec<NativeXmap05TextureUpload>,
    receipt: Xmap05D3dHandoffReceipt,
}

#[cfg(windows)]
#[derive(Clone, Copy)]
pub struct NativeXmap05ShaderResource<'a> {
    pub owner_packet: &'static str,
    pub symbol: &'a str,
    pub resource_sha256: &'a str,
    pub descriptor_sha256: &'a str,
    pub width: u32,
    pub height: u32,
    pub dxgi_format: u32,
    pub shader_resource_view: *mut winapi::um::d3d11::ID3D11ShaderResourceView,
}

#[cfg(windows)]
struct NativeXmap05TextureUpload {
    symbol: String,
    resource_sha256: String,
    descriptor_sha256: String,
    width: u32,
    height: u32,
    dxgi_format: u32,
    _texture: crate::d3d11_device::D3dPtr<winapi::um::d3d11::ID3D11Texture2D>,
    view: crate::d3d11_device::D3dPtr<winapi::um::d3d11::ID3D11ShaderResourceView>,
}

#[cfg(windows)]
impl NativeXmap05PoliticalHandoff {
    pub unsafe fn create_on_device(
        device: *mut winapi::um::d3d11::ID3D11Device,
        frame: &PreparedPoliticalFrame,
    ) -> Result<Self> {
        frame.validate()?;
        if device.is_null() {
            return Err("XMAP05 app-device handoff received a null D3D11 device".into());
        }
        let resources = upload_all(device, frame)?;
        let receipt = handoff_receipt(frame);
        Ok(Self { resources, receipt })
    }

    pub unsafe fn update_frame(
        &mut self,
        device: *mut winapi::um::d3d11::ID3D11Device,
        frame: &PreparedPoliticalFrame,
    ) -> Result<&Xmap05D3dHandoffReceipt> {
        frame.validate()?;
        if frame.deterministic_sha256 == self.receipt.political_frame_sha256 {
            return Ok(&self.receipt);
        }
        let replacement = upload_all(device, frame)?;
        self.resources = replacement;
        self.receipt = handoff_receipt(frame);
        Ok(&self.receipt)
    }

    pub fn receipt(&self) -> &Xmap05D3dHandoffReceipt {
        &self.receipt
    }

    pub(crate) fn shader_resource_view(
        &self,
        symbol: &str,
    ) -> Result<*mut winapi::um::d3d11::ID3D11ShaderResourceView> {
        Ok(self.shader_resource(symbol)?.shader_resource_view)
    }

    pub fn shader_resource(&self, symbol: &str) -> Result<NativeXmap05ShaderResource<'_>> {
        let resource = self
            .resources
            .iter()
            .find(|resource| resource.symbol == symbol)
            .ok_or_else(|| format!("XMAP05 app-device handoff omitted SRV {symbol}"))?;
        Ok(NativeXmap05ShaderResource {
            owner_packet: "XMAP05",
            symbol: &resource.symbol,
            resource_sha256: &resource.resource_sha256,
            descriptor_sha256: &resource.descriptor_sha256,
            width: resource.width,
            height: resource.height,
            dxgi_format: resource.dxgi_format,
            shader_resource_view: resource.view.as_raw(),
        })
    }

    pub fn province_secondary_color_map(&self) -> Result<NativeXmap05ShaderResource<'_>> {
        self.shader_resource(crate::PROVINCE_SECONDARY_COLOR_MAP_SYMBOL)
    }

    pub fn gradient_border_channel1(&self) -> Result<NativeXmap05ShaderResource<'_>> {
        self.shader_resource("GradientBorderChannel1")
    }

    pub fn gradient_border_channel2(&self) -> Result<NativeXmap05ShaderResource<'_>> {
        self.shader_resource("GradientBorderChannel2")
    }

    pub fn gradient_border_channel3(&self) -> Result<NativeXmap05ShaderResource<'_>> {
        self.shader_resource("GradientBorderChannel3")
    }
}

#[cfg(windows)]
unsafe fn upload_all(
    device: *mut winapi::um::d3d11::ID3D11Device,
    frame: &PreparedPoliticalFrame,
) -> Result<Vec<NativeXmap05TextureUpload>> {
    [
        &frame.province_primary,
        &frame.province_secondary,
        &frame.gradient_border_channel1,
        &frame.gradient_border_channel2,
        &frame.gradient_border_channel3,
        &frame.intel_map,
    ]
    .into_iter()
    .map(|texture| create_texture(device, texture))
    .collect()
}

#[cfg(windows)]
unsafe fn create_texture(
    device: *mut winapi::um::d3d11::ID3D11Device,
    texture: &Xmap05GeneratedTexture,
) -> Result<NativeXmap05TextureUpload> {
    use crate::d3d11_device::{native_d3d11_hr, D3dPtr};
    use std::ptr;
    use winapi::shared::dxgitype::DXGI_SAMPLE_DESC;
    use winapi::um::d3d11::{
        ID3D11ShaderResourceView, ID3D11Texture2D, D3D11_BIND_SHADER_RESOURCE,
        D3D11_SUBRESOURCE_DATA, D3D11_TEXTURE2D_DESC, D3D11_USAGE_DEFAULT,
    };

    let descriptor = &texture.descriptor;
    let native = D3D11_TEXTURE2D_DESC {
        Width: descriptor.width,
        Height: descriptor.height,
        MipLevels: descriptor.mip_levels,
        ArraySize: 1,
        Format: descriptor.dxgi_format,
        SampleDesc: DXGI_SAMPLE_DESC {
            Count: 1,
            Quality: 0,
        },
        Usage: D3D11_USAGE_DEFAULT,
        BindFlags: D3D11_BIND_SHADER_RESOURCE,
        CPUAccessFlags: 0,
        MiscFlags: 0,
    };
    let initial = D3D11_SUBRESOURCE_DATA {
        pSysMem: texture.bytes.as_ptr() as *const _,
        SysMemPitch: descriptor.row_pitch,
        SysMemSlicePitch: u32::try_from(texture.bytes.len())?,
    };
    let mut raw_texture = ptr::null_mut::<ID3D11Texture2D>();
    native_d3d11_hr(
        (*device).CreateTexture2D(&native, &initial, &mut raw_texture),
        &format!("CreateTexture2D(XMAP05 {})", descriptor.symbol),
    )?;
    let texture_ptr = D3dPtr::from_raw(raw_texture, "XMAP05 generated texture")?;
    let mut raw_view = ptr::null_mut::<ID3D11ShaderResourceView>();
    native_d3d11_hr(
        (*device).CreateShaderResourceView(
            texture_ptr.as_raw() as *mut _,
            ptr::null(),
            &mut raw_view,
        ),
        &format!("CreateShaderResourceView(XMAP05 {})", descriptor.symbol),
    )?;
    Ok(NativeXmap05TextureUpload {
        symbol: descriptor.symbol.clone(),
        resource_sha256: descriptor.payload_sha256.clone(),
        descriptor_sha256: descriptor.descriptor_sha256.clone(),
        width: descriptor.width,
        height: descriptor.height,
        dxgi_format: descriptor.dxgi_format,
        _texture: texture_ptr,
        view: D3dPtr::from_raw(raw_view, "XMAP05 generated SRV")?,
    })
}

fn handoff_receipt(frame: &PreparedPoliticalFrame) -> Xmap05D3dHandoffReceipt {
    let textures = [
        &frame.province_primary,
        &frame.province_secondary,
        &frame.gradient_border_channel1,
        &frame.gradient_border_channel2,
        &frame.gradient_border_channel3,
        &frame.intel_map,
    ];
    let ordered_resource_sha256 = textures
        .iter()
        .map(|texture| texture.descriptor.payload_sha256.clone())
        .collect::<Vec<_>>();
    let ordered_descriptor_sha256 = textures
        .iter()
        .map(|texture| texture.descriptor.descriptor_sha256.clone())
        .collect::<Vec<_>>();
    let terrain_dependency_bindings = frame.terrain_dependency_bindings();
    let mut identity = Vec::new();
    identity.extend_from_slice(frame.deterministic_sha256.as_bytes());
    for hash in ordered_resource_sha256
        .iter()
        .chain(&ordered_descriptor_sha256)
    {
        identity.extend_from_slice(hash.as_bytes());
    }
    for binding in &terrain_dependency_bindings {
        identity.extend_from_slice(binding.symbol.as_bytes());
        identity.extend_from_slice(binding.resource_sha256.as_bytes());
        identity.extend_from_slice(binding.descriptor_sha256.as_bytes());
    }
    Xmap05D3dHandoffReceipt {
        political_frame_sha256: frame.deterministic_sha256.clone(),
        uploaded_resources: textures.len() as u32,
        ordered_resource_sha256,
        ordered_descriptor_sha256,
        terrain_dependency_bindings,
        deterministic_sha256: Sha256::hex(&identity),
    }
}
