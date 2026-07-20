//! XUI17 replay-product seam.
//!
//! The app consumes only replay_core's typed exact roster views and the
//! authenticated presentation producer input. Raw JSON never crosses this
//! boundary; an absent/invalid roster or incomplete presentation draws no
//! character UI.

use game_ui::character_ui::{
    admit_character_ui, apply_exact_a39_observations, AdmittedCharacterUi,
    CharacterPresentationInput,
};
use replay_core::WorldFrame;

pub fn admit_country_character_ui(
    frame: &WorldFrame,
    country_tag: &str,
    input: &CharacterPresentationInput,
) -> std::result::Result<AdmittedCharacterUi, String> {
    let roster = frame
        .character_roster_exact(country_tag)
        .ok_or_else(|| format!("XUI17 {country_tag}: exact character roster absent"))?;
    let aces = frame.ace_roster_exact(country_tag);
    admit_character_ui(roster, aces, input).map_err(|error| error.to_string())
}

/// Production A39 path: apply recorded exact trait state/order and
/// owner-qualified ace portraits before the existing d47/d46 bijection gate.
pub fn admit_country_character_ui_with_exact_a39(
    frame: &WorldFrame,
    country_tag: &str,
    owner_tag_db_id: u32,
    remaining_input: &CharacterPresentationInput,
) -> std::result::Result<AdmittedCharacterUi, String> {
    let exact_input = apply_exact_a39_observations(
        owner_tag_db_id,
        frame.character_presentation_exact(),
        remaining_input,
    )
    .map_err(|error| error.to_string())?;
    admit_country_character_ui(frame, country_tag, &exact_input)
}
