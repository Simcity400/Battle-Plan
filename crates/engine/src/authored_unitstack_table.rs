//! Native owner of map/unitstacks.txt locator arrays; its C++ class is unknown.
//! This content table is distinct from CProvinceGraphics' copied runtime locators.

/// Kind-array count 39; va_rail_z_frame/evidence/func_14163d0a0.decompile.c:94-95 initializes exactly 0x27 headers.
pub const KIND_COUNT: usize = 0x27;
/// First inline kind-array header; func_14163d0a0.decompile.c:94,108,153 follows owner+0xc count and advances by 0x18.
pub const KINDS: usize = 0;
/// Kind-array header stride; func_14163da30.decompile.c:112 reads the count at owner+0xc+kind*0x18.
pub const KIND_STRIDE: usize = 0x18;
/// Native table initializer and file parser; func_14163d0a0.decompile.c:94-153 seeds each kind from province-template anchors before parsing authored overrides.
pub const INITIALIZE_RVA: usize = 0x0163_d0a0;
/// Native table exporter; func_14163da30.decompile.c:112-117 walks the same kind headers and tests each record's presence byte.
pub const EXPORT_RVA: usize = 0x0163_da30;
/// Related free kind-name lookup, no receiver; func_14163cec0.decompile.c:7 accepts only an integer kind and returns its display name.
pub const KIND_NAME_RVA: usize = 0x0163_cec0;
