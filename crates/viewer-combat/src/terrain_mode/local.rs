//! A land or lake province's local effects: its weather record's modifier accumulator
//! (`WeatherProvinces.modifiers`), every key the game summed there from the province's
//! mud, temperature range, ground snow and the modifiers script and building damage added,
//! described as the game's modifier list describes it, under `LOCAL_PROVINCE_MODIFIERS`.
//! An empty description adds nothing.
use std::collections::{BTreeMap, BTreeSet};

use installed_content_localisation::localisation::Localisation;
use replay_data::rows::tables;
use replay_frame::frame::DecodedFrame;
use viewer_foundation::tooltip_text::{TextIcons, builtin, tooltip_keys};

use super::super::{content::Content, modifier_text::Block};
use super::Definitions;

tooltip_keys! {
    LOCAL_PROVINCE_MODIFIERS = "LOCAL_PROVINCE_MODIFIERS",
}

/// Fixed5 one, the describer's scale.
const ONE: i64 = 100_000;

/// The section `province` appends this hour.
pub(super) fn effects(
    content: &Content,
    frame: &DecodedFrame,
    definitions: &Definitions<'_>,
    province: u32,
) -> Result<String, String> {
    section(
        content.assets.as_ref(),
        &content.localisation,
        accumulator(content, frame, definitions, province)?,
    )
}

/// The province's accumulator described alone, without the section heading.
pub(super) fn accumulator(
    content: &Content,
    frame: &DecodedFrame,
    definitions: &Definitions<'_>,
    province: u32,
) -> Result<String, String> {
    let rows = frame
        .complete::<tables::WeatherProvinces>()
        .map_err(|why| why.to_string())?;
    let at = rows
        .binary_search_by_key(&province, |row| row.id)
        .map_err(|_| format!("province {province} has no weather record"))?;
    let modifiers = rows[at]
        .modifiers
        .require("WeatherProvinces.modifiers")?
        .as_ref()
        .ok_or("the province's weather modifier accumulator was refused when recorded")?;
    plain(definitions, content.assets.as_ref(), &content.localisation, modifiers)
}

/// The section for one accumulator.
#[cfg(test)]
pub(super) fn describe(
    definitions: &Definitions<'_>,
    icons: &dyn TextIcons,
    loc: &Localisation,
    modifiers: &replay_data::rows::ProvinceWeatherModifiers,
) -> Result<String, String> {
    section(icons, loc, plain(definitions, icons, loc, modifiers)?)
}

fn section(icons: &dyn TextIcons, loc: &Localisation, text: String) -> Result<String, String> {
    if text.is_empty() {
        return Ok(text);
    }
    Ok(format!(
        "{}\n{text}",
        builtin(icons, loc, LOCAL_PROVINCE_MODIFIERS, &[])?
    ))
}

/// One accumulator's entries as the game's modifier list describes them.
fn plain(
    definitions: &Definitions<'_>,
    icons: &dyn TextIcons,
    loc: &Localisation,
    modifiers: &replay_data::rows::ProvinceWeatherModifiers,
) -> Result<String, String> {
    let block = Block {
        values: modifiers
            .entries
            .iter()
            .map(|entry| (entry.key, entry.value))
            .collect::<BTreeMap<_, _>>(),
        hidden: BTreeSet::new(),
        custom_tooltips: Vec::new(),
        added: false,
    };
    block
        .describe(definitions, icons, loc, ONE, "", u32::MAX)
        .map_err(|why| why.to_string())
}
