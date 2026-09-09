//! SGfxShaderPassReader, named by the archived FUN142348f80 construction branch.
//!
//! Source: `portable_replay/native/engine_evidence/artifacts/func_142348f80.*`
//! and `func_142344df0.decompile.c` at `archive/pre-rewrite-2026-09-09`.
//! Presentation parser knowledge only; no capture or hook is enabled.

/// SGfxShaderPassReader vtable; FUN142348f80 instruction 142348fcc loads 142b3b128 in its named construction branch.
pub const VTABLE_RVA: usize = 0x02b3_b128;
/// Pass name std::string; FUN142348f80 constructs it immediately after the reader vptr.
pub const NAME: usize = 0x08;
/// Vertex shader name std::string; FUN142348f80 copies the reader payload to a row consumed at +0x20 by FUN142344df0.
pub const VERTEX_SHADER_NAME: usize = 0x28;
/// Pixel shader name std::string; FUN142348f80 copies the reader payload to a row consumed at +0x40 by FUN142344df0.
pub const PIXEL_SHADER_NAME: usize = 0x48;
/// Blend declaration selector std::string; FUN142348f80 constructs BlendState at reader+0x68.
pub const BLEND_STATE_NAME: usize = 0x68;
/// Depth/stencil declaration selector std::string; FUN142348f80 constructs DepthStencilState at reader+0x88.
pub const DEPTH_STENCIL_STATE_NAME: usize = 0x88;
/// Rasterizer declaration selector std::string; FUN142348f80 constructs RasterizerState at reader+0xa8.
pub const RASTERIZER_STATE_NAME: usize = 0xa8;
