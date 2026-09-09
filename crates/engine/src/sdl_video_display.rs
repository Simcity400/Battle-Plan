//! SDL_VideoDisplay presentation record in the pinned a729 image.
//!
//! Field names match SDL release-2.0.22 `src/video/SDL_sysvideo.h` and the
//! observed operations below; the shipped SDL minor version is not established.
//! Decompiles are archived under `C:/hoi4_evidence/ghidra_agent_exports/`.

/// SDL_VideoDisplay array stride; func_1420d34b0.decompile.c:23-28 grows the allocation by 0x68 bytes per display.
pub const STRIDE: usize = 0x68;
/// SDL_VideoDisplay UTF-8 name pointer; func_1420d34b0.decompile.c:60-66 duplicates the supplied display name into the first field.
pub const NAME: usize = 0;
/// SDL_VideoDisplay num_display_modes, native i32; func_1420d7ff0.decompile.c:20-28 frees this many 0x18-byte mode records.
pub const NUM_DISPLAY_MODES: usize = 0x0c;
/// SDL_VideoDisplay display_modes pointer; func_1420d7ff0.decompile.c:25-31 follows and frees this array.
pub const DISPLAY_MODES: usize = 0x10;
/// SDL_VideoDisplay desktop_mode inline record; func_1420d8180.decompile.c:10 copies 0x18 bytes here, matching SDL_SetDesktopDisplayMode.
pub const DESKTOP_MODE: usize = 0x18;
/// SDL_VideoDisplay current_mode inline record; func_1420d8170.decompile.c:10 copies 0x18 bytes here, matching SDL_SetCurrentDisplayMode.
pub const CURRENT_MODE: usize = 0x30;
/// SDL_VideoDisplay orientation, native enum/i32; func_142118ac0.decompile.c:21-28 rejects unknown/unchanged values before storing here.
pub const ORIENTATION: usize = 0x48;
/// SDL_VideoDisplay SDL_VideoDevice back-pointer; func_1420d34b0.decompile.c:58 stores the shared device owner here.
pub const DEVICE: usize = 0x58;
/// SDL_VideoDisplay SDL_DisplayData pointer; func_1421076f0.decompile.c:53-57 follows this to compare DeviceName and update MonitorHandle.
pub const DRIVER_DATA: usize = 0x60;
/// SDL_SetCurrentDisplayMode; func_1420d8170.decompile.c:10 copies a supplied SDL_DisplayMode into receiver +0x30.
pub const SET_CURRENT_MODE_RVA: usize = 0x020d_8170;
/// SDL_SetDesktopDisplayMode; func_1420d8180.decompile.c:10 copies a supplied SDL_DisplayMode into receiver +0x18.
pub const SET_DESKTOP_MODE_RVA: usize = 0x020d_8180;
