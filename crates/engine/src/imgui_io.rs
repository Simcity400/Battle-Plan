//! ImGui output-sink fields; this module defines no replay domain.
//! Filename bindings do not identify the file-open operation.

/// ImGuiIO constructor.
pub const CONSTRUCTOR_RVA: usize = 0x021b_5390;
/// ImGuiIO IniFilename pointer.
pub const INI_FILENAME: usize = 0x18;
/// ImGuiIO LogFilename pointer.
pub const LOG_FILENAME: usize = 0x20;
/// ImGuiIO default INI filename literal.
pub const INI_FILENAME_LITERAL_RVA: usize = 0x02b1_eb48;
/// ImGuiIO default log filename literal.
pub const LOG_FILENAME_LITERAL_RVA: usize = 0x02b1_eb58;
/// ImGuiIO first INI filename constructor reference.
pub const INI_FILENAME_REFERENCE_1_RVA: usize = 0x021b_541f;
/// ImGuiIO second INI filename constructor reference.
pub const INI_FILENAME_REFERENCE_2_RVA: usize = 0x021b_5426;
/// ImGuiIO first log filename constructor reference.
pub const LOG_FILENAME_REFERENCE_1_RVA: usize = 0x021b_542a;
/// ImGuiIO second log filename constructor reference.
pub const LOG_FILENAME_REFERENCE_2_RVA: usize = 0x021b_5431;
