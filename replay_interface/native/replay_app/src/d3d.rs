//! D3D11 device + DXGI swapchain plumbing. Raw winapi COM with explicit
//! Release handling via [`ComPtr`], matching the map_viewer house style.
//!
//! Two backends behind one `Gfx` type: a windowed swapchain (the product
//! window) and an offscreen render target (`create_offscreen`) used by the
//! headless frame-dump path and the future visual-QA harness.

use std::ptr;

use winapi::shared::dxgi::{
    IDXGISwapChain, DXGI_PRESENT_DO_NOT_WAIT, DXGI_SWAP_CHAIN_DESC, DXGI_SWAP_EFFECT_DISCARD,
    DXGI_SWAP_EFFECT_FLIP_DISCARD,
};
use winapi::shared::dxgiformat::{DXGI_FORMAT_B8G8R8A8_UNORM, DXGI_FORMAT_UNKNOWN};
use winapi::shared::dxgitype::{
    DXGI_MODE_DESC, DXGI_RATIONAL, DXGI_SAMPLE_DESC, DXGI_USAGE_RENDER_TARGET_OUTPUT,
};
use winapi::shared::minwindef::UINT;
use winapi::shared::windef::HWND;
use winapi::shared::winerror::{
    DXGI_ERROR_DEVICE_HUNG, DXGI_ERROR_DEVICE_REMOVED, DXGI_ERROR_DEVICE_RESET,
    DXGI_ERROR_WAS_STILL_DRAWING, DXGI_STATUS_OCCLUDED,
};
use winapi::um::d3d11::{
    D3D11CreateDevice, D3D11CreateDeviceAndSwapChain, ID3D11Device, ID3D11DeviceContext,
    ID3D11RenderTargetView, ID3D11Texture2D, D3D11_BIND_RENDER_TARGET, D3D11_CPU_ACCESS_READ,
    D3D11_CREATE_DEVICE_BGRA_SUPPORT, D3D11_MAPPED_SUBRESOURCE, D3D11_MAP_READ, D3D11_SDK_VERSION,
    D3D11_TEXTURE2D_DESC, D3D11_USAGE_DEFAULT, D3D11_USAGE_STAGING,
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
    /// Wrap a raw COM pointer, failing with the given role when null.
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

/// HRESULT check with an explicit action string.
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

/// A device-loss HRESULT is not an ordinary feature-local draw omission. All
/// cached shaders, buffers, views and swapchain objects belong to the lost
/// device, so replay must stop before another frame can reuse any of them.
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

/// Typed terminal device-loss error. Some renderer paths intentionally catch
/// ordinary feature-local failures and omit only that feature; the type lets
/// those boundaries distinguish a dead D3D device without matching text and
/// unwind the whole app instead.
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

/// Typed terminal error for a failed void D3D11 command when the device
/// reports an HRESULT outside the three named DXGI loss reasons. Preserve the
/// exact observation instead of guessing which device-loss category occurred.
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

/// True only for a typed terminal device failure, including an unclassified
/// negative `GetDeviceRemovedReason` retained in an error source chain. This
/// is the escape hatch from feature-local omission boundaries; it deliberately
/// does not inspect diagnostic strings.
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

/// What `Gfx::present` did. The render loop must keep pumping input on
/// `Skipped`/`Occluded` (sleep briefly instead of blocking) — a vsync
/// Present that blocks while the window is fully occluded freezes the
/// whole app, which is exactly the liveness bug this replaces.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PresentOutcome {
    Presented,
    /// The present queue was full (`DXGI_ERROR_WAS_STILL_DRAWING`); the
    /// frame was dropped. Retry next loop iteration.
    Skipped,
    /// The window is fully occluded (`DXGI_STATUS_OCCLUDED`); rendering is
    /// pointless until it becomes visible again.
    Occluded,
}

/// Device, immediate context, and either a windowed swapchain or an
/// offscreen render target.
pub struct Gfx {
    pub device: ComPtr<ID3D11Device>,
    pub context: ComPtr<ID3D11DeviceContext>,
    swapchain: Option<ComPtr<IDXGISwapChain>>,
    /// The offscreen color texture when this is a headless target.
    offscreen: Option<ComPtr<ID3D11Texture2D>>,
    rtv: Option<ComPtr<ID3D11RenderTargetView>>,
    pub width: u32,
    pub height: u32,
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

/// Pick the hardware adapter with the most dedicated video memory. This is a
/// hybrid rig (RTX 5090 dGPU + Radeon iGPU); every capture was rendered by
/// the game on the dGPU, and replaying captured passes on a different GPU
/// changes rasterization/filtering subtly. Pin the high-memory adapter
/// instead of trusting the D3D11 default.
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
    /// App-owned presentation/offscreen target format. Both constructors and
    /// resize use this exact format; CAP6 uses it to reject the retained
    /// R8G8B8A8 counter receipt on the current B8G8R8A8 product target rather
    /// than treating the channel-layout difference as implicit parity.
    pub(crate) const fn target_format(&self) -> u32 {
        DXGI_FORMAT_B8G8R8A8_UNORM
    }

