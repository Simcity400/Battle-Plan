//! SDL_DisplayMode value record in the pinned a729 image.
//!
//! Names structurally match SDL release-2.0.22 `include/SDL_video.h`;
//! no exact shipped SDL minor version is asserted. Decompiles are archived
//! under `C:/hoi4_evidence/ghidra_agent_exports/`.

/// SDL_DisplayMode stride; func_1420d8170.decompile.c:10 copies 0x18 bytes and func_1420d7ff0.decompile.c:22-25 iterates the same stride.
pub const STRIDE: usize = 0x18;
/// SDL_DisplayMode format, native u32 enum; func_142107bb0.decompile.c:90 initializes it and func_142108200.decompile.c:34-82 selects SDL pixel formats.
pub const FORMAT: usize = 0;
/// SDL_DisplayMode w, native i32 pixels; func_142107bb0.decompile.c:91 copies dmPelsWidth, then func_142108200.decompile.c:25 may replace it.
pub const WIDTH: usize = 4;
/// SDL_DisplayMode h, native i32 pixels; func_142107bb0.decompile.c:92 copies dmPelsHeight, then func_142108200.decompile.c:26 may replace it.
pub const HEIGHT: usize = 8;
/// SDL_DisplayMode refresh_rate, native i32; func_142107bb0.decompile.c:93 copies the native DEVMODEW display frequency here.
pub const REFRESH_RATE: usize = 0x0c;
/// SDL_DisplayMode SDL_DisplayModeData pointer; func_142107bb0.decompile.c:54-57 allocates the copied DEVMODEW and stores its address here.
pub const DRIVER_DATA: usize = 0x10;
