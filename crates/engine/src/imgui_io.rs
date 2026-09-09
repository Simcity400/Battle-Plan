//! The game's ImGuiIO output-sink fields; retained knowledge only, no replay domain.
//! Archived extractor/game_defs/engine_runtime_artifacts.json proves constructor
//! bindings. It deliberately does not identify the file-open function reached through them.

/// ImGuiIO constructor; engine_runtime_artifacts.json:32,37 records the two output filename bindings.
pub const CONSTRUCTOR_RVA: usize = 0x021b_5390;
/// ImGuiIO IniFilename pointer; engine_runtime_artifacts.json:34 confirms constructor assignment at native +0x18.
pub const INI_FILENAME: usize = 0x18;
/// ImGuiIO LogFilename pointer; engine_runtime_artifacts.json:50 confirms constructor assignment at native +0x20.
pub const LOG_FILENAME: usize = 0x20;
/// ImGuiIO default INI filename literal; engine_runtime_artifacts.json:28-30 records both constructor references.
pub const INI_FILENAME_LITERAL_RVA: usize = 0x02b1_eb48;
/// ImGuiIO default log filename literal; engine_runtime_artifacts.json:44-46 records both constructor references.
pub const LOG_FILENAME_LITERAL_RVA: usize = 0x02b1_eb58;
/// ImGuiIO first INI filename constructor reference; engine_runtime_artifacts.json:30 records the literal-reference census.
pub const INI_FILENAME_REFERENCE_1_RVA: usize = 0x021b_541f;
/// ImGuiIO second INI filename constructor reference; engine_runtime_artifacts.json:30 records the literal-reference census.
pub const INI_FILENAME_REFERENCE_2_RVA: usize = 0x021b_5426;
/// ImGuiIO first log filename constructor reference; engine_runtime_artifacts.json:46 records the literal-reference census.
pub const LOG_FILENAME_REFERENCE_1_RVA: usize = 0x021b_542a;
/// ImGuiIO second log filename constructor reference; engine_runtime_artifacts.json:46 records the literal-reference census.
pub const LOG_FILENAME_REFERENCE_2_RVA: usize = 0x021b_5431;
