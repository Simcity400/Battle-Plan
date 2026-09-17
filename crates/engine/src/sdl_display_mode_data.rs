//! SDL_DisplayModeData Windows mode record in the pinned a729 image.
//!
//! The one-member wrapper structurally matches SDL release-2.0.22
//! `src/video/windows/SDL_windowsmodes.h`. Its payload is the Win32 DEVMODEW
//! layout, not a game simulation struct; the shipped SDL version is unproven.
//! Read a `func_<va>.decompile.c:N` citation as line N of `hoi4re fn <va> --raw`.

/// SDL_DisplayModeData allocation size; func_142107bb0.decompile.c:54 allocates 0xdc bytes, the observed Win32 DEVMODEW size.
pub const SIZE: usize = 0xdc;
/// SDL_DisplayModeData DeviceMode inline DEVMODEW; func_142107bb0.decompile.c:58-89 copies the native mode beginning at the allocation start.
pub const DEVICE_MODE: usize = 0;
