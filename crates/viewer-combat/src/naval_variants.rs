use dashboard_text::scripted_text::stored_text;
use content_reader_localisation::localisation::Localisation;
use replay_data::{
    rows::CountryAppearanceRow, rows::CountryRow, rows::VariantDefinitionRow, rows::tables,
};
use replay_frame::frame::DecodedFrame;
use std::collections::BTreeMap;
use dashboard_text::table_access::{Available, country_slot};
use viewer_foundation::tooltip_text::TextIcons;
use viewer_foundation::tooltip_text::country_localised;
use viewer_foundation::tooltip_text::displayable;
use super::content::Content as StaticContent;
use dashboard_text::table_access::{TokenNames, rows};

pub(super) struct Names<'a> {
    static_content: &'a StaticContent,
    tokens: Result<TokenNames<'a>, String>,
    tags: Tags<'a>,
}
impl<'a> Names<'a> {
    pub(super) fn new(static_content: &'a StaticContent, frame: &'a DecodedFrame) -> Self {
        Self {
            static_content,
            tokens: TokenNames::new(frame).map_err(|error| error.to_string()),
            tags: Tags::new(frame),
        }
    }
}

struct Tags<'a> {
    links: Result<Available<'a, tables::CountryLinks>, String>,
    countries: Result<BTreeMap<u32, &'a CountryRow>, String>,
    appearances: Result<BTreeMap<u32, &'a CountryAppearanceRow>, String>,
}
impl<'a> Tags<'a> {
    fn new(frame: &'a DecodedFrame) -> Self {
        Self {
            links: rows::<tables::CountryLinks>(frame).map_err(|error| error.to_string()),
            countries: rows::<tables::Countries>(frame)
                .map(|rows| {
                    rows.rows()
                        .iter()
                        .map(|row| (row.canonical_slot, row))
                        .collect()
                })
                .map_err(|error| error.to_string()),
            appearances: rows::<tables::CountryAppearance>(frame)
                .map(|rows| rows.rows().iter().map(|row| (row.country, row)).collect())
                .map_err(|error| error.to_string()),
        }
    }

    fn naming_tag(&self, row: &VariantDefinitionRow) -> Result<String, String> {
        let origin = (row.origin_link as i32) >= 1;
        let raw = if origin {
            row.origin_link
        } else {
            row.creator_link
        };
        let links = self.links.as_ref().map_err(Clone::clone)?;
        let Some(slot) =
            country_slot(links, raw).map_err(|reason| format!("variant {}: {reason}", row.id))?
        else {
            return Ok("---".into());
        };
        let country = self
            .countries
            .as_ref()
            .map_err(Clone::clone)?
            .get(&slot)
            .copied()
            .ok_or_else(|| {
                format!(
                    "variant {} country {raw} (slot {slot}) has no Countries row",
                    row.id
                )
            })?;
        let tag = if origin {
            &country.tag
        } else {
            let appearance = self
                .appearances
                .as_ref()
                .map_err(Clone::clone)?
                .get(&country.id)
                .copied()
                .ok_or_else(|| {
                    format!(
                        "variant {} country {raw} has no CountryAppearance original-tag row",
                        row.id
                    )
                })?;
            if appearance.name_tag.is_empty() {
                &country.tag
            } else {
                &appearance.name_tag
            }
        };
        std::str::from_utf8(tag)
            .map(str::to_owned)
            .map_err(|_| format!("variant {} country {raw} tag is not UTF-8", row.id))
    }
}

pub(super) fn name(
    row: &VariantDefinitionRow,
    icons: &dyn TextIcons,
    loc: &Localisation,
    context: Option<&Names<'_>>,
    recorded_language: Option<&str>,
) -> Result<String, String> {
    let stored = |bytes: &[u8], field: &str| {
        let what = format!("variant {} {field}", row.id);
        let language =
            recorded_language.ok_or_else(|| format!("{what} has no recording language"))?;
        stored_text(loc, language, bytes, &what)
    };
    let mut name = if !row.display_name.is_empty() {
        stored(&row.display_name, "explicit name")?
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
            .static_content
            .assets
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
        let tag = context.tags.naming_tag(row)?;
        let localised = country_localised(loc, Ok(tag.as_str()), raw)?;
        displayable(icons, localised)?
    };
    if row.append_variant_name != 0 && !row.variant_name.is_empty() {
        name.push(' ');
        name.push_str(&stored(&row.variant_name, "suffix")?);
    }
    Ok(name)
}

#[cfg(test)]
mod tests;
