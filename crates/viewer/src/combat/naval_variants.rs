//! Equipment variant display names shared by active and retained-result hovers.
use super::binding::TextIcons;
use crate::{
    document::Document,
    table_access::{TokenNames, rows},
};
use installed_content::localisation::Localisation;
use replay::{
    frame::DecodedFrame,
    rows::{VariantDefinitionRow, tables},
};
use std::collections::BTreeMap;

pub(super) struct Names<'a> {
    document: &'a Document,
    tokens: Result<TokenNames<'a>, String>,
    countries: Result<BTreeMap<u32, &'a replay::rows::CountryRow>, String>,
    appearances: Result<BTreeMap<u32, &'a replay::rows::CountryAppearanceRow>, String>,
}
impl<'a> Names<'a> {
    pub(super) fn new(document: &'a Document, frame: &'a DecodedFrame) -> Self {
        Self {
            document,
            tokens: TokenNames::new(frame).map_err(|error| error.to_string()),
            countries: rows::<tables::Countries>(frame)
                .map(|rows| rows.rows().iter().map(|row| (row.id, row)).collect())
                .map_err(|error| error.to_string()),
            appearances: rows::<tables::CountryAppearance>(frame)
                .map(|rows| rows.rows().iter().map(|row| (row.country, row)).collect())
                .map_err(|error| error.to_string()),
        }
    }
}

/// Return a nonempty saved variant name verbatim. Otherwise localise the
/// effective definition's key for the origin (or creator) country, through
/// the stacked equipment and recorded lexicon. Append the suffix only when
/// its recorded flag permits it.
pub(super) fn name(
    row: &VariantDefinitionRow,
    icons: &dyn TextIcons,
    loc: &Localisation,
    context: Option<&Names<'_>>,
) -> Result<String, String> {
    let mut name = if !row.display_name.is_empty() {
        std::str::from_utf8(&row.display_name)
            .map_err(|_| format!("variant {} explicit name is not UTF-8", row.id))?
            .to_owned()
    } else {
        let context = context.ok_or_else(|| {
            format!(
                "variant {} has no context to localise its equipment definition",
                row.id
            )
        })?;
        let key = context
            .tokens
            .as_ref()
            .map_err(Clone::clone)?
            .name(row.equipment_token)
            .map_err(|error| error.to_string())?;
        let database = context
            .document
            .icons
            .combat
            .naval_air_groups
            .equipment()
            .ok_or("the stacked equipment database is unavailable")?;
        let index = database.get(key.as_bytes()).ok_or_else(|| {
            format!(
                "variant {} equipment key {key} is absent from stacked content",
                row.id
            )
        })?;
        let raw = std::str::from_utf8(&database.equipment[index].name)
            .map_err(|_| format!("variant {} equipment key is not UTF-8", row.id))?;
        let country = if row.origin_link > 0 {
            row.origin_link
        } else {
            row.creator_link
        };
        let tag =
            if country == 0 {
                // The null country has this fallback tag.
                "---"
            } else {
                let linked = context
                    .countries
                    .as_ref()
                    .map_err(Clone::clone)?
                    .get(&country)
                    .copied()
                    .ok_or_else(|| {
                        format!("variant {} country {country} has no Countries row", row.id)
                    })?;
                let appearance = context.appearances.as_ref().map_err(Clone::clone)?
                .get(&country).copied()
                .ok_or_else(|| {
                    format!(
                        "variant {} country {country} has no CountryAppearance original-tag row",
                        row.id
                    )
                })?;
                let tag = if appearance.name_tag.is_empty() {
                    &linked.tag
                } else {
                    &appearance.name_tag
                };
                std::str::from_utf8(tag)
                    .map_err(|_| format!("variant {} country {country} tag is not UTF-8", row.id))?
            };
        let localised = super::text::country_localised(loc, Ok(tag), raw)?;
        super::binding::displayable(icons, localised)?
    };
    if row.append_variant_name != 0 && !row.variant_name.is_empty() {
        name.push(' ');
        name.push_str(
            std::str::from_utf8(&row.variant_name)
                .map_err(|_| format!("variant {} suffix is not UTF-8", row.id))?,
        );
    }
    Ok(name)
}
