//! A20/X9 replay-product seam for exact special-project counters and
//! breakthrough specialization identity.
//!
//! Raw D41 JSON never crosses this boundary. The app consumes replay_core's
//! owner-joined typed view and game_ui repeats the authority, completeness,
//! identity, and exact-bound checks before exposing a selected-country model.

use game_ui::special_projects::{
    admit_special_project_breakthroughs,
    admit_special_project_progress,
    SpecialProjectBreakthroughPresentation, SpecialProjectProgressPresentation,
};
use replay_core::WorldFrame;

pub fn admit_country_special_project_progress(
    frame: &WorldFrame,
    country_tag: &str,
) -> std::result::Result<Vec<SpecialProjectProgressPresentation>, String> {
    admit_special_project_progress(frame.special_project_progress_exact(), Some(country_tag))
        .map_err(|error| error.0)
}

pub fn admit_global_special_project_progress(
    frame: &WorldFrame,
) -> std::result::Result<Vec<SpecialProjectProgressPresentation>, String> {
    admit_special_project_progress(frame.special_project_progress_exact(), None)
        .map_err(|error| error.0)
}

pub fn admit_global_special_project_breakthroughs(
    frame: &WorldFrame,
) -> std::result::Result<Vec<SpecialProjectBreakthroughPresentation>, String> {
    admit_special_project_breakthroughs(frame.special_project_breakthrough_exact(), None)
        .map_err(|error| error.0)
}
