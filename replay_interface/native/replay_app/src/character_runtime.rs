//! XUI17 replay-product seam.
//!
//! The app consumes only replay_core's typed exact roster views and the
//! authenticated presentation producer input. Raw JSON never crosses this
//! boundary; an absent/invalid roster or incomplete presentation draws no
//! character UI.

use game_ui::character_ui::{
    admit_character_ui, apply_exact_a39_observations, apply_exact_static_leader_portraits,
    AdmittedCharacterUi, CharacterPortraitCatalog, CharacterPresentationInput,
};
use game_ui::gfx::SpriteRegistry;
use replay_core::{Leaf, WorldFrame};

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
/// owner-qualified ace portraits and final modifier text before the existing
/// d47/d46 bijection gate.
pub fn admit_country_character_ui_with_exact_a39(
    frame: &WorldFrame,
    country_tag: &str,
    owner_tag_db_id: u32,
    remaining_input: &CharacterPresentationInput,
) -> std::result::Result<AdmittedCharacterUi, String> {
    let mut exact_input = apply_exact_a39_observations(
        owner_tag_db_id,
        frame.character_presentation_exact(),
        remaining_input,
    )
    .map_err(|error| error.to_string())?;
    resolve_exact_a25_trait_reasons(frame, country_tag, &mut exact_input)?;
    admit_country_character_ui(frame, country_tag, &exact_input)
}

fn resolve_exact_a25_trait_reasons(
    frame: &WorldFrame,
    country_tag: &str,
    input: &mut CharacterPresentationInput,
) -> std::result::Result<(), String> {
    let has_reason_context = input.leaders.iter().any(|leader| {
        leader
            .trait_nodes
            .iter()
            .any(|node| node.reason_context_id.is_some())
    });
    if !has_reason_context {
        return Ok(());
    }
    let exact_hour = frame
        .character_presentation_exact()
        .presentation
        .as_ref()
        .ok_or_else(|| "XUI17 d82 Present view omitted its exact presentation".to_string())?
        .frame_hour;
    let exact_hour = i64::try_from(exact_hour)
        .map_err(|_| "XUI17 d82 frame hour exceeds A25 signed range".to_string())?;
    let runtime_text = frame
        .runtime_text()
        .map_err(|error| format!("XUI17 A25 runtime-text admission failed: {error}"))?;
    let Leaf::Measured(catalog) = runtime_text else {
        return Err("XUI17 trait reason contexts require measured A25 runtime text".into());
    };
    if !catalog.contexts_complete {
        return Err("XUI17 trait reason contexts require complete A25 runtime text".into());
    }
    for leader in &mut input.leaders {
        for node in &mut leader.trait_nodes {
            let Some(context_id) = node.reason_context_id.as_deref() else {
                node.reason.clear();
                continue;
            };
            let record = catalog.contexts.get(context_id).ok_or_else(|| {
                format!("XUI17 trait reason references missing A25 context {context_id}")
            })?;
            if record.context_id != context_id
                || record.frame_hour != exact_hour
                || (record.pov_country_or_global != country_tag
                    && record.pov_country_or_global != "global")
            {
                return Err(format!(
                    "XUI17 trait reason A25 identity mismatch for context {context_id}"
                ));
            }
            node.reason.clone_from(&record.final_text_utf8);
        }
    }
    Ok(())
}

/// Production path when effective installed content are available: resolve the exact
/// army/navy portrait role from d47 first, then apply d82's ordered trait and
/// ace observations, and finally run the existing roster bijection gate.
pub fn admit_country_character_ui_with_static_portraits_and_exact_a39(
    frame: &WorldFrame,
    country_tag: &str,
    owner_tag_db_id: u32,
    portraits: &CharacterPortraitCatalog,
    sprites: &SpriteRegistry,
    remaining_input: &CharacterPresentationInput,
) -> std::result::Result<AdmittedCharacterUi, String> {
    let roster = frame
        .character_roster_exact(country_tag)
        .ok_or_else(|| format!("XUI17 {country_tag}: exact character roster absent"))?;
    let with_static_portraits =
        apply_exact_static_leader_portraits(roster, portraits, sprites, remaining_input)
            .map_err(|error| error.to_string())?;
    admit_country_character_ui_with_exact_a39(
        frame,
        country_tag,
        owner_tag_db_id,
        &with_static_portraits,
    )
}