    /// Check an HRESULT from an operation on this live device. Device removal,
    /// reset, or hang is returned as an explicit terminal/fail-closed error;
    /// callers must unwind and drop every device-bound cache rather than
    /// attempting to continue with stale COM resources.
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

    /// D3D11 immediate-context upload calls such as `UpdateSubresource` return
    /// `void`. Query the owning device immediately after the command so a
    /// removed, reset, hung, or otherwise unavailable device is propagated as
    /// a typed terminal error instead of admitting the upload silently.
    pub(crate) fn check_void_device_call(&self, action: &str) -> Result<()> {
        let removed_reason = unsafe { (*self.device.as_raw()).GetDeviceRemovedReason() };
        void_device_call_result(removed_reason, action)
    }

    /// Clear and bind the product target when no admitted non-UI pass is
    /// available. This is a neutral compositor surface, not a fabricated map.
    pub fn clear_target(&self, color: [f32; 4]) -> Result<()> {
        unsafe {
            let rtv = self.rtv()?.as_raw();
            (*self.context.as_raw()).ClearRenderTargetView(rtv, &color);
            (*self.context.as_raw()).OMSetRenderTargets(1, &rtv, ptr::null_mut());
        }
        Ok(())
    }

    /// Create a hardware device and a windowed BGRA8 swapchain. Prefers
    /// flip-model (FLIP_DISCARD); falls back to blt-model (DISCARD) when
    /// the OS/driver rejects it.
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
                offscreen: None,
                rtv: None,
                width: width.max(1),
                height: height.max(1),
            };
            gfx.create_backbuffer_rtv()?;
            Ok(gfx)
        }
    }

    /// Create a windowless device rendering into an offscreen BGRA8 target
    /// (headless frame dumps / visual-QA renders). `present` is a no-op;
    /// read pixels back with [`Gfx::read_target`].
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

            Ok(Gfx {
                device,
                context,
                swapchain: None,
                offscreen: Some(texture),
                rtv: Some(rtv),
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
        let mut rtv: *mut ID3D11RenderTargetView = ptr::null_mut();
        let hr = (*self.device.as_raw()).CreateRenderTargetView(
            backbuffer.as_raw() as *mut _,
            ptr::null(),
            &mut rtv,
        );
        self.runtime_hr(hr, "CreateRenderTargetView for swapchain backbuffer")?;
        self.rtv = Some(ComPtr::from_raw(rtv, "backbuffer render-target view")?);
        Ok(())
    }

    pub fn rtv(&self) -> Result<&ComPtr<ID3D11RenderTargetView>> {
        self.rtv
            .as_ref()
            .ok_or_else(|| "render-target view is not initialized".into())
    }

    /// Resize the swapchain buffers (no-op for zero/unchanged sizes and for
    /// offscreen targets).
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
            (*self.context.as_raw()).ClearState();
            let swapchain = self
                .swapchain
                .as_ref()
                .ok_or("resize called on an offscreen target")?;
            let hr = (*swapchain.as_raw()).ResizeBuffers(0, width, height, DXGI_FORMAT_UNKNOWN, 0);
            self.runtime_hr(hr, "IDXGISwapChain::ResizeBuffers")?;
            self.width = width;
            self.height = height;
            self.create_backbuffer_rtv()
        }
    }

    /// Present with vsync but WITHOUT blocking the loop:
    /// `DXGI_PRESENT_DO_NOT_WAIT` turns a full present queue into
    /// [`PresentOutcome::Skipped`] instead of an indefinite block, and a
    /// fully-occluded window reports [`PresentOutcome::Occluded`]. The
    /// caller throttles by sleeping on those outcomes; blocking here froze
    /// the message pump (and all input) whenever the window was covered.
    pub fn present(&self) -> Result<PresentOutcome> {
        let Some(swapchain) = self.swapchain.as_ref() else {
            return Ok(PresentOutcome::Presented); // offscreen: nothing to present
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

    /// Read the current render target back as tightly-packed BGRA8 rows
    /// (top-down). Offscreen targets only.
    pub fn read_target(&self) -> Result<Vec<u8>> {
        let source = self
            .offscreen
            .as_ref()
            .ok_or("read_target is only supported on offscreen targets")?;
        unsafe {
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
        let unavailable = void_device_call_result(
            unknown_reason,
            "UpdateSubresource RestoreScene ColorCube",
        )
        .expect_err("an unknown negative device reason must reject the upload");
        assert!(unavailable
            .downcast_ref::<DeviceUnavailableError>()
            .is_some());
        assert!(is_device_loss_error(unavailable.as_ref()));
        assert!(unavailable.to_string().contains("cannot be admitted"));
    }
}
