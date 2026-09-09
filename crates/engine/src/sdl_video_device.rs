//! SDL_VideoDevice presentation state in the pinned a729 image.
//!
//! Ownership is identified by the structural match to SDL release-2.0.22's
//! `src/video/windows/SDL_windowsmodes.c` and `src/video/SDL_sysvideo.h`;
//! this does not establish the exact SDL version shipped by the game.
//! Decompile references are archived under `C:/hoi4_evidence/ghidra_agent_exports/`.

pub const CURRENT_VIDEO_DEVICE_RVA: usize = 0x0343_7c68;
/// SDL_VideoDevice num_displays, native i32; func_142107fb0.decompile.c:21 and func_1421076f0.decompile.c:50-70 read this bound.
pub const NUM_DISPLAYS: usize = 0x2b8;
/// SDL_VideoDevice displays pointer; func_1420d34b0.decompile.c:23-59 reallocates and stores its contiguous SDL_VideoDisplay array.
pub const DISPLAYS: usize = 0x2c0;
/// SDL_VideoDevice setting_display_mode, native SDL_bool/i32; func_1421076f0.decompile.c:58 skips mode replacement here, matching WIN_AddDisplay.
pub const SETTING_DISPLAY_MODE: usize = 0x2e8;
/// WIN_InitModes with WIN_AddDisplays inlined; func_142107fb0.decompile.c:15-23 enumerates primary then secondary monitors and checks num_displays.
pub const INITIALIZE_MODES_RVA: usize = 0x0210_7fb0;
/// WIN_GetDisplayMode with SDL_VideoDevice receiver; func_142107bb0.decompile.c:51-94 copies DEVMODEW into the supplied SDL_DisplayMode.
pub const GET_DISPLAY_MODE_RVA: usize = 0x0210_7bb0;
/// WIN_UpdateDisplayMode with SDL_VideoDevice receiver; func_142108200.decompile.c:20-32 replaces current-mode dimensions from device caps.
pub const UPDATE_DISPLAY_MODE_RVA: usize = 0x0210_8200;
