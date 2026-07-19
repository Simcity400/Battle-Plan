//! XUI17 replay-product seam.
//!
//! The app consumes only replay_core's typed exact roster views and the
//! authenticated presentation producer input. Raw JSON never crosses this
//! boundary; an absent/invalid roster or incomplete presentation draws no
//! character UI.

use game_ui::character_ui::{admit_character_ui, AdmittedCharacterUi, CharacterPresentationInput};
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
