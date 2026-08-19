//! Release handling via [`ComPtr`], matching the map_viewer house style.

use std::ptr;
use std::sync::atomic::{AtomicU64, Ordering};

use sha2::{Digest, Sha256};
use winapi::ctypes::c_void;
use winapi::shared::dxgi::{
    IDXGIAdapter, IDXGIDevice, IDXGISwapChain, DXGI_ADAPTER_DESC, DXGI_PRESENT_DO_NOT_WAIT,
    DXGI_SWAP_CHAIN_DESC, DXGI_SWAP_EFFECT_DISCARD, DXGI_SWAP_EFFECT_FLIP_DISCARD,
};
use winapi::shared::dxgiformat::{DXGI_FORMAT_B8G8R8A8_UNORM, DXGI_FORMAT_UNKNOWN};
use winapi::shared::dxgitype::{
    DXGI_MODE_DESC, DXGI_RATIONAL, DXGI_SAMPLE_DESC, DXGI_USAGE_RENDER_TARGET_OUTPUT,
};
use winapi::shared::guiddef::GUID;
use winapi::shared::minwindef::UINT;
use winapi::shared::windef::HWND;
use winapi::shared::winerror::{
    DXGI_ERROR_DEVICE_HUNG, DXGI_ERROR_DEVICE_REMOVED, DXGI_ERROR_DEVICE_RESET,
    DXGI_ERROR_WAS_STILL_DRAWING, DXGI_STATUS_OCCLUDED,
};
use winapi::um::d3d11::{
    D3D11CreateDevice, D3D11CreateDeviceAndSwapChain, ID3D11Asynchronous, ID3D11Device,
    ID3D11DeviceContext, ID3D11Query, ID3D11RenderTargetView, ID3D11Texture2D,
    D3D11_BIND_RENDER_TARGET, D3D11_CPU_ACCESS_READ, D3D11_CREATE_DEVICE_BGRA_SUPPORT,
    D3D11_MAPPED_SUBRESOURCE, D3D11_MAP_READ, D3D11_QUERY_DESC, D3D11_QUERY_EVENT,
    D3D11_SDK_VERSION, D3D11_TEXTURE2D_DESC, D3D11_USAGE_DEFAULT, D3D11_USAGE_STAGING,
};
use winapi::um::d3dcommon::{
    D3D_DRIVER_TYPE_HARDWARE, D3D_FEATURE_LEVEL_10_0, D3D_FEATURE_LEVEL_10_1,
    D3D_FEATURE_LEVEL_11_0,
};
use winapi::um::unknwnbase::IUnknown;
use winapi::Interface;

use crate::Result;

/// Owning COM pointer with Release on drop (map_viewer `D3dPtr` pattern).
pub struct ComPtr<T>(*mut T);

impl<T> ComPtr<T> {
    pub unsafe fn from_raw(ptr: *mut T, role: &str) -> Result<Self> {
        if ptr.is_null() {
            Err(format!("replay_app D3D11: {role} returned null").into())
        } else {
            Ok(Self(ptr))
        }
    }

    pub fn as_raw(&self) -> *mut T {
        self.0
    }
}

impl<T> Drop for ComPtr<T> {
    fn drop(&mut self) {
        unsafe {
            if !self.0.is_null() {
                (*(self.0 as *mut IUnknown)).Release();
            }
        }
    }
}

