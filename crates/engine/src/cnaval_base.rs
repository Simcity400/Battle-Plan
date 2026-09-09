//! CNavalBase, identified by the named vtable store in the pinned a729 constructor.
//! Sources are retained under extractor/native/runs/va_rail_z_frame/evidence/.

/// CNavalBase constructor; func_140e8a080.decompile.c:19-29 allocates the object and writes CNavalBase::vftable.
pub const CONSTRUCT_RVA: usize = 0x00e8_a080;
/// Native object allocation size; func_140e8a080.decompile.c:19 passes 0x58 to the allocator.
pub const SIZE: usize = 0x58;
/// Owning country-holder pointer; func_140e8a080.decompile.c:26 stores the caller's holder argument at +8.
pub const HOLDER: usize = 0x08;
/// Native 32-bit province id; func_140e8a080.decompile.c:27 stores its province argument at +0x10.
pub const PROVINCE_ID: usize = 0x10;
/// Native signed i32 healthy port level sum; func_140e9cc60.decompile.c:63 adds each eligible building's signed short at +0x42.
pub const HEALTHY_LEVEL: usize = 0x14;
/// Native signed i32 total port level sum; func_140e9cc60.decompile.c:62 adds each eligible building's signed short at +0x40.
pub const LEVEL: usize = 0x18;
/// Rebuild cached port levels; func_140e9cc60.decompile.c:20,59-63 clears the pair and sums the database's eligible port buildings.
pub const UPDATE_LEVELS_RVA: usize = 0x00e9_cc60;
