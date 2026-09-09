//! Native texture conversion and upload functions with no proven C++ receiver type.
//! a729 presentation knowledge only, harvested from container_sprite_resources_gpu.rs
//! and its retained decompiles; resource cache/lifetime and execution order remain unproved.
//! Values here describe the native format mapping, not viewer fallbacks or D3D SDK layouts.

/// Native D3D11 texture creator; container_sprite_resources_gpu.rs:26,125-128 and retained FUN1423FB4F0 confirm descriptor lowering and source-row upload.
pub const CREATE_TEXTURE_RVA: usize = 0x023f_b4f0;
/// Native source-mip to upload-row conversion; container_sprite_resources_gpu.rs:65,73 and retained FUN1423FC160 confirm compressed pitches and BGR expansion.
pub const BUILD_UPLOAD_ROWS_RVA: usize = 0x023f_c160;
/// BGR expansion helper; container_sprite_resources_gpu.rs:65 and retained FUN1423F8E90 confirm triples become B,G,R,0 with no alpha repair.
pub const EXPAND_BGR_RVA: usize = 0x023f_8e90;
/// Native pixel-format bit-count helper; retained FUN1423FC160 calls FUN1423F9E10 before row-pitch calculation, overriding formats 11/12 to 32 upload bits.
pub const FORMAT_BITS_PER_PIXEL_RVA: usize = 0x023f_9e10;
/// Native-to-DXGI format conversion; container_sprite_resources_gpu.rs:60,65 and retained FUN142411320 confirm the switch below.
pub const FORMAT_TO_DXGI_RVA: usize = 0x0241_1320;
/// Native DDS DXT1 code; container_sprite_resources_gpu.rs:73 and FUN142410030 select 1 while retaining compressed blocks.
pub const FORMAT_DXT1: u32 = 1;
/// Native DDS DXT3 code; container_sprite_resources_gpu.rs:73 and FUN142410030 select 3 while retaining compressed blocks.
pub const FORMAT_DXT3: u32 = 3;
/// Native DDS DXT5 code; container_sprite_resources_gpu.rs:73 and FUN142410030 select 5 while retaining compressed blocks.
pub const FORMAT_DXT5: u32 = 5;
/// Native 24-bit BGR code; container_sprite_resources_gpu.rs:65 and FUN142410030 select 11, then FUN1423F8E90 expands its upload bytes.
pub const FORMAT_BGR24: u32 = 11;
/// Native 32-bit BGRA code; container_sprite_resources_gpu.rs:60 and FUN142410030 select 13, preserving the fourth source byte.
pub const FORMAT_BGRA32: u32 = 13;
/// Fourth byte written when expanding native BGR triples; retained FUN1423F8E90 explicitly stores zero, with sampled opacity provided by the DXGI X format.
pub const EXPANDED_BGR_FOURTH_BYTE: u8 = 0;
/// Native format-switch cases; retained FUN142411320 maps these engine codes to exact DXGI values, independently of SDK enum names.
pub const FORMAT_TO_DXGI_CASES: &[(u32, u32)] = &[
    (1, 0x47),
    (2, 0x48),
    (3, 0x4a),
    (4, 0x4b),
    (5, 0x4d),
    (6, 0x4e),
    (7, 0x55),
    (8, 0x56),
    (0x0c, 0x5d),
    (0x0d, 0x57),
    (0x0e, 0x5b),
    (0x10, 0x5d),
    (0x11, 0x41),
    (0x12, 0x3d),
    (0x13, 0x36),
    (0x14, 0x22),
    (0x15, 10),
    (0x16, 0x29),
    (0x17, 0x10),
    (0x18, 6),
    (0x19, 2),
];
/// Native format switch default result, including code 11; retained FUN142411320 returns 0x58 for every code absent from its cases.
pub const FORMAT_TO_DXGI_DEFAULT: u32 = 0x58;
