//! WIN_AddDisplaysData monitor-enumeration callback context in a729.
//!
//! The name structurally matches SDL release-2.0.22
//! `src/video/windows/SDL_windowsmodes.c`; this does not identify the shipped
//! SDL minor version. Decompiles are under `C:/hoi4_evidence/ghidra_agent_exports/`.

/// WIN_AddDisplaysData video_device pointer; func_142107fb0.decompile.c:17 supplies the receiver and func_1421076f0.decompile.c:46 reads it.
pub const VIDEO_DEVICE: usize = 0;
/// WIN_AddDisplaysData send_event, native SDL_bool/i32; func_142107fb0.decompile.c:15 initializes 0 and func_1421076f0.decompile.c:45,96 forwards its low 32 bits.
pub const SEND_EVENT: usize = 8;
/// WIN_AddDisplaysData want_primary, native SDL_bool/i32; func_142107fb0.decompile.c:16-20 selects 1 then 0, compared with monitor flags at func_1421076f0.decompile.c:44.
pub const WANT_PRIMARY: usize = 0x0c;
/// WIN_AddDisplaysCallback with WIN_AddDisplay inlined; func_1421076f0.decompile.c:41-47 reads this context through its fourth Win32 callback argument.
pub const ADD_DISPLAYS_CALLBACK_RVA: usize = 0x0210_76f0;
