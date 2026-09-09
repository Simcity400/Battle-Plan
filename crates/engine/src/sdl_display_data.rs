//! SDL_DisplayData Windows monitor record in the pinned a729 image.
//!
//! Names structurally match SDL release-2.0.22
//! `src/video/windows/SDL_windowsmodes.h`; no shipped minor version is asserted.
//! Decompiles are archived under `C:/hoi4_evidence/ghidra_agent_exports/`.

/// SDL_DisplayData allocation size; func_1421076f0.decompile.c:72 allocates 0x50 bytes for each new monitor's driverdata.
pub const SIZE: usize = 0x50;
/// SDL_DisplayData DeviceName UTF-16 buffer; func_1421076f0.decompile.c:74 copies the MONITORINFOEXW device name at the allocation start.
pub const DEVICE_NAME: usize = 0;
/// SDL_DisplayData DeviceName byte span; func_1421076f0.decompile.c:74 copies 0x40 bytes, matching 32 UTF-16 code units.
pub const DEVICE_NAME_BYTES: usize = 0x40;
/// SDL_DisplayData MonitorHandle, native HMONITOR; func_1421076f0.decompile.c:56 and :75 assign the enumeration callback's monitor handle here.
pub const MONITOR_HANDLE: usize = 0x40;
/// SDL_DisplayData IsValid, native SDL_bool/i32; func_1421076f0.decompile.c:57 and :76 mark successfully registered monitors with 1.
pub const IS_VALID: usize = 0x48;
