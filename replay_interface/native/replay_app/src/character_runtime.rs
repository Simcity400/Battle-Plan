//! XUI17 replay-product seam.
//!
//! The app consumes typed exact views and effective-file catalogs only. Raw
//! JSON, recorder POV, and recorder panel lifecycle never cross this boundary.

use game_ui::character_ui::{
    admit_observer_character_ui, CharacterPortraitCatalog, CharacterViewerSelection,
    LeaderSkillCostCatalog, ObserverCharacterUi, UnitLeaderTraitCatalog,
};
use game_ui::gfx::SpriteRegistry;
use replay_core::{ViewStatus, WorldFrame};

pub fn admit_country_observer_character_ui(
    frame: &WorldFrame,
    country_tag: &str,
    portraits: &CharacterPortraitCatalog,
    traits: &UnitLeaderTraitCatalog,
    skill_costs: &LeaderSkillCostCatalog,
    sprites: &SpriteRegistry,
    selection: &CharacterViewerSelection,
) -> std::result::Result<ObserverCharacterUi, String> {
    let d18 = frame.d18_country_roster();
    if d18.status != ViewStatus::Present {
        return Err(d18
            .diagnostic
            .unwrap_or_else(|| "XUI17 complete D18 identity roster unavailable".into()));
    }
    if frame.character_rosters_exact_status() != ViewStatus::Present {
        return Err("XUI17 complete same-hour D47 roster is unavailable for the D18 census".into());
    }
    let owner_tag_db_id = d18
        .entries
        .iter()
        .find(|entry| entry.tag == country_tag)
        .map(|entry| entry.tag_db_id)
        .ok_or_else(|| format!("XUI17 {country_tag}: selected country absent from D18"))?;
    let roster = frame
        .character_roster_exact(country_tag)
        .ok_or_else(|| format!("XUI17 {country_tag}: exact character roster absent"))?;
    admit_observer_character_ui(
        country_tag,
        owner_tag_db_id,
        roster,
        frame.ace_roster_exact(country_tag),
        portraits,
        traits,
        Ok(skill_costs),
        frame.character_presentation_g8_exact(),
        sprites,
        selection,
    )
    .map_err(|error| error.to_string())
}
