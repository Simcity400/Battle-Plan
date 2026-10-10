//! D3D11 renderer, distinct from the graphics owner. State lowering receives
//! this object; `render_context_layout` describes a separate binding cache whose
//! renderer pointer reaches these default-state fields.

/// D3D11 renderer creation entry.
pub const CREATE_RVA: usize = 0x023f_f310;
/// Creation-time raw 32-bit antialiasing request.
pub const CREATION_AA_REQUEST: usize = 0x54;
/// Default blend-wrapper pointer.
pub const DEFAULT_BLEND_STATE: usize = 0x58;
/// Default depth/stencil-wrapper pointer.
pub const DEFAULT_DEPTH_STENCIL_STATE: usize = 0x60;
/// Default rasterizer-wrapper pointer.
pub const DEFAULT_RASTERIZER_STATE: usize = 0x68;
/// D3D device pointer used by state lowering.
pub const DEVICE: usize = 0x70;
