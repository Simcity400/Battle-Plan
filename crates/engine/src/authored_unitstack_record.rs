//! Native authored locator record; no independent C++ class name is established.

/// Record stride 24 bytes; va_rail_z_frame/evidence/func_14163da30.decompile.c:116-138 addresses five floats and a flag through index*0x18.
pub const STRIDE: usize = 0x18;
/// Native f32[3] world position; func_14163d0a0.decompile.c:265-267 copies parsed x/y/z into record+0/+4/+8.
pub const POSITION: usize = 0;
/// First additional native f32; func_14163da30.decompile.c:123 exports record+0xc. Its downstream meaning is not established here.
pub const FLOAT_A: usize = 0x0c;
/// Second additional native f32; func_14163da30.decompile.c:118 exports record+0x10. Its downstream meaning is not established here.
pub const FLOAT_B: usize = 0x10;
/// Authored-presence native u8; func_14163d0a0.decompile.c:278 sets it, and func_14163da30.decompile.c:117 exports only set records.
pub const PRESENT: usize = 0x14;
