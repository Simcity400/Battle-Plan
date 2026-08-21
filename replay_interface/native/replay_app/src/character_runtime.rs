
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
    // PARTIAL: this builds ONE country's character UI and needs only that
    // country's tag_db_id. The all-country D47 coverage claim is made separately
    // by `character_rosters_exact_status`, which `build_xui17_exact_rosters`
    // already derives from the COMPLETE roster - so the census question is
    // still asked, once, where it belongs.
    let d18 = frame.d18_country_roster();
    let partial = d18
        .partial()
        .map_err(|refusal| format!("XUI17 D18 identity roster is unusable: {refusal}"))?;
    if frame.character_rosters_exact_status() != ViewStatus::Present {
        return Err("XUI17 complete same-hour D47 roster is unavailable for the D18 census".into());
    }
    let owner_tag_db_id = partial
        .resolved
        .iter()
        .find(|entry| entry.tag == country_tag)
        .map(|entry| entry.tag_db_id)
        .ok_or_else(|| {
            let unnamed = if partial.excluded.is_empty() {
                String::new()
            } else {
                format!(
                    " ({} emitted country key(s) carry no D18 identity this hour: {})",
                    partial.excluded.len(),
                    partial.excluded.join(",")
                )
            };
            format!("XUI17 {country_tag}: selected country absent from D18{unnamed}")
        })?;
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
