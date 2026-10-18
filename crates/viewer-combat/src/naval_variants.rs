//! Equipment variant display names shared by active and retained-result hovers.
use installed_content_localisation::localisation::Localisation;
use replay_data::{rows::VariantDefinitionRow, rows::tables};
use replay_frame::frame::DecodedFrame;
use std::collections::BTreeMap;
use viewer_foundation::tooltip_text::TextIcons;
use viewer_foundation::tooltip_text::country_localised;
use viewer_foundation::tooltip_text::displayable;
use {
    super::content::Content as StaticContent,
    viewer_foundation::table_access::{TokenNames, rows},
};

pub(super) struct Names<'a> {
    static_content: &'a StaticContent,
    tokens: Result<TokenNames<'a>, String>,
    countries: Result<BTreeMap<u32, &'a replay_data::rows::CountryRow>, String>,
    appearances: Result<BTreeMap<u32, &'a replay_data::rows::CountryAppearanceRow>, String>,
}
impl<'a> Names<'a> {
    pub(super) fn new(static_content: &'a StaticContent, frame: &'a DecodedFrame) -> Self {
        Self {
            static_content,
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
        let localised = country_localised(loc, Ok(tag), raw)?;
        displayable(icons, localised)?
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
