//! ImGui settings-output formatter metadata.
//! Output formatting is excluded from hourly capture.

/// Settings-text formatter; this location does not identify a file-open site.
pub const FORMAT_SETTINGS_RVA: usize = 0x021c_a520;
/// Settings section-heading format operation.
pub const SECTION_FORMAT_RVA: usize = 0x02b1_ecf8;
/// Settings window-position format operation.
pub const POSITION_FORMAT_RVA: usize = 0x02b1_ed08;
/// Settings collapsed-state format operation.
pub const COLLAPSED_FORMAT_RVA: usize = 0x02b1_ed28;
