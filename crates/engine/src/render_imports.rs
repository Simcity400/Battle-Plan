//! Import slots in the supported executable, separate from platform DLL
//! export bytes. No import patching is enabled.

/// D3DCompile import-address slot.
pub const D3D_COMPILE_IAT_RVA: usize = 0x026f_1040;
/// D3D11CreateDevice import-address slot.
pub const D3D11_CREATE_DEVICE_IAT_RVA: usize = 0x026f_2068;
