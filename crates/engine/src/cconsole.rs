//! CConsole runtime-output knowledge, excluded from hourly capture and content identity.
//! Archived extractor/game_defs/engine_runtime_artifacts.json records the pinned
//! executable's string-reference census. These paths are raw I/O sinks, not VFS content.

/// CConsole constructor reads command history; engine_runtime_artifacts.json:21 identifies its history-file reference.
pub const CONSTRUCTOR_RVA: usize = 0x0206_ac60;
/// CConsole accepted-command handler appends history; engine_runtime_artifacts.json:16-21 records raw fopen/fprintf/fclose calls.
pub const ACCEPT_COMMAND_RVA: usize = 0x0206_b710;
/// CConsole history filename literal; engine_runtime_artifacts.json:12-14 records the complete a729 reference census.
pub const HISTORY_FILENAME_RVA: usize = 0x02ae_28e8;
/// CConsole history filename pointer slot; engine_runtime_artifacts.json:13 records the only data reference.
pub const HISTORY_FILENAME_POINTER_RVA: usize = 0x0309_e568;
/// CConsole constructor history filename reference; engine_runtime_artifacts.json:14,21 binds it to the constructor.
pub const HISTORY_READ_REFERENCE_RVA: usize = 0x0206_af4b;
/// CConsole command-handler history filename reference; engine_runtime_artifacts.json:14,21 binds it to the append path.
pub const HISTORY_WRITE_REFERENCE_RVA: usize = 0x0206_b82d;
/// CConsole history append-mode literal; engine_runtime_artifacts.json:20-21 confirms the string "a".
pub const HISTORY_APPEND_MODE_RVA: usize = 0x02ad_47e4;
/// CConsole history line format literal; engine_runtime_artifacts.json:21 confirms the string "%s\n".
pub const HISTORY_LINE_FORMAT_RVA: usize = 0x02ae_2ab8;