pub fn hr_ok(hr: i32, action: &str) -> Result<()> {
    if hr < 0 {
        if let Some(loss) = DeviceLoss::from_hresult(hr) {
            return Err(Box::new(DeviceLossError::new(action, loss, hr, None)));
        }
        Err(format!(
            "replay_app D3D11: {action} failed with HRESULT 0x{:08X}",
            hr as u32
        )
        .into())
    } else {
        Ok(())
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum DeviceLoss {
    Removed,
    Reset,
    Hung,
}

impl DeviceLoss {
    fn from_hresult(hr: i32) -> Option<Self> {
        match hr {
            DXGI_ERROR_DEVICE_REMOVED => Some(Self::Removed),
            DXGI_ERROR_DEVICE_RESET => Some(Self::Reset),
            DXGI_ERROR_DEVICE_HUNG => Some(Self::Hung),
            _ => None,
        }
    }

    const fn label(self) -> &'static str {
        match self {
            Self::Removed => "removed",
            Self::Reset => "reset",
            Self::Hung => "hung",
        }
    }
}

#[derive(Debug)]
pub struct DeviceLossError {
    action: String,
    loss: DeviceLoss,
    operation_hr: i32,
    removed_reason: Option<i32>,
}

impl DeviceLossError {
    fn new(action: &str, loss: DeviceLoss, operation_hr: i32, removed_reason: Option<i32>) -> Self {
        Self {
            action: action.to_string(),
            loss,
            operation_hr,
            removed_reason,
        }
    }
}

impl std::fmt::Display for DeviceLossError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "replay_app D3D11: {} reported device {} (HRESULT 0x{:08X}",
            self.action,
            self.loss.label(),
            self.operation_hr as u32,
        )?;
        if let Some(reason) = self.removed_reason {
            write!(f, "; GetDeviceRemovedReason 0x{:08X}", reason as u32)?;
        }
        write!(
            f,
            "); rendering stopped fail-closed because device-bound replay resources cannot be reused; restart replay_app"
        )
    }
}

impl std::error::Error for DeviceLossError {}

#[derive(Debug)]
pub struct DeviceUnavailableError {
    action: String,
    removed_reason: i32,
}

impl std::fmt::Display for DeviceUnavailableError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "replay_app D3D11: {} left the device unavailable (GetDeviceRemovedReason 0x{:08X}); rendering stopped fail-closed because the upload result cannot be admitted; restart replay_app",
            self.action,
            self.removed_reason as u32,
        )
    }
}

impl std::error::Error for DeviceUnavailableError {}

fn void_device_call_result(device_removed_reason: i32, action: &str) -> Result<()> {
    if device_removed_reason >= 0 {
        return Ok(());
    }
    if let Some(loss) = DeviceLoss::from_hresult(device_removed_reason) {
        return Err(Box::new(DeviceLossError::new(
            action,
            loss,
            device_removed_reason,
            Some(device_removed_reason),
        )));
    }
    Err(Box::new(DeviceUnavailableError {
        action: action.to_string(),
        removed_reason: device_removed_reason,
    }))
}

