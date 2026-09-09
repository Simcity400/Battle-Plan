//! Anonymous shader pass declaration stored after SGfxShaderPassReader parsing.
//!
//! Source: `portable_replay/native/engine_evidence/artifacts/func_142348f80.decompile.c`
//! and `func_142344df0.decompile.c` at `archive/pre-rewrite-2026-09-09`.
//! The original row class name is unproved. Presentation knowledge only.

/// Shader pass row stride; FUN142348f80 allocates and appends rows in 0xd8-byte steps.
pub const STRIDE: usize = 0xd8;
/// Pass name std::string; FUN142344df0 compares the row base with the requested effect name.
pub const NAME: usize = 0;
/// Vertex shader name std::string; FUN142344df0 reads row+0x20 before its Failed adding vertex shader diagnostic.
pub const VERTEX_SHADER_NAME: usize = 0x20;
/// Pixel shader name std::string; FUN142344df0 reads row+0x40 before its Failed adding pixel shader diagnostic.
pub const PIXEL_SHADER_NAME: usize = 0x40;
/// Blend declaration selector std::string; FUN142344df0 compares row+0x60 with the named blend declaration list.
pub const BLEND_STATE_NAME: usize = 0x60;
/// Depth/stencil declaration selector std::string; FUN142344df0 compares row+0x80 with the named depth/stencil list.
pub const DEPTH_STENCIL_STATE_NAME: usize = 0x80;
/// Rasterizer declaration selector std::string; FUN142344df0 compares row+0xa0 with the named rasterizer list.
pub const RASTERIZER_STATE_NAME: usize = 0xa0;
/// Shader pass declaration parser/append owner; FUN142348f80 constructs SGfxShaderPassReader and appends its payload.
pub const READ_DECLARATION_RVA: usize = 0x0234_8f80;
/// Pass selection and native program construction; FUN142344df0 resolves named states and adds vertex/pixel shaders.
pub const RESOLVE_RVA: usize = 0x0234_4df0;
