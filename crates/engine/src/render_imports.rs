//! Import slots in the pinned hoi4.exe image; not platform DLL export bytes.
//! Source: extractor/native/lanes/production_render_submit_live/src/a729_hook.rs
//! at archive/pre-rewrite-2026-09-09. No import patching is enabled.

/// D3DCompile import-address slot, checked by old import-hook code; a729_hook.rs:151.
pub const D3D_COMPILE_IAT_RVA: usize = 0x026f_1040;
/// D3D11CreateDevice import-address slot, checked by old import-hook code; a729_hook.rs:152.
pub const D3D11_CREATE_DEVICE_IAT_RVA: usize = 0x026f_2068;