pub fn is_device_loss_error(error: &(dyn std::error::Error + 'static)) -> bool {
    let mut current = Some(error);
    while let Some(candidate) = current {
        if candidate.downcast_ref::<DeviceLossError>().is_some()
            || candidate.downcast_ref::<DeviceUnavailableError>().is_some()
        {
            return true;
        }
        current = candidate.source();
    }
    false
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PresentOutcome {
    Presented,
    Skipped,
    Occluded,
}

pub struct Gfx {
    pub device: ComPtr<ID3D11Device>,
    pub context: ComPtr<ID3D11DeviceContext>,
    swapchain: Option<ComPtr<IDXGISwapChain>>,
    target_texture: Option<ComPtr<ID3D11Texture2D>>,
    rtv: Option<ComPtr<ID3D11RenderTargetView>>,
    target_allocation_generation: u64,
    target_resource_identity_sha256: String,
    pub width: u32,
    pub height: u32,
}

static NEXT_PRESENTATION_ALLOCATION_SERIAL: AtomicU64 = AtomicU64::new(1);

const REPLAY_PRESENTATION_RESOURCE_IDENTITY_GUID: GUID = GUID {
    Data1: 0x7d3f2c91,
    Data2: 0x8b2e,
    Data3: 0x4f6d,
    Data4: [0xa1, 0x73, 0x24, 0x68, 0x91, 0xcf, 0x52, 0x0b],
};

pub(crate) struct GpuCompletionQuery {
    query: ComPtr<ID3D11Query>,
}

#[derive(Debug, Clone)]
pub(crate) struct GpuAdapterIdentity {
    pub(crate) name: String,
    pub(crate) dedicated_video_memory_bytes: u64,
    pub(crate) vendor_id: u32,
    pub(crate) device_id: u32,
}

fn swapchain_desc(hwnd: HWND, width: u32, height: u32, swap_effect: u32) -> DXGI_SWAP_CHAIN_DESC {
    DXGI_SWAP_CHAIN_DESC {
        BufferDesc: DXGI_MODE_DESC {
            Width: width,
            Height: height,
            RefreshRate: DXGI_RATIONAL {
                Numerator: 0,
                Denominator: 1,
            },
            Format: DXGI_FORMAT_B8G8R8A8_UNORM,
            ScanlineOrdering: 0,
            Scaling: 0,
        },
        SampleDesc: DXGI_SAMPLE_DESC {
            Count: 1,
            Quality: 0,
        },
        BufferUsage: DXGI_USAGE_RENDER_TARGET_OUTPUT,
        BufferCount: 2,
        OutputWindow: hwnd,
        Windowed: 1,
        SwapEffect: swap_effect,
        Flags: 0,
    }
}

unsafe fn stamp_presentation_resource_identity(
    texture: *mut ID3D11Texture2D,
    allocation_generation: u64,
) -> Result<String> {
    if texture.is_null() || allocation_generation == 0 {
        return Err("cannot identify a null presentation allocation".into());
    }
    let mut desc: D3D11_TEXTURE2D_DESC = std::mem::zeroed();
    (*texture).GetDesc(&mut desc);
    let serial = NEXT_PRESENTATION_ALLOCATION_SERIAL.fetch_add(1, Ordering::SeqCst);
    if serial == 0 {
        return Err("presentation allocation serial overflowed".into());
    }
    let mut hasher = Sha256::new();
    hasher.update(b"hoi4.replay.presentation-resource-allocation.v2\0");
    hasher.update(serial.to_le_bytes());
    hasher.update(allocation_generation.to_le_bytes());
    hasher.update(desc.Width.to_le_bytes());
    hasher.update(desc.Height.to_le_bytes());
    hasher.update((desc.Format as u32).to_le_bytes());
    hasher.update(desc.SampleDesc.Count.to_le_bytes());
    hasher.update(desc.SampleDesc.Quality.to_le_bytes());
    let identity = hasher
        .finalize()
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect::<String>();
    let mut tag = Vec::with_capacity(72);
    tag.extend_from_slice(&allocation_generation.to_le_bytes());
    tag.extend_from_slice(identity.as_bytes());
    hr_ok(
        (*texture).SetPrivateData(
            &REPLAY_PRESENTATION_RESOURCE_IDENTITY_GUID,
            u32::try_from(tag.len())?,
            tag.as_ptr().cast(),
        ),
        "SetPrivateData (presentation resource identity)",
    )?;
    Ok(identity)
}

unsafe fn preferred_adapter() -> Option<ComPtr<winapi::shared::dxgi::IDXGIAdapter1>> {
    use winapi::shared::dxgi::{CreateDXGIFactory1, IDXGIAdapter1, IDXGIFactory1};
    use winapi::Interface;
    let mut factory: *mut IDXGIFactory1 = ptr::null_mut();
    if CreateDXGIFactory1(&IDXGIFactory1::uuidof(), &mut factory as *mut _ as *mut _) < 0
        || factory.is_null()
    {
        return None;
    }
    let factory = ComPtr::from_raw(factory, "DXGI factory").ok()?;
    let mut best: Option<(usize, ComPtr<IDXGIAdapter1>)> = None;
    let mut index = 0u32;
    loop {
        let mut adapter: *mut IDXGIAdapter1 = ptr::null_mut();
        if (*factory.as_raw()).EnumAdapters1(index, &mut adapter) < 0 || adapter.is_null() {
            break;
        }
        index += 1;
        let Ok(adapter) = ComPtr::from_raw(adapter, "DXGI adapter") else {
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
        if best
            .as_ref()
            .map(|(memory, _)| desc.DedicatedVideoMemory > *memory)
            .unwrap_or(true)
        {
            best = Some((desc.DedicatedVideoMemory, adapter));
        }
    }
    best.map(|(_, adapter)| adapter)
}

impl Gfx {
    pub(crate) const fn target_format(&self) -> u32 {
        DXGI_FORMAT_B8G8R8A8_UNORM
    }

    pub(crate) fn target_allocation_generation(&self) -> u64 {
        self.target_allocation_generation
    }

    pub(crate) fn target_resource_identity_sha256(&self) -> Result<&str> {
        if self.target_resource_identity_sha256.len() != 64 {
            return Err("presentation target has no concrete resource identity".into());
        }
        Ok(&self.target_resource_identity_sha256)
    }

    pub(crate) fn adapter_identity(&self) -> Result<GpuAdapterIdentity> {
        unsafe {
            let mut dxgi_device: *mut IDXGIDevice = ptr::null_mut();
            let hr = (*(self.device.as_raw() as *mut IUnknown)).QueryInterface(
                &IDXGIDevice::uuidof(),
                &mut dxgi_device as *mut _ as *mut *mut c_void,
            );
            self.runtime_hr(hr, "QueryInterface IDXGIDevice")?;
            let dxgi_device = ComPtr::from_raw(dxgi_device, "DXGI device")?;
            let mut adapter: *mut IDXGIAdapter = ptr::null_mut();
            let hr = (*dxgi_device.as_raw()).GetAdapter(&mut adapter);
            self.runtime_hr(hr, "IDXGIDevice::GetAdapter")?;
            let adapter = ComPtr::from_raw(adapter, "DXGI device adapter")?;
            let mut desc: DXGI_ADAPTER_DESC = std::mem::zeroed();
            let hr = (*adapter.as_raw()).GetDesc(&mut desc);
            self.runtime_hr(hr, "IDXGIAdapter::GetDesc")?;
            let name_len = desc
                .Description
                .iter()
                .position(|value| *value == 0)
                .unwrap_or(desc.Description.len());
            let name = String::from_utf16(&desc.Description[..name_len])?;
            if name.trim().is_empty() {
                return Err("DXGI adapter description is empty".into());
            }
            Ok(GpuAdapterIdentity {
                name,
                dedicated_video_memory_bytes: desc.DedicatedVideoMemory as u64,
                vendor_id: desc.VendorId,
                device_id: desc.DeviceId,
            })
        }
    }

    fn runtime_hr(&self, hr: i32, action: &str) -> Result<()> {
        if hr >= 0 {
            return Ok(());
        }
        let removed_reason = unsafe { (*self.device.as_raw()).GetDeviceRemovedReason() };
        if let Some(loss) =
            DeviceLoss::from_hresult(hr).or_else(|| DeviceLoss::from_hresult(removed_reason))
        {
            return Err(Box::new(DeviceLossError::new(
                action,
                loss,
                hr,
                Some(removed_reason),
            )));
        }
        hr_ok(hr, action)
    }

    pub(crate) fn check_void_device_call(&self, action: &str) -> Result<()> {
        let removed_reason = unsafe { (*self.device.as_raw()).GetDeviceRemovedReason() };
        void_device_call_result(removed_reason, action)
    }

    pub fn clear_target(&self, color: [f32; 4]) -> Result<()> {
        unsafe {
            let rtv = self.rtv()?.as_raw();
            (*self.context.as_raw()).ClearRenderTargetView(rtv, &color);
            (*self.context.as_raw()).OMSetRenderTargets(1, &rtv, ptr::null_mut());
        }
        Ok(())
    }

    pub fn create(hwnd: HWND, width: u32, height: u32) -> Result<Gfx> {
        unsafe {
            let feature_levels = [
                D3D_FEATURE_LEVEL_11_0,
                D3D_FEATURE_LEVEL_10_1,
                D3D_FEATURE_LEVEL_10_0,
            ];
            let adapter = preferred_adapter();
            let (adapter_ptr, driver_type) = match adapter.as_ref() {
                Some(adapter) => (
                    adapter.as_raw() as *mut winapi::shared::dxgi::IDXGIAdapter,
                    winapi::um::d3dcommon::D3D_DRIVER_TYPE_UNKNOWN,
                ),
                None => (ptr::null_mut(), D3D_DRIVER_TYPE_HARDWARE),
            };
            let mut created = None;
            for swap_effect in [DXGI_SWAP_EFFECT_FLIP_DISCARD, DXGI_SWAP_EFFECT_DISCARD] {
                let desc = swapchain_desc(hwnd, width.max(1), height.max(1), swap_effect);
                let mut swapchain: *mut IDXGISwapChain = ptr::null_mut();
                let mut device: *mut ID3D11Device = ptr::null_mut();
                let mut context: *mut ID3D11DeviceContext = ptr::null_mut();
                let mut feature_level = D3D_FEATURE_LEVEL_11_0;
                let hr = D3D11CreateDeviceAndSwapChain(
                    adapter_ptr,
                    driver_type,
                    ptr::null_mut(),
                    D3D11_CREATE_DEVICE_BGRA_SUPPORT,
                    feature_levels.as_ptr(),
                    feature_levels.len() as UINT,
                    D3D11_SDK_VERSION,
                    &desc,
                    &mut swapchain,
                    &mut device,
                    &mut feature_level,
                    &mut context,
                );
                if hr >= 0 {
                    created = Some((
                        ComPtr::from_raw(swapchain, "DXGI swapchain")?,
                        ComPtr::from_raw(device, "D3D11 device")?,
                        ComPtr::from_raw(context, "D3D11 immediate context")?,
                    ));
                    break;
                }
            }
            let (swapchain, device, context) = created.ok_or(
                "D3D11CreateDeviceAndSwapChain failed for both flip-model and blt-model swapchains",
            )?;
            let mut gfx = Gfx {
                device,
                context,
                swapchain: Some(swapchain),
                target_texture: None,
                rtv: None,
                target_allocation_generation: 1,
                target_resource_identity_sha256: String::new(),
                width: width.max(1),
                height: height.max(1),
            };
            gfx.create_backbuffer_rtv()?;
            Ok(gfx)
        }
    }

    pub fn create_offscreen(width: u32, height: u32) -> Result<Gfx> {
        unsafe {
            let feature_levels = [
                D3D_FEATURE_LEVEL_11_0,
                D3D_FEATURE_LEVEL_10_1,
                D3D_FEATURE_LEVEL_10_0,
            ];
            let mut device: *mut ID3D11Device = ptr::null_mut();
            let mut context: *mut ID3D11DeviceContext = ptr::null_mut();
            let mut feature_level = D3D_FEATURE_LEVEL_11_0;
            let adapter = preferred_adapter();
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
            hr_ok(hr, "D3D11CreateDevice (offscreen)")?;
            let device = ComPtr::from_raw(device, "D3D11 device")?;
            let context = ComPtr::from_raw(context, "D3D11 immediate context")?;

            let width = width.max(1);
            let height = height.max(1);
            let desc = D3D11_TEXTURE2D_DESC {
                Width: width,
                Height: height,
                MipLevels: 1,
                ArraySize: 1,
                Format: DXGI_FORMAT_B8G8R8A8_UNORM,
                SampleDesc: DXGI_SAMPLE_DESC {
                    Count: 1,
                    Quality: 0,
                },
                Usage: D3D11_USAGE_DEFAULT,
                BindFlags: D3D11_BIND_RENDER_TARGET,
                CPUAccessFlags: 0,
                MiscFlags: 0,
            };
            let mut texture: *mut ID3D11Texture2D = ptr::null_mut();
            let hr = (*device.as_raw()).CreateTexture2D(&desc, ptr::null(), &mut texture);
            hr_ok(hr, "CreateTexture2D (offscreen render target)")?;
            let texture = ComPtr::from_raw(texture, "offscreen render target")?;

            let mut rtv: *mut ID3D11RenderTargetView = ptr::null_mut();
            let hr = (*device.as_raw()).CreateRenderTargetView(
                texture.as_raw() as *mut _,
                ptr::null(),
                &mut rtv,
            );
            hr_ok(hr, "CreateRenderTargetView (offscreen)")?;
            let rtv = ComPtr::from_raw(rtv, "offscreen render-target view")?;
            let target_resource_identity_sha256 =
                stamp_presentation_resource_identity(texture.as_raw(), 1)?;

            Ok(Gfx {
                device,
                context,
                swapchain: None,
                target_texture: Some(texture),
                rtv: Some(rtv),
                target_allocation_generation: 1,
                target_resource_identity_sha256,
                width,
                height,
            })
        }
    }

    unsafe fn create_backbuffer_rtv(&mut self) -> Result<()> {
        let swapchain = self
            .swapchain
            .as_ref()
            .ok_or("create_backbuffer_rtv called on an offscreen target")?;
        let mut backbuffer: *mut ID3D11Texture2D = ptr::null_mut();
        let hr = (*swapchain.as_raw()).GetBuffer(
            0,
            &ID3D11Texture2D::uuidof(),
            &mut backbuffer as *mut *mut ID3D11Texture2D as *mut *mut _,
        );
        self.runtime_hr(hr, "IDXGISwapChain::GetBuffer(0)")?;
        let backbuffer = ComPtr::from_raw(backbuffer, "swapchain backbuffer")?;
        let target_resource_identity_sha256 = stamp_presentation_resource_identity(
            backbuffer.as_raw(),
            self.target_allocation_generation,
        )?;
        let mut rtv: *mut ID3D11RenderTargetView = ptr::null_mut();
        let hr = (*self.device.as_raw()).CreateRenderTargetView(
            backbuffer.as_raw() as *mut _,
            ptr::null(),
            &mut rtv,
        );
        self.runtime_hr(hr, "CreateRenderTargetView for swapchain backbuffer")?;
        self.rtv = Some(ComPtr::from_raw(rtv, "backbuffer render-target view")?);
        self.target_texture = Some(backbuffer);
        self.target_resource_identity_sha256 = target_resource_identity_sha256;
        Ok(())
    }

    pub fn rtv(&self) -> Result<&ComPtr<ID3D11RenderTargetView>> {
        self.rtv
            .as_ref()
            .ok_or_else(|| "render-target view is not initialized".into())
    }

    pub fn resize(&mut self, width: u32, height: u32) -> Result<()> {
        if self.swapchain.is_none() {
            return Ok(());
        }
        if width == 0 || height == 0 || (width == self.width && height == self.height) {
            return Ok(());
        }
        unsafe {
            // The backbuffer view must be released before ResizeBuffers.
            self.rtv = None;
            self.target_texture = None;
            (*self.context.as_raw()).ClearState();
            let swapchain = self
                .swapchain
                .as_ref()
                .ok_or("resize called on an offscreen target")?;
            let hr = (*swapchain.as_raw()).ResizeBuffers(0, width, height, DXGI_FORMAT_UNKNOWN, 0);
            self.runtime_hr(hr, "IDXGISwapChain::ResizeBuffers")?;
            self.width = width;
            self.height = height;
            self.target_allocation_generation = self
                .target_allocation_generation
                .checked_add(1)
                .ok_or("presentation target allocation generation overflowed")?;
            self.create_backbuffer_rtv()
        }
    }

    pub fn present(&self) -> Result<PresentOutcome> {
        let Some(swapchain) = self.swapchain.as_ref() else {
            return Ok(PresentOutcome::Presented);
        };
        unsafe {
            let hr = (*swapchain.as_raw()).Present(1, DXGI_PRESENT_DO_NOT_WAIT);
            if hr == DXGI_ERROR_WAS_STILL_DRAWING {
                return Ok(PresentOutcome::Skipped);
            }
            if hr == DXGI_STATUS_OCCLUDED {
                return Ok(PresentOutcome::Occluded);
            }
            self.runtime_hr(hr, "IDXGISwapChain::Present")?;
            Ok(PresentOutcome::Presented)
        }
    }

    pub fn read_target(&self) -> Result<Vec<u8>> {
        unsafe {
            let source = self
                .target_texture
                .as_ref()
                .ok_or("product target texture is not retained for readback")?;
            let desc = D3D11_TEXTURE2D_DESC {
                Width: self.width,
                Height: self.height,
                MipLevels: 1,
                ArraySize: 1,
                Format: DXGI_FORMAT_B8G8R8A8_UNORM,
                SampleDesc: DXGI_SAMPLE_DESC {
                    Count: 1,
                    Quality: 0,
                },
                Usage: D3D11_USAGE_STAGING,
                BindFlags: 0,
                CPUAccessFlags: D3D11_CPU_ACCESS_READ,
                MiscFlags: 0,
            };
            let mut staging: *mut ID3D11Texture2D = ptr::null_mut();
            let hr = (*self.device.as_raw()).CreateTexture2D(&desc, ptr::null(), &mut staging);
            self.runtime_hr(hr, "CreateTexture2D (staging readback)")?;
            let staging = ComPtr::from_raw(staging, "staging readback texture")?;
            (*self.context.as_raw())
                .CopyResource(staging.as_raw() as *mut _, source.as_raw() as *mut _);

            let mut mapped: D3D11_MAPPED_SUBRESOURCE = std::mem::zeroed();
            let hr = (*self.context.as_raw()).Map(
                staging.as_raw() as *mut _,
                0,
                D3D11_MAP_READ,
                0,
                &mut mapped,
            );
            self.runtime_hr(hr, "Map (staging readback)")?;
            let row_bytes = self.width as usize * 4;
            let mut out = vec![0u8; row_bytes * self.height as usize];
            for y in 0..self.height as usize {
                let src = (mapped.pData as *const u8).add(y * mapped.RowPitch as usize);
                let dst = &mut out[y * row_bytes..(y + 1) * row_bytes];
                std::ptr::copy_nonoverlapping(src, dst.as_mut_ptr(), row_bytes);
            }
            (*self.context.as_raw()).Unmap(staging.as_raw() as *mut _, 0);
            Ok(out)
        }
    }

    pub(crate) fn create_gpu_completion_query(&self) -> Result<GpuCompletionQuery> {
        unsafe {
            let desc = D3D11_QUERY_DESC {
                Query: D3D11_QUERY_EVENT,
                MiscFlags: 0,
            };
            let mut query: *mut ID3D11Query = ptr::null_mut();
            let hr = (*self.device.as_raw()).CreateQuery(&desc, &mut query);
            self.runtime_hr(hr, "CreateQuery (headless performance completion)")?;
            let query = ComPtr::from_raw(query, "headless performance completion query")?;
            Ok(GpuCompletionQuery { query })
        }
    }
}

impl GpuCompletionQuery {
    pub(crate) fn wait(&self, gfx: &Gfx) -> Result<()> {
        const GPU_WAIT_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(30);
        unsafe {
            (*gfx.context.as_raw()).End(self.query.as_raw() as *mut ID3D11Asynchronous);
            (*gfx.context.as_raw()).Flush();

            let started = std::time::Instant::now();
            loop {
                let mut completed: i32 = 0;
                let hr = (*gfx.context.as_raw()).GetData(
                    self.query.as_raw() as *mut ID3D11Asynchronous,
                    &mut completed as *mut _ as *mut c_void,
                    std::mem::size_of_val(&completed) as UINT,
                    0,
                );
                if hr == 0 && completed != 0 {
                    return Ok(());
                }
                if hr < 0 {
                    gfx.runtime_hr(hr, "GetData (headless performance completion)")?;
                }
                if started.elapsed() >= GPU_WAIT_TIMEOUT {
                    return Err(
                        "D3D11 headless performance frame did not complete within 30 seconds"
                            .into(),
                    );
                }
                std::thread::yield_now();
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn device_loss_hresult_classification_is_explicit_and_fail_closed() {
        assert_eq!(
            DeviceLoss::from_hresult(DXGI_ERROR_DEVICE_REMOVED),
            Some(DeviceLoss::Removed)
        );
        assert_eq!(
            DeviceLoss::from_hresult(DXGI_ERROR_DEVICE_RESET),
            Some(DeviceLoss::Reset)
        );
        assert_eq!(
            DeviceLoss::from_hresult(DXGI_ERROR_DEVICE_HUNG),
            Some(DeviceLoss::Hung)
        );
        assert_eq!(DeviceLoss::from_hresult(DXGI_ERROR_WAS_STILL_DRAWING), None);
        assert_eq!(DeviceLoss::from_hresult(DXGI_STATUS_OCCLUDED), None);
        assert_eq!(DeviceLoss::from_hresult(0), None);
    }

    #[test]
    fn device_loss_labels_are_operator_stable() {
        assert_eq!(DeviceLoss::Removed.label(), "removed");
        assert_eq!(DeviceLoss::Reset.label(), "reset");
        assert_eq!(DeviceLoss::Hung.label(), "hung");
    }

    #[test]
    fn generic_hresult_gate_also_rejects_device_loss_terminally() {
        for (hr, label) in [
            (DXGI_ERROR_DEVICE_REMOVED, "removed"),
            (DXGI_ERROR_DEVICE_RESET, "reset"),
            (DXGI_ERROR_DEVICE_HUNG, "hung"),
        ] {
            let error = hr_ok(hr, "synthetic runtime operation")
                .expect_err("device loss cannot be a feature-local omission");
            assert!(is_device_loss_error(error.as_ref()));
            let message = error.to_string();
            assert!(message.contains(&format!("device {label}")), "{message}");
            assert!(message.contains("stopped fail-closed"), "{message}");
            assert!(message.contains("restart replay_app"), "{message}");
        }
    }

    #[test]
    fn void_upload_result_is_admitted_only_while_device_is_available() {
        void_device_call_result(0, "UpdateSubresource RestoreScene ColorCube")
            .expect("S_OK device state admits the completed void upload");

        let removed = void_device_call_result(
            DXGI_ERROR_DEVICE_REMOVED,
            "UpdateSubresource RestoreScene ColorCube",
        )
        .expect_err("device removal must reject the upload");
        assert!(removed.downcast_ref::<DeviceLossError>().is_some());
        assert!(is_device_loss_error(removed.as_ref()));

        let unknown_reason = 0x8000_4005u32 as i32;
        let unavailable =
            void_device_call_result(unknown_reason, "UpdateSubresource RestoreScene ColorCube")
                .expect_err("an unknown negative device reason must reject the upload");
        assert!(unavailable
            .downcast_ref::<DeviceUnavailableError>()
            .is_some());
        assert!(is_device_loss_error(unavailable.as_ref()));
        assert!(unavailable.to_string().contains("cannot be admitted"));
    }
}
