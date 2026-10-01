//! Likely CProvinceGraphics, passed to FUN14127b2e0; no independent RTTI binding was found.
//! FUN140b4c120's decompile loads the CGraphicalMap+0x68 province-indexed array at
//! line 531, passes its element at 550, and names CProvinceGraphics cache RTTI at 616.
//! Presentation-only knowledge retained from the old post-update visual capture.

/// Province graphics building-vector update FUN14127b2e0, o05_building_capture.rs:13.
pub const BUILDING_VECTOR_UPDATE_RVA: usize = 0x0127_b2e0;
/// FUN14127b2e0 entry bytes checked by the old hook, o05_building_capture.rs:26-28.
pub const BUILDING_VECTOR_UPDATE_SENTINEL: [u8; 16] = [
    0x48, 0x89, 0x5c, 0x24, 0x08, 0x48, 0x89, 0x6c, 0x24, 0x10, 0x48, 0x89, 0x74, 0x24, 0x18, 0x57,
];
/// Province graphics visual-record pointer array, read after FUN14127b2e0 by o05_building_capture.rs:237.
pub const BUILDING_RECORDS: usize = 0x1548;
/// Province graphics visual-record count u32, read by o05_building_capture.rs:241.
pub const BUILDING_RECORD_COUNT: usize = 0x1554;
/// Province graphics visual-record pointer slot width on the pinned x64 build, o05_building_capture.rs:265.
pub const BUILDING_RECORD_POINTER_STRIDE: usize = 8;
/// CProvinceGraphics native counter-position table, used by land/naval consumers in xmap14_combat_icon_placement.rs:337-340.
pub const UNIT_STACKS: usize = 0x11d8;
/// CProvinceGraphics counter-position slot stride (three native f32 components), xmap14_combat_icon_placement.rs:341.
pub const UNIT_STACK_STRIDE: usize = 0x0c;
/// CProvinceGraphics state graphics pointer, func_140b43b60.decompile.c:146-149 stores the owning state's CStateGraphics object here.
pub const STATE_GRAPHICS: usize = 0x1508;
/// CProvinceGraphics stack-table writer FUN14127a390, qr_counter_residuals/func_14127a390.decompile.c:7,64-69 shows the receiver stores.
pub const FILL_STACK_TABLE_RVA: usize = 0x0127_a390;
/// CProvinceGraphics province-definition pointer used for the authored locator index at definition+0xc4, func_14127a390.decompile.c:43,63.
pub const PROVINCE_DEFINITION: usize = 0x11b8;
/// CProvinceGraphics native locator float-A array, func_14127a390.decompile.c:68 copies each record+0x0c; stationary consumer uses it as yaw.
pub const LOCATOR_FLOAT_A: usize = 0x13ac;
/// CProvinceGraphics native locator float-B array, func_14127a390.decompile.c:69 copies each record+0x10; downstream unit-matrix semantics remain unresolved.
pub const LOCATOR_FLOAT_B: usize = 0x1448;
/// CProvinceGraphics locator float-array element width, func_14127a390.decompile.c:68-69 multiplies each slot by four.
pub const LOCATOR_FLOAT_STRIDE: usize = 4;
/// CProvinceGraphics special slot position `f32[3]`, func_14127a390.decompile.c:45-46 fills +0x13a0..+0x13ab.
pub const SPECIAL_STACK_POSITION: usize = 0x13a0;
/// CProvinceGraphics special slot float A, func_14127a390.decompile.c:47 copies record+0x0c.
pub const SPECIAL_LOCATOR_FLOAT_A: usize = 0x1444;
/// CProvinceGraphics special slot float B, func_14127a390.decompile.c:48 copies record+0x10.
pub const SPECIAL_LOCATOR_FLOAT_B: usize = 0x14e0;
/// CProvinceGraphics special slot selector, func_14127a390.decompile.c:42 stores slot0x26 separately from the ordinary arrays.
pub const SPECIAL_STACK_SLOT: usize = 0x26;
/// CProvinceGraphics land combat icon producer, render_stream/src/xmap14_combat_indicators.rs:256 and combat_icon_pov_law.json:106 identify FUN14127bfc0.
pub const LAND_ICON_PRODUCER_RVA: usize = 0x0127_bfc0;
/// CProvinceGraphics supply-icon update; exact 20260719T223856Z-wave2-r10-sup-02-d1-p1-ea60087c/evidence/func_14127dac0.decompile.c:72 follows this+0x11b8 to its province definition before applying hub cutoff predicates.
pub const UPDATE_SUPPLY_ICONS_RVA: usize = 0x0127_dac0;

///14127EBF0 walks38 source stack-icon pointers, skipping slot19 for ordinary counters.
pub const STACK_ICONS: usize = 0x1560;
