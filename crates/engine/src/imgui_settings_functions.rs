//! ImGui settings formatter; function ownership is not established as an ImGuiIO method.
//! Retained output-sink knowledge only, excluded from hourly capture.

/// ImGui settings formatter; extractor/game_defs/engine_runtime_artifacts.json:37 confirms emitted settings text, not a file-open site.
pub const FORMAT_SETTINGS_RVA: usize = 0x021c_a520;
/// ImGui settings section format `"[%s][%s]"`; engine_runtime_artifacts.json:37 records its formatter reference.
pub const SECTION_FORMAT_RVA: usize = 0x02b1_ecf8;
/// ImGui settings position format "Pos=%d,%d"; engine_runtime_artifacts.json:37 records its formatter reference.
pub const POSITION_FORMAT_RVA: usize = 0x02b1_ed08;
/// ImGui settings collapsed format "Collapsed=%d"; engine_runtime_artifacts.json:37 records its formatter reference.
pub const COLLAPSED_FORMAT_RVA: usize = 0x02b1_ed28;
