use super::*;

pub(super) struct Reference<'context, 'hour> {
    assets: &'context UnitModelAssets,
    hour: Option<&'context Hour<'hour>>,
}

impl<'context, 'hour> Reference<'context, 'hour> {
    pub(super) fn new(
        assets: &'context UnitModelAssets,
        hour: Option<&'context Hour<'hour>>,
    ) -> Self {
        Self { assets, hour }
    }
}

impl<'rows> selection::Names<'rows> for Reference<'_, '_> {
    fn name<'tag>(
        &mut self,
        tag: impl FnOnce() -> Result<&'tag str, String>,
        row: &'rows ShipModelRow,
        graphics: &[u8],
        definition: Option<(u32, u32)>,
    ) -> Result<Option<String>, String> {
        name(self.assets, tag, row, graphics, definition, self.hour)
    }
}

use installed_content_core::{parser::c_string, parser::text};

fn name<'t>(
    assets: &UnitModelAssets,
    tag: impl FnOnce() -> Result<&'t str, String>,
    row: &replay_data::rows::ShipModelRow,
    graphics: &[u8],
    definition: Option<(u32, u32)>,
    hour: Option<&Hour<'_>>,
) -> Result<Option<String>, String> {
    choose(
        row,
        graphics,
        tag,
        || {
            let hour = hour.ok_or("naval_entity: hourly graphics inputs are unavailable")?;
            let variant =
                hour.variants.get(row.variant).ok_or_else(|| {
                    format!("naval_entity: variant {} is unavailable", row.variant)
                })?;
            // Use the variant creator directly; the division adapter instead
            // chooses a model country. The explicit override never reaches these operands.
            let country = creator_country(variant.creator_link)?;
            let (owner, token) = definition
                .ok_or("naval_entity: ship's roster owner or Ships.class_id is unavailable")?;
            let subunit = assets
                .sub_units
                .subunit_of_token(hour.tokens, token)
                .map_err(|error| format!("naval_entity: {error}"))?;
            if (owner as i32) <= 0 {
                return Err(format!(
                    "naval_entity: generated variant model has unsupported nonpositive definition owner {}",
                    owner as i32
                ));
            }
            let owner = hour
                .country
                .links
                .get(&owner)
                .copied()
                .ok_or("naval_entity: ship's definition owner has no CountryLinks row")?;
            let identity = Definition::Local {
                country: owner,
                token,
            };
            // The game rebuilds this pool from the roster owner's deployment
            // definition. Its first entry can differ from the global need.
            let requirements = assets
                .sub_units
                .requirements(identity, subunit, &hour.sources)
                .map_err(|error| format!("naval_entity: {error}"))?;
            let first = requirements.first().map(|(key, _)| key.as_str());
            let mut effective = subunit.clone();
            effective.category = row.formation_flags;
            // The game returns before reading the sprite when count is zero.
            if first.is_some() {
                effective.sprite =
                    identifier(&row.fallback_sprite, "embedded sub-unit sprite")?.to_owned();
            }
            let pair = SelectedPair {
                variant,
                subunit: &effective,
                identity,
            };
            graphics_model_for_country(assets, hour, pair, first, country)
                .map(|model| model.map(|(_, name)| name))
        },
        |name| {
            if let Some(reason) = assets.roster.unreadable_names.get(name) {
                return Err(format!(
                    "naval_entity: selected {name} is unreadable: {reason}"
                ));
            }
            Ok(assets.registered(name))
        },
    )
}

fn creator_country(raw: u32) -> Result<u32, String> {
    match raw as i32 {
        1.. => Ok(raw),
        0 => Err("naval_entity: generated variant model needs unsupported null-country ideology/culture operands".into()),
        _ => Err(format!("naval_entity: generated variant model has unsafe signed creator country {}", raw as i32)),
    }
}

// Explicit winners leave generation and fallback country unread.
fn choose<'t>(
    row: &replay_data::rows::ShipModelRow,
    graphics: &[u8],
    tag: impl FnOnce() -> Result<&'t str, String>,
    generated: impl FnOnce() -> Result<Option<String>, String>,
    mut registered: impl FnMut(&str) -> Result<bool, String>,
) -> Result<Option<String>, String> {
    let counter = row
        .counter
        .as_ref()
        .ok_or("naval_entity: ShipModels definition-valid byte is unavailable")?;
    if counter.definition_valid == 0 {
        return Ok(registered(PLACEHOLDER_ENTITY)?.then(|| PLACEHOLDER_ENTITY.to_owned()));
    }
    // A stored NUL remains a nonempty explicit override before C-string lookup.
    if row.variant != 0 {
        let primary = if graphics.is_empty() {
            generated()?
        } else {
            Some(identifier(graphics, "explicit variant entity")?.to_owned())
        };
        if let Some(name) = primary
            && registered(&name)?
        {
            return Ok(Some(name));
        }
    }
    // The fallback country is read before retrying a missed explicit entity.
    let tag = tag()?;
    if row.variant != 0 && !graphics.is_empty() {
        let name = identifier(graphics, "explicit variant entity")?;
        if registered(name)? {
            return Ok(Some(name.to_owned()));
        }
    }
    if row.variant != 0 && !row.hull_sprite.is_empty() {
        let sprite = identifier(&row.hull_sprite, "variant hull sprite")?;
        for name in ship_candidates(tag, sprite, row.model_number) {
            if registered(&name)? {
                return Ok(Some(name));
            }
        }
    }
    // Try the ship's own sprite even when it is empty.
    let sprite = identifier(&row.fallback_sprite, "fallback hull sprite")?;
    for name in ship_candidates(tag, sprite, row.model_number) {
        if registered(&name)? {
            return Ok(Some(name));
        }
    }
    Ok(registered(PLACEHOLDER_ENTITY)?.then(|| PLACEHOLDER_ENTITY.to_owned()))
}

fn identifier<'a>(bytes: &'a [u8], field: &str) -> Result<&'a str, String> {
    text(c_string(bytes)).map_err(|error| format!("naval_entity: {field}: {error}"))
}
