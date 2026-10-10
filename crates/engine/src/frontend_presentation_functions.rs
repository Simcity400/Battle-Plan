//! Frontend presentation-function metadata, separate from object layouts.
//! No function here is a capture hook.

/// Achievement availability and completion text update.
pub const ACHIEVEMENT_TEXT_UPDATE_RVA: usize = 0x0167_55e0;
/// Music shuffle, play, pause and current-track tooltip producer.
pub const MUSIC_TOOLTIP_RVA: usize = 0x01de_3160;
/// Child visibility setter from a boolean.
pub const MUSIC_ASSOCIATED_VISIBILITY_RVA: usize = 0x01f7_9420;
/// Child visibility refresh.
pub const MUSIC_ASSOCIATED_REFRESH_RVA: usize = 0x01f7_9740;
/// Settings and accessibility key processing.
pub const SETTINGS_PROCESS_RVA: usize = 0x0021_78c0;
/// Resolution, display-mode, autosave and texture-quality control binding.
pub const SETTINGS_BIND_CONTROLS_RVA: usize = 0x019a_7a20;
/// Multiplayer message, server-ID and version control update.
pub const LOBBY_UPDATE_RVA: usize = 0x0186_1420;
/// Frontend, social, DLC and menu control binding.
pub const FRONTEND_MAIN_BIND_CONTROLS_RVA: usize = 0x01cc_e1c0;
/// Debug and logging category initialization.
pub const DEBUG_LOGGING_SETUP_RVA: usize = 0x0012_2410;
